//! T3 WoW companion: brings T3 Code into World of Warcraft: Forever.
//!
//! Double-clicked (on a Mac, started without a command), it lives in the system tray or the
//! menu bar: finds WoW, keeps the addon files in place,
//! signs in to T3 when needed, and carries commands and updates between the two.
//! With a command (`t3wow.exe status`, `install`, ...) it works as a console tool.

// Release builds have no console window; commands attach to the one they were run from.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

// Off Windows the key watcher and clipboard paste aren't built (and off Windows and macOS
// the tray), which leaves what only they use unused.
#[cfg_attr(not(windows), allow(dead_code))]
mod app;
mod bridge;
mod capture;
mod cites;
#[cfg_attr(not(windows), allow(dead_code, unused_imports))]
mod clip;
mod codec;
#[cfg_attr(not(windows), allow(dead_code))]
mod config;
#[cfg(windows)]
mod dialog;
#[cfg_attr(not(windows), allow(dead_code))]
mod keys;
mod log;
mod lua;
mod made_for;
mod render;
mod slots;
mod snooze;
mod t3;
#[cfg(target_os = "macos")]
mod token_key;
#[cfg(any(windows, target_os = "macos"))]
mod tray;
mod updater;
#[cfg(windows)]
mod winget;
mod wow;

use config::Config;
use slots::Slots;
use std::path::PathBuf;

/// This build's version: "0.9.1" from the release workflow, "0.9.1-dev+57ee2ec" from any
/// other build (see build.rs). Shown wherever the companion or the addon says its version.
pub const VERSION: &str = env!("T3WOW_VERSION");

pub fn message_box(title: &str, text: &str) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        MessageBoxW(std::ptr::null_mut(), wide(text).as_ptr(), wide(title).as_ptr(), MB_OK | MB_ICONINFORMATION);
    }
    #[cfg(not(windows))]
    eprintln!("{title}: {text}");
    // The menu bar app has no terminal anyone reads: an alert too.
    #[cfg(target_os = "macos")]
    if app::menu_bar() {
        tray::message(title, text);
    }
}

/// One companion at a time: two would both write the slot files.
mod single_instance {
    #[cfg(windows)]
    static mut HANDLE: windows_sys::Win32::Foundation::HANDLE = std::ptr::null_mut();

    /// Kept open for as long as the companion runs; the system lets go of the lock when the
    /// process ends, however it ends.
    #[cfg(not(windows))]
    static LOCK: std::sync::Mutex<Option<std::fs::File>> = std::sync::Mutex::new(None);

    /// The lock on `path`, or None when another companion holds it (or it can't be taken:
    /// like Windows, better not to start than to risk two writing the slots).
    #[cfg(not(windows))]
    pub(super) fn take_lock(path: &std::path::Path) -> Option<std::fs::File> {
        use std::fs::{OpenOptions, TryLockError};
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let file = match OpenOptions::new().create(true).truncate(false).write(true).open(path) {
            Ok(file) => file,
            Err(error) => {
                crate::log::line(&format!("Couldn't open {}: {error}", path.display()));
                return None;
            }
        };
        match file.try_lock() {
            Ok(()) => Some(file),
            Err(TryLockError::WouldBlock) => None,
            Err(TryLockError::Error(error)) => {
                crate::log::line(&format!("Couldn't lock {}: {error}", path.display()));
                None
            }
        }
    }

