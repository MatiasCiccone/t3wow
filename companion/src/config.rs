//! Settings and sign-in, in %APPDATA%\t3wow\config.json (the same file the earlier Node
//! companion used, so an existing sign-in carries over).

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_INTERFACE: &str = "16001";

/// The key the sign-in for the T3 on this PC is saved under, whatever port it runs on.
pub const LOCAL: &str = "local";

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_default()
}

pub fn config_dir() -> PathBuf {
    match std::env::var_os("APPDATA") {
        Some(appdata) => PathBuf::from(appdata).join("t3wow"),
        None => home().join(".config").join("t3wow"),
    }
}

pub fn t3_home() -> PathBuf {
    std::env::var_os("T3CODE_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".t3"))
}

/// One spelling per server: lowercase scheme and host, no default port, no trailing slash.
pub fn canonical_origin(origin: &str) -> String {
    let origin = origin.trim().trim_end_matches('/');
    let Some((scheme, rest)) = origin.split_once("://") else {
        return origin.to_string();
    };
    let scheme = scheme.to_ascii_lowercase();
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let mut authority = authority.to_ascii_lowercase();
    let default_port = if scheme == "https" { ":443" } else { ":80" };
    if authority.ends_with(default_port) {
        authority.truncate(authority.len() - default_port.len());
    }
    format!("{scheme}://{authority}{path}")
}

/// What is saved for one T3 server.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerAuth {
    /// The bearer token as stored: `dpapi:<base64>` on Windows (only this Windows user can
    /// read it back), plain text elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    /// Epoch milliseconds; none means the token doesn't expire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token_expires_at: Option<f64>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wow_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<String>,
    /// The T3 on another machine (`login --origin`); none means the T3 on this PC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// Sign-ins by server (see `server_key`): a token is only ever sent to the server that
    /// issued it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    servers: BTreeMap<String, ServerAuth>,
    /// Up to 0.4.1, one sign-in for whichever T3 was in use; read once and moved to `servers`.
    #[serde(default, skip_serializing)]
    token: Option<String>,
    #[serde(default, skip_serializing)]
    token_expires_at: Option<f64>,
}

impl Config {
    pub fn file() -> PathBuf {
        config_dir().join("config.json")
    }

    pub fn load() -> Config {
        match Self::read(&Self::file()) {
            // Save the moved or protected token right away, so plain text doesn't linger.
            Ok((_, true)) => Self::update(|_| {}),
            Ok((config, false)) => config,
            Err(error) => {
                warn(&format!("Couldn't read {}: {error}", Self::file().display()));
                Config::default()
            }
        }
    }

