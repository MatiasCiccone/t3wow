//! T3 WoW companion: brings T3 Code into World of Warcraft: Forever.
//!
//! Double-clicked, it lives in the system tray: finds WoW, keeps the addon files in place,
//! signs in to T3 when needed, and carries commands and updates between the two.
//! With a command (`t3wow.exe status`, `install`, ...) it works as a console tool.

// Release builds have no console window; commands attach to the one they were run from.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod bridge;
mod capture;
mod codec;
mod config;
mod log;
mod lua;
mod slots;
mod t3;
#[cfg(windows)]
mod tray;
mod updater;
mod wow;

use config::Config;
use slots::Slots;
use std::path::PathBuf;

pub fn message_box(title: &str, text: &str) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        MessageBoxW(std::ptr::null_mut(), wide(text).as_ptr(), wide(title).as_ptr(), MB_OK | MB_ICONINFORMATION);
    }
    #[cfg(not(windows))]
    eprintln!("{title}: {text}");
}

/// One companion at a time: two would both write the slot files.
mod single_instance {
    #[cfg(windows)]
    static mut HANDLE: windows_sys::Win32::Foundation::HANDLE = std::ptr::null_mut();

    pub fn acquire() -> bool {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
            use windows_sys::Win32::System::Threading::CreateMutexW;
            let name: Vec<u16> = "Local\\T3WoWCompanion".encode_utf16().chain(Some(0)).collect();
            HANDLE = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            return !HANDLE.is_null() && GetLastError() != ERROR_ALREADY_EXISTS;
        }
        #[allow(unreachable_code)]
        true
    }

    pub fn release() {
        #[cfg(windows)]
        unsafe {
            if !HANDLE.is_null() {
                windows_sys::Win32::Foundation::CloseHandle(HANDLE);
                HANDLE = std::ptr::null_mut();
            }
        }
    }
}

fn attach_console() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

struct Args {
    command: Option<String>,
    wow: Option<String>,
    interface: Option<String>,
    origin: Option<String>,
}

fn parse_args() -> Args {
    let mut args = Args { command: None, wow: None, interface: None, origin: None };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--wow" => args.wow = it.next(),
            "--interface" => args.interface = it.next(),
            "--origin" => args.origin = it.next(),
            "-h" | "--help" => args.command = Some("help".into()),
            other if args.command.is_none() => args.command = Some(other.to_string()),
            _ => {}
        }
    }
    args
}