    pub fn acquire() -> bool {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
            use windows_sys::Win32::System::Threading::CreateMutexW;
            let name: Vec<u16> = "Local\\T3WoWCompanion".encode_utf16().chain(Some(0)).collect();
            HANDLE = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            return !HANDLE.is_null() && GetLastError() != ERROR_ALREADY_EXISTS;
        }
        #[cfg(not(windows))]
        if let Some(file) = take_lock(&crate::config::config_dir().join("companion.lock")) {
            *LOCK.lock().unwrap() = Some(file);
        } else {
            return false;
        }
        #[allow(unreachable_code)]
        true
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn release() {
        #[cfg(not(windows))]
        {
            *LOCK.lock().unwrap() = None;
        }
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

/// The command, as the messages below name it.
const BIN: &str = if cfg!(windows) { "t3wow.exe" } else { "t3wow" };

#[cfg(not(windows))]
const HELP: &str = "t3wow: T3 Code inside World of Warcraft: Forever

  t3wow                start the companion in the menu bar
  t3wow run            run in this terminal instead, logging here
  t3wow install        [--wow <_forever_ folder>] [--interface 16001]   reinstall the addon from scratch
  t3wow login          [--origin http://<host>:<port> | local]          sign in to T3 again (local: the T3 on this Mac)
  t3wow status         [--origin http://<host>:<port> | local]          check the addon install and the T3 connection
  t3wow uninstall      remove the addon
  t3wow update         install the newest release, if there is one (in the T3 WoW app; elsewhere, say whether one is out)";

#[cfg(windows)]
const HELP: &str = "t3wow: T3 Code inside World of Warcraft: Forever

  t3wow.exe            start in the system tray (what double-clicking does)
  t3wow.exe run        run in this console instead, logging here
  t3wow.exe install    [--wow <_forever_ folder>] [--interface 16001]   reinstall the addon from scratch
  t3wow.exe login      [--origin http://<host>:<port> | local]          sign in to T3 again (local: the T3 on this PC)
  t3wow.exe status     [--origin http://<host>:<port> | local]          check the addon install and the T3 connection
  t3wow.exe uninstall  remove the addon
  t3wow.exe update     install the newest release, if there is one";

#[cfg(not(target_os = "macos"))]
const NO_WOW: &str = "Could not find WoW: Forever. Pass --wow \"C:\\...\\World of Warcraft\\_forever_\".";
#[cfg(target_os = "macos")]
const NO_WOW: &str = "Could not find WoW: Forever. Pass --wow \"/Applications/World of Warcraft/_forever_\".";

// On a Mac the other one may be in the menu bar, or `t3wow run` in a Terminal window.
#[cfg(not(target_os = "macos"))]
const ALREADY_RUNNING: &str = "T3 WoW is already running. Look for its icon in the system tray (next to the clock).";
#[cfg(target_os = "macos")]
const ALREADY_RUNNING: &str = "T3 WoW is already running. Look for its T3 icon in the menu bar, or a t3wow in a Terminal window.";
#[cfg(not(target_os = "macos"))]
const ALREADY_RUNNING_HERE: &str = "T3 WoW is already running (see the system tray).";
#[cfg(target_os = "macos")]
const ALREADY_RUNNING_HERE: &str = "T3 WoW is already running (its T3 icon in the menu bar, or another t3wow in a Terminal window).";
#[cfg(not(target_os = "macos"))]
const NO_LOCAL_T3: &str = "T3: not running on this PC";
#[cfg(target_os = "macos")]
const NO_LOCAL_T3: &str = "T3: not running on this Mac";
#[cfg(not(target_os = "macos"))]
const UPDATED: &str = "Updated. Restart T3 WoW (Quit from its tray menu, then start it again), then restart WoW if it is running: /reload does not load new addon files.";
#[cfg(target_os = "macos")]
const UPDATED: &str = "Updated. Restart T3 WoW (Quit T3 WoW from its menu bar icon, then open it again), then restart WoW if it is running: /reload does not load new addon files.";

fn wow_dir(args: &Args, config: &Config) -> Result<PathBuf, String> {
    let dir = match &args.wow {
        Some(given) => wow::normalize(given),
        None => config.wow_dir.as_ref().map(PathBuf::from).filter(|d| wow::looks_like_client(d)).or_else(wow::find_wow_dir),
    };
    let dir = dir.ok_or(NO_WOW)?;
    if config.wow_dir.as_deref() != Some(&dir.to_string_lossy()) {
        Config::update(|c| c.wow_dir = Some(dir.to_string_lossy().into_owned()));
    }
    Ok(dir)
}

/// The T3 `status` checks: `--origin` when given, else the saved one (`login` switches
/// through `t3::connect`). Its token is the one saved for that server, if any.
fn target(origin: &Option<Option<String>>) -> Option<t3::Target> {
    let mut config = Config::load();
    if let Some(origin) = origin {
        config.origin = origin.clone();
    }
    t3::target_in(&config)
}

/// What the terminal says when this T3 runs an older T3 Code than this release is made for
/// (nothing when it doesn't, or its version can't be read).
fn older_t3_note(client: &t3::McpClient) -> Option<String> {
    use t3::T3;
    let read = client.call("t3_environment_read", serde_json::json!({})).ok()?;
    let older = made_for::older_t3(read["serverVersion"].as_str().unwrap_or_default())?;
    Some(format!("{}: {}", older.title(), older.note()))
}

fn command(args: Args) -> Result<(), String> {
    let config = Config::load();
    let command = args.command.as_deref().unwrap_or("help");
    // Some(None) is the T3 on this PC.
    let origin = match args.origin.as_deref() {
        Some("local") => Some(None),
        Some(origin) if origin.starts_with("http://") || origin.starts_with("https://") => Some(Some(config::canonical_origin(origin))),
        Some(origin) => return Err(format!("--origin takes http://<host>:<port> or local, not {origin}")),
        None => None,
    };
    if origin.is_some() && !matches!(command, "login" | "status") {
        return Err("--origin goes with login (to switch T3) or status (to check one).".into());
    }
    if let Some(typed) = args.interface.as_deref().filter(|i| !wow::is_interface(i)) {
        return Err(format!("--interface takes WoW's interface version, such as 16001 for 1.60.1, not {typed}. In the game, /dump select(4, GetBuildInfo()) prints it."));
    }
    let interface = args.interface.clone().unwrap_or_else(|| config.interface());
    match command {
        "install" => {
            let dir = wow_dir(&args, &config)?;
            // Its own --interface wins; otherwise what the client's app says it is, since
            // on a fresh Mac install no Config.wtf has saved it yet.
            #[cfg(target_os = "macos")]
            let interface = if args.interface.is_some() { interface } else { wow::interface_version(&dir).map_or(interface, |found| found.interface) };
            let slots = Slots::for_wow(&dir, &interface).with_key(config.game_key.clone());
            slots.install().map_err(|e| e.to_string())?;
            Config::update(|c| c.interface = Some(interface.clone()));
            println!("Installed the T3WoW addon and its reply slots into {}.", slots.addons_dir.display());
            println!("Restart WoW (a full restart, not /reload) so the game sees the new files.");
        }
        "uninstall" => {
            // First: a winget install has its shortcut even when WoW was never found.
            #[cfg(windows)]
            if let Some(lnk) = winget::remove_shortcut() {
                println!("Removed the Start menu shortcut ({}).", lnk.display());
            }
            let slots = Slots::for_wow(&wow_dir(&args, &config)?, &interface);
            slots.uninstall().map_err(|e| e.to_string())?;
            println!("Removed the T3WoW addon and its slots. Your sign-in stays in {}", Config::file().display());
        }
        "login" => {
            // The same path as the tray's, always through the browser: the new T3 and its
            // token are saved together, once the sign-in succeeds, a running companion moves
            // to both, and the game drops what the previous T3 showed.
            let connected = t3::connect(origin, false, &|m| println!("{m}"), &|| {}, &|| false)
                .map_err(|e| if e.starts_with("T3 Code isn't running") { "T3 Code is not running here. Start the T3 Code desktop app, or pass --origin.".into() } else { e })?;
            println!("Signed in to {} ({}).", connected.origin, connected.scope.unwrap_or_default());
            if connected.read_only {
                println!("Warning: {}", t3::READ_ONLY_NOTE.replace("Sign in again", &format!("Run {BIN} login again")));
            }
            if let Some(note) = t3::target().and_then(|t| t.token.map(|token| t3::McpClient::new(&t.origin, &token))).as_ref().and_then(older_t3_note) {
                println!("{note}");
            }
        }
        "status" => {
            let dir = wow_dir(&args, &config)?;
            let slots = Slots::for_wow(&dir, &interface);
            match slots.installed_version() {
                Some(version) if slots.installed() => println!("WoW: {} (addon installed, {version}; companion ships {})", dir.display(), slots::shipped_version()),
                _ => println!("WoW: {} (addon NOT installed: run {BIN} install)", dir.display()),
            }
            #[cfg(any(windows, target_os = "macos"))]
            println!("WoW client: {}", if capture::find_wow(&dir).is_some() { "running" } else { "not running" });
            let t3::Target { origin, token, key } = target(&origin).ok_or(NO_LOCAL_T3)?;
            // The key file is there but unusable: "not signed in" would be wrong, and
            // `login` couldn't save a sign-in either.
            #[cfg(target_os = "macos")]
            if let Some(problem) = token_key::problem().filter(|_| token.is_none()) {
                return Err(format!("T3 ({origin}): {problem}"));
            }
            let token = token.ok_or(format!("T3 ({origin}): not signed in: run {BIN} login"))?;
            let client = t3::McpClient::new(&origin, &token);
            use t3::T3;
            let links = client.call("t3_environment_links", serde_json::json!({})).map_err(|e| format!("T3 ({origin}): {e}"))?;
            if Config::load().sign_in_read_only(&key) {
                println!("T3: {origin}, signed in, read only");
                println!("  {}", t3::READ_ONLY_NOTE.replace("Sign in again", &format!("Run {BIN} login again")));
            } else {
                println!("T3: {origin}, signed in");
            }
            let machines = 1 + links["links"].as_array().map(Vec::len).unwrap_or(0);
            println!("  {machines} machine(s) reachable through T3");
            if let Some(note) = older_t3_note(&client) {
                println!("  {note}");
            }
        }
        "update" => match updater::check()? {
            None => println!("Up to date ({}), or no release is visible yet.", updater::current_version()),
            // Where it can't be installed (a bare t3wow on a Mac): say it's out, not "Updating…".
            Some(release) if !updater::can_install() => println!("{} is out (this is {}). {}", release.version, updater::current_version(), updater::NOT_INSTALLABLE),
            Some(release) => {
                println!("Updating {} to {}…", updater::current_version(), release.version);
                updater::install(&release, &|step| match step {
                    updater::Step::Checking => println!("Downloaded; checking it against the release…"),
                    updater::Step::Installing => println!("Installing…"),
                    updater::Step::Downloading(_) => {}
                })?;
                println!("{UPDATED}");
            }
        },
        "help" => println!("{HELP}"),
        other => return Err(format!("Unknown command '{other}'.\n\n{HELP}")),
    }
    Ok(())
}

fn main() {
    let args = parse_args();
    // The addon's signing key, and a first start's "Commands from the game", before
    // anything reads the settings.
    Config::prepare();
    match args.command.as_deref() {
        // Double-clicked: the tray app (on a Mac, the menu bar app).
        None => {
            #[cfg(target_os = "macos")]
            app::set_menu_bar();
            log::init(false);
            if !single_instance::acquire() {
                message_box("T3 WoW", ALREADY_RUNNING);
                return;
            }
            log::line(&format!("T3 WoW companion {} starting", crate::VERSION));
            updater::clean_up();
            #[cfg(windows)]
            winget::on_start();
            let app = app::App::new();
            let worker = app.clone();
            std::thread::Builder::new().name("companion".into()).spawn(move || app::run(worker)).expect("worker thread");
            #[cfg(any(windows, target_os = "macos"))]
            tray::run(app);
            #[cfg(not(any(windows, target_os = "macos")))]
            loop {
                std::thread::park();
            }
        }
        // In this console, logging here.
        Some("run") => {
            attach_console();
            log::init(true);
            if !single_instance::acquire() {
                eprintln!("{ALREADY_RUNNING_HERE}");
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

#[cfg(all(test, not(windows)))]
mod tests {
    use super::single_instance::take_lock;

    #[test]
    fn a_second_companion_is_turned_away_until_the_first_lets_go() {
        let dir = std::env::temp_dir().join(format!("t3wow-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("made-for-it").join("companion.lock");
        let first = take_lock(&path).expect("the first one starts");
        assert!(take_lock(&path).is_none(), "the second one doesn't");
        drop(first);
        assert!(take_lock(&path).is_some(), "the next one starts once the first has gone");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