    /// The file's settings, and whether an old sign-in was moved or protected on the way.
    /// A file that isn't valid JSON is kept as config.json.bad and reads as defaults.
    fn read(file: &Path) -> io::Result<(Config, bool)> {
        let text = match fs::read_to_string(file) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((Config::default(), false)),
            Err(e) => return Err(e),
        };
        match serde_json::from_str::<Config>(&text) {
            Ok(mut config) => {
                let changed = config.migrate();
                Ok((config, changed))
            }
            Err(error) => {
                let bad = file.with_extension("json.bad");
                let kept = fs::rename(file, &bad).is_ok();
                warn(&format!(
                    "{} is damaged ({error}); {}. Settings and the T3 sign-in start over.",
                    file.display(),
                    if kept { format!("kept it as {}", bad.display()) } else { "couldn't move it aside".into() }
                ));
                Ok((Config::default(), false))
            }
        }
    }

    /// Moves a 0.4.1 sign-in under the server it was made for, and protects plain tokens.
    fn migrate(&mut self) -> bool {
        let mut changed = false;
        if let Some(token) = self.token.take() {
            let expires_at = self.token_expires_at.take();
            let key = self.server_key();
            if self.servers.get(&key).is_none_or(|s| s.token.is_none()) {
                let server = self.servers.entry(key).or_default();
                server.token = Some(protect(&token));
                server.token_expires_at = expires_at;
            }
            changed = true;
        }
        for server in self.servers.values_mut() {
            if let Some(stored) = server.token.as_deref().filter(|s| !s.starts_with(PROTECTED)) {
                let protected = protect(stored);
                if protected != stored {
                    server.token = Some(protected);
                    changed = true;
                }
            }
        }
        changed
    }

    /// Writes a temporary file and renames it over the old one, so a reader never sees half.
    fn save(&self, file: &Path) -> io::Result<()> {
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        let temp = file.with_extension("json.tmp");
        fs::write(&temp, serde_json::to_string_pretty(self).unwrap_or_default())?;
        fs::rename(&temp, file)
    }

    /// Reads the file, applies `change`, and writes it back (other fields untouched). The tray
    /// app and a `t3wow.exe login` in a terminal can both write: a lock file keeps one from
    /// undoing the other's change.
    pub fn update(change: impl FnOnce(&mut Config)) -> Config {
        Self::update_at(&Self::file(), change)
    }

    fn update_at(file: &Path, change: impl FnOnce(&mut Config)) -> Config {
        let dir = file.parent().unwrap_or(Path::new("."));
        let _ = fs::create_dir_all(dir);
        let lock = fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("config.lock"));
        let _guard = lock.ok().filter(|file| file.lock().is_ok());
        match Self::read(file) {
            Ok((mut config, _)) => {
                change(&mut config);
                if let Err(error) = config.save(file) {
                    warn(&format!("Couldn't save {}: {error}", file.display()));
                }
                config
            }
            // Unreadable but there: writing now would replace it with defaults.
            Err(error) => {
                warn(&format!("Couldn't read {}, so the change isn't saved: {error}", file.display()));
                let mut config = Config::default();
                change(&mut config);
                config
            }
        }
    }

    /// Which server the current sign-in belongs to: the saved origin, or `local`.
    pub fn server_key(&self) -> String {
        self.origin.as_deref().map(canonical_origin).unwrap_or_else(|| LOCAL.to_string())
    }

    /// The token for the current server (or `T3WOW_TOKEN`), unless it has expired.
    pub fn token(&self) -> Option<String> {
        if let Ok(token) = std::env::var("T3WOW_TOKEN") {
            return Some(token);
        }
        self.saved_token(&self.server_key())
    }

    /// The token saved for one server, unless it has expired.
    pub fn saved_token(&self, key: &str) -> Option<String> {
        let server = self.servers.get(key)?;
        let expired = server.token_expires_at.is_some_and(|at| at > 0.0 && (at as i64) < now_ms());
        server.token.as_deref().filter(|_| !expired).and_then(unprotect)
    }

    pub fn set_sign_in(&mut self, key: &str, token: &str, expires_at: Option<f64>) {
        let server = self.servers.entry(key.to_string()).or_default();
        server.token = Some(protect(token));
        server.token_expires_at = expires_at;
    }

    /// Forgets the current server's sign-in.
    pub fn sign_out(&mut self) {
        if let Some(server) = self.servers.get_mut(&self.server_key()) {
            server.token = None;
            server.token_expires_at = None;
        }
    }

    pub fn interface(&self) -> String {
        self.interface.clone().unwrap_or_else(|| DEFAULT_INTERFACE.to_string())
    }
}

/// To the log, and to the console when a command runs in one.
fn warn(message: &str) {
    crate::log::line(message);
    eprintln!("{message}");
}

const PROTECTED: &str = "dpapi:";

/// A token as stored. On Windows, encrypted for this Windows user (DPAPI); if that fails,
/// or elsewhere, as it is.
fn protect(token: &str) -> String {
    #[cfg(windows)]
    if let Some(blob) = dpapi::protect(token.as_bytes()) {
        return format!("{PROTECTED}{}", STANDARD.encode(blob));
    }
    token.to_string()
}

/// A stored token back as text; none if it can't be decrypted (another user or PC).
fn unprotect(stored: &str) -> Option<String> {
    let Some(encoded) = stored.strip_prefix(PROTECTED) else {
        return Some(stored.to_string());
    };
    let blob = STANDARD.decode(encoded).ok()?;
    #[cfg(windows)]
    let plain = dpapi::unprotect(&blob);
    #[cfg(not(windows))]
    let plain: Option<Vec<u8>> = {
        let _ = blob;
        None
    };
    plain.and_then(|bytes| String::from_utf8(bytes).ok())
}

