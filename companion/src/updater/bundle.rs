//! A Mac's update, when the companion runs as T3 WoW.app (scripts/build-app.mjs makes it):
//! the release's zipped app is checked like Windows' exe (checksum, signature), unzipped
//! next to the app, checked again as an app (its identifier, its code signature), and swapped
//! in for the whole running app. Nothing is ever written inside an app: a file added there
//! breaks its code signature. So the update lock lives in the settings folder, and the
//! version on disk is read from the app's own Info.plist.

use super::try_lock_at;
use crate::slots::compare_versions;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// The app's bundle identifier, as scripts/app-bundle.mjs writes it into Info.plist.
pub const BUNDLE_ID: &str = "com.t3wow.companion";
/// The program inside the app: Contents/MacOS/t3wow.
const EXECUTABLE: &str = "t3wow";

/// Says why an app macOS runs from a temporary copy can't update itself, and what to do: the
/// update's failure notice, the notice at startup and the menu's update item all say it.
pub const TRANSLOCATED: &str = "macOS runs T3 WoW from a temporary, read-only copy (it does that for an app opened where it was downloaded, or moved without Finder), so T3 WoW can't update itself. Quit T3 WoW, drag T3 WoW into Applications in Finder, and open it from there.";

/// The app `exe` runs in: `<X>.app` when `exe` is `<X>.app/Contents/MacOS/<program>`.
pub fn app_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    let named = |path: &Path, name: &str| path.file_name().is_some_and(|n| n == name);
    let is_app = app.extension().is_some_and(|e| e == "app") && app.file_stem().is_some_and(|s| !s.is_empty());
    (named(macos, "MacOS") && named(contents, "Contents") && is_app).then(|| app.to_path_buf())
}

/// The app this companion runs in, or None for a bare t3wow (started from where it was
/// built or copied, not from an app).
pub fn running_app() -> Option<PathBuf> {
    static APP: OnceLock<Option<PathBuf>> = OnceLock::new();
    APP.get_or_init(|| std::env::current_exe().and_then(|exe| exe.canonicalize()).ok().and_then(|exe| app_of(&exe))).clone()
}

/// The update lock, shared by the menu bar and `t3wow update` run from inside the app.
pub fn lock_path() -> PathBuf {
    crate::config::config_dir().join("update.lock")
}

const LOCK_PLACE: &str = "in T3 WoW's settings folder";

/// App Translocation: macOS runs a quarantined app opened where it was downloaded (or moved
/// there by anything but Finder) from a random, read-only mount in the user's temporary
/// folder: /private/var/folders/<..>/<..>/T/AppTranslocation/<UUID>/d/T3 WoW.app. Read from
/// the path rather than Security's SecTranslocateIsTranslocatedURL, which isn't public API.
fn translocated(app: &Path) -> bool {
    let in_temp = app.starts_with("/private/var/folders") || app.starts_with("/var/folders");
    in_temp && app.components().any(|c| c.as_os_str() == "AppTranslocation")
}

/// Whether macOS runs this app from a translocated copy, which can't update itself.
pub fn running_translocated() -> bool {
    running_app().is_some_and(|app| translocated(&app))
}

/// Puts the release `fetch` downloads and checks (the zipped app) in place of `app`.
pub fn install(app: &Path, version: &str, fetch: impl FnOnce() -> Result<Vec<u8>, String>) -> Result<(), String> {
    if translocated(app) {
        return Err(TRANSLOCATED.into());
    }
    update_bundle(app, &lock_path(), version, fetch, unzip, verify, |from, to| fs::rename(from, to))
}

