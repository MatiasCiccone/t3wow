//! The system tray icon and its menu. Runs the Windows message loop on the main thread;
//! the work happens in app::run on a background thread. Every action chosen here says how it
//! went: a notification (start, success, failure), the menu's activity line and tooltip
//! while it runs, or a message box where an answer is needed.

use crate::app::{self, App, Control, Level, Notice, Status, Update};
use crate::config::{canonical_origin, server_label, Config, Trust};
use crate::log;
use crate::slots::Slots;
use std::collections::VecDeque;
use std::ptr::null_mut;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, PostQuitMessage, SetTimer, TranslateMessage, MSG, WM_TIMER};

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "T3 WoW";
/// Between two notifications, so one doesn't replace the other before it's read.
const NOTICE_GAP: Duration = Duration::from_secs(2);

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

fn set_autostart(on: bool) -> std::io::Result<()> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY)?;
    if on {
        key.set_value(RUN_VALUE, &exe_command())
    } else {
        key.delete_value(RUN_VALUE)
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
            crate::message_box("T3 WoW", &format!("Done. {}", app::RESTART_WOW));
            app::notice_after_restart(&Notice::new(
                Level::Info,
                format!("Addon set to interface {}", found.interface),
                format!("T3 WoW restarted with the new interface version. {}", app::RESTART_WOW),
            ));
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
fn notify(tray: &TrayIcon, notice: &Notice) {
    use windows_sys::Win32::UI::Shell::{Shell_NotifyIconW, NIF_INFO, NIIF_ERROR, NIIF_INFO, NIIF_WARNING, NIM_MODIFY, NOTIFYICONDATAW};
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
    data.dwInfoFlags = match notice.level {
        Level::Info => NIIF_INFO,
        Level::Warning => NIIF_WARNING,
        Level::Error => NIIF_ERROR,
    };
    copy(&mut data.szInfoTitle, &notice.title);
    copy(&mut data.szInfo, &notice.text);
    for id in 1..=16 {
        data.uID = id;
        if unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) } != 0 {
            return;
        }
    }
    log::line("Couldn't show the notification.");
}

fn icon() -> Icon {
    Icon::from_rgba(include_bytes!("../assets/tray-32.rgba").to_vec(), 32, 32).expect("tray icon")
}

/// The tooltip: what is under way first, then the status lines; Windows keeps 127 characters.
fn tooltip(activity: Option<&str>, status: &Status) -> String {
    let mut tip = String::from("T3 WoW");
    for line in activity.into_iter().chain([status.t3.as_str(), status.wow.as_str()]) {
        if !line.is_empty() {
            tip.push('\n');
            tip.push_str(line);
        }
    }
    tip.chars().take(127).collect()
}

/// Which T3 the config says to use, and the other machine's used last.
#[derive(Clone, Debug, Default, PartialEq)]
struct Servers {
    origin: Option<String>,
    last_remote: Option<String>,
}

impl Servers {
    fn load() -> Servers {
        let config = Config::load();
        Servers { origin: config.origin.as_deref().map(canonical_origin), last_remote: config.last_remote.as_deref().map(canonical_origin) }
    }
}

