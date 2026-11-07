//! The system tray icon and its menu. Runs the Windows message loop on the main thread;
//! the work happens in app::run on a background thread.

use crate::app::{App, Control, Status, Update};
use crate::config::{Config, Trust};
use crate::log;
use crate::slots::Slots;
use std::ptr::null_mut;
use std::sync::Arc;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, PostQuitMessage, SetTimer, TranslateMessage, MSG, WM_TIMER};

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "T3 WoW";

fn exe_command() -> String {
    format!("\"{}\"", std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default())
}

fn autostart_enabled() -> bool {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .and_then(|k| k.get_value::<String, _>(RUN_VALUE))
        .is_ok_and(|v| v.eq_ignore_ascii_case(&exe_command()))
}

fn set_autostart(on: bool) {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let Ok((key, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY) else { return };
    let result = if on { key.set_value(RUN_VALUE, &exe_command()) } else { key.delete_value(RUN_VALUE) };
    if let Err(error) = result {
        log::line(&format!("Start with Windows: {error}"));
    }
}

fn open_path(path: &std::path::Path) {
    let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
}

/// Starts a fresh copy of the companion (after the single-instance lock is released) and quits.
fn relaunch() -> ! {
    crate::single_instance::release();
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).spawn();
    }
    std::process::exit(0)
}

/// A Yes/No question; true for Yes.
fn confirm(title: &str, text: &str) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONQUESTION, MB_YESNO};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    unsafe { MessageBoxW(null_mut(), wide(text).as_ptr(), wide(title).as_ptr(), MB_YESNO | MB_ICONQUESTION) == IDYES }
}

/// Reads WoW's interface version and, when the addon declares another, rewrites the
/// `## Interface:` lines (nothing else) and restarts the companion so it uses the new number
/// from then on. Slot addons show as out of date when WoW's patch raised the number.
fn match_interface(app: &App) {
    let Some(dir) = app.wow_dir.lock().unwrap().clone() else {
        crate::message_box("T3 WoW", "T3 WoW hasn't found the WoW folder yet: choose it first.");
        return;
    };
    let current = Config::load().interface();
    let Some(found) = crate::wow::interface_version(&dir) else {
        crate::message_box(
            "T3 WoW",
            &format!(
                "Couldn't read WoW's interface version in {}.\n\nIn the game, /dump select(4, GetBuildInfo()) prints it; then run this in a terminal:\nt3wow.exe install --interface <number>",
                dir.display()
            ),
        );
        return;
    };
    let wow = found.wow_version.map(|v| format!("WoW {v}")).unwrap_or_else(|| "WoW".into());
    if found.interface == current {
        crate::message_box("T3 WoW", &format!("The addon already matches {wow} (interface {current})."));
        return;
    }
    let question = format!(
        "{wow} uses interface {}; the addon says {current}.\n\nSwitch the addon to {}? Your sign-in and settings stay. WoW needs a full restart afterwards.",
        found.interface, found.interface
    );
    if !confirm("T3 WoW", &question) {
        return;
    }
    match Slots::for_wow(&dir, &found.interface).set_interface() {
        Ok(_) => {
            Config::update(|c| c.interface = Some(found.interface.clone()));
            log::line(&format!("Interface version set to {} (was {current}); restarting the companion.", found.interface));
            crate::message_box("T3 WoW", "Done. Restart WoW completely (not /reload) so it reads the new version.");
            relaunch();
        }
        Err(error) => crate::message_box("T3 WoW", &format!("Couldn't rewrite the addon files: {error}")),
    }
}

/// Shows a notification from the tray icon (a toast on Windows 10 and 11). tray-icon has
/// no call for it, so this changes its icon directly. Windows names a tray icon by its
/// window and an id; tray-icon gives one hidden window to each icon but keeps the id to
/// itself, so the first few ids are tried, and only the real one exists on that window.
/// Needs the shell, so it isn't unit-tested.
fn notify(tray: &TrayIcon, title: &str, text: &str) {
    use windows_sys::Win32::UI::Shell::{Shell_NotifyIconW, NIF_INFO, NIIF_INFO, NIM_MODIFY, NOTIFYICONDATAW};
    fn copy(to: &mut [u16], text: &str) {
        let room = to.len() - 1; // the last unit stays 0
        for (slot, unit) in to.iter_mut().zip(text.encode_utf16().take(room)) {
            *slot = unit;
        }
    }
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = tray.window_handle();
    data.uFlags = NIF_INFO;
    data.dwInfoFlags = NIIF_INFO;
    copy(&mut data.szInfoTitle, title);
    copy(&mut data.szInfo, text);
    for id in 1..=16 {
        data.uID = id;
        if unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) } != 0 {
            return;
        }
    }
    log::line("Couldn't show the update notification.");
}

fn icon() -> Icon {
    Icon::from_rgba(include_bytes!("../assets/tray-32.rgba").to_vec(), 32, 32).expect("tray icon")
}

