//! Finds the WoW: Forever client folder (the one named _forever_, holding Wow.exe and
//! Interface\AddOns) without asking: a running client first, then Battle.net's install
//! record, then the usual places.

use std::path::{Path, PathBuf};

const FLAVOR: &str = "_forever_";
#[cfg(not(target_os = "macos"))]
const GUESSES: &[&str] = &[
    "C:\\Program Files (x86)\\World of Warcraft",
    "C:\\Program Files\\World of Warcraft",
    "D:\\World of Warcraft",
    "D:\\Games\\World of Warcraft",
    "E:\\World of Warcraft",
    "E:\\Games\\World of Warcraft",
];

#[cfg(not(target_os = "macos"))]
pub fn looks_like_client(dir: &Path) -> bool {
    dir.join("Wow.exe").exists() || dir.join("Interface").exists()
}

/// A fresh install has no Interface folder until the game's first launch, but always has its
/// app. The launcher app sits one level up, in the install root, so it isn't mistaken for it.
#[cfg(target_os = "macos")]
pub fn looks_like_client(dir: &Path) -> bool {
    dir.join(mac::CLIENT_APP).is_dir() || dir.join("Interface").exists()
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

#[cfg(not(target_os = "macos"))]
pub fn find_wow_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    if let Some(dir) = from_running_client().or_else(from_registry) {
        return Some(dir);
    }
    GUESSES.iter().map(|root| Path::new(root).join(FLAVOR)).find(|d| looks_like_client(d))
}

/// The client's folder from the path of its executable.
#[cfg(target_os = "macos")]
pub fn client_folder_of_exe(exe: &Path) -> Option<PathBuf> {
    mac::client_folder_of_exe(exe)
}

#[cfg(target_os = "macos")]
pub fn find_wow_dir() -> Option<PathBuf> {
    mac::find_wow_dir()
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
    #[cfg(target_os = "macos")]
    let exe = mac::app_version(dir);
    #[cfg(not(any(windows, target_os = "macos")))]
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

/// Accepts the _forever_ folder, its parent, or Wow.exe itself (on macOS, the client's or the
/// launcher's .app, or anything inside one).
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
    #[cfg(target_os = "macos")]
    if let Some(outside) = mac::folder_holding_app(&dir) {
        dir = outside;
    }
    if !is_flavor(&dir) && looks_like_client(&dir.join(FLAVOR)) {
        dir = dir.join(FLAVOR);
    }
    Some(dir)
}

#[cfg(target_os = "macos")]
mod mac {
    use super::{is_flavor, looks_like_client, ExeVersion, FLAVOR};
    use core_foundation::base::TCFType;
    use core_foundation::data::CFData;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::propertylist::{create_with_data, kCFPropertyListImmutable, CFPropertyList};
    use core_foundation::string::CFString;
    use std::fs;
    use std::path::{Path, PathBuf};

    pub const CLIENT_APP: &str = "World of Warcraft.app";
    const PRODUCT_DB: &str = "/Users/Shared/Battle.net/Agent/product.db";
    const PRODUCT_CODE: &str = "wowf";
    const WOW_ROOT: &str = "World of Warcraft";

