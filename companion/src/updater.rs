//! Updates from the project's GitHub releases. Each release carries t3wow.exe, its
//! SHA-256 (t3wow.exe.sha256) and, once release signing is set up, an Ed25519 signature of
//! that checksum file (t3wow.exe.sha256.sig), made in CI with the release key (see
//! .github/workflows/release.yml and scripts/sign-release.mjs). An update installs only
//! when the exe matches the checksum and, once the real key is built in, the signature verifies.
//! The addon is embedded in the exe, so updating the companion also updates the addon on
//! its next start.

use crate::slots::compare_versions;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ring::signature::{UnparsedPublicKey, ED25519};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// Releases live in their own public repository while the source stays private.
const REPO: &str = "MatiasCiccone/t3wow-releases";
const MAX_EXE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SMALL_FILE_BYTES: u64 = 4096;
const EXE: &str = "t3wow.exe";
const SHA: &str = "t3wow.exe.sha256";
const SIG: &str = "t3wow.exe.sha256.sig";

// The public half of the release key, in its own file (all zeros until signing is set up).
mod release_key;
use release_key::RELEASE_PUBLIC_KEY;

/// The release key's public half, or None while it's the placeholder: signing isn't set up
/// yet, so a release with a valid SHA-256 installs without a signature, as in 0.4.1. Once
/// the real key is built in, a release must carry a valid signature made with it.
fn release_key() -> Option<&'static [u8; 32]> {
    (RELEASE_PUBLIC_KEY != [0; 32]).then_some(&RELEASE_PUBLIC_KEY)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub exe_url: String,
    pub sha_url: String,
    /// Required once release signing is set up.
    pub sig_url: Option<String>,
}

/// Debug builds can test updates against another repository (`T3WOW_UPDATE_REPO`); release
/// builds have no such switch, so nothing in the environment can redirect their updates.
fn repo() -> String {
    #[cfg(debug_assertions)]
    if let Ok(repo) = std::env::var("T3WOW_UPDATE_REPO") {
        return repo;
    }
    REPO.to_string()
}

/// Debug builds can also send the API call elsewhere (`T3WOW_UPDATE_API`, say a local server
/// answering 403, to try the release page fallback).
fn api() -> String {
    #[cfg(debug_assertions)]
    if let Ok(api) = std::env::var("T3WOW_UPDATE_API") {
        return api;
    }
    "https://api.github.com".to_string()
}

fn http() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(120)))
            .user_agent(format!("t3wow-companion/{}", crate::VERSION))
            .http_status_as_error(false)
            .build(),
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn current_version() -> &'static str {
    crate::VERSION
}

/// What to say when no release is newer than this companion. A test build
/// ("0.9.1-dev+57ee2ec") isn't a release itself, so it doesn't say it's the newest one.
pub fn up_to_date_text() -> String {
    up_to_date_for(current_version())
}

fn up_to_date_for(current: &str) -> String {
    match current.split_once(['-', '+']) {
        Some((release, _)) => format!("You have {current}, a test build: no release is newer than {release}."),
        None => format!("You have {current}, the newest release."),
    }
}

/// The newest release if it is newer than this companion. Ok(None): up to date, or no
/// release can be seen (a private repository looks the same as one without releases).
pub fn check() -> Result<Option<Release>, String> {
    let url = format!("{}/repos/{}/releases/latest", api(), repo());
    let mut response = http().get(&url).header("accept", "application/vnd.github+json").call().map_err(|e| e.to_string())?;
    let release: Value = match response.status().as_u16() {
        200 => response.body_mut().read_json().map_err(|e| e.to_string())?,
        404 => return Ok(None),
        // Without a sign-in, GitHub's API allows 60 calls an hour per network address, shared
        // by every program behind it; past that it answers 403 (or 429). The release page
        // isn't counted against that, and names the newest release too.
        code @ (403 | 429) => latest_from_page()
            .map_err(|e| format!("GitHub is limiting update checks from this network ({code}), and its release page didn't answer: {e}"))?,
        code => return Err(format!("GitHub answered {code}")),
    };
    newer_release(&release, current_version(), release_key().is_some())
}

