//! Installs made with winget (`winget install MatiasCiccone.T3WoW`). winget puts t3wow.exe in a
//! folder of its own and records it under the user's Uninstall key, with the version it
//! installed and the exe's SHA-256. The companion updates itself, so it writes both again
//! (`sync`): otherwise winget keeps offering the version that's already running, and refuses
//! to uninstall an exe whose hash no longer matches ("it has been modified"). winget makes no
//! Start menu entry for a single exe, so the first start of a winget install adds one.

use crate::log;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
/// Our own value in winget's entry, set once the shortcut is made: a shortcut the user
/// deletes stays deleted. `winget uninstall` removes the entry, so a new install gets one.
const SHORTCUT_MADE: &str = "T3WoWStartMenuShortcut";
/// In the user's Start menu folder.
pub const SHORTCUT: &str = "T3 WoW.lnk";

/// As the tray starts: brings winget's record of this exe up to date (an update made by a
/// companion from before this existed didn't), and adds the Start menu shortcut once.
pub fn on_start() {
    let Ok(exe) = crate::updater::real_exe() else { return };
    let Ok(uninstall) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(UNINSTALL) else { return };
    let Some(entry) = entry(&uninstall, &exe) else { return };
    if let Err(error) = record(&entry, &exe, crate::VERSION) {
        log::line(&format!("Couldn't update winget's record of T3 WoW: {error}"));
    }
    let (Some(folder), Err(_)) = (start_menu(), entry.get_value::<u32, _>(SHORTCUT_MADE)) else { return };
    let lnk = folder.join(SHORTCUT);
    match shortcut::create(&lnk, &exe, "T3 Code inside World of Warcraft: Forever") {
        Ok(()) => {
            let _ = entry.set_value(SHORTCUT_MADE, &1u32);
            log::line(&format!("Installed with winget: added {} to the Start menu.", lnk.display()));
        }
        Err(error) => log::line(&format!("Couldn't add T3 WoW to the Start menu: {error}")),
    }
}

/// After an update put `version` in place of `exe`: winget's record of it, if winget
/// installed it.
pub fn sync(exe: &Path, version: &str) {
    let Ok(uninstall) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(UNINSTALL) else { return };
    if let Some(Err(error)) = entry(&uninstall, exe).map(|entry| record(&entry, exe, version)) {
        log::line(&format!("Couldn't update winget's record of T3 WoW: {error}"));
    }
}

/// The user's Start menu programs folder.
fn start_menu() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|appdata| PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs"))
}

/// Removes the Start menu shortcut (`t3wow.exe uninstall`), if there is one.
pub fn remove_shortcut() -> Option<PathBuf> {
    let lnk = start_menu()?.join(SHORTCUT);
    std::fs::remove_file(&lnk).ok().map(|()| lnk)
}

/// winget's entry for `exe` under `uninstall`, open for writing: a portable install whose
/// target is this exe.
fn entry(uninstall: &RegKey, exe: &Path) -> Option<RegKey> {
    uninstall.enum_keys().flatten().find_map(|name| {
        let key = uninstall.open_subkey(&name).ok()?;
        let kind: String = key.get_value("WinGetInstallerType").ok()?;
        let target: String = key.get_value("TargetFullPath").ok()?;
        (kind.eq_ignore_ascii_case("portable") && same_path(Path::new(&target), exe))
            .then(|| uninstall.open_subkey_with_flags(&name, KEY_READ | KEY_WRITE).ok())
            .flatten()
    })
}

fn same_path(a: &Path, b: &Path) -> bool {
    let plain = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase();
    plain(a) == plain(b)
}

