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

/// Interface versions a WoW client can have: 1.0.0 (10000) through 99.99.99 (999999).
fn plausible(interface: u32) -> bool {
    (10_000..=999_999).contains(&interface)
}

/// Whether a saved or typed interface version can be a real one (16001 can, 1607186 can't).
pub fn is_interface(text: &str) -> bool {
    text.parse().is_ok_and(plausible)
}

/// Blizzard's rule for the interface version: major * 10000 + minor * 100 + patch
/// (1.15.7 is 11507, 11.0.2 is 110002). None when the numbers can't be a WoW version: a
/// minor or patch of 100 or more would spill into the next field.
fn interface_of_version(major: u32, minor: u32, patch: u32) -> Option<u32> {
    if minor >= 100 || patch >= 100 {
        return None;
    }
    major.checked_mul(10_000).map(|n| n + minor * 100 + patch).filter(|n| plausible(*n))
}

/// "1.60.1.70862" (or "1, 60, 1, 70862") as (major, minor, patch); the build may be missing.
fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    let parts = text.trim().split(['.', ',']).map(|part| part.trim().parse().ok()).collect::<Option<Vec<u32>>>()?;
    match parts[..] {
        [major, minor, patch] | [major, minor, patch, _] => Some((major, minor, patch)),
        _ => None,
    }
}

/// Wow.exe's version resource: the FileVersion and ProductVersion texts ("1.60.1.70862"),
/// and the numeric fields. The texts come first: the numeric fields hold 16 bits each, so a
/// build like 70862 doesn't fit, and WoW: Forever's Wow.exe says 160.1.7086.2 there.
#[derive(Debug, Default)]
struct ExeVersion {
    texts: Vec<String>,
    fixed: Option<(u32, u32, u32, u32)>,
}