const HELP: &str = "t3wow: T3 Code inside World of Warcraft: Forever

  t3wow.exe            start in the system tray (what double-clicking does)
  t3wow.exe run        run in this console instead, logging here
  t3wow.exe install    [--wow <_forever_ folder>] [--interface 16001]   reinstall the addon from scratch
  t3wow.exe login      [--origin http://<host>:<port> | local]          sign in to T3 again (local: the T3 on this PC)
  t3wow.exe status     check the addon install and the T3 connection
  t3wow.exe uninstall  remove the addon
  t3wow.exe update     install the newest release, if there is one";

fn wow_dir(args: &Args, config: &Config) -> Result<PathBuf, String> {
    let dir = match &args.wow {
        Some(given) => wow::normalize(given),
        None => config.wow_dir.as_ref().map(PathBuf::from).filter(|d| wow::looks_like_client(d)).or_else(wow::find_wow_dir),
    };
    let dir = dir.ok_or("Could not find WoW: Forever. Pass --wow \"C:\\...\\World of Warcraft\\_forever_\".")?;
    if config.wow_dir.as_deref() != Some(&dir.to_string_lossy()) {
        Config::update(|c| c.wow_dir = Some(dir.to_string_lossy().into_owned()));
    }
    Ok(dir)
}

fn command(args: Args) -> Result<(), String> {
    let config = Config::load();
    match args.origin.as_deref() {
        // Back to finding the T3 on this PC.
        Some("local") => {
            Config::update(|c| c.origin = None);
            if t3::discover_origin().is_none() {
                println!("Back to the T3 on this PC, but it isn't running: a running companion keeps its current T3 until it starts.");
            }
        }
        Some(origin) if origin.starts_with("http://") || origin.starts_with("https://") => {
            Config::update(|c| c.origin = Some(origin.trim_end_matches('/').to_string()));
        }
        Some(origin) => return Err(format!("--origin takes http://<host>:<port> or local, not {origin}")),
        None => {}
    }
    let interface = args.interface.clone().unwrap_or_else(|| config.interface());
    match args.command.as_deref().unwrap_or("help") {
        "install" => {
            let dir = wow_dir(&args, &config)?;
            let slots = Slots::for_wow(&dir, &interface);
            slots.install().map_err(|e| e.to_string())?;
            Config::update(|c| c.interface = Some(interface.clone()));
            println!("Installed the T3WoW addon and its reply slots into {}.", slots.addons_dir.display());
            println!("Restart WoW (a full restart, not /reload) so the game sees the new files.");
        }
        "uninstall" => {
            let slots = Slots::for_wow(&wow_dir(&args, &config)?, &interface);
            slots.uninstall().map_err(|e| e.to_string())?;
            println!("Removed the T3WoW addon and its slots. Your sign-in stays in {}", Config::file().display());
        }
        "login" => {
            let origin = t3::origin().ok_or("T3 Code is not running here. Start the T3 Code desktop app, or pass --origin.")?;
            let signed = t3::login(&origin, &|m| println!("{m}"))?;
            Config::update(|c| {
                c.token = Some(signed.token.clone());
                c.token_expires_at = Some(signed.expires_at);
            });
            println!("Signed in to {origin} ({}).", signed.scope);
        }
        "status" => {
            let dir = wow_dir(&args, &config)?;
            let slots = Slots::for_wow(&dir, &interface);
            match slots.installed_version() {
                Some(version) if slots.installed() => println!("WoW: {} (addon installed, {version}; companion ships {})", dir.display(), slots::shipped_version()),
                _ => println!("WoW: {} (addon NOT installed: run t3wow.exe install)", dir.display()),
            }
            #[cfg(windows)]
            println!("WoW client: {}", if capture::find_wow(&dir).is_some() { "running" } else { "not running" });
            let origin = t3::origin().ok_or("T3: not running on this PC")?;
            let token = Config::load().token().ok_or("T3: not signed in: run t3wow.exe login")?;
            let client = t3::McpClient::new(&origin, &token);
            use t3::T3;
            let links = client.call("t3_environment_links", serde_json::json!({})).map_err(|e| format!("T3 ({origin}): {e}"))?;
            println!("T3: {origin}, signed in");
            let machines = 1 + links["links"].as_array().map(Vec::len).unwrap_or(0);
            println!("  {machines} machine(s) reachable through T3");
        }
        "update" => match updater::check()? {
            None => println!("Up to date ({}), or no release is visible yet.", updater::current_version()),
            Some(release) => {
                println!("Updating {} to {}…", updater::current_version(), release.version);
                updater::install(&release)?;
                println!("Updated. Restart T3 WoW (Quit from its tray menu, then start it again).");
            }
        },
        "help" => println!("{HELP}"),
        other => return Err(format!("Unknown command '{other}'.\n\n{HELP}")),
    }
    Ok(())
}

fn main() {
    let args = parse_args();
    match args.command.as_deref() {
        // Double-clicked: the tray app.
        None => {
            log::init(false);
            if !single_instance::acquire() {
                message_box("T3 WoW", "T3 WoW is already running. Look for its icon in the system tray (next to the clock).");
                return;
            }
            log::line(&format!("T3 WoW companion {} starting", env!("CARGO_PKG_VERSION")));
            updater::clean_up();
            let app = app::App::new();
            let worker = app.clone();
            std::thread::Builder::new().name("companion".into()).spawn(move || app::run(worker)).expect("worker thread");
            #[cfg(windows)]
            tray::run(app);
            #[cfg(not(windows))]
            loop {
                std::thread::park();
            }
        }
        // In this console, logging here.
        Some("run") => {
            attach_console();
            log::init(true);
            if !single_instance::acquire() {
                eprintln!("T3 WoW is already running (see the system tray).");
                std::process::exit(1);
            }
            app::run(app::App::new());
        }
        Some(_) => {
            attach_console();
            if let Err(error) = command(args) {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    }
}
