//! The companion's log: %APPDATA%\t3wow\t3wow.log, or on a Mac
//! ~/Library/Application Support/t3wow/t3wow.log (the menu's "Open log"), echoed to the
//! console when there is one.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

const MAX_BYTES: u64 = 2 * 1024 * 1024;

static FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();
static CONSOLE: AtomicBool = AtomicBool::new(false);

pub fn path() -> PathBuf {
    crate::config::config_dir().join("t3wow.log")
}

/// Opens the log (starting it over when it has grown past 2 MB).
pub fn init(console: bool) {
    CONSOLE.store(console, Ordering::Relaxed);
    let _ = std::fs::create_dir_all(crate::config::config_dir());
    let path = path();
    let fresh = std::fs::metadata(&path).map(|m| m.len() > MAX_BYTES).unwrap_or(false);
    let file = OpenOptions::new().create(true).append(!fresh).write(true).truncate(fresh).open(&path).ok();
    let _ = FILE.set(Mutex::new(file));
}

#[cfg(windows)]
fn clock() -> String {
    let mut t: windows_sys::Win32::Foundation::SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut t) };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

#[cfg(not(windows))]
fn clock() -> String {
    format!("{}", crate::config::now_ms() / 1000)
}

pub fn line(message: &str) {
    let text = format!("[{}] {message}", clock());
    if CONSOLE.load(Ordering::Relaxed) {
        println!("{text}");
    }
    if let Some(Ok(mut guard)) = FILE.get().map(|m| m.lock()) {
        if let Some(file) = guard.as_mut() {
            let _ = writeln!(file, "{text}");
        }
    }
}