pub fn run(app: Arc<App>) -> ! {
    let activity_line = MenuItem::new("", false, None);
    let t3_line = MenuItem::new("T3: starting…", false, None);
    let wow_line = MenuItem::new("WoW: starting…", false, None);
    let sign_in = MenuItem::new("Sign in to T3…", true, None);
    // The T3 in use: this PC, the other machine used last, or a new one.
    let server_menu = Submenu::new("T3 server: this PC", true);
    let this_pc = CheckMenuItem::new("This PC", true, true, None);
    let remote = CheckMenuItem::new("", true, false, None);
    let connect_remote = MenuItem::new("Connect to T3 on another machine…", true, None);
    let _ = server_menu.append_items(&[&this_pc, &PredefinedMenuItem::separator(), &connect_remote]);
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
        &server_menu,
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
            app.check_updates(false);
            std::thread::sleep(std::time::Duration::from_secs(6 * 3600));
        });
    }
    let mut shown_update: Option<Update> = None;
    let tray = TrayIconBuilder::new()
        .with_icon(icon())
        .with_tooltip("T3 WoW")
        .with_menu(Box::new(menu.clone()))
        .with_menu_on_left_click(true)
        .build()
        .expect("tray icon");

    // What the previous companion said before it restarted (an update, a new WoW folder).
    let mut notices: VecDeque<Notice> = app::take_restart_notice().into_iter().collect();
    let mut last_notice: Option<Instant> = None;
    let mut shown_activity: Option<String> = None;
    let mut servers: Option<Servers> = None;
    let mut servers_read = Instant::now();

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
                } else if id == *this_pc.id() || id == *remote.id() || id == *connect_remote.id() {
                    let current = Servers::load();
                    let wanted = if id == *this_pc.id() {
                        Some(None)
                    } else if id == *remote.id() {
                        current.last_remote.clone().map(Some)
                    } else {
                        let start = current.origin.clone().or(current.last_remote.clone()).unwrap_or_default();
                        crate::dialog::ask_origin(&start).map(Some)
                    };
                    match wanted {
                        Some(origin) if origin == current.origin && shown.signed_in => {
                            let name = app::t3_named(&server_label(origin.as_deref()));
                            app.notify(Notice::new(Level::Info, "T3 server", format!("T3 WoW already uses {name}.")));
                        }
                        Some(origin) => app.connect(Some(origin), true),
                        None => {}
                    }
                    // A check item ticks itself when chosen: show what the config says.
                    servers = None;
                } else if id == *updates.id() {
                    let state = app.update.lock().unwrap().clone();
                    match state {
                        Update::Available(release) => app.install_update(release, || relaunch()),
                        Update::Checking | Update::Installing(_) => {}
                        _ => app.check_updates(true),
                    }
                } else if id == *open_log.id() {
                    open_path(&log::path());
                } else if id == *reinstall.id() {
                    app.send(Control::Reinstall);
                } else if id == *interface.id() {
                    match_interface(&app);
                } else if let Some(&(trust, _)) = trust_items.iter().find(|(_, item)| *item.id() == id) {
                    app.set_trust(trust);
                    for (t, item) in &trust_items {
                        item.set_checked(*t == trust);
                    }
                } else if id == *autostart.id() {
                    let wanted = autostart.is_checked();
                    match set_autostart(wanted) {
                        Ok(()) if wanted => app.notify(Notice::new(Level::Info, "Start with Windows: on", "T3 WoW starts by itself when you sign in to Windows.")),
                        Ok(()) => app.notify(Notice::new(Level::Info, "Start with Windows: off", "T3 WoW no longer starts with Windows. Start it yourself before you play.")),
                        Err(error) => app.notify(Notice::new(Level::Error, "Couldn't change Start with Windows", error.to_string())),
                    }
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
                                app::notice_after_restart(&Notice::new(Level::Info, "WoW folder set", format!("T3 WoW restarted to use {}.", dir.display())));
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
                shown_update = Some(update);
            }
            // The T3 in use, from the config: a terminal `login --origin` changes it too.
            if servers.is_none() || servers_read.elapsed() > Duration::from_secs(2) {
                let now = Servers::load();
                if servers.as_ref() != Some(&now) {
                    server_menu.set_text(format!("T3 server: {}", server_label(now.origin.as_deref())));
                    this_pc.set_checked(now.origin.is_none());
                    let _ = server_menu.remove(&remote);
                    if let Some(last) = &now.last_remote {
                        remote.set_text(last);
                        remote.set_checked(now.origin.as_ref() == Some(last));
                        let _ = server_menu.insert(&remote, 1);
                    }
                    servers = Some(now);
                }
                servers_read = Instant::now();
            }
            let activity = app.activity();
            let status = app.status.lock().unwrap().clone();
            if activity != shown_activity {
                // The activity line heads the menu while something runs.
                let _ = menu.remove(&activity_line);
                if let Some(text) = &activity {
                    activity_line.set_text(text.replace('\n', " · "));
                    let _ = menu.insert(&activity_line, 0);
                }
            }
            if status != shown || activity != shown_activity {
                t3_line.set_text(&status.t3);
                wow_line.set_text(&status.wow);
                sign_in.set_text(if status.signed_in { "Sign out of T3" } else { "Sign in to T3…" });
                let _ = tray.set_tooltip(Some(tooltip(activity.as_deref(), &status)));
                shown = status;
                shown_activity = activity;
            }
            notices.extend(app.take_notices());
            if last_notice.is_none_or(|at| at.elapsed() >= NOTICE_GAP) {
                if let Some(notice) = notices.pop_front() {
                    notify(&tray, &notice);
                    last_notice = Some(Instant::now());
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tooltip_leads_with_what_is_under_way_and_fits() {
        let status = Status { t3: "T3: connected (2 machines)".into(), wow: "WoW: not running".into(), ..Status::default() };
        assert_eq!(tooltip(None, &status), "T3 WoW\nT3: connected (2 machines)\nWoW: not running");
        assert_eq!(tooltip(Some("Downloading update 0.6.0… 42%"), &status), "T3 WoW\nDownloading update 0.6.0… 42%\nT3: connected (2 machines)\nWoW: not running");
        let long = Status { t3: "x".repeat(200), ..Status::default() };
        assert_eq!(tooltip(None, &long).chars().count(), 127);
    }
}