    fn named(path: &Path, name: &str) -> bool {
        path.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name))
    }

    fn is_app(path: &Path) -> bool {
        path.extension().is_some_and(|e| e.eq_ignore_ascii_case("app"))
    }

    /// `_forever_` from `.../_forever_/World of Warcraft.app/Contents/MacOS/World of Warcraft`.
    /// The launcher, at the install root, has no `_forever_` above its app, so it doesn't match.
    pub fn client_folder_of_exe(exe: &Path) -> Option<PathBuf> {
        let macos = exe.parent()?;
        let contents = macos.parent()?;
        let app = contents.parent()?;
        let flavor = app.parent()?;
        (named(macos, "MacOS") && named(contents, "Contents") && is_app(app) && is_flavor(flavor)).then(|| flavor.to_path_buf())
    }

    /// The folder holding the innermost `.app` on this path, if the path is, or is in, one.
    pub fn folder_holding_app(path: &Path) -> Option<PathBuf> {
        path.ancestors().find(|a| is_app(a))?.parent().map(Path::to_path_buf)
    }

    /// A protobuf field: its number, and its bytes when it is length-delimited (sub-messages
    /// and text; numbers have none).
    type Field<'a> = (u64, Option<&'a [u8]>);

    /// The fields of a message, or None for anything that isn't protobuf.
    fn fields(buf: &[u8]) -> Option<Vec<Field<'_>>> {
        let mut out = Vec::new();
        let mut at = 0;
        while at < buf.len() {
            let tag = varint(buf, &mut at)?;
            let number = tag >> 3;
            if number == 0 {
                return None;
            }
            let value = match tag & 7 {
                0 => {
                    varint(buf, &mut at)?;
                    None
                }
                1 => {
                    at = at.checked_add(8).filter(|end| *end <= buf.len())?;
                    None
                }
                2 => {
                    let len = usize::try_from(varint(buf, &mut at)?).ok()?;
                    let end = at.checked_add(len).filter(|end| *end <= buf.len())?;
                    let bytes = &buf[at..end];
                    at = end;
                    Some(bytes)
                }
                5 => {
                    at = at.checked_add(4).filter(|end| *end <= buf.len())?;
                    None
                }
                _ => return None,
            };
            out.push((number, value));
        }
        Some(out)
    }

    fn varint(buf: &[u8], at: &mut usize) -> Option<u64> {
        let mut out = 0u64;
        for shift in (0..70).step_by(7) {
            let byte = *buf.get(*at)?;
            *at += 1;
            out |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(out);
            }
        }
        None
    }

    fn bytes_of<'a>(message: &[Field<'a>], number: u64) -> Option<&'a [u8]> {
        message.iter().find(|(n, _)| *n == number).and_then(|(_, bytes)| *bytes)
    }

    /// A text field, as UTF-8: an accented folder name has to come through intact.
    fn text_of<'a>(message: &[Field<'a>], number: u64) -> Option<&'a str> {
        std::str::from_utf8(bytes_of(message, number)?).ok()
    }

    /// Where Battle.net installed WoW: Forever, from product.db. The field meanings are
    /// inferred: each field 1 of the file is a product (2: its code; 3: its settings, where
    /// 1 is the install root and 13 the flavor folder).
    fn install_root(db: &[u8]) -> Option<String> {
        fields(db)?.iter().filter(|(number, _)| *number == 1).find_map(|(_, raw)| {
            let record = fields((*raw)?)?;
            if text_of(&record, 2)? != PRODUCT_CODE {
                return None;
            }
            let settings = fields(bytes_of(&record, 3)?)?;
            if text_of(&settings, 13).is_some_and(|flavor| !flavor.eq_ignore_ascii_case(FLAVOR)) {
                return None;
            }
            text_of(&settings, 1).filter(|root| !root.is_empty()).map(str::to_string)
        })
    }

    /// The client folder under an install root, when the client is there.
    fn client_under(root: &Path) -> Option<PathBuf> {
        let client = if is_flavor(root) { root.to_path_buf() } else { root.join(FLAVOR) };
        looks_like_client(&client).then_some(client)
    }

    fn from_product_db(db: &Path) -> Option<PathBuf> {
        client_under(Path::new(&install_root(&fs::read(db).ok()?)?))
    }

    /// Battle.net's default install folder: where a new game would go, so at most a hint.
    fn parent_hint(config: &str) -> Option<PathBuf> {
        let config: serde_json::Value = serde_json::from_str(config).ok()?;
        config["Client"]["Install"]["DefaultInstallPath"].as_str().filter(|p| !p.is_empty()).map(PathBuf::from)
    }

    /// The client in the Applications folder given, the user's own Applications, or the folder
    /// Battle.net would install into.
    fn from_guesses(applications: &Path, home: Option<&Path>, hint: Option<&Path>) -> Option<PathBuf> {
        let roots = [Some(applications.join(WOW_ROOT)), home.map(|h| h.join("Applications").join(WOW_ROOT)), hint.map(|p| p.join(WOW_ROOT))];
        roots.iter().flatten().find_map(|root| client_under(root))
    }

    pub fn find_wow_dir() -> Option<PathBuf> {
        let running = crate::capture::wow_processes().into_iter().find_map(|(_, exe, _)| client_folder_of_exe(&exe));
        running.or_else(|| from_product_db(Path::new(PRODUCT_DB))).or_else(|| {
            let home = std::env::var_os("HOME").map(PathBuf::from);
            let hint = home.as_ref().and_then(|h| fs::read_to_string(h.join("Library/Application Support/Battle.net/Battle.net.config")).ok()).and_then(|text| parent_hint(&text));
            from_guesses(Path::new("/Applications"), home.as_deref(), hint.as_deref())
        })
    }

    /// The client's version from its app bundle (Config.wtf can't be relied on: it may not exist
    /// even with the game running). Fed through the same reading as Wow.exe's version texts.
    pub fn app_version(dir: &Path) -> ExeVersion {
        fs::read(dir.join(CLIENT_APP).join("Contents").join("Info.plist")).map(|bytes| plist_version(&bytes)).unwrap_or_default()
    }

    fn plist_version(bytes: &[u8]) -> ExeVersion {
        let Ok((list, _)) = create_with_data(CFData::from_buffer(bytes), kCFPropertyListImmutable) else {
            return ExeVersion::default();
        };
        let list = unsafe { CFPropertyList::wrap_under_create_rule(list) };
        let Some(dictionary) = list.downcast_into::<CFDictionary>() else {
            return ExeVersion::default();
        };
        let texts = ["CFBundleVersion", "CFBundleShortVersionString"]
            .iter()
            .filter_map(|key| {
                let key = CFString::new(key);
                let value = dictionary.find(key.as_CFTypeRef())?;
                let value = unsafe { CFPropertyList::wrap_under_get_rule(*value) };
                Some(value.downcast_into::<CFString>()?.to_string())
            })
            .collect();
        ExeVersion { texts, fixed: None }
    }

    #[cfg(test)]
    mod tests {
        use super::super::{interface_version, normalize, Interface};
        use super::*;

        fn temp_dir(name: &str) -> PathBuf {
            let dir = std::env::temp_dir().join(format!("t3wow-mac-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            dir
        }

        fn app_at(client: &Path) {
            fs::create_dir_all(client.join(CLIENT_APP).join("Contents/MacOS")).unwrap();
        }

        fn length_delimited(number: u8, bytes: &[u8]) -> Vec<u8> {
            let mut out = vec![(number << 3) | 2];
            let mut len = bytes.len();
            while len >= 0x80 {
                out.push((len as u8 & 0x7f) | 0x80);
                len >>= 7;
            }
            out.push(len as u8);
            out.extend_from_slice(bytes);
            out
        }

        fn record(code: &str, root: &str, flavor: Option<&str>) -> Vec<u8> {
            let mut settings = length_delimited(1, root.as_bytes());
            // A number field among the text ones, as in the real file.
            settings.extend_from_slice(&[3 << 3, 2]);
            if let Some(flavor) = flavor {
                settings.extend(length_delimited(13, flavor.as_bytes()));
            }
            let mut out = length_delimited(1, code.as_bytes());
            out.extend(length_delimited(2, code.as_bytes()));
            out.extend(length_delimited(3, &settings));
            length_delimited(1, &out)
        }

        fn db(records: &[Vec<u8>]) -> Vec<u8> {
            let mut out = records.concat();
            // Fields after the products, in the file's other shapes.
            out.extend_from_slice(&[6 << 3, 0xac, 0x02]);
            out.extend(length_delimited(5, b"\x00\x00\x00\x00"));
            out
        }

        #[test]
        fn the_forever_record_gives_the_install_root() {
            let file = db(&[record("wow", "/Games/Retail", Some("_retail_")), record("wowf", "/Applications/World of Warcraft", Some("_forever_"))]);
            assert_eq!(install_root(&file).as_deref(), Some("/Applications/World of Warcraft"));
        }

        #[test]
        fn an_accented_install_root_comes_through_as_utf8() {
            let file = db(&[record("wowf", "/Users/José/Juegos/World of Warcraft", Some("_forever_"))]);
            assert_eq!(install_root(&file).as_deref(), Some("/Users/José/Juegos/World of Warcraft"));
        }

        #[test]
        fn product_db_without_the_forever_record_or_with_damage_gives_nothing() {
            assert_eq!(install_root(&db(&[record("wow", "/Games/Retail", Some("_retail_"))])), None);
            assert_eq!(install_root(&db(&[record("wowf", "/x", Some("_retail_"))])), None, "another flavor's folder");
            assert_eq!(install_root(&db(&[record("wowf", "", Some("_forever_"))])), None, "no root");
            assert_eq!(install_root(&[]), None);
            let file = db(&[record("wowf", "/Applications/World of Warcraft", Some("_forever_"))]);
            assert_eq!(install_root(&file[..file.len() - 3]), None, "cut short");
            assert_eq!(install_root(&[0x0f]), None, "a tag no protobuf uses");
            // A root that isn't UTF-8 is refused rather than mangled.
            assert_eq!(install_root(&db(&[record_bytes(b"wowf", &[b'/', 0xff, 0xfe])])), None);
        }

        fn record_bytes(code: &[u8], root: &[u8]) -> Vec<u8> {
            let mut out = length_delimited(1, code);
            out.extend(length_delimited(2, code));
            out.extend(length_delimited(3, &length_delimited(1, root)));
            length_delimited(1, &out)
        }

        #[test]
        fn product_db_points_at_a_client_that_is_really_there() {
            let dir = temp_dir("product-db");
            let client = dir.join("Juegos Ñ").join(WOW_ROOT).join(FLAVOR);
            app_at(&client);
            let file = dir.join("product.db");
            let root = client.parent().unwrap().to_str().unwrap().to_string();
            fs::write(&file, db(&[record("wowf", &root, Some("_forever_"))])).unwrap();
            assert_eq!(from_product_db(&file), Some(client));
            fs::write(&file, db(&[record("wowf", "/nowhere", Some("_forever_"))])).unwrap();
            assert_eq!(from_product_db(&file), None);
            assert_eq!(from_product_db(&dir.join("missing.db")), None);
            let _ = fs::remove_dir_all(dir);
        }

        #[test]
        fn the_client_is_recognized_by_its_app_even_before_its_first_launch() {
            let dir = temp_dir("app");
            let client = dir.join(FLAVOR);
            assert!(!looks_like_client(&client));
            app_at(&client);
            assert!(looks_like_client(&client), "a fresh install has no Interface folder");
            // The launcher sits at the install root and doesn't count as the client.
            let root = dir.join("root");
            fs::create_dir_all(root.join("World of Warcraft Launcher.app")).unwrap();
            assert!(!looks_like_client(&root));
            let _ = fs::remove_dir_all(dir);
        }

        #[test]
        fn the_running_client_climbs_out_of_its_app_to_the_client_folder() {
            let exe = Path::new("/Applications/World of Warcraft/_forever_/World of Warcraft.app/Contents/MacOS/World of Warcraft");
            assert_eq!(client_folder_of_exe(exe), Some(PathBuf::from("/Applications/World of Warcraft/_forever_")));
            let launcher = Path::new("/Applications/World of Warcraft/World of Warcraft Launcher.app/Contents/MacOS/World of Warcraft Launcher");
            assert_eq!(client_folder_of_exe(launcher), None, "the launcher isn't the client");
            let retail = Path::new("/Applications/World of Warcraft/_retail_/World of Warcraft.app/Contents/MacOS/World of Warcraft");
            assert_eq!(client_folder_of_exe(retail), None, "another flavor isn't this game");
            assert_eq!(client_folder_of_exe(Path::new("/_forever_/World of Warcraft.app/Contents/Resources/x")), None);
            assert_eq!(client_folder_of_exe(Path::new("World of Warcraft")), None);
        }

        #[test]
        fn normalize_accepts_the_app_a_path_in_it_and_the_install_root() {
            let dir = temp_dir("normalize");
            let root = dir.join(WOW_ROOT);
            let client = root.join(FLAVOR);
            app_at(&client);
            let app = client.join(CLIENT_APP);
            fs::create_dir_all(root.join("World of Warcraft Launcher.app")).unwrap();
            assert_eq!(normalize(app.to_str().unwrap()), Some(client.clone()));
            assert_eq!(normalize(&format!("{}/", app.display())), Some(client.clone()), "a trailing slash, as a drag into Terminal gives");
            assert_eq!(normalize(app.join("Contents/MacOS/World of Warcraft").to_str().unwrap()), Some(client.clone()));
            assert_eq!(normalize(root.to_str().unwrap()), Some(client.clone()));
            assert_eq!(normalize(client.to_str().unwrap()), Some(client.clone()));
            assert_eq!(normalize(root.join("World of Warcraft Launcher.app").to_str().unwrap()), Some(client));
            let _ = fs::remove_dir_all(dir);
        }

        #[test]
        fn the_guesses_look_in_applications_and_the_battle_net_folder() {
            let dir = temp_dir("guesses");
            let applications = dir.join("Applications");
            let home = dir.join("home");
            let hinted = dir.join("Volumes/Games");
            let found = || from_guesses(&applications, Some(&home), Some(&hinted));
            assert_eq!(found(), None);
            let in_hint = hinted.join(WOW_ROOT).join(FLAVOR);
            app_at(&in_hint);
            assert_eq!(found(), Some(in_hint));
            let in_home = home.join("Applications").join(WOW_ROOT).join(FLAVOR);
            app_at(&in_home);
            assert_eq!(found(), Some(in_home), "~/Applications comes before the hint");
            let in_applications = applications.join(WOW_ROOT).join(FLAVOR);
            app_at(&in_applications);
            assert_eq!(found(), Some(in_applications), "/Applications comes first");
            let _ = fs::remove_dir_all(dir);
        }

        #[test]
        fn only_the_install_folder_is_taken_from_battle_net_config() {
            assert_eq!(parent_hint(r#"{"Client":{"Install":{"DefaultInstallPath":"\/Volumes\/Juegos Ñ"}}}"#), Some(PathBuf::from("/Volumes/Juegos Ñ")));
            assert_eq!(parent_hint(r#"{"Client":{}}"#), None);
            assert_eq!(parent_hint("not json"), None);
        }

        const INFO_PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>World of Warcraft</string>
<key>CFBundleShortVersionString</key><string>1.60.1</string>
<key>CFBundleVersion</key><string>1.60.1.70862</string>
</dict></plist>"#;

        #[test]
        fn the_interface_version_comes_from_the_app_with_no_config_wtf() {
            let client = temp_dir("plist").join(FLAVOR);
            app_at(&client);
            fs::write(client.join(CLIENT_APP).join("Contents/Info.plist"), INFO_PLIST).unwrap();
            assert_eq!(interface_version(&client), Some(Interface { interface: "16001".into(), wow_version: Some("1.60.1.70862".into()) }));
            let _ = fs::remove_dir_all(client.parent().unwrap());
        }

        #[test]
        fn a_short_version_alone_is_enough_and_a_broken_plist_says_nothing() {
            let short = "<plist version=\"1.0\"><dict><key>CFBundleShortVersionString</key><string>1.60.1</string></dict></plist>";
            let version = plist_version(short.as_bytes());
            assert_eq!(version.texts, ["1.60.1"]);
            assert_eq!(super::super::exe_interface(&version), Some((16001, "1.60.1".into())));
            assert!(plist_version(b"not a plist").texts.is_empty());
            // No app at all (a non-client folder) reads as nothing rather than failing.
            assert!(app_version(Path::new("/nonexistent-t3wow")).texts.is_empty());
        }
    }
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