/// The interface version Wow.exe's version gives, with that version, from the first reading
/// that can be a WoW version.
fn exe_interface(exe: &ExeVersion) -> Option<(u32, String)> {
    let from_texts = exe.texts.iter().filter_map(|text| {
        let (major, minor, patch) = parse_version(text)?;
        Some((interface_of_version(major, minor, patch)?, text.trim().to_string()))
    });
    let from_fixed = exe.fixed.and_then(|(a, b, c, d)| Some((interface_of_version(a, b, c)?, format!("{a}.{b}.{c}.{d}"))));
    from_texts.chain(from_fixed).next()
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

/// The higher of the two readings that can be real: Wow.exe changes as soon as the game
/// patches, while Config.wtf only catches up when the client exits; and the interface version
/// never goes down, so a higher saved one means the rule doesn't fit this client.
fn best_interface(from_exe: Option<u32>, from_config: Option<u32>) -> Option<u32> {
    from_exe.filter(|n| plausible(*n)).max(from_config.filter(|n| plausible(*n)))
}

/// Reads the interface version of the client in `dir`, from Wow.exe's version (works with
/// the game running or not) and WTF\Config.wtf. None when neither says, or says something
/// no WoW version can be.
pub fn interface_version(dir: &Path) -> Option<Interface> {
    #[cfg(windows)]
    let exe = exe_version(&dir.join("Wow.exe"));
    #[cfg(not(windows))]
    let exe = ExeVersion::default();
    let from_exe = exe_interface(&exe);
    let from_config = std::fs::read_to_string(dir.join("WTF").join("Config.wtf")).ok().and_then(|text| last_addon_version(&text));
    let interface = best_interface(from_exe.as_ref().map(|(n, _)| *n), from_config)?;
    Some(Interface { interface: interface.to_string(), wow_version: from_exe.map(|(_, version)| version) })
}

/// An exe's version resource: its FileVersion and ProductVersion texts (in each language it
/// has) and its numeric file version. Empty when it has none.
#[cfg(windows)]
fn exe_version(exe: &Path) -> ExeVersion {
    use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO};
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let path = wide(&exe.to_string_lossy());
    let mut version = ExeVersion::default();
    unsafe {
        let size = GetFileVersionInfoSizeW(path.as_ptr(), std::ptr::null_mut());
        if size == 0 {
            return version;
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(path.as_ptr(), 0, size, data.as_mut_ptr().cast()) == 0 {
            return version;
        }
        // A block of the resource: a pointer into `data` and its length (in characters for text).
        let query = |block: &str| -> Option<(*const std::ffi::c_void, usize)> {
            let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
            let mut len = 0u32;
            (VerQueryValueW(data.as_ptr().cast(), wide(block).as_ptr(), &mut ptr, &mut len) != 0 && !ptr.is_null() && len > 0).then_some((ptr as *const _, len as usize))
        };
        // The languages the texts come in, as (language, code page) pairs, then US English
        // with the Unicode or Western code page in case the resource doesn't list them.
        let mut languages: Vec<(u16, u16)> = query("\\VarFileInfo\\Translation")
            .map(|(ptr, len)| std::slice::from_raw_parts(ptr as *const u16, len / 2).as_chunks::<2>().0.iter().map(|&[language, code_page]| (language, code_page)).collect())
            .unwrap_or_default();
        languages.extend([(0x0409, 0x04b0), (0x0409, 0x04e4)]);
        for (language, code_page) in languages {
            for name in ["FileVersion", "ProductVersion"] {
                let Some((ptr, len)) = query(&format!("\\StringFileInfo\\{language:04x}{code_page:04x}\\{name}")) else { continue };
                let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr as *const u16, len));
                let text = text.trim_end_matches('\0').trim().to_string();
                if !text.is_empty() && !version.texts.contains(&text) {
                    version.texts.push(text);
                }
            }
        }
        if let Some((ptr, _)) = query("\\").filter(|(_, len)| *len >= std::mem::size_of::<VS_FIXEDFILEINFO>()) {
            let info = &*(ptr as *const VS_FIXEDFILEINFO);
            version.fixed = Some((info.dwFileVersionMS >> 16, info.dwFileVersionMS & 0xffff, info.dwFileVersionLS >> 16, info.dwFileVersionLS & 0xffff));
        }
    }
    version
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
        assert_eq!(interface_of_version(1, 15, 7), Some(11507));
        assert_eq!(interface_of_version(11, 0, 2), Some(110002));
        assert_eq!(interface_of_version(1, 60, 1), Some(16001));
        let wtf = "SET locale \"enUS\"\r\nSET lastAddonVersion \"16001\"\r\nSET gxWindow \"1\"\r\n";
        assert_eq!(last_addon_version(wtf), Some(16001));
        assert_eq!(last_addon_version("SET lastAddonVersionX \"1\"\n"), None);
        assert_eq!(best_interface(Some(16002), Some(16001)), Some(16002));
        assert_eq!(best_interface(Some(11600), Some(16001)), Some(16001));
        assert_eq!(best_interface(None, Some(16001)), Some(16001));
        assert_eq!(best_interface(None, None), None);
        // A reading no WoW can have never wins, however high.
        assert_eq!(best_interface(Some(1607186), Some(16001)), Some(16001));
        assert_eq!(best_interface(Some(1607186), None), None);
        assert_eq!(best_interface(None, Some(0)), None);

        // A folder with only Config.wtf (no readable Wow.exe).
        let dir = std::env::temp_dir().join(format!("t3wow-wtf-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("WTF")).unwrap();
        std::fs::write(dir.join("WTF").join("Config.wtf"), wtf).unwrap();
        assert_eq!(interface_version(&dir), Some(Interface { interface: "16001".into(), wow_version: None }));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn wow_forevers_exe_version_comes_from_its_text_not_its_numeric_fields() {
        // What WoW: Forever's Wow.exe said on 2026-11-07: the numeric fields hold 16 bits
        // each, so the build 70862 doesn't fit and the digits shift.
        assert_eq!(interface_of_version(160, 1, 7086), None);
        assert_eq!(parse_version("1.60.1.70862"), Some((1, 60, 1)));
        assert_eq!(parse_version(" 1, 60, 1, 70862 "), Some((1, 60, 1)));
        assert_eq!(parse_version("1.60.1"), Some((1, 60, 1)));
        assert_eq!(parse_version("1.60"), None);
        assert_eq!(parse_version("1.60.1.70862 (beta)"), None);
        assert_eq!(parse_version(""), None);

        let exe = ExeVersion { texts: vec!["1.60.1.70862".into()], fixed: Some((160, 1, 7086, 2)) };
        assert_eq!(exe_interface(&exe), Some((16001, "1.60.1.70862".into())));
        // Without the text, the numeric fields alone can't be a version: no reading at all.
        assert_eq!(exe_interface(&ExeVersion { texts: vec![], fixed: Some((160, 1, 7086, 2)) }), None);
        // A client whose numeric fields do fit still reads from them.
        assert_eq!(exe_interface(&ExeVersion { texts: vec![], fixed: Some((1, 15, 7, 61582)) }), Some((11507, "1.15.7.61582".into())));
        // An unreadable text falls through to the next reading.
        assert_eq!(exe_interface(&ExeVersion { texts: vec!["n/a".into()], fixed: Some((1, 15, 7, 1)) }), Some((11507, "1.15.7.1".into())));
    }

    #[test]
    fn only_numbers_a_wow_client_can_have_are_interface_versions() {
        assert!(is_interface("16001"));
        assert!(is_interface("11507"));
        assert!(is_interface("110002"));
        assert!(!is_interface("1607186"));
        assert!(!is_interface("1"));
        assert!(!is_interface("0"));
        assert!(!is_interface(""));
        assert!(!is_interface("16001x"));
        assert_eq!(interface_of_version(1, 100, 0), None);
        assert_eq!(interface_of_version(1, 0, 100), None);
        assert_eq!(interface_of_version(100, 0, 0), None);
        assert_eq!(interface_of_version(u32::MAX, 0, 0), None);
    }

    /// Reads a real client, read-only. `cargo test real_wow_exe -- --ignored --nocapture`,
    /// with T3WOW_WOW_DIR set when it isn't D:\World of Warcraft\_forever_.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn reads_the_interface_version_of_a_real_wow_exe() {
        let dir = std::env::var("T3WOW_WOW_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("D:\\World of Warcraft\\_forever_"));
        let exe = exe_version(&dir.join("Wow.exe"));
        println!("Wow.exe texts: {:?}", exe.texts);
        println!("Wow.exe numeric fields: {:?}", exe.fixed);
        let found = interface_version(&dir);
        println!("interface_version: {found:?}");
        assert!(found.is_some_and(|f| is_interface(&f.interface)));
    }
}