/// The newest release as github.com/<repo>/releases/latest names it (it redirects to the
/// release's tag), with the files every release carries. Null when there's none to see.
fn latest_from_page() -> Result<Value, String> {
    let repo = repo();
    let url = format!("https://github.com/{repo}/releases/latest");
    let response = http().get(&url).config().max_redirects(0).build().call().map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let location = response.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or_default();
    match (status, tag_in(location)) {
        (300..=399, Some(tag)) => Ok(release_at(&repo, tag)),
        // No release yet (the page sends you to the empty list), or no such repository.
        (300..=399, None) | (404, _) => Ok(Value::Null),
        _ => Err(format!("it answered {status}")),
    }
}

/// The tag in a release page's address: ".../releases/tag/v0.9.1" -> "v0.9.1".
fn tag_in(location: &str) -> Option<&str> {
    let (_, rest) = location.split_once("/releases/tag/")?;
    let tag = rest.split(['?', '#']).next().unwrap_or_default();
    (!tag.is_empty() && !tag.contains('/')).then_some(tag)
}

/// A release as the API would describe it, from its tag alone: its files are where every
/// release keeps them. One missing (say, a signature) shows when the update downloads it.
fn release_at(repo: &str, tag: &str) -> Value {
    let assets: Vec<Value> = [EXE, SHA, SIG]
        .iter()
        .map(|name| serde_json::json!({ "name": name, "browser_download_url": format!("https://github.com/{repo}/releases/download/{tag}/{name}") }))
        .collect();
    serde_json::json!({ "tag_name": tag, "assets": assets })
}

/// The release if it is newer than `current`. A newer release without its checksum (or,
/// when `signed`, its signature) is an error, not a quiet "up to date": it can't be installed.
fn newer_release(release: &Value, current: &str, signed: bool) -> Result<Option<Release>, String> {
    let Some(version) = release["tag_name"].as_str().map(|tag| tag.trim_start_matches('v').to_string()) else {
        return Ok(None);
    };
    if release["draft"] == true || release["prerelease"] == true || !compare_versions(&version, current).is_gt() {
        return Ok(None);
    }
    let assets = release["assets"].as_array().map(Vec::as_slice).unwrap_or_default();
    let url = |name: &str| {
        assets
            .iter()
            .find(|a| a["name"] == name)
            .and_then(|a| a["browser_download_url"].as_str())
            .map(str::to_string)
            .ok_or_else(|| format!("release {version} has no {name}, so it can't be installed"))
    };
    let sig_url = if signed { Some(url(SIG)?) } else { url(SIG).ok() };
    Ok(Some(Release { exe_url: url(EXE)?, sha_url: url(SHA)?, sig_url, version }))
}

/// How far an update has got, for the tray.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// The percentage downloaded, when the server says how big the file is.
    Downloading(Option<u8>),
    Checking,
    Installing,
}

fn download(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    download_with_progress(url, limit, &|_| {})
}

fn download_with_progress(url: &str, limit: u64, progress: &dyn Fn(Option<u8>)) -> Result<Vec<u8>, String> {
    let mut response = http().get(url).call().map_err(|e| format!("download failed: {e}"))?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(format!("download failed: {status}"));
    }
    let total = response.body().content_length();
    read_counting(response.body_mut().with_config().limit(limit).reader(), total, progress).map_err(|e| format!("download failed: {e}"))
}

