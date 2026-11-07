//! Finds the WoW: Forever client folder (the one named _forever_, holding Wow.exe and
//! Interface\AddOns) without asking: a running client first, then Battle.net's install
//! record, then the usual places.

use std::path::{Path, PathBuf};

const FLAVOR: &str = "_forever_";
const GUESSES: &[&str] = &[
    "C:\\Program Files (x86)\\World of Warcraft",
    "C:\\Program Files\\World of Warcraft",
    "D:\\World of Warcraft",
    "D:\\Games\\World of Warcraft",
    "E:\\World of Warcraft",
    "E:\\Games\\World of Warcraft",
];

pub fn looks_like_client(dir: &Path) -> bool {
    dir.join("Wow.exe").exists() || dir.join("Interface").exists()
}

fn is_flavor(dir: &Path) -> bool {
    dir.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(FLAVOR))
}

#[cfg(windows)]
fn from_running_client() -> Option<PathBuf> {
    crate::capture::wow_processes().into_iter().find_map(|(_, path, _)| path.parent().filter(|d| is_flavor(d)).map(Path::to_path_buf))
}

#[cfg(windows)]
fn from_registry() -> Option<PathBuf> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    for key in ["SOFTWARE\\WOW6432Node\\Blizzard Entertainment\\World of Warcraft", "SOFTWARE\\Blizzard Entertainment\\World of Warcraft"] {
        let Ok(path) = hklm.open_subkey(key).and_then(|k| k.get_value::<String, _>("InstallPath")) else { continue };
        let path = PathBuf::from(path.trim_end_matches(['\\', '/']));
        // InstallPath points at one flavor (e.g. ...\_retail_) or at the root.
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let root = if name.starts_with('_') && name.ends_with('_') { path.parent().map(Path::to_path_buf).unwrap_or(path) } else { path };
        let candidate = root.join(FLAVOR);
        if looks_like_client(&candidate) {
            return Some(candidate);
        }
    }
    None
}

pub fn find_wow_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    if let Some(dir) = from_running_client().or_else(from_registry) {
        return Some(dir);
    }
    GUESSES.iter().map(|root| Path::new(root).join(FLAVOR)).find(|d| looks_like_client(d))
}

/// The interface version (the `## Interface:` number addons declare) of the client in a
/// folder, and the client's version when Wow.exe says.
#[derive(Debug, PartialEq)]
pub struct Interface {
    pub interface: String,
    pub wow_version: Option<String>,
}

/// Blizzard's rule for the interface version: major * 10000 + minor * 100 + patch
/// (1.15.7 is 11507, 11.0.2 is 110002).
fn interface_of_version(major: u32, minor: u32, patch: u32) -> u32 {
    major * 10000 + minor * 100 + patch
}

/// `SET lastAddonVersion "16001"` from WTF\Config.wtf: the interface version the client
/// last ran with, saved when it exits.
fn last_addon_version(config_wtf: &str) -> Option<u32> {
    config_wtf.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        (words.next()? == "SET" && words.next()? == "lastAddonVersion").then_some(())?;
        words.next()?.trim_matches('"').parse().ok()
    })
}

/// The higher of the two readings: Wow.exe changes as soon as the game patches, while
/// Config.wtf only catches up when the client exits; and the interface version never goes
/// down, so a higher saved one means the rule doesn't fit this client.
fn best_interface(from_exe: Option<u32>, from_config: Option<u32>) -> Option<u32> {
    from_exe.max(from_config).filter(|n| *n > 0)
}

/// Reads the interface version of the client in `dir`, from Wow.exe's version (works with
/// the game running or not) and WTF\Config.wtf. None when neither says.
pub fn interface_version(dir: &Path) -> Option<Interface> {
    #[cfg(windows)]
    let version = exe_version(&dir.join("Wow.exe"));
    #[cfg(not(windows))]
    let version: Option<(u32, u32, u32, u32)> = None;
    let from_config = std::fs::read_to_string(dir.join("WTF").join("Config.wtf")).ok().and_then(|text| last_addon_version(&text));
    let interface = best_interface(version.map(|(a, b, c, _)| interface_of_version(a, b, c)), from_config)?;
    Some(Interface { interface: interface.to_string(), wow_version: version.map(|(a, b, c, d)| format!("{a}.{b}.{c}.{d}")) })
}

