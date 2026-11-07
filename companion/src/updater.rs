//! Updates from the project's GitHub releases. Each release carries t3wow.exe and
//! t3wow.exe.sha256 (see .github/workflows/release.yml). The addon is embedded in the exe,
//! so updating the companion also updates the addon on its next start.

use crate::slots::compare_versions;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::time::Duration;

const REPO: &str = "MatiasCiccone/t3wow";
const MAX_EXE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub exe_url: String,
    pub sha_url: Option<String>,
}

fn repo() -> String {
    std::env::var("T3WOW_UPDATE_REPO").unwrap_or_else(|_| REPO.to_string())
}

fn http() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .user_agent(&format!("t3wow-companion/{}", env!("CARGO_PKG_VERSION")))
        .build()
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The newest release if it is newer than this companion. Ok(None): up to date, or no
/// release can be seen (a private repository looks the same as one without releases).
pub fn check() -> Result<Option<Release>, String> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo());
    let release: Value = match http().get(&url).set("accept", "application/vnd.github+json").call() {
        Ok(response) => response.into_json().map_err(|e| e.to_string())?,
        Err(ureq::Error::Status(404, _)) => return Ok(None),
        Err(ureq::Error::Status(code, _)) => return Err(format!("GitHub answered {code}")),
        Err(error) => return Err(error.to_string()),
    };
    Ok(newer_release(&release, current_version()))
}

fn newer_release(release: &Value, current: &str) -> Option<Release> {
    let version = release["tag_name"].as_str()?.trim_start_matches('v').to_string();
    if release["draft"] == true || release["prerelease"] == true || !compare_versions(&version, current).is_gt() {
        return None;
    }
    let assets = release["assets"].as_array()?;
    let url = |name: &str| assets.iter().find(|a| a["name"] == name).and_then(|a| a["browser_download_url"].as_str()).map(str::to_string);
    Some(Release { version, exe_url: url("t3wow.exe")?, sha_url: url("t3wow.exe.sha256") })
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    let response = http().get(url).call().map_err(|e| format!("download failed: {e}"))?;
    let mut bytes = Vec::new();
    response.into_reader().take(MAX_EXE_BYTES).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// Checks a downloaded exe: a Windows program, and the release's SHA-256 when it has one.
fn verify(exe: &[u8], sha_file: Option<&str>) -> Result<(), String> {
    if exe.len() < 64 * 1024 || !exe.starts_with(b"MZ") {
        return Err("the download is not a Windows program".into());
    }
    if let Some(sha_file) = sha_file {
        let expected = sha_file.split_whitespace().next().unwrap_or("").to_lowercase();
        let actual = format!("{:x}", Sha256::digest(exe));
        if expected != actual {
            return Err("the download doesn't match the release's checksum".into());
        }
    }
    Ok(())
}

/// Downloads the release, checks it, and swaps it in for the running exe (Windows lets a
/// running exe be renamed, not overwritten). The caller restarts the companion.
pub fn install(release: &Release) -> Result<(), String> {
    let exe_bytes = download(&release.exe_url)?;
    let sha = match &release.sha_url {
        Some(url) => Some(String::from_utf8_lossy(&download(url)?).into_owned()),
        None => None,
    };
    verify(&exe_bytes, sha.as_deref())?;
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

    fn release(tag: &str) -> Value {
        json!({
            "tag_name": tag, "draft": false, "prerelease": false,
            "assets": [
                { "name": "T3WoW-v9.zip", "browser_download_url": "https://x/zip" },
                { "name": "t3wow.exe", "browser_download_url": "https://x/exe" },
                { "name": "t3wow.exe.sha256", "browser_download_url": "https://x/sha" },
            ],
        })
    }

    #[test]
    fn only_a_newer_release_with_an_exe_counts() {
        assert_eq!(
            newer_release(&release("v0.4.0"), "0.3.0"),
            Some(Release { version: "0.4.0".into(), exe_url: "https://x/exe".into(), sha_url: Some("https://x/sha".into()) })
        );
        assert_eq!(newer_release(&release("v0.3.0"), "0.3.0"), None);
        assert_eq!(newer_release(&release("v0.2.9"), "0.3.0"), None);
        let mut no_exe = release("v1.0.0");
        no_exe["assets"] = json!([]);
        assert_eq!(newer_release(&no_exe, "0.3.0"), None);
        let mut pre = release("v1.0.0");
        pre["prerelease"] = json!(true);
        assert_eq!(newer_release(&pre, "0.3.0"), None);
    }

    #[test]
    fn downloads_are_checked() {
        let mut exe = b"MZ".to_vec();
        exe.resize(70_000, 0);
        let sha = format!("{:x}  t3wow.exe\n", Sha256::digest(&exe));
        assert!(verify(&exe, Some(&sha)).is_ok());
        assert!(verify(&exe, None).is_ok());
        assert!(verify(&exe, Some("deadbeef")).is_err());
        assert!(verify(b"<html>not found</html>", None).is_err());
    }
}