/// The string under `key` in an XML property list (Info.plist as build-app.mjs writes it).
fn plist_string(plist: &str, key: &str) -> Option<String> {
    let (_, after) = plist.split_once(&format!("<key>{key}</key>"))?;
    let value = after.trim_start().strip_prefix("<string>")?;
    let (value, _) = value.split_once("</string>")?;
    Some(value.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&"))
}

fn info_plist(app: &Path) -> Option<String> {
    fs::read_to_string(app.join("Contents").join("Info.plist")).ok()
}

/// The version the app on disk says it is.
fn bundle_version(app: &Path) -> Option<String> {
    plist_string(&info_plist(app)?, "CFBundleShortVersionString")
}

/// Checks that `app` is a T3 WoW app: its identifier, and its program in place.
fn check_bundle(app: &Path) -> Result<(), String> {
    let plist = info_plist(app).ok_or("the update holds no Info.plist, so it isn't the T3 WoW app")?;
    match plist_string(&plist, "CFBundleIdentifier") {
        Some(id) if id == BUNDLE_ID => {}
        Some(id) => return Err(format!("the update is another app ({id}), not T3 WoW")),
        None => return Err("the update's Info.plist names no app, so it isn't the T3 WoW app".into()),
    }
    if plist_string(&plist, "CFBundleExecutable").as_deref() != Some(EXECUTABLE) || !app.join("Contents").join("MacOS").join(EXECUTABLE).is_file() {
        return Err("the update's app has no t3wow program in it".into());
    }
    Ok(())
}

/// Unzips `zip` into the folder `into` (made by ditto), as Finder would.
fn unzip(zip: &Path, into: &Path) -> Result<(), String> {
    let out = Command::new("/usr/bin/ditto").arg("-x").arg("-k").arg(zip).arg(into).output().map_err(|e| format!("can't unzip the update: {e}"))?;
    if !out.status.success() {
        return Err(format!("can't unzip the update: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

/// Checks the app's code signature: every file in it as signed, nothing added or changed.
fn verify(app: &Path) -> Result<(), String> {
    let out = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(app)
        .output()
        .map_err(|e| format!("can't check the update's code signature: {e}"))?;
    if !out.status.success() {
        return Err(format!("the update's code signature doesn't check out: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(())
}

/// `<app name>.<suffix>`, next to the app.
fn beside(app: &Path, suffix: &str) -> PathBuf {
    let name = app.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    app.with_file_name(format!("{name}.{suffix}"))
}

/// One whole update under the update lock (`lock`), from download to swap, like Windows'
/// `update_exe`: a second update stops at the lock, and one whose version isn't newer than
/// the app on disk stops before downloading. The zip is unpacked in a folder of its own next
/// to the app (the same volume, so the swap is two renames), checked as an app (`verify`:
/// its code signature), and swapped in. `unzip`, `verify` and `rename` are ditto, codesign
/// and fs::rename, or fakes in tests.
fn update_bundle(
    app: &Path,
    lock: &Path,
    version: &str,
    fetch: impl FnOnce() -> Result<Vec<u8>, String>,
    unzip: impl Fn(&Path, &Path) -> Result<(), String>,
    verify: impl Fn(&Path) -> Result<(), String>,
    rename: impl Fn(&Path, &Path) -> io::Result<()>,
) -> Result<(), String> {
    if let Some(dir) = lock.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _lock = try_lock_at(lock, LOCK_PLACE)?;
    if let Some(installed) = bundle_version(app).filter(|v| !compare_versions(version, v).is_gt()) {
        return Err(format!("already updated to {installed}: quit T3 WoW from its menu bar icon, then open it again"));
    }
    let zip = fetch()?;
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let tag = format!("{}-{nanos}", std::process::id());
    let staging = beside(app, &format!("new-{tag}"));
    if let Err(error) = fs::create_dir(&staging) {
        return Err(format!("can't unpack the update next to T3 WoW ({}): {error}", app.parent().unwrap_or(app).display()));
    }
    let result = stage(&staging, &zip, &unzip, &verify).and_then(|new_app| swap_bundles(app, &new_app, &beside(app, &format!("old-{tag}")), &rename));
    let _ = fs::remove_dir_all(&staging);
    result
}

/// Unpacks the zip in `staging` and checks what came out: the new app, ready to swap in.
fn stage(staging: &Path, zip: &[u8], unzip: &dyn Fn(&Path, &Path) -> Result<(), String>, verify: &dyn Fn(&Path) -> Result<(), String>) -> Result<PathBuf, String> {
    let zip_path = staging.join("update.zip");
    fs::write(&zip_path, zip).map_err(|e| format!("can't write the update next to T3 WoW: {e}"))?;
    let unpacked = staging.join("unpacked");
    unzip(&zip_path, &unpacked)?;
    let apps: Vec<PathBuf> = fs::read_dir(&unpacked)
        .map_err(|e| format!("can't read the unzipped update: {e}"))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "app") && path.is_dir())
        .collect();
    let [new_app] = apps.as_slice() else {
        return Err(format!("the update should hold one app, and holds {}", apps.len()));
    };
    check_bundle(new_app)?;
    verify(new_app)?;
    Ok(new_app.clone())
}

/// What to add when macOS refuses to move or replace the app. App Management (macOS 13 and
/// later) guards app bundles, and macOS doesn't ask about it for an app like this one: it
/// refuses, and shows a notification that names the app.
fn refused_hint(error: &io::Error) -> &'static str {
    if error.kind() == io::ErrorKind::PermissionDenied {
        " (if macOS said T3 WoW was prevented from modifying apps, turn T3 WoW on in System Settings › Privacy & Security › App Management, adding it with + if it isn't listed, then update again; or download the new T3 WoW.app from the releases page)"
    } else {
        ""
    }
}

/// Moves the running app aside to `old` and the new one into its place; on any failure the
/// old app goes back. The old one stays (the running companion is in it) until `clean_up_at`
/// on the next start.
fn swap_bundles(app: &Path, new_app: &Path, old: &Path, rename: &dyn Fn(&Path, &Path) -> io::Result<()>) -> Result<(), String> {
    if let Err(error) = rename(app, old) {
        return Err(format!("can't move T3 WoW aside: {error}{}", refused_hint(&error)));
    }
    if let Err(error) = rename(new_app, app) {
        return Err(match rename(old, app) {
            Ok(()) => format!("can't put the update in place: {error}{}", refused_hint(&error)),
            Err(again) => format!("can't put the update in place ({error}) nor T3 WoW back ({again}); it is at {}", old.display()),
        });
    }
    Ok(())
}

/// Removes the previous apps and unfinished updates earlier updates left next to `app`;
/// skipped while an update runs (its files aren't leftovers yet).
pub fn clean_up_at(app: &Path, lock: &Path) {
    let (Ok(_lock), Some(dir), Some(name)) = (try_lock_at(lock, LOCK_PLACE), app.parent(), app.file_name()) else { return };
    let name = name.to_string_lossy();
    let (old, new) = (format!("{name}.old-"), format!("{name}.new-"));
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        if file.starts_with(&old) || file.starts_with(&new) {
            let path = entry.path();
            let _ = if path.is_dir() { fs::remove_dir_all(&path) } else { fs::remove_file(&path) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_is_found_from_its_program() {
        assert_eq!(app_of(Path::new("/Applications/T3 WoW.app/Contents/MacOS/t3wow")), Some(PathBuf::from("/Applications/T3 WoW.app")));
        assert_eq!(app_of(Path::new("/Users/me/Apps/T3WoW.app/Contents/MacOS/t3wow")), Some(PathBuf::from("/Users/me/Apps/T3WoW.app")));
        // A bare t3wow, wherever it is.
        assert_eq!(app_of(Path::new("/Users/me/dev/t3wow/companion/target/release/t3wow")), None);
        assert_eq!(app_of(Path::new("/usr/local/bin/t3wow")), None);
        assert_eq!(app_of(Path::new("t3wow")), None);
        assert_eq!(app_of(Path::new("/Applications/T3 WoW.app/Contents/Resources/t3wow")), None);
        assert_eq!(app_of(Path::new("/Applications/T3 WoW/Contents/MacOS/t3wow")), None, "a folder that isn't an app");
        assert_eq!(app_of(Path::new("/.app/Contents/MacOS/t3wow")), None);
        // A test binary isn't in an app either.
        assert_eq!(running_app(), None);
        assert!(!super::super::can_install());
    }

    #[test]
    fn a_translocated_app_is_told_to_move() {
        assert!(translocated(Path::new("/private/var/folders/x/T/AppTranslocation/1234-5678/d/T3 WoW.app")));
        // As the install test's process ran (its path, with the folder names made up).
        assert!(translocated(Path::new("/private/var/folders/xd/abc123/T/AppTranslocation/6A1B2C3D-0000-4E5F-8A9B-0C1D2E3F4A5B/d/T3 WoW.app")));
        assert!(translocated(Path::new("/var/folders/xd/abc123/T/AppTranslocation/1/d/T3 WoW.app")));
        assert!(!translocated(Path::new("/Applications/T3 WoW.app")));
        assert!(!translocated(Path::new("/Users/me/Downloads/T3 WoW.app")), "opened in place but not translocated (Finder moved it, say)");
        assert!(!translocated(Path::new("/Users/me/AppTranslocation/T3 WoW.app")), "a folder of the user's own with that name");
        assert!(!translocated(Path::new("/private/var/folders/xd/abc123/T/T3 WoW.app")));
        // A test binary isn't in an app, so it isn't translocated either.
        assert!(!running_translocated());
        // What to do, said in full and once: no doubled full stop when a sentence follows.
        assert!(TRANSLOCATED.contains("Quit T3 WoW, drag T3 WoW into Applications in Finder, and open it from there."), "{TRANSLOCATED}");
        assert!(!TRANSLOCATED.contains("..") && !TRANSLOCATED.contains("tray"), "{TRANSLOCATED}");
        let error = install(Path::new("/private/var/folders/x/T/AppTranslocation/1/d/T3 WoW.app"), "99.0.0", || panic!("nothing is downloaded"));
        assert_eq!(error, Err(TRANSLOCATED.to_string()));
    }

    const PLIST: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
	<key>CFBundleExecutable</key>
	<string>t3wow</string>
	<key>CFBundleIdentifier</key>
	<string>com.t3wow.companion</string>
	<key>CFBundleName</key>
	<string>T3 &amp; WoW</string>
	<key>CFBundleShortVersionString</key>
	<string>VERSION</string>
	<key>LSUIElement</key>
	<true/>
</dict>
</plist>
"#;

    #[test]
    fn info_plist_strings_are_read() {
        assert_eq!(plist_string(PLIST, "CFBundleIdentifier").as_deref(), Some(BUNDLE_ID));
        assert_eq!(plist_string(PLIST, "CFBundleName").as_deref(), Some("T3 & WoW"));
        assert_eq!(plist_string(PLIST, "CFBundleShortVersionString").as_deref(), Some("VERSION"));
        assert_eq!(plist_string(PLIST, "LSUIElement"), None, "not a string");
        assert_eq!(plist_string(PLIST, "Missing"), None);
    }

    /// The identifier and the zip's name here are the ones the bundle script writes.
    #[test]
    fn the_bundle_script_agrees_on_the_identifier_and_the_zip() {
        let script = include_str!("../../../scripts/app-bundle.mjs");
        assert!(script.contains(&format!("\"{BUNDLE_ID}\"")), "scripts/app-bundle.mjs has another bundle identifier");
        assert!(script.contains(&format!("\"{EXECUTABLE}\"")));
        assert!(script.contains("\"t3wow-macos-arm64.zip\""));
    }

    struct Rig {
        dir: PathBuf,
        app: PathBuf,
        lock: PathBuf,
        // Declared last, so it's let go of after the files are gone.
        _serial: std::sync::MutexGuard<'static, ()>,
    }

    /// One rig at a time. A process started on another thread (ditto, codesign) holds a copy
    /// of every open file until it starts its program, the update lock's too, so for that
    /// instant the lock looks taken: these tests, which take it again and again, would see
    /// "another update is already running" now and then.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
        SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// An app at `<temp>/Applications/T3 WoW.app` with `version`, its program saying `body`.
    fn make_app(at: &Path, version: &str, id: &str, body: &str) {
        let macos = at.join("Contents").join("MacOS");
        fs::create_dir_all(&macos).unwrap();
        fs::write(macos.join("t3wow"), body).unwrap();
        fs::write(at.join("Contents").join("Info.plist"), PLIST.replace("VERSION", version).replace(BUNDLE_ID, id)).unwrap();
    }

    fn rig(name: &str) -> Rig {
        let serial = serial();
        let dir = std::env::temp_dir().join(format!("t3wow-bundle-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let app = dir.join("Applications").join("T3 WoW.app");
        make_app(&app, "0.10.0", BUNDLE_ID, "old program");
        Rig { lock: dir.join("settings").join("update.lock"), dir, app, _serial: serial }
    }

    impl Rig {
        fn program(&self) -> String {
            fs::read_to_string(self.app.join("Contents/MacOS/t3wow")).unwrap()
        }

        /// What is next to the app, other than the app.
        fn leftovers(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(self.app.parent().unwrap()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            names.retain(|n| n != "T3 WoW.app");
            names.sort();
            names
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    /// A fake ditto: checks it gets the downloaded zip, and unpacks the app `make` makes.
    fn fake_unzip(make: impl Fn(&Path)) -> impl Fn(&Path, &Path) -> Result<(), String> {
        move |zip, into| {
            assert_eq!(fs::read(zip).unwrap(), b"PK zip");
            assert_eq!(zip.parent(), into.parent(), "unpacked next to the zip, in the staging folder");
            fs::create_dir_all(into).unwrap();
            make(into);
            Ok(())
        }
    }

    fn new_app(into: &Path) {
        make_app(&into.join("T3 WoW.app"), "0.11.0", BUNDLE_ID, "new program");
    }

    fn signed(_: &Path) -> Result<(), String> {
        Ok(())
    }

    fn rename(from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }

    #[test]
    fn an_update_swaps_the_whole_app_and_keeps_the_old_one_until_the_next_start() {
        let rig = rig("ok");
        let verified = std::cell::RefCell::new(Vec::new());
        let verify = |app: &Path| {
            verified.borrow_mut().push(app.to_path_buf());
            Ok(())
        };
        update_bundle(&rig.app, &rig.lock, "0.11.0", || Ok(b"PK zip".to_vec()), fake_unzip(new_app), verify, rename).unwrap();
        assert_eq!(rig.program(), "new program");
        assert_eq!(bundle_version(&rig.app).as_deref(), Some("0.11.0"));
        // The new app was checked where it was unpacked, next to the app (the same volume).
        let checked = verified.into_inner();
        assert_eq!(checked.len(), 1);
        assert!(checked[0].starts_with(rig.app.parent().unwrap()) && checked[0].to_string_lossy().contains("T3 WoW.app.new-"), "{checked:?}");
        // The old app waits beside it; the staging folder is gone.
        let left = rig.leftovers();
        assert_eq!(left.len(), 1, "{left:?}");
        assert!(left[0].starts_with("T3 WoW.app.old-"), "{left:?}");
        let old = rig.app.with_file_name(&left[0]);
        assert_eq!(fs::read_to_string(old.join("Contents/MacOS/t3wow")).unwrap(), "old program");
        // Nothing was written inside either app.
        assert_eq!(fs::read_dir(rig.app.join("Contents/MacOS")).unwrap().count(), 1);
        assert_eq!(fs::read_dir(old.join("Contents/MacOS")).unwrap().count(), 1);
        // The next start clears it, and a leftover staging folder, and nothing else.
        fs::create_dir_all(rig.app.with_file_name("T3 WoW.app.new-1-2/unpacked")).unwrap();
        fs::create_dir_all(rig.app.with_file_name("T3 WoW.app.older-copy")).unwrap();
        fs::create_dir_all(rig.app.with_file_name("Other.app.old-1-2")).unwrap();
        clean_up_at(&rig.app, &rig.lock);
        assert_eq!(rig.leftovers(), ["Other.app.old-1-2", "T3 WoW.app.older-copy"]);
        assert_eq!(rig.program(), "new program");
    }

    #[test]
    fn a_failed_swap_puts_the_old_app_back() {
        let rig = rig("putback");
        let app = rig.app.clone();
        let refuse_new = move |from: &Path, to: &Path| {
            if to == app && from.to_string_lossy().contains(".new-") {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            fs::rename(from, to)
        };
        let error = update_bundle(&rig.app, &rig.lock, "0.11.0", || Ok(b"PK zip".to_vec()), fake_unzip(new_app), signed, refuse_new).unwrap_err();
        assert!(error.starts_with("can't put the update in place: permission denied"), "{error}");
        assert!(error.contains("App Management"), "{error}");
        assert_eq!(rig.program(), "old program");
        assert!(rig.leftovers().is_empty(), "{:?}", rig.leftovers());

        // Moving the running app aside refused: nothing changes.
        let error = update_bundle(&rig.app, &rig.lock, "0.11.0", || Ok(b"PK zip".to_vec()), fake_unzip(new_app), signed, |_: &Path, _: &Path| Err(io::Error::other("busy")))
            .unwrap_err();
        assert_eq!(error, "can't move T3 WoW aside: busy");
        assert_eq!(rig.program(), "old program");
        assert!(rig.leftovers().is_empty());
    }

    #[test]
    fn an_update_that_isnt_a_signed_t3_wow_app_is_refused_before_the_swap() {
        let rig = rig("refused");
        let fetch = || Ok(b"PK zip".to_vec());
        type MakeApp = Box<dyn Fn(&Path)>;
        let cases: Vec<(MakeApp, &str)> = vec![
            (Box::new(|into: &Path| make_app(&into.join("Other.app"), "0.11.0", "com.example.other", "x")), "the update is another app (com.example.other), not T3 WoW"),
            (Box::new(|_: &Path| {}), "the update should hold one app, and holds 0"),
            (
                Box::new(|into: &Path| {
                    new_app(into);
                    make_app(&into.join("Second.app"), "0.11.0", BUNDLE_ID, "x");
                }),
                "the update should hold one app, and holds 2",
            ),
            (
                Box::new(|into: &Path| {
                    new_app(into);
                    fs::remove_file(into.join("T3 WoW.app/Contents/MacOS/t3wow")).unwrap();
                }),
                "the update's app has no t3wow program in it",
            ),
        ];
        for (make, expected) in cases {
            assert_eq!(update_bundle(&rig.app, &rig.lock, "0.11.0", fetch, fake_unzip(make), signed, rename), Err(expected.to_string()));
            assert_eq!(rig.program(), "old program");
            assert!(rig.leftovers().is_empty(), "{expected}: {:?}", rig.leftovers());
        }
        // A bad signature, an unzip that fails, a download that fails.
        let unsigned = |_: &Path| Err("the update's code signature doesn't check out: a sealed resource is missing or invalid".to_string());
        assert!(update_bundle(&rig.app, &rig.lock, "0.11.0", fetch, fake_unzip(new_app), unsigned, rename).unwrap_err().contains("code signature"));
        let broken = |_: &Path, _: &Path| Err("can't unzip the update: bad zip".to_string());
        assert_eq!(update_bundle(&rig.app, &rig.lock, "0.11.0", fetch, broken, signed, rename), Err("can't unzip the update: bad zip".into()));
        assert_eq!(update_bundle(&rig.app, &rig.lock, "0.11.0", || Err("download failed: 404".into()), fake_unzip(new_app), signed, rename), Err("download failed: 404".into()));
        assert_eq!(rig.program(), "old program");
        assert!(rig.leftovers().is_empty(), "{:?}", rig.leftovers());
    }

    #[test]
    fn one_update_at_a_time_and_never_an_older_one() {
        let rig = rig("lock");
        fs::create_dir_all(rig.lock.parent().unwrap()).unwrap();
        let held = try_lock_at(&rig.lock, LOCK_PLACE).unwrap();
        let fetched = std::cell::Cell::new(false);
        let fetch = || {
            fetched.set(true);
            Ok(b"PK zip".to_vec())
        };
        assert_eq!(update_bundle(&rig.app, &rig.lock, "0.11.0", fetch, fake_unzip(new_app), signed, rename), Err("another update is already running".into()));
        assert!(!fetched.get(), "a second update stops before downloading");
        // Clean-up leaves a running update's folder alone.
        fs::create_dir_all(rig.app.with_file_name("T3 WoW.app.new-1-2")).unwrap();
        clean_up_at(&rig.app, &rig.lock);
        assert_eq!(rig.leftovers(), ["T3 WoW.app.new-1-2"]);
        drop(held);
        // The app on disk is already 0.10.0: the same or older isn't installed.
        for version in ["0.10.0", "0.9.9"] {
            let error = update_bundle(&rig.app, &rig.lock, version, || panic!("nothing is downloaded"), fake_unzip(new_app), signed, rename).unwrap_err();
            assert!(error.starts_with("already updated to 0.10.0"), "{error}");
        }
        update_bundle(&rig.app, &rig.lock, "0.11.0", || Ok(b"PK zip".to_vec()), fake_unzip(new_app), signed, rename).unwrap();
        assert_eq!(rig.program(), "new program");
    }

    /// The real ditto and codesign, on an app made and ad-hoc signed here: what the update
    /// runs, end to end, short of downloading.
    #[test]
    fn a_real_zipped_and_signed_app_unzips_and_verifies() {
        let rig = rig("real");
        let built = rig.dir.join("build").join("T3 WoW.app");
        make_app(&built, "0.11.0", BUNDLE_ID, "");
        // A real program: codesign signs any Mach-O; copy /usr/bin/true's.
        fs::copy("/usr/bin/true", built.join("Contents/MacOS/t3wow")).unwrap();
        let ok = |cmd: &mut Command| assert!(cmd.status().unwrap().success(), "{cmd:?}");
        ok(Command::new("/usr/bin/codesign").args(["--force", "--sign", "-", "--timestamp=none"]).arg(&built));
        let zip = rig.dir.join("app.zip");
        ok(Command::new("/usr/bin/ditto").args(["-c", "-k", "--norsrc", "--keepParent"]).arg(&built).arg(&zip));
        let bytes = fs::read(&zip).unwrap();
        update_bundle(&rig.app, &rig.lock, "0.11.0", || Ok(bytes), unzip, verify, rename).unwrap();
        assert_eq!(bundle_version(&rig.app).as_deref(), Some("0.11.0"));
        assert!(verify(&rig.app).is_ok());
        // A file added inside a signed app breaks its signature, which the update notices.
        fs::write(rig.app.join("Contents/MacOS/extra"), "x").unwrap();
        assert!(verify(&rig.app).unwrap_err().contains("code signature"));
    }
}
