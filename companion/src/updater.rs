//! Updates from the project's GitHub releases. Each release carries t3wow.exe, its
//! SHA-256 (t3wow.exe.sha256) and an Ed25519 signature of that checksum file
//! (t3wow.exe.sha256.sig), made in CI with the release key (see
//! .github/workflows/release.yml and scripts/sign-release.mjs). An update installs only
//! when the signature verifies with the key below and the exe matches the checksum. The
//! addon is embedded in the exe, so updating the companion also updates the addon on its
//! next start.

use crate::slots::compare_versions;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ring::signature::{UnparsedPublicKey, ED25519};
use serde_json::Value;
use sha2::{Digest, Sha256};

use std::time::Duration;

// Releases live in their own public repository while the source stays private.
const REPO: &str = "MatiasCiccone/t3wow-releases";
const MAX_EXE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SMALL_FILE_BYTES: u64 = 4096;
const EXE: &str = "t3wow.exe";
const SHA: &str = "t3wow.exe.sha256";
const SIG: &str = "t3wow.exe.sha256.sig";

/// The public half of the release key: `node scripts/gen-signing-key.mjs` prints it.
// REPLACE before release: this all-zero placeholder verifies nothing, so every update is
// refused until the real key is here (the release workflow refuses to sign until then).
const RELEASE_PUBLIC_KEY: [u8; 32] = [0; 32];

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub exe_url: String,
    pub sha_url: String,
    pub sig_url: String,
}

fn repo() -> String {
    std::env::var("T3WOW_UPDATE_REPO").unwrap_or_else(|_| REPO.to_string())
}

fn http() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(120)))
            .user_agent(format!("t3wow-companion/{}", env!("CARGO_PKG_VERSION")))
            .http_status_as_error(false)
            .build(),
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The newest release if it is newer than this companion. Ok(None): up to date, or no
/// release can be seen (a private repository looks the same as one without releases).
pub fn check() -> Result<Option<Release>, String> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo());
    let mut response = http().get(&url).header("accept", "application/vnd.github+json").call().map_err(|e| e.to_string())?;
    match response.status().as_u16() {
        200 => {}
        404 => return Ok(None),
        code => return Err(format!("GitHub answered {code}")),
    }
    let release: Value = response.body_mut().read_json().map_err(|e| e.to_string())?;
    newer_release(&release, current_version())
}

/// The release if it is newer than `current`. A newer release without its checksum or
/// signature is an error, not a quiet "up to date": it can't be installed.
fn newer_release(release: &Value, current: &str) -> Result<Option<Release>, String> {
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
    Ok(Some(Release { exe_url: url(EXE)?, sha_url: url(SHA)?, sig_url: url(SIG)?, version }))
}

fn download(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut response = http().get(url).call().map_err(|e| format!("download failed: {e}"))?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(format!("download failed: {status}"));
    }
    response.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| format!("download failed: {e}"))
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

/// Checks a downloaded exe: a Windows program with the signed SHA-256.
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
/// running exe be renamed, not overwritten). The caller restarts the companion.
pub fn install(release: &Release) -> Result<(), String> {
    let sha_file = download(&release.sha_url, MAX_SMALL_FILE_BYTES)?;
    let sig_file = download(&release.sig_url, MAX_SMALL_FILE_BYTES)?;
    let expected = signed_checksum(&sha_file, &sig_file, &RELEASE_PUBLIC_KEY)?;
    let exe_bytes = download(&release.exe_url, MAX_EXE_BYTES)?;
    check_exe(&exe_bytes, &expected)?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let new = exe.with_extension("exe.new");
    let old = exe.with_extension("exe.old");
    std::fs::write(&new, &exe_bytes).map_err(|e| format!("can't write the update next to the exe: {e}"))?;
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|e| format!("can't move the running exe aside: {e}"))?;
    if let Err(error) = std::fs::rename(&new, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(format!("can't put the update in place: {error}"));
    }
    Ok(())
}

/// Removes the previous exe an update left behind.
pub fn clean_up() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("exe.old"));
        let _ = std::fs::remove_file(exe.with_extension("exe.new"));
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
            newer_release(&release("v0.4.0"), "0.3.0"),
            Ok(Some(Release {
                version: "0.4.0".into(),
                exe_url: "https://x/exe".into(),
                sha_url: "https://x/sha".into(),
                sig_url: "https://x/sig".into(),
            }))
        );
        assert_eq!(newer_release(&release("v0.3.0"), "0.3.0"), Ok(None));
        assert_eq!(newer_release(&release("v0.2.9"), "0.3.0"), Ok(None));
        let mut pre = release("v1.0.0");
        pre["prerelease"] = json!(true);
        assert_eq!(newer_release(&pre, "0.3.0"), Ok(None));
        // A newer release missing any of its files can't be installed, and says so.
        for missing in [EXE, SHA, SIG] {
            let mut partial = release("v1.0.0");
            partial["assets"].as_array_mut().unwrap().retain(|a| a["name"] != missing);
            assert_eq!(newer_release(&partial, "0.3.0"), Err(format!("release 1.0.0 has no {missing}, so it can't be installed")));
        }
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

    /// Fails while RELEASE_PUBLIC_KEY is the placeholder. Ignored so everyday tests pass
    /// before the real key is in; run it with `cargo test -- --ignored` before a release
    /// (the release workflow's signing step checks the same thing).
    #[test]
    #[ignore = "fails until the real release public key replaces the placeholder"]
    fn the_release_public_key_is_set() {
        assert_ne!(RELEASE_PUBLIC_KEY, [0; 32], "RELEASE_PUBLIC_KEY is still the placeholder");
    }
}