/// winget compares DisplayVersion with its catalog, and SHA256 (lowercase hex) with the exe
/// before it uninstalls.
fn record(entry: &RegKey, exe: &Path, version: &str) -> Result<(), String> {
    let bytes = std::fs::read(exe).map_err(|e| format!("can't read {}: {e}", exe.display()))?;
    let sha = crate::updater::hex(&Sha256::digest(&bytes));
    if entry.get_value::<String, _>("DisplayVersion").ok().as_deref() != Some(version) {
        entry.set_value("DisplayVersion", &version).map_err(|e| e.to_string())?;
    }
    if entry.get_value::<String, _>("SHA256").ok().as_deref() != Some(sha.as_str()) {
        entry.set_value("SHA256", &sha).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A .lnk file, through the shell's own IShellLinkW (windows-sys has the calls, not the
/// interfaces, so their tables are declared here, in Windows' order).
mod shortcut {
    use std::ffi::{c_void, OsStr};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::{null, null_mut};
    use windows_sys::core::{GUID, HRESULT};
    use windows_sys::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};

    const CLSID_SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);
    const IID_ISHELL_LINK_W: GUID = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
    const IID_IPERSIST_FILE: GUID = GUID::from_u128(0x0000010b_0000_0000_c000_000000000046);

    type Text = *const u16;

    #[repr(C)]
    struct Unknown {
        query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
    }

    #[repr(C)]
    struct ShellLink {
        unknown: Unknown,
        get_path: unsafe extern "system" fn(*mut c_void, *mut u16, i32, *mut c_void, u32) -> HRESULT,
        get_id_list: usize,
        set_id_list: usize,
        get_description: usize,
        set_description: unsafe extern "system" fn(*mut c_void, Text) -> HRESULT,
        get_working_directory: usize,
        set_working_directory: unsafe extern "system" fn(*mut c_void, Text) -> HRESULT,
        get_arguments: usize,
        set_arguments: usize,
        get_hotkey: usize,
        set_hotkey: usize,
        get_show_cmd: usize,
        set_show_cmd: usize,
        get_icon_location: usize,
        set_icon_location: usize,
        set_relative_path: usize,
        resolve: usize,
        set_path: unsafe extern "system" fn(*mut c_void, Text) -> HRESULT,
    }

    #[repr(C)]
    struct PersistFile {
        unknown: Unknown,
        get_class_id: usize,
        is_dirty: usize,
        load: unsafe extern "system" fn(*mut c_void, Text, u32) -> HRESULT,
        save: unsafe extern "system" fn(*mut c_void, Text, i32) -> HRESULT,
        save_completed: usize,
        get_cur_file: usize,
    }

    fn wide(text: &OsStr) -> Vec<u16> {
        text.encode_wide().chain(Some(0)).collect()
    }

    fn check(hr: HRESULT, what: &str) -> Result<(), String> {
        if hr < 0 {
            Err(format!("{what} failed (0x{:08x})", hr as u32))
        } else {
            Ok(())
        }
    }

    /// A shortcut at `lnk` that starts `target` in its own folder.
    pub fn create(lnk: &Path, target: &Path, description: &str) -> Result<(), String> {
        let (lnk, folder) = (wide(lnk.as_os_str()), wide(target.parent().unwrap_or(target).as_os_str()));
        let (target, description) = (wide(target.as_os_str()), wide(OsStr::new(description)));
        with_link(move |link, file| unsafe {
            let table = &**(link as *mut *const ShellLink);
            check((table.set_path)(link, target.as_ptr()), "SetPath")?;
            check((table.set_working_directory)(link, folder.as_ptr()), "SetWorkingDirectory")?;
            check((table.set_description)(link, description.as_ptr()), "SetDescription")?;
            let persist = &**(file as *mut *const PersistFile);
            check((persist.save)(file, lnk.as_ptr(), 1), "saving the shortcut")
        })
    }

    /// The program the shortcut at `lnk` starts.
    #[cfg(test)]
    pub fn target(lnk: &Path) -> Result<std::path::PathBuf, String> {
        use std::os::windows::ffi::OsStringExt;
        let lnk = wide(lnk.as_os_str());
        with_link(move |link, file| unsafe {
            let persist = &**(file as *mut *const PersistFile);
            check((persist.load)(file, lnk.as_ptr(), 0), "loading the shortcut")?;
            let mut path = vec![0u16; 32768];
            let table = &**(link as *mut *const ShellLink);
            check((table.get_path)(link, path.as_mut_ptr(), path.len() as i32, null_mut(), 0), "GetPath")?;
            let len = path.iter().position(|&c| c == 0).unwrap_or(path.len());
            Ok(std::ffi::OsString::from_wide(&path[..len]).into())
        })
    }

    /// Runs `work` with a new shell link and its IPersistFile, on a thread of its own, so the
    /// COM apartment it needs is its own too.
    fn with_link<T: Send + 'static>(work: impl FnOnce(*mut c_void, *mut c_void) -> Result<T, String> + Send + 'static) -> Result<T, String> {
        std::thread::spawn(move || unsafe {
            check(CoInitializeEx(null(), COINIT_APARTMENTTHREADED as _), "starting COM")?;
            let result = (|| {
                let mut link = null_mut();
                check(CoCreateInstance(&CLSID_SHELL_LINK, null_mut(), CLSCTX_INPROC_SERVER, &IID_ISHELL_LINK_W, &mut link), "making a shell link")?;
                let unknown = &**(link as *mut *const Unknown);
                let mut file = null_mut();
                let result = check((unknown.query_interface)(link, &IID_IPERSIST_FILE, &mut file), "IPersistFile").and_then(|()| {
                    let result = work(link, file);
                    ((**(file as *mut *const Unknown)).release)(file);
                    result
                });
                (unknown.release)(link);
                result
            })();
            CoUninitialize();
            result
        })
        .join()
        .map_err(|_| "the shortcut thread panicked".to_string())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A throwaway Uninstall key under HKCU\Software\t3wow-tests, removed on drop.
    struct Scratch(String);

    impl Scratch {
        fn new(name: &str) -> (Scratch, RegKey) {
            let path = format!(r"Software\t3wow-tests\{name}-{}", std::process::id());
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            let _ = hkcu.delete_subkey_all(&path);
            let (key, _) = hkcu.create_subkey(&path).unwrap();
            (Scratch(path), key)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&self.0);
        }
    }

    fn add(uninstall: &RegKey, name: &str, values: &[(&str, &str)]) {
        let (key, _) = uninstall.create_subkey(name).unwrap();
        for (value, data) in values {
            key.set_value(value, data).unwrap();
        }
    }

    fn get(uninstall: &RegKey, name: &str, value: &str) -> String {
        uninstall.open_subkey(name).unwrap().get_value(value).unwrap()
    }

    #[test]
    fn an_update_rewrites_the_version_and_hash_winget_recorded_for_this_exe_only() {
        let (_scratch, uninstall) = Scratch::new("sync");
        let dir = std::env::temp_dir().join(format!("t3wow-winget-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("t3wow.exe");
        fs::write(&exe, b"exe 9.9.9").unwrap();
        let ours = "MatiasCiccone.T3WoW_Microsoft.Winget.Source_8wekyb3d8bbwe";
        // As winget writes it, but with forward slashes and another case: still this exe.
        let target = exe.to_string_lossy().replace('\\', "/").to_uppercase();
        add(&uninstall, ours, &[("WinGetInstallerType", "portable"), ("TargetFullPath", &target), ("DisplayVersion", "0.11.0"), ("SHA256", "00")]);
        add(&uninstall, "Other.Tool_x", &[("WinGetInstallerType", "portable"), ("TargetFullPath", r"C:\elsewhere\tool.exe"), ("DisplayVersion", "1.0")]);
        add(&uninstall, "SomeMsi", &[("TargetFullPath", &exe.to_string_lossy()), ("DisplayVersion", "2.0")]);

        let found = entry(&uninstall, &exe).expect("winget's entry for this exe");
        record(&found, &exe, "9.9.9").unwrap();

        assert_eq!(get(&uninstall, ours, "DisplayVersion"), "9.9.9");
        assert_eq!(get(&uninstall, ours, "SHA256"), crate::updater::hex(&Sha256::digest(b"exe 9.9.9")));
        assert_eq!(get(&uninstall, "Other.Tool_x", "DisplayVersion"), "1.0");
        assert_eq!(get(&uninstall, "SomeMsi", "DisplayVersion"), "2.0");
        assert!(entry(&uninstall, &dir.join("other.exe")).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_shortcut_starts_the_exe() {
        let dir = std::env::temp_dir().join(format!("t3wow-shortcut-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("t3wow.exe");
        fs::write(&exe, b"exe").unwrap();
        let lnk = dir.join(SHORTCUT);
        shortcut::create(&lnk, &exe, "T3 Code inside World of Warcraft: Forever").unwrap();
        assert!(same_path(&shortcut::target(&lnk).unwrap(), &exe));
        let _ = fs::remove_dir_all(&dir);
    }
}