/// An exe's file version (major, minor, patch, build) from its version resource. Needs a
/// real Wow.exe, so it isn't unit-tested.
#[cfg(windows)]
fn exe_version(exe: &Path) -> Option<(u32, u32, u32, u32)> {
    use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let path = wide(&exe.to_string_lossy());
    unsafe {
        let size = GetFileVersionInfoSizeW(path.as_ptr(), std::ptr::null_mut());
        if size == 0 {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(path.as_ptr(), 0, size, data.as_mut_ptr().cast()) == 0 {
            return None;
        }
        let mut info: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        if VerQueryValueW(data.as_ptr().cast(), wide("\\").as_ptr(), &mut info, &mut len) == 0 || info.is_null() || (len as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>() {
            return None;
        }
        let info = &*(info as *const VS_FIXEDFILEINFO);
        Some((info.dwFileVersionMS >> 16, info.dwFileVersionMS & 0xffff, info.dwFileVersionLS >> 16, info.dwFileVersionLS & 0xffff))
    }
}

/// Accepts the _forever_ folder, its parent, or Wow.exe itself.
pub fn normalize(input: &str) -> Option<PathBuf> {
    let text = input.replace(['"', '\''], "");
    let text = text.trim().trim_end_matches(['\\', '/']);
    if text.is_empty() {
        return None;
    }
    let mut dir = PathBuf::from(text);
    if dir.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) {
        dir = dir.parent()?.to_path_buf();
    }
    if !is_flavor(&dir) && looks_like_client(&dir.join(FLAVOR)) {
        dir = dir.join(FLAVOR);
    }
    Some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_normalize_to_the_client_folder() {
        let root = std::env::temp_dir().join(format!("t3wow-wow-{}", std::process::id()));
        let client = root.join("World of Warcraft").join(FLAVOR);
        std::fs::create_dir_all(client.join("Interface")).unwrap();
        let parent = root.join("World of Warcraft");
        assert_eq!(normalize(&format!("\"{}\\\"", parent.display())), Some(client.clone()));
        assert_eq!(normalize(&client.join("Wow.exe").display().to_string()), Some(client.clone()));
        assert!(looks_like_client(&client));
        assert_eq!(normalize("  "), None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_interface_version_comes_from_the_client_version_or_its_saved_settings() {
        assert_eq!(interface_of_version(1, 15, 7), 11507);
        assert_eq!(interface_of_version(11, 0, 2), 110002);
        assert_eq!(interface_of_version(1, 60, 1), 16001);
        let wtf = "SET locale \"enUS\"\r\nSET lastAddonVersion \"16001\"\r\nSET gxWindow \"1\"\r\n";
        assert_eq!(last_addon_version(wtf), Some(16001));
        assert_eq!(last_addon_version("SET lastAddonVersionX \"1\"\n"), None);
        assert_eq!(best_interface(Some(16002), Some(16001)), Some(16002));
        assert_eq!(best_interface(Some(11600), Some(16001)), Some(16001));
        assert_eq!(best_interface(None, Some(16001)), Some(16001));
        assert_eq!(best_interface(None, None), None);

        // A folder with only Config.wtf (no readable Wow.exe).
        let dir = std::env::temp_dir().join(format!("t3wow-wtf-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("WTF")).unwrap();
        std::fs::write(dir.join("WTF").join("Config.wtf"), wtf).unwrap();
        assert_eq!(interface_version(&dir), Some(Interface { interface: "16001".into(), wow_version: None }));
        let _ = std::fs::remove_dir_all(dir);
    }
}