pub fn run(app: Arc<App>) -> ! {
    let t3_line = MenuItem::new("T3: starting…", false, None);
    let wow_line = MenuItem::new("WoW: starting…", false, None);
    let sign_in = MenuItem::new("Sign in to T3…", true, None);
    let wow_folder = MenuItem::new("Choose WoW folder…", true, None);
    let reinstall = MenuItem::new("Reinstall addon", true, None);
    let interface = MenuItem::new("Match WoW's interface version…", true, None);
    // What the game may make T3 do: any addon can draw the strip the companion reads.
    let trust_now = Config::load().trust();
    let trust_items: Vec<(Trust, CheckMenuItem)> = Trust::ALL.iter().map(|&t| (t, CheckMenuItem::new(t.label(), true, t == trust_now, None))).collect();
    let trust_menu = Submenu::new("Commands from the game", true);
    for (_, item) in &trust_items {
        let _ = trust_menu.append(item);
    }
    let open_log = MenuItem::new("Open log", true, None);
    let autostart = CheckMenuItem::new("Start with Windows", true, autostart_enabled(), None);
    let updates = MenuItem::new("Check for updates", true, None);
    let quit = MenuItem::new("Quit T3 WoW", true, None);
    let menu = Menu::new();
    let _ = menu.append_items(&[
        &t3_line,
        &wow_line,
        &PredefinedMenuItem::separator(),
        &sign_in,
        &wow_folder,
        &reinstall,
        &interface,
        &trust_menu,
        &open_log,
        &autostart,
        &updates,
        &PredefinedMenuItem::separator(),
        &quit,
    ]);
    // Look for updates now and every few hours.
    {
        let app = app.clone();
        std::thread::spawn(move || loop {
            app.check_updates();
            std::thread::sleep(std::time::Duration::from_secs(6 * 3600));
        });
    }
    let mut shown_update: Option<Update> = None;
    // The version a notification went out for: one per new release, not one per check.
    let mut notified: Option<String> = None;
    let tray = TrayIconBuilder::new()
        .with_icon(icon())
        .with_tooltip("T3 WoW")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(true)
        .build()
        .expect("tray icon");

    // A thread timer: WM_TIMER arrives in this loop twice a second to refresh the menu.
    unsafe { SetTimer(null_mut(), 0, 500, None) };
    let mut shown = Status::default();
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    loop {
        if unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) } <= 0 {
            break;
        }
        if msg.message == WM_TIMER && msg.hwnd.is_null() {
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                let id = event.id;
                if id == *quit.id() {
                    log::line("Quit from the tray.");
                    unsafe { PostQuitMessage(0) };
                } else if id == *sign_in.id() {
                    if shown.signed_in {
                        app.sign_out();
                    } else {
                        app.sign_in();
                    }
                } else if id == *updates.id() {
                    let state = app.update.lock().unwrap().clone();
                    match state {
                        Update::Available(release) => app.install_update(release, || relaunch()),
                        Update::Checking | Update::Installing(_) => {}
                        _ => app.check_updates(),
                    }
                } else if id == *open_log.id() {
                    open_path(&log::path());
                } else if id == *reinstall.id() {
                    app.send(Control::Reinstall);
                } else if id == *interface.id() {
                    match_interface(&app);
                } else if let Some(&(trust, _)) = trust_items.iter().find(|(_, item)| *item.id() == id) {
                    // Saved first, so the worker's next look at config.json agrees.
                    Config::update(|c| c.game_commands = Some(trust));
                    app.send(Control::Trust(trust));
                    for (t, item) in &trust_items {
                        item.set_checked(*t == trust);
                    }
                } else if id == *autostart.id() {
                    set_autostart(autostart.is_checked());
                    autostart.set_checked(autostart_enabled());
                } else if id == *wow_folder.id() {
                    let start = app.wow_dir.lock().unwrap().clone();
                    let mut dialog = rfd::FileDialog::new().set_title("Choose your World of Warcraft _forever_ folder");
                    if let Some(start) = start {
                        dialog = dialog.set_directory(start);
                    }
                    if let Some(picked) = dialog.pick_folder() {
                        match crate::wow::normalize(&picked.to_string_lossy()).filter(|d| crate::wow::looks_like_client(d)) {
                            Some(dir) => {
                                Config::update(|c| c.wow_dir = Some(dir.to_string_lossy().into_owned()));
                                log::line(&format!("WoW folder set to {}; restarting the companion.", dir.display()));
                                relaunch();
                            }
                            None => crate::message_box("T3 WoW", &format!("{} doesn't look like a WoW client folder (no Wow.exe or Interface folder).", picked.display())),
                        }
                    }
                }
            }
            let update = app.update.lock().unwrap().clone();
            if shown_update.as_ref() != Some(&update) {
                let current = crate::updater::current_version();
                updates.set_text(match &update {
                    Update::NotChecked => "Check for updates".to_string(),
                    Update::Checking => "Checking for updates…".to_string(),
                    Update::UpToDate => format!("Up to date ({current}): check again"),
                    Update::Available(release) => format!("Update to {} (now {current})", release.version),
                    Update::Installing(version) => format!("Updating to {version}…"),
                    Update::Failed(_) => "Update check failed: try again".to_string(),
                });
                updates.set_enabled(!matches!(update, Update::Checking | Update::Installing(_)));
                if let Update::Available(release) = &update {
                    if notified.as_ref() != Some(&release.version) {
                        let text = format!("T3 WoW {} is out. Choose Update to {} in the tray menu.", release.version, release.version);
                        notify(&tray, "T3 WoW update", &text);
                        notified = Some(release.version.clone());
                    }
                }
                shown_update = Some(update);
            }
            let status = app.status.lock().unwrap().clone();
            if status != shown {
                t3_line.set_text(&status.t3);
                wow_line.set_text(&status.wow);
                sign_in.set_text(if status.signed_in { "Sign out of T3" } else { "Sign in to T3…" });
                let tip = format!("T3 WoW\n{}\n{}", status.t3, status.wow);
                let _ = tray.set_tooltip(Some(tip.chars().take(120).collect::<String>()));
                shown = status;
            }
            continue;
        }
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    std::process::exit(0)
}