/// Reads all of `reader`, telling `progress` the percentage of `total` (when known) read so
/// far, each time it changes; once at the start either way.
fn read_counting(mut reader: impl io::Read, total: Option<u64>, progress: &dyn Fn(Option<u8>)) -> io::Result<Vec<u8>> {
    let total = total.filter(|&t| t > 0);
    let mut data = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    let mut shown = total.map(|_| 0u8);
    progress(shown);
    loop {
        let read = match reader.read(&mut chunk) {
            Ok(0) => return Ok(data),
            Ok(read) => read,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        data.extend_from_slice(&chunk[..read]);
        let percent = total.map(|t| (data.len() as u64 * 100 / t).min(100) as u8);
        if percent != shown {
            shown = percent;
            progress(percent);
        }
    }
}

/// Checks the signature file (the 64-byte Ed25519 signature in base64) over `data`.
fn check_signature(data: &[u8], sig_file: &[u8], public_key: &[u8; 32]) -> Result<(), String> {
    let malformed = || "the release's signature file is malformed".to_string();
    let text = std::str::from_utf8(sig_file).map_err(|_| malformed())?.trim();
    let signature = STANDARD.decode(text).map_err(|_| malformed())?;
    if signature.len() != 64 {
        return Err(malformed());
    }
    UnparsedPublicKey::new(&ED25519, public_key)
        .verify(data, &signature)
        .map_err(|_| "the release isn't signed with the T3 WoW release key".to_string())
}

/// The SHA-256 in a checksum file ("<64 hex digits>  t3wow.exe"), lowercase.
fn parse_checksum(sha_file: &[u8]) -> Result<String, String> {
    let malformed = || "the release's checksum file is malformed".to_string();
    let text = std::str::from_utf8(sha_file).map_err(|_| malformed())?;
    let mut words = text.split_whitespace();
    match (words.next(), words.next(), words.next()) {
        (Some(hash), None | Some(EXE) | Some("*t3wow.exe"), None) if hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) => {
            Ok(hash.to_ascii_lowercase())
        }
        _ => Err(malformed()),
    }
}

/// Checks the checksum file's signature first, then returns the SHA-256 it vouches for.
fn signed_checksum(sha_file: &[u8], sig_file: &[u8], public_key: &[u8; 32]) -> Result<String, String> {
    check_signature(sha_file, sig_file, public_key)?;
    parse_checksum(sha_file)
}

/// The SHA-256 a release vouches for: from its signed checksum file when the companion has
/// a release key, else from the checksum file alone.
fn trusted_checksum(sha_file: &[u8], sig_file: Option<&[u8]>, key: Option<&[u8; 32]>) -> Result<String, String> {
    match key {
        Some(key) => signed_checksum(sha_file, sig_file.ok_or("the release isn't signed, so it can't be installed")?, key),
        None => parse_checksum(sha_file),
    }
}

/// Checks a downloaded exe: a Windows program with the release's SHA-256.
fn check_exe(exe: &[u8], expected_sha: &str) -> Result<(), String> {
    if exe.len() < 64 * 1024 || !exe.starts_with(b"MZ") {
        return Err("the download is not a Windows program".into());
    }
    if hex(&Sha256::digest(exe)) != expected_sha {
        return Err("the download doesn't match the release's checksum".into());
    }
    Ok(())
}

/// Downloads the release, checks it, and swaps it in for the running exe (Windows lets a
/// running exe be renamed, not overwritten). The caller restarts the companion. `step`
/// hears how far it has got.
pub fn install(release: &Release, step: &dyn Fn(Step)) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    update_exe(
        &exe,
        &release.version,
        || {
            let key = release_key();
            let sha_file = download(&release.sha_url, MAX_SMALL_FILE_BYTES)?;
            let sig_file = match (key, &release.sig_url) {
                (Some(_), Some(url)) => Some(download(url, MAX_SMALL_FILE_BYTES)?),
                _ => None,
            };
            let expected = trusted_checksum(&sha_file, sig_file.as_deref(), key)?;
            let exe_bytes = download_with_progress(&release.exe_url, MAX_EXE_BYTES, &|percent| step(Step::Downloading(percent)))?;
            step(Step::Checking);
            check_exe(&exe_bytes, &expected)?;
            step(Step::Installing);
            Ok(exe_bytes)
        },
        |from, to| fs::rename(from, to),
    )
}

