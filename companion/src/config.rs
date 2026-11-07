//! Settings and sign-in, in %APPDATA%\t3wow\config.json (the same file the earlier Node
//! companion used, so an existing sign-in carries over).

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
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
        match Self::read(&Self::file()) {
            Ok(config) => config,
            Err(error) => {
                warn(&format!("Couldn't read {}: {error}", Self::file().display()));
                Config::default()
            }
        }
    }

    /// The file's settings. A file that isn't valid JSON is kept as config.json.bad and reads
    /// as defaults.
    fn read(file: &Path) -> io::Result<Config> {
        let text = match fs::read_to_string(file) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => return Err(e),
        };
        match serde_json::from_str::<Config>(&text) {
            Ok(config) => Ok(config),
            Err(error) => {
                let bad = file.with_extension("json.bad");
                let kept = fs::rename(file, &bad).is_ok();
                warn(&format!(
                    "{} is damaged ({error}); {}. Settings and the T3 sign-in start over.",
                    file.display(),
                    if kept { format!("kept it as {}", bad.display()) } else { "couldn't move it aside".into() }
                ));
                Ok(Config::default())
            }
        }
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
            Ok(mut config) => {
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

/// To the log, and to the console when a command runs in one.
fn warn(message: &str) {
    crate::log::line(message);
    eprintln!("{message}");
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

    #[test]
    fn a_damaged_file_is_kept_aside_not_overwritten_silently() {
        let file = temp_file("damaged");
        fs::write(&file, "{ not json").unwrap();
        let config = Config::update_at(&file, |c| c.interface = Some("1".into()));
        assert_eq!(config.interface.as_deref(), Some("1"));
        assert_eq!(fs::read_to_string(file.with_extension("json.bad")).unwrap(), "{ not json");
        assert!(Config::read(&file).is_ok());
    }
}
