//! Settings and sign-in, in %APPDATA%\t3wow\config.json (the same file the earlier Node
//! companion used, so an existing sign-in carries over).

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_INTERFACE: &str = "16001";

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

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wow_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Epoch milliseconds; none means the token doesn't expire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_expires_at: Option<f64>,
}

impl Config {
    pub fn file() -> PathBuf {
        config_dir().join("config.json")
    }

    pub fn load() -> Config {
        fs::read_to_string(Self::file()).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    /// Writes a temporary file and renames it over the old one, so a reader never sees half.
    fn save(&self) -> std::io::Result<()> {
        fs::create_dir_all(config_dir())?;
        let temp = Self::file().with_extension("json.tmp");
        fs::write(&temp, serde_json::to_string_pretty(self).unwrap_or_default())?;
        fs::rename(&temp, Self::file())
    }

    /// Reads the file, applies `change`, and writes it back (other fields untouched). The tray
    /// app and a `t3wow.exe login` in a terminal can both write: a lock file keeps one from
    /// undoing the other's change.
    pub fn update(change: impl FnOnce(&mut Config)) -> Config {
        let _ = fs::create_dir_all(config_dir());
        let lock = fs::OpenOptions::new().create(true).truncate(false).write(true).open(config_dir().join("config.lock"));
        let _guard = lock.ok().filter(|file| file.lock().is_ok());
        let mut config = Config::load();
        change(&mut config);
        let _ = config.save();
        config
    }

    pub fn token(&self) -> Option<String> {
        if let Ok(token) = std::env::var("T3WOW_TOKEN") {
            return Some(token);
        }
        let expired = self.token_expires_at.is_some_and(|at| at > 0.0 && (at as i64) < now_ms());
        self.token.clone().filter(|_| !expired)
    }

    pub fn interface(&self) -> String {
        self.interface.clone().unwrap_or_else(|| DEFAULT_INTERFACE.to_string())
    }
}