/// One whole update under the update lock, from download to swap: a second update (the
/// tray and `t3wow.exe update` at once) stops at the lock, and one that runs after another
/// stops when the exe on disk is already that version or newer, so a slow, older update
/// can't replace a newer one. `fetch` downloads and checks the new exe.
fn update_exe(
    exe: &Path,
    version: &str,
    fetch: impl FnOnce() -> Result<Vec<u8>, String>,
    rename: impl Fn(&Path, &Path) -> io::Result<()>,
) -> Result<(), String> {
    let _lock = try_lock(exe)?;
    if let Some(installed) = installed_version(exe).filter(|v| !compare_versions(version, v).is_gt()) {
        return Err(format!("already updated to {installed}: restart T3 WoW (Quit from its tray menu, then start it again)"));
    }
    let bytes = fetch()?;
    swap_files(exe, &bytes, rename)?;
    // What is now on disk, tied to the exe's modified time: an exe replaced by hand later
    // doesn't match, so its record no longer counts.
    if let Some(modified) = modified_ms(exe) {
        let _ = fs::write(sibling(exe, "version"), format!("{version}\n{modified}\n"));
    }
    Ok(())
}

fn modified_ms(exe: &Path) -> Option<u128> {
    fs::metadata(exe).ok()?.modified().ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_millis())
}

/// The version an update put on disk, while the exe is still the file it wrote.
fn installed_version(exe: &Path) -> Option<String> {
    let text = fs::read_to_string(sibling(exe, "version")).ok()?;
    let mut lines = text.lines();
    let (version, modified) = (lines.next()?, lines.next()?.parse::<u128>().ok()?);
    (modified_ms(exe) == Some(modified)).then(|| version.to_string())
}

/// `<exe name>.<suffix>`, next to the exe.
fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    exe.with_file_name(format!("{name}.{suffix}"))
}

/// The update lock: held by whoever is swapping exes (the tray, or `t3wow.exe update`).
fn try_lock(exe: &Path) -> Result<File, String> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(sibling(exe, "update-lock"))
        .map_err(|e| format!("can't create the update lock next to the exe: {e}"))?;
    match lock.try_lock() {
        Ok(()) => Ok(lock),
        Err(TryLockError::WouldBlock) => Err("another update is already running".into()),
        Err(TryLockError::Error(e)) => Err(format!("can't take the update lock: {e}")),
    }
}

/// Puts `bytes` in place of `exe` (the caller holds the update lock). The new exe is staged
/// under a name of its own, and the previous exe is kept until the new one is in place, so a
/// failed swap puts it back. After a swap, the previous exe stays (a running exe can't be
/// deleted) until `clean_up` on the next start. `rename` is fs::rename, or one that fails,
/// in tests.
fn swap_files(exe: &Path, bytes: &[u8], rename: impl Fn(&Path, &Path) -> io::Result<()>) -> Result<(), String> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let tag = format!("{}-{nanos}", std::process::id());
    let new = sibling(exe, &format!("new-{tag}"));
    let old = sibling(exe, &format!("old-{tag}"));
    if let Err(error) = fs::write(&new, bytes) {
        let _ = fs::remove_file(&new);
        return Err(format!("can't write the update next to the exe: {error}"));
    }
    // fs::write makes an ordinary file: without this the updated program wouldn't start.
    #[cfg(not(windows))]
    if let Err(error) = make_executable(&new) {
        let _ = fs::remove_file(&new);
        return Err(format!("can't make the update executable: {error}"));
    }
    if let Err(error) = rename(exe, &old) {
        let _ = fs::remove_file(&new);
        return Err(format!("can't move the running exe aside: {error}"));
    }
    if let Err(error) = rename(&new, exe) {
        let restored = rename(&old, exe);
        let _ = fs::remove_file(&new);
        return Err(match restored {
            Ok(()) => format!("can't put the update in place: {error}"),
            Err(again) => format!("can't put the update in place ({error}) nor the old exe back ({again}); it is at {}", old.display()),
        });
    }
    Ok(())
}

#[cfg(not(windows))]
fn make_executable(file: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(file, fs::Permissions::from_mode(0o755))
}

