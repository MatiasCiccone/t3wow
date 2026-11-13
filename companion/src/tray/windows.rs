//! The tray on Windows: the message loop on the main thread, notifications from the tray
//! icon, Start with Windows (the Run key), message boxes, and the texts that name Windows.

use super::{Tick, Tray};
use crate::app::{App, Level, Notice};
use crate::log;
use std::ptr::null_mut;
use std::sync::Arc;
use tray_icon::TrayIcon;
use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, PostQuitMessage, SetTimer, TranslateMessage, MSG, WM_TIMER};

pub use crate::dialog::ask_origin;

/// The server menu's item for the T3 on this machine.
pub const THIS_MACHINE: &str = "This PC";
/// The autostart item, and the start of its notifications' titles.
pub const AUTOSTART: &str = "Start with Windows";
pub const AUTOSTART_ON: &str = "T3 WoW starts by itself when you sign in to Windows.";
pub const AUTOSTART_OFF: &str = "T3 WoW no longer starts with Windows. Start it yourself before you play.";
/// After the chosen folder's path, when it isn't a WoW client.
pub const NOT_A_CLIENT: &str = "doesn't look like a WoW client folder (no Wow.exe or Interface folder).";

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "T3 WoW";

fn exe_command() -> String {
    format!("\"{}\"", std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default())
}

pub fn autostart_enabled() -> bool {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .and_then(|k| k.get_value::<String, _>(RUN_VALUE))
        .is_ok_and(|v| v.eq_ignore_ascii_case(&exe_command()))
}

pub fn set_autostart(on: bool) -> std::io::Result<()> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY)?;
    if on {
        key.set_value(RUN_VALUE, &exe_command())
    } else {
        key.delete_value(RUN_VALUE)
    }
}

pub fn open_path(path: &std::path::Path) {
    let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
}

/// A Yes/No question; true for Yes.
pub fn confirm(title: &str, text: &str) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONQUESTION, MB_YESNO};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    unsafe { MessageBoxW(null_mut(), wide(text).as_ptr(), wide(title).as_ptr(), MB_YESNO | MB_ICONQUESTION) == IDYES }
}

/// Shows a notification from the tray icon (a toast on Windows 10 and 11). tray-icon has
/// no call for it, so this changes its icon directly. Windows names a tray icon by its
/// window and an id; tray-icon gives one hidden window to each icon but keeps the id to
/// itself, so the first few ids are tried, and only the real one exists on that window.
/// Needs the shell, so it isn't unit-tested.
pub fn notify(tray: &TrayIcon, notice: &Notice) {
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

/// Runs the Windows message loop on the main thread until Quit is chosen, then exits.
pub fn run(app: Arc<App>) -> ! {
    let mut tray = Tray::new(app);
    // A thread timer: WM_TIMER arrives in this loop twice a second to refresh the menu.
    unsafe { SetTimer(null_mut(), 0, 500, None) };
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    loop {
        if unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) } <= 0 {
            break;
        }
        if msg.message == WM_TIMER && msg.hwnd.is_null() {
            if tray.tick() == Tick::Quit {
                unsafe { PostQuitMessage(0) };
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
    use crate::config::server_label;

    /// The texts the shared menu builds from these read as they always did on Windows.
    #[test]
    fn the_texts_name_windows_and_this_pc() {
        assert_eq!(format!("T3 server: {}", server_label(None)), "T3 server: this PC");
        assert_eq!(THIS_MACHINE, "This PC");
        assert_eq!(format!("{AUTOSTART}: on"), "Start with Windows: on");
        assert_eq!(format!("{AUTOSTART}: off"), "Start with Windows: off");
        assert_eq!(format!("Couldn't change {AUTOSTART}"), "Couldn't change Start with Windows");
        assert_eq!(
            format!("{} {NOT_A_CLIENT}", std::path::Path::new(r"C:\Games").display()),
            r"C:\Games doesn't look like a WoW client folder (no Wow.exe or Interface folder)."
        );
    }
}