#[cfg(windows)]
mod dpapi {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
    }

    /// Copies the output out and frees the buffer Windows allocated for it.
    unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData.cast());
        bytes
    }

    pub fn protect(data: &[u8]) -> Option<Vec<u8>> {
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = unsafe {
            CryptProtectData(&blob(data), std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        };
        (ok != 0).then(|| unsafe { take(out) })
    }

    pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = unsafe {
            CryptUnprotectData(
                &blob(data),
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        (ok != 0).then(|| unsafe { take(out) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("t3wow-config-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("config.json")
    }

    fn signed_in(key: &str, token: &str, expires_at: Option<f64>) -> Config {
        let mut config = Config::default();
        config.set_sign_in(key, token, expires_at);
        config
    }

    #[test]
    fn origins_have_one_spelling() {
        assert_eq!(canonical_origin("HTTP://Box.Lan:3773/"), "http://box.lan:3773");
        assert_eq!(canonical_origin("https://t3.example.com:443"), "https://t3.example.com");
        assert_eq!(canonical_origin("http://10.0.0.2:80/t3/"), "http://10.0.0.2/t3");
    }

    #[test]
    fn tokens_expire_only_when_they_say_so() {
        let now = now_ms() as f64;
        assert_eq!(signed_in(LOCAL, "t", None).saved_token(LOCAL).as_deref(), Some("t"));
        assert_eq!(signed_in(LOCAL, "t", Some(now + 60_000.0)).saved_token(LOCAL).as_deref(), Some("t"));
        assert_eq!(signed_in(LOCAL, "t", Some(now - 1.0)).saved_token(LOCAL), None);
    }

    #[test]
    fn a_token_belongs_to_its_server() {
        let mut config = signed_in("http://a:1", "token-a", None);
        config.origin = Some("http://a:1".into());
        assert_eq!(config.saved_token(&config.server_key()).as_deref(), Some("token-a"));
        // Pointed at B, the saved token for A isn't B's.
        config.origin = Some("HTTP://B:2/".into());
        assert_eq!(config.server_key(), "http://b:2");
        assert_eq!(config.saved_token(&config.server_key()), None);
        config.set_sign_in("http://b:2", "token-b", None);
        config.origin = None;
        assert_eq!(config.server_key(), LOCAL);
        assert_eq!(config.saved_token(LOCAL), None);
        config.origin = Some("http://a:1".into());
        config.sign_out();
        assert_eq!(config.saved_token("http://a:1"), None);
        assert_eq!(config.saved_token("http://b:2").as_deref(), Some("token-b"));
    }

    #[test]
    fn an_old_single_sign_in_moves_under_its_server_and_is_protected() {
        let file = temp_file("migrate");
        let later = now_ms() as f64 + 3_600_000.0;
        fs::write(&file, format!(r#"{{"wowDir":"C:\\wow","token":"old-token","tokenExpiresAt":{later}}}"#)).unwrap();
        let (config, changed) = Config::read(&file).unwrap();
        assert!(changed);
        assert_eq!(config.saved_token(LOCAL).as_deref(), Some("old-token"));
        assert_eq!(config.wow_dir.as_deref(), Some("C:\\wow"));

        let saved = Config::update_at(&file, |_| {});
        assert_eq!(saved.saved_token(LOCAL).as_deref(), Some("old-token"));
        let text = fs::read_to_string(&file).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert!(json.get("token").is_none() && json.get("tokenExpiresAt").is_none(), "the old fields are gone: {text}");
        #[cfg(windows)]
        assert!(!text.contains("old-token"), "the token is encrypted: {text}");
        let (again, changed) = Config::read(&file).unwrap();
        assert!(!changed);
        assert_eq!(again.saved_token(LOCAL).as_deref(), Some("old-token"));
    }

    #[test]
    fn an_old_sign_in_with_a_saved_origin_stays_with_that_origin() {
        let file = temp_file("migrate-origin");
        fs::write(&file, r#"{"origin":"http://box:3773","token":"remote"}"#).unwrap();
        let (config, _) = Config::read(&file).unwrap();
        assert_eq!(config.saved_token("http://box:3773").as_deref(), Some("remote"));
        assert_eq!(config.saved_token(LOCAL), None);
    }

    #[test]
    fn a_damaged_file_is_kept_aside_not_overwritten_silently() {
        let file = temp_file("damaged");
        fs::write(&file, "{ not json").unwrap();
        let config = Config::update_at(&file, |c| c.interface = Some("1".into()));
        assert_eq!(config.interface.as_deref(), Some("1"));
        assert_eq!(fs::read_to_string(file.with_extension("json.bad")).unwrap(), "{ not json");
        assert!(Config::read(&file).is_ok());
    }

    #[test]
    fn stored_tokens_read_back() {
        let stored = protect("secret");
        #[cfg(windows)]
        assert!(stored.starts_with(PROTECTED));
        assert_eq!(unprotect(&stored).as_deref(), Some("secret"));
        assert_eq!(unprotect("plain").as_deref(), Some("plain"));
        assert_eq!(unprotect("dpapi:AAAA"), None);
    }
}