/// Removes the previous exes and unfinished downloads earlier updates left behind.
pub fn clean_up() {
    if let Ok(exe) = std::env::current_exe() {
        clean_up_at(&exe);
    }
}

/// Skipped while an update runs: its files aren't leftovers yet.
fn clean_up_at(exe: &Path) {
    let (Ok(_lock), Some(dir), Some(name)) = (try_lock(exe), exe.parent(), exe.file_name()) else { return };
    let name = name.to_string_lossy();
    let (old, new) = (format!("{name}.old"), format!("{name}.new"));
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        if file.starts_with(&old) || file.starts_with(&new) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    fn release(tag: &str) -> Value {
        json!({
            "tag_name": tag, "draft": false, "prerelease": false,
            "assets": [
                { "name": "T3WoW-v9.zip", "browser_download_url": "https://x/zip" },
                { "name": "t3wow.exe", "browser_download_url": "https://x/exe" },
                { "name": "t3wow.exe.sha256", "browser_download_url": "https://x/sha" },
                { "name": "t3wow.exe.sha256.sig", "browser_download_url": "https://x/sig" },
            ],
        })
    }

    #[test]
    fn only_a_newer_signed_release_counts() {
        assert_eq!(
            newer_release(&release("v0.4.0"), "0.3.0", true),
            Ok(Some(Release {
                version: "0.4.0".into(),
                exe_url: "https://x/exe".into(),
                sha_url: "https://x/sha".into(),
                sig_url: Some("https://x/sig".into()),
            }))
        );
        assert_eq!(newer_release(&release("v0.3.0"), "0.3.0", true), Ok(None));
        assert_eq!(newer_release(&release("v0.2.9"), "0.3.0", true), Ok(None));
        let mut pre = release("v1.0.0");
        pre["prerelease"] = json!(true);
        assert_eq!(newer_release(&pre, "0.3.0", true), Ok(None));
        // A newer release missing any of its files can't be installed, and says so.
        for missing in [EXE, SHA, SIG] {
            let mut partial = release("v1.0.0");
            partial["assets"].as_array_mut().unwrap().retain(|a| a["name"] != missing);
            assert_eq!(newer_release(&partial, "0.3.0", true), Err(format!("release 1.0.0 has no {missing}, so it can't be installed")));
        }
    }

    #[test]
    fn a_test_build_compares_as_the_release_it_was_built_from() {
        assert_eq!(newer_release(&release("v0.9.1"), "0.9.1-dev+57ee2ec", true), Ok(None));
        assert_eq!(newer_release(&release("v0.9.1"), "0.9.1-dev+57ee2ec.dirty", true), Ok(None));
        assert_eq!(newer_release(&release("v0.9.2"), "0.9.1-dev+57ee2ec", true).unwrap().unwrap().version, "0.9.2");
        assert_eq!(up_to_date_for("0.9.1"), "You have 0.9.1, the newest release.");
        assert_eq!(up_to_date_for("0.9.1-dev+57ee2ec"), "You have 0.9.1-dev+57ee2ec, a test build: no release is newer than 0.9.1.");
    }

    #[test]
    fn the_release_page_names_the_newest_release_when_the_api_is_rate_limited() {
        assert_eq!(tag_in("https://github.com/o/r/releases/tag/v0.9.1"), Some("v0.9.1"));
        assert_eq!(tag_in("/o/r/releases/tag/v0.9.1?x=1"), Some("v0.9.1"));
        assert_eq!(tag_in("https://github.com/o/r/releases"), None);
        assert_eq!(tag_in(""), None);
        assert_eq!(
            newer_release(&release_at("o/r", "v1.0.0"), "0.9.1", true),
            Ok(Some(Release {
                version: "1.0.0".into(),
                exe_url: "https://github.com/o/r/releases/download/v1.0.0/t3wow.exe".into(),
                sha_url: "https://github.com/o/r/releases/download/v1.0.0/t3wow.exe.sha256".into(),
                sig_url: Some("https://github.com/o/r/releases/download/v1.0.0/t3wow.exe.sha256.sig".into()),
            }))
        );
        assert_eq!(newer_release(&release_at("o/r", "v0.9.1"), "0.9.1", true), Ok(None));
        assert_eq!(newer_release(&Value::Null, "0.9.1", true), Ok(None));
    }

    /// The real release page, as the fallback reads it, against the API. Needs the network:
    /// `cargo test updater::tests::live -- --ignored`.
    #[test]
    #[ignore = "needs the network"]
    fn live_the_release_page_names_the_same_release_as_the_api() {
        let page = latest_from_page().unwrap();
        let mut response = http().get(&format!("https://api.github.com/repos/{REPO}/releases/latest")).call().unwrap();
        assert_eq!(response.status().as_u16(), 200, "the API is rate limited right now: try later");
        let api: Value = response.body_mut().read_json().unwrap();
        assert_eq!(page["tag_name"], api["tag_name"]);
        for asset in page["assets"].as_array().unwrap() {
            let listed = api["assets"].as_array().unwrap().iter().any(|a| a["browser_download_url"] == asset["browser_download_url"]);
            assert!(listed || asset["name"] == SIG, "{asset} isn't in the release");
        }
    }

    #[test]
    fn before_signing_is_set_up_a_release_needs_its_checksum_but_no_signature() {
        let mut unsigned = release("v1.0.0");
        unsigned["assets"].as_array_mut().unwrap().retain(|a| a["name"] != SIG);
        let found = newer_release(&unsigned, "0.3.0", false).unwrap().unwrap();
        assert_eq!(found.sig_url, None);
        assert_eq!(newer_release(&release("v1.0.0"), "0.3.0", false).unwrap().unwrap().sig_url, Some("https://x/sig".into()));
        for missing in [EXE, SHA] {
            let mut partial = release("v1.0.0");
            partial["assets"].as_array_mut().unwrap().retain(|a| a["name"] != missing);
            assert!(newer_release(&partial, "0.3.0", false).is_err(), "{missing}");
        }

        let exe = exe();
        let sha = format!("{}  t3wow.exe", hex(&Sha256::digest(&exe))).into_bytes();
        // No key: the checksum file alone, which still has to be well formed and match.
        let expected = trusted_checksum(&sha, None, None).unwrap();
        assert!(check_exe(&exe, &expected).is_ok());
        assert!(trusted_checksum(b"not a checksum", None, None).is_err());
        // With a key: a signature is required, and must verify.
        let (pair, public) = key_pair();
        assert!(trusted_checksum(&sha, None, Some(&public)).is_err());
        assert_eq!(trusted_checksum(&sha, Some(&sig_file(&pair, &sha)), Some(&public)), Ok(expected));
        let (stranger, _) = key_pair();
        assert!(trusted_checksum(&sha, Some(&sig_file(&stranger, &sha)), Some(&public)).is_err());
    }

    fn exe() -> Vec<u8> {
        let mut exe = b"MZ".to_vec();
        exe.resize(70_000, 0);
        exe
    }

    fn key_pair() -> (Ed25519KeyPair, [u8; 32]) {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let public: [u8; 32] = pair.public_key().as_ref().try_into().unwrap();
        (pair, public)
    }

    fn sig_file(pair: &Ed25519KeyPair, data: &[u8]) -> Vec<u8> {
        format!("{}\n", STANDARD.encode(pair.sign(data))).into_bytes()
    }

    #[test]
    fn a_release_installs_only_with_a_valid_signature_and_checksum() {
        let (pair, public) = key_pair();
        let exe = exe();
        let sha = format!("{}  t3wow.exe", hex(&Sha256::digest(&exe))).into_bytes();
        let sig = sig_file(&pair, &sha);
        let expected = signed_checksum(&sha, &sig, &public).unwrap();
        assert!(check_exe(&exe, &expected).is_ok());

        // Another exe, or anything that isn't a program, doesn't match.
        let mut other = exe.clone();
        other[100] = 1;
        assert!(check_exe(&other, &expected).is_err());
        assert!(check_exe(b"<html>not found</html>", &expected).is_err());

        // Signed with another key, or the checksum changed after signing.
        let (stranger, _) = key_pair();
        assert!(signed_checksum(&sha, &sig_file(&stranger, &sha), &public).is_err());
        let mut forged = sha.clone();
        forged[0] = if forged[0] == b'0' { b'1' } else { b'0' };
        assert!(signed_checksum(&forged, &sig, &public).is_err());

        // Missing or malformed signature files.
        assert!(signed_checksum(&sha, b"", &public).is_err());
        assert!(signed_checksum(&sha, b"not base64!", &public).is_err());
        assert!(signed_checksum(&sha, STANDARD.encode([7u8; 63]).as_bytes(), &public).is_err());
        assert!(signed_checksum(&sha, &sig, &[0; 32]).is_err());

        // A validly signed but malformed checksum file still fails.
        for bad in [&b""[..], b"deadbeef  t3wow.exe", b"<html>", &sha[..63]] {
            assert!(signed_checksum(bad, &sig_file(&pair, bad), &public).is_err(), "{}", String::from_utf8_lossy(bad));
        }
        let other_name = format!("{}  other.exe", hex(&Sha256::digest(&exe))).into_bytes();
        assert!(signed_checksum(&other_name, &sig_file(&pair, &other_name), &public).is_err());
    }

    /// The release workflow signs with Node (scripts/sign-release.mjs); this fixture, signed
    /// that way with a throwaway key, must verify here too.
    #[test]
    fn a_checksum_signed_by_the_release_script_verifies() {
        let fixture: Value = serde_json::from_str(include_str!("../tests/fixtures/signed-release.json")).unwrap();
        let hex_key = fixture["publicKey"].as_str().unwrap();
        let public: [u8; 32] = (0..32).map(|i| u8::from_str_radix(&hex_key[i * 2..i * 2 + 2], 16).unwrap()).collect::<Vec<_>>().try_into().unwrap();
        let sha = fixture["checksum"].as_str().unwrap().as_bytes();
        let sig = fixture["signature"].as_str().unwrap().as_bytes();
        let expected = signed_checksum(sha, sig, &public).unwrap();
        assert!(check_exe(&exe(), &expected).is_ok());
    }

    fn exe_dir(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("t3wow-swap-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("t3wow.exe");
        fs::write(&exe, b"old exe").unwrap();
        (dir, exe)
    }

    /// The files next to the exe, other than the exe and the lock.
    fn leftovers(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.retain(|n| n != "t3wow.exe" && n != "t3wow.exe.update-lock" && n != "t3wow.exe.version");
        names.sort();
        names
    }

    #[test]
    fn a_swap_puts_the_new_exe_in_place_and_keeps_the_old_one_until_the_next_start() {
        let (dir, exe) = exe_dir("ok");
        update_exe(&exe, "9.0.0", || Ok(b"new exe".to_vec()), |from, to| fs::rename(from, to)).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"new exe");
        let left = leftovers(&dir);
        assert_eq!(left.len(), 1);
        assert!(left[0].starts_with("t3wow.exe.old-"));
        assert_eq!(fs::read(dir.join(&left[0])).unwrap(), b"old exe");
        fs::write(dir.join("t3wow.exe.new"), b"from 0.4.1").unwrap();
        clean_up_at(&exe);
        assert!(leftovers(&dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(not(windows))]
    #[test]
    fn an_updated_program_can_be_started() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, exe) = exe_dir("executable");
        update_exe(&exe, "9.0.0", || Ok(b"new exe".to_vec()), |from, to| fs::rename(from, to)).unwrap();
        assert_eq!(fs::metadata(&exe).unwrap().permissions().mode() & 0o777, 0o755);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_swap_puts_the_old_exe_back() {
        let (dir, exe) = exe_dir("rollback");
        let error = update_exe(&exe, "9.0.0", || Ok(b"new exe".to_vec()), |from, to| {
            if to == exe && from.to_string_lossy().contains(".new-") {
                return Err(io::Error::other("in use"));
            }
            fs::rename(from, to)
        })
        .unwrap_err();
        assert_eq!(error, "can't put the update in place: in use");
        assert_eq!(fs::read(&exe).unwrap(), b"old exe");
        assert!(leftovers(&dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_update_at_a_time() {
        let (dir, exe) = exe_dir("lock");
        let held = try_lock(&exe).unwrap();
        let fetched = std::cell::Cell::new(false);
        let fetch = || {
            fetched.set(true);
            Ok(b"new exe".to_vec())
        };
        assert_eq!(update_exe(&exe, "9.0.0", fetch, |from, to| fs::rename(from, to)), Err("another update is already running".into()));
        assert!(!fetched.get(), "a second update stops before downloading");
        assert_eq!(fs::read(&exe).unwrap(), b"old exe");
        // Clean-up leaves a running update's files alone.
        fs::write(dir.join("t3wow.exe.new-1-2"), b"downloading").unwrap();
        clean_up_at(&exe);
        assert_eq!(leftovers(&dir), ["t3wow.exe.new-1-2"]);
        drop(held);
        update_exe(&exe, "9.0.0", || Ok(b"new exe".to_vec()), |from, to| fs::rename(from, to)).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"new exe");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_update_after_another_never_installs_an_older_or_the_same_version() {
        let (dir, exe) = exe_dir("order");
        let rename = |from: &Path, to: &Path| fs::rename(from, to);
        update_exe(&exe, "0.6.0", || Ok(b"exe 0.6.0".to_vec()), rename).unwrap();
        let older = update_exe(&exe, "0.5.0", || Ok(b"exe 0.5.0".to_vec()), rename);
        assert!(older.unwrap_err().starts_with("already updated to 0.6.0"));
        assert!(update_exe(&exe, "0.6.0", || Ok(b"again".to_vec()), rename).is_err());
        assert_eq!(fs::read(&exe).unwrap(), b"exe 0.6.0");
        update_exe(&exe, "0.7.0", || Ok(b"exe 0.7.0".to_vec()), rename).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"exe 0.7.0");
        // An exe replaced by hand (another modified time): the record no longer counts.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&exe, b"exe 0.4.1, by hand").unwrap();
        update_exe(&exe, "0.6.0", || Ok(b"exe 0.6.0".to_vec()), rename).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"exe 0.6.0");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_download_reports_its_progress_as_it_grows() {
        let seen = std::cell::RefCell::new(Vec::new());
        let data = vec![7u8; 200 * 1024];
        let read = read_counting(io::Cursor::new(data.clone()), Some(data.len() as u64), &|p| seen.borrow_mut().push(p)).unwrap();
        assert_eq!(read, data);
        let seen = seen.into_inner();
        assert_eq!(seen.first(), Some(&Some(0)));
        assert_eq!(seen.last(), Some(&Some(100)));
        assert!(seen.windows(2).all(|w| w[0] < w[1]), "each report is new and larger: {seen:?}");

        // No size given: one report without a percentage.
        let seen = std::cell::RefCell::new(Vec::new());
        read_counting(io::Cursor::new(vec![1u8; 10]), None, &|p| seen.borrow_mut().push(p)).unwrap();
        assert_eq!(seen.into_inner(), [None]);
        // A size that turns out too small never reports past 100.
        let seen = std::cell::RefCell::new(Vec::new());
        read_counting(io::Cursor::new(vec![1u8; 100]), Some(10), &|p| seen.borrow_mut().push(p)).unwrap();
        assert_eq!(seen.into_inner(), [Some(0), Some(100)]);
    }

    /// Fails while RELEASE_PUBLIC_KEY is the placeholder. Ignored so everyday tests pass
    /// before the real key is in; run it with `cargo test -- --ignored` once you've embedded
    /// the key.
    #[test]
    #[ignore = "fails until the real release public key replaces the placeholder"]
    fn the_release_public_key_is_set() {
        assert_ne!(RELEASE_PUBLIC_KEY, [0; 32], "RELEASE_PUBLIC_KEY is still the placeholder");
    }
}
