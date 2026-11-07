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
}
