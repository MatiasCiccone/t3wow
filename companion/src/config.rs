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

/// The address typed into the tray's "Connect to T3 on another machine…" box, as an origin:
/// `192.168.1.20:3773`, `gaming-pc.lan:3773`, or a full `http(s)://` URL. Without a scheme
/// it is plain HTTP (how T3 serves a LAN) and needs the port, since T3 doesn't run on 80.
pub fn parse_origin_input(input: &str) -> Result<String, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Type the address of the PC running T3, like 192.168.1.20:3773.".into());
    }
    if input.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("The address can't contain spaces.".into());
    }
    let (scheme, rest, explicit) = match input.split_once("://") {
        Some((scheme, rest)) => match scheme.to_ascii_lowercase().as_str() {
            "http" => ("http", rest, true),
            "https" => ("https", rest, true),
            _ => return Err(format!("{scheme}:// isn't a web address: use http:// or https://, or just host:port.")),
        },
        None => ("http", input, false),
    };
    if rest.contains(['?', '#']) {
        return Err("Leave out anything after ? or #: just the host and port.".into());
    }
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    if authority.contains('@') {
        return Err("Leave out the user name: just the host and port.".into());
    }
    // [fe80::1]:3773 for IPv6; everything else is host[:port].
    let (host, port) = if let Some(after) = authority.strip_prefix('[') {
        let (inside, tail) = after.split_once(']').ok_or("An IPv6 address needs its closing ].")?;
        if inside.is_empty() || !inside.chars().all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.') {
            return Err(format!("[{inside}] isn't an IPv6 address."));
        }
        let host = &authority[..inside.len() + 2];
        match tail {
            "" => (host, None),
            _ => (host, Some(tail.strip_prefix(':').ok_or("Put a : between the address and the port.")?)),
        }
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    if host.is_empty() {
        return Err("The address needs a host name or IP address before the port.".into());
    }
    if !host.starts_with('[') && !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_') {
        return Err(format!("{host} isn't a host name or IP address."));
    }
    match port {
        Some(port) if port.parse::<u16>().is_ok_and(|p| p > 0) => {}
        Some(port) => return Err(format!("{port} isn't a port number (1 to 65535).")),
        None if !explicit => return Err(format!("Add the port T3 listens on, like {host}:3773.")),
        None => {}
    }
    Ok(canonical_origin(&format!("{scheme}://{authority}{path}")))
}

/// Plain HTTP to another machine: the sign-in token crosses the network unencrypted.
pub fn unencrypted_over_network(origin: &str) -> bool {
    let origin = canonical_origin(origin);
    let Some(rest) = origin.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or("");
    let host = match authority.split_once(']') {
        Some((inside, _)) if authority.starts_with('[') => format!("{inside}]"),
        _ => authority.split(':').next().unwrap_or("").to_string(),
    };
    !(host == "localhost" || host == "[::1]" || host.starts_with("127."))
}

/// The T3 in use, as the tray names it: this PC, or the other machine's origin.
pub fn server_label(origin: Option<&str>) -> String {
    match origin {
        None => "this PC".into(),
        Some(origin) => canonical_origin(origin),
    }
}

/// How far the companion trusts commands from the game. Any addon in the game can draw
/// the strip, so this caps what one could make T3 do.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Trust {
    /// Everything the window offers.
    #[default]
    Full,
    /// Everything but starting threads.
    NoNewThreads,
    /// Reading only: nothing that changes T3 (no messages, starts, answers, Stop or Settle).
    ReadOnly,
}

/// What the game may still do under Read-only: look, and bring T3 to the front.
const READ_ONLY_KINDS: [&str; 11] = ["hello", "ui", "open", "more", "detail", "list", "refresh", "probe", "t3", "pr", "unpaste"];

impl Trust {
    pub const ALL: [Trust; 3] = [Trust::Full, Trust::NoNewThreads, Trust::ReadOnly];

    /// As the tray menu names it.
    pub fn label(self) -> &'static str {
        match self {
            Trust::Full => "Full",
            Trust::NoNewThreads => "No new threads",
            Trust::ReadOnly => "Read-only",
        }
    }

    /// What it means, for the tray's confirmation.
    pub fn description(self) -> &'static str {
        match self {
            Trust::Full => "Commands from the game can do everything the T3 WoW window offers.",
            Trust::NoNewThreads => "Commands from the game can do everything but start new threads.",
            Trust::ReadOnly => "Commands from the game can only read: messages, answers, new threads, Stop and Settle are refused.",
        }
    }

    /// As the game's updates name it (left out for Full).
    pub fn code(self) -> Option<&'static str> {
        match self {
            Trust::Full => None,
            Trust::NoNewThreads => Some("nonew"),
            Trust::ReadOnly => Some("readonly"),
        }
    }

    /// Whether a command of this kind (its `t`) may run.
    pub fn allows(self, kind: &str) -> bool {
        match self {
            Trust::Full => true,
            Trust::NoNewThreads => kind != "new",
            Trust::ReadOnly => READ_ONLY_KINDS.contains(&kind),
        }
    }
}

/// A value this version doesn't know (a typo in a hand edit) reads as Read-only, the
/// safest, instead of making the whole file unreadable.
fn lenient_trust<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Trust>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value.map(|v| serde_json::from_value(v).unwrap_or(Trust::ReadOnly)))
}

/// How often the companion looks for a new release by itself, from the tray's Update checks.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateInterval {
    Hour,
    #[default]
    SixHours,
    Day,
    Week,
    /// No timed checks (a check when the companion starts is its own setting).
    Never,
}

impl UpdateInterval {
    pub const ALL: [UpdateInterval; 5] = [UpdateInterval::Hour, UpdateInterval::SixHours, UpdateInterval::Day, UpdateInterval::Week, UpdateInterval::Never];

    /// As the tray menu names it.
    pub fn label(self) -> &'static str {
        match self {
            UpdateInterval::Hour => "Every hour",
            UpdateInterval::SixHours => "Every 6 hours",
            UpdateInterval::Day => "Every day",
            UpdateInterval::Week => "Every week",
            UpdateInterval::Never => "Never",
        }
    }

    /// Between two timed checks, in milliseconds; None for Never.
    pub fn millis(self) -> Option<i64> {
        let hours = match self {
            UpdateInterval::Hour => 1,
            UpdateInterval::SixHours => 6,
            UpdateInterval::Day => 24,
            UpdateInterval::Week => 24 * 7,
            UpdateInterval::Never => return None,
        };
        Some(hours * 3600 * 1000)
    }
}

/// When the companion looks for a new release without being asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateChecks {
    pub on_start: bool,
    pub every: UpdateInterval,
}

impl UpdateChecks {
    /// What a new install does: look when it starts and every 6 hours.
    pub const DEFAULT: UpdateChecks = UpdateChecks { on_start: true, every: UpdateInterval::SixHours };
    pub const MANUAL_ONLY: UpdateChecks = UpdateChecks { on_start: false, every: UpdateInterval::Never };

    /// No automatic checks at all: only the tray's Check for updates (or `t3wow.exe update`).
    pub fn manual_only(self) -> bool {
        self == Self::MANUAL_ONLY
    }

    /// The tray's "Only when I check": turns every automatic check off, or, chosen again,
    /// back to what a new install does.
    pub fn toggle_manual_only(self) -> UpdateChecks {
        if self.manual_only() {
            Self::DEFAULT
        } else {
            Self::MANUAL_ONLY
        }
    }

    /// For the tray's submenu: "Update checks: at start, every 6 hours".
    pub fn summary(self) -> String {
        let every = self.every.label().to_lowercase();
        match (self.on_start, self.every) {
            (false, UpdateInterval::Never) => "Update checks: only when I check".into(),
            (true, UpdateInterval::Never) => "Update checks: when T3 WoW starts".into(),
            (false, _) => format!("Update checks: {every}"),
            (true, _) => format!("Update checks: at start, {every}"),
        }
    }

    /// What it means, for the tray's confirmation.
    pub fn description(self) -> String {
        let every = self.every.label().to_lowercase();
        match (self.on_start, self.every) {
            (false, UpdateInterval::Never) => "T3 WoW no longer looks for updates by itself. Choose Check for updates in the tray menu to look.".into(),
            (true, UpdateInterval::Never) => "T3 WoW looks for a new release when it starts, and tells you when there is one.".into(),
            (false, _) => format!("T3 WoW looks for a new release {every}, and tells you when there is one."),
            (true, _) => format!("T3 WoW looks for a new release when it starts and {every}, and tells you when there is one."),
        }
    }
}

/// A value this version doesn't know (a typo in a hand edit) reads as the default interval.
fn lenient_interval<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<UpdateInterval>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value.map(|v| serde_json::from_value(v).unwrap_or_default()))
}

/// Whether a timed update check is due: `last` is when the last check (timed or asked for)
/// started, in epoch milliseconds. A clock set back counts as due rather than waiting out
/// the gap.
pub fn update_check_due(every: UpdateInterval, last: i64, now: i64) -> bool {
    every.millis().is_some_and(|gap| now < last || now - last >= gap)
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
    /// The OAuth client registered with this server, reused by the next sign-in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_id: Option<String>,
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
    /// The other machine's T3 used last, kept when switching back to this PC so the tray's
    /// T3 server menu can switch to it again in one click.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_remote: Option<String>,
    /// The tray's "Commands from the game"; none means Full.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_trust")]
    pub game_commands: Option<Trust>,
    /// The tray's "When T3 WoW starts" under Update checks; none means on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_check_on_start: Option<bool>,
    /// How often to look for a new release while running; none means every 6 hours.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_interval")]
    pub update_check_every: Option<UpdateInterval>,
    /// The tray's "Put back missing addon files automatically"; none means on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub put_back_files: Option<bool>,
    /// When the last update check started (epoch milliseconds), so a timed check counts
    /// from it across restarts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_update_check: Option<i64>,
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
                // Already on disk in plain text: if it can't be encrypted now, it stays as it
                // was rather than being lost (the loop below tries again on every load).
                server.token = Some(protect(&token).unwrap_or(token));
                server.token_expires_at = expires_at;
            }
            changed = true;
        }
        for server in self.servers.values_mut() {
            if let Some(stored) = server.token.as_deref().filter(|s| !s.starts_with(PROTECTED)) {
                if let Some(protected) = protect(stored).filter(|p| p != stored) {
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

    /// Saves a new sign-in for the server `key`. Refuses (and saves nothing) when the token
    /// can't be encrypted for this Windows user, rather than writing it in plain text.
    pub fn set_sign_in(&mut self, key: &str, token: &str, expires_at: Option<f64>) -> Result<(), String> {
        let stored = protect(token).ok_or("Signed in, but Windows couldn't encrypt the sign-in for this user, so it wasn't saved. Try again.")?;
        let server = self.servers.entry(key.to_string()).or_default();
        server.token = Some(stored);
        server.token_expires_at = expires_at;
        Ok(())
    }

    /// Saves a sign-in for the server `key` (none: the one saved for it was checked and still
    /// works) and, with `switch_to`, makes that T3 the one the companion uses (`Some(None)`:
    /// the T3 on this PC). The one place the T3 in use changes: `login --origin` and the
    /// tray's T3 server menu both come here, through `t3::connect`, and only once the sign-in
    /// succeeded. Nothing changes when the sign-in can't be saved.
    pub fn apply_sign_in(&mut self, key: &str, sign_in: Option<(&str, Option<f64>)>, switch_to: Option<Option<String>>) -> Result<(), String> {
        if let Some((token, expires_at)) = sign_in {
            self.set_sign_in(key, token, expires_at)?;
        }
        if let Some(origin) = switch_to {
            let origin = origin.map(|o| canonical_origin(&o));
            if origin.is_some() {
                self.last_remote.clone_from(&origin);
            }
            self.origin = origin;
        }
        Ok(())
    }

    /// Forgets the current server's sign-in (its client registration stays).
    pub fn sign_out(&mut self) {
        if let Some(server) = self.servers.get_mut(&self.server_key()) {
            server.token = None;
            server.token_expires_at = None;
        }
    }

    pub fn client_id(&self, key: &str) -> Option<String> {
        self.servers.get(key)?.client_id.clone()
    }

    pub fn set_client_id(&mut self, key: &str, client_id: Option<String>) {
        self.servers.entry(key.to_string()).or_default().client_id = client_id;
    }

    pub fn trust(&self) -> Trust {
        self.game_commands.unwrap_or_default()
    }

    /// Whether the companion puts back addon files it finds gone, by itself.
    pub fn put_back_files(&self) -> bool {
        self.put_back_files.unwrap_or(true)
    }

    pub fn update_checks(&self) -> UpdateChecks {
        UpdateChecks {
            on_start: self.update_check_on_start.unwrap_or(UpdateChecks::DEFAULT.on_start),
            every: self.update_check_every.unwrap_or(UpdateChecks::DEFAULT.every),
        }
    }

    pub fn set_update_checks(&mut self, checks: UpdateChecks) {
        self.update_check_on_start = Some(checks.on_start);
        self.update_check_every = Some(checks.every);
    }

    /// The saved interface version, or the shipped addon's when none is saved or the saved
    /// one can't be real (0.5.0 could save 1607186 from a misread Wow.exe).
    pub fn interface(&self) -> String {
        self.interface.clone().filter(|i| crate::wow::is_interface(i)).unwrap_or_else(|| DEFAULT_INTERFACE.to_string())
    }
}

/// To the log, and to the console when a command runs in one.
fn warn(message: &str) {
    crate::log::line(message);
    eprintln!("{message}");
}

const PROTECTED: &str = "dpapi:";

/// A token as stored: on Windows, encrypted for this Windows user (DPAPI), or None when
/// that fails, so a token is never written there in plain text; elsewhere, as it is.
fn protect(token: &str) -> Option<String> {
    #[cfg(windows)]
    return dpapi::protect(token.as_bytes()).map(|blob| format!("{PROTECTED}{}", STANDARD.encode(blob)));
    #[cfg(not(windows))]
    Some(token.to_string())
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
        config.set_sign_in(key, token, expires_at).unwrap();
        config
    }

    #[test]
    fn origins_have_one_spelling() {
        assert_eq!(canonical_origin("HTTP://Box.Lan:3773/"), "http://box.lan:3773");
        assert_eq!(canonical_origin("https://t3.example.com:443"), "https://t3.example.com");
        assert_eq!(canonical_origin("http://10.0.0.2:80/t3/"), "http://10.0.0.2/t3");
    }

    #[test]
    fn typed_addresses_become_origins() {
        assert_eq!(parse_origin_input(" 192.168.1.20:3773 ").as_deref(), Ok("http://192.168.1.20:3773"));
        assert_eq!(parse_origin_input("Gaming-PC.lan:3773/").as_deref(), Ok("http://gaming-pc.lan:3773"));
        assert_eq!(parse_origin_input("http://box:3773").as_deref(), Ok("http://box:3773"));
        assert_eq!(parse_origin_input("HTTPS://t3.example.com").as_deref(), Ok("https://t3.example.com"));
        assert_eq!(parse_origin_input("http://box").as_deref(), Ok("http://box"), "a full URL may use the default port");
        assert_eq!(parse_origin_input("https://proxy.lan/t3/").as_deref(), Ok("https://proxy.lan/t3"));
        assert_eq!(parse_origin_input("[fe80::1]:3773").as_deref(), Ok("http://[fe80::1]:3773"));
        assert_eq!(parse_origin_input("http://[::1]").as_deref(), Ok("http://[::1]"));
        for bad in [
            "", "   ", "box", "192.168.1.20", ":3773", "box:", "box:99999", "box:0", "box:abc", "ftp://box:21", "box :3773", "user@box:3773", "box:3773?x=1",
            "http://box:3773#a", "bad host!:3773", "bad!host:3773", "[::1", "[zz]:1", "[::1]3773",
        ] {
            assert!(parse_origin_input(bad).is_err(), "{bad:?} should be refused");
        }
        assert!(parse_origin_input("box").unwrap_err().contains("box:3773"), "it says what's missing");
    }

    #[test]
    fn plain_http_to_another_machine_is_flagged() {
        assert!(unencrypted_over_network("http://192.168.1.20:3773"));
        assert!(unencrypted_over_network("HTTP://Box.lan:3773/"));
        assert!(!unencrypted_over_network("https://box.lan"));
        assert!(!unencrypted_over_network("http://127.0.0.1:3773"));
        assert!(!unencrypted_over_network("http://localhost:3773"));
        assert!(!unencrypted_over_network("http://[::1]:3773"));
        assert!(unencrypted_over_network("http://[fe80::1]:3773"));
        assert_eq!(server_label(None), "this PC");
        assert_eq!(server_label(Some("HTTP://Box:3773/")), "http://box:3773");
    }

    #[test]
    fn a_sign_in_switches_the_t3_in_use_only_when_it_is_saved() {
        let mut config = Config::default();
        config.apply_sign_in("http://box:3773", Some(("remote", None)), Some(Some("HTTP://Box:3773/".into()))).unwrap();
        assert_eq!(config.origin.as_deref(), Some("http://box:3773"));
        assert_eq!(config.last_remote.as_deref(), Some("http://box:3773"));
        assert_eq!(config.saved_token(&config.server_key()).as_deref(), Some("remote"));
        // Back to this PC with its saved sign-in: the other machine's stays, and is remembered.
        config.set_sign_in(LOCAL, "local", None).unwrap();
        config.apply_sign_in(LOCAL, None, Some(None)).unwrap();
        assert_eq!(config.origin, None);
        assert_eq!(config.last_remote.as_deref(), Some("http://box:3773"));
        assert_eq!(config.saved_token(&config.server_key()).as_deref(), Some("local"));
        assert_eq!(config.saved_token("http://box:3773").as_deref(), Some("remote"));
        // A fresh sign-in without a switch stays on the T3 in use.
        config.apply_sign_in(LOCAL, Some(("local-2", None)), None).unwrap();
        assert_eq!(config.origin, None);
        assert_eq!(config.saved_token(LOCAL).as_deref(), Some("local-2"));
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
        config.set_sign_in("http://b:2", "token-b", None).unwrap();
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
    fn client_ids_are_kept_per_server_through_sign_out() {
        let mut config = Config { origin: Some("http://a:1".into()), ..Config::default() };
        config.set_client_id("http://a:1", Some("client-a".into()));
        config.set_sign_in("http://a:1", "t", None).unwrap();
        config.sign_out();
        assert_eq!(config.client_id("http://a:1").as_deref(), Some("client-a"));
        assert_eq!(config.client_id(LOCAL), None);
    }

    #[test]
    fn the_game_commands_setting_saves_and_an_unknown_value_reads_as_read_only() {
        let file = temp_file("trust");
        assert_eq!(Config::update_at(&file, |_| {}).trust(), Trust::Full);
        Config::update_at(&file, |c| c.game_commands = Some(Trust::NoNewThreads));
        assert!(fs::read_to_string(&file).unwrap().contains(r#""gameCommands": "noNewThreads""#));
        assert_eq!(Config::read(&file).unwrap().0.trust(), Trust::NoNewThreads);
        fs::write(&file, r#"{"gameCommands":"read only","wowDir":"C:\\wow"}"#).unwrap();
        let (config, _) = Config::read(&file).unwrap();
        assert_eq!(config.trust(), Trust::ReadOnly);
        assert_eq!(config.wow_dir.as_deref(), Some("C:\\wow"), "the rest of the file still reads");
    }

    #[test]
    fn update_checks_default_to_start_and_six_hours_and_save() {
        let file = temp_file("update-checks");
        assert_eq!(Config::update_at(&file, |_| {}).update_checks(), UpdateChecks::DEFAULT);
        Config::update_at(&file, |c| c.set_update_checks(UpdateChecks::MANUAL_ONLY));
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains(r#""updateCheckOnStart": false"#) && text.contains(r#""updateCheckEvery": "never""#), "{text}");
        assert!(Config::read(&file).unwrap().0.update_checks().manual_only());
        fs::write(&file, r#"{"updateCheckEvery":"fortnightly","updateCheckOnStart":false,"wowDir":"C:\\wow"}"#).unwrap();
        let (config, _) = Config::read(&file).unwrap();
        assert_eq!(config.update_checks(), UpdateChecks { on_start: false, every: UpdateInterval::SixHours }, "an unknown interval reads as the default");
        assert_eq!(config.wow_dir.as_deref(), Some("C:\\wow"), "the rest of the file still reads");
    }

    #[test]
    fn a_timed_update_check_is_due_once_its_interval_has_passed() {
        let hour = 3600 * 1000;
        assert!(!update_check_due(UpdateInterval::SixHours, 0, 5 * hour));
        assert!(update_check_due(UpdateInterval::SixHours, 0, 6 * hour));
        assert!(update_check_due(UpdateInterval::Hour, 0, hour));
        assert!(!update_check_due(UpdateInterval::Day, 0, 23 * hour));
        assert!(!update_check_due(UpdateInterval::Week, 0, 6 * 24 * hour));
        assert!(!update_check_due(UpdateInterval::Never, 0, 1000 * 24 * hour), "never means never");
        assert!(update_check_due(UpdateInterval::Day, 10 * hour, hour), "a clock set back doesn't wait out the gap");
    }

    #[test]
    fn update_check_settings_read_as_the_menu_says_them() {
        assert_eq!(UpdateChecks::DEFAULT.summary(), "Update checks: at start, every 6 hours");
        assert_eq!(UpdateChecks::MANUAL_ONLY.summary(), "Update checks: only when I check");
        assert_eq!(UpdateChecks { on_start: true, every: UpdateInterval::Never }.summary(), "Update checks: when T3 WoW starts");
        assert_eq!(UpdateChecks { on_start: false, every: UpdateInterval::Day }.summary(), "Update checks: every day");
        assert!(UpdateChecks::MANUAL_ONLY.description().contains("Check for updates"));
        assert!(!UpdateChecks { on_start: true, every: UpdateInterval::Never }.manual_only());
    }

    #[test]
    fn only_when_i_check_turns_every_automatic_check_off_and_back() {
        assert_eq!(UpdateChecks::DEFAULT.toggle_manual_only(), UpdateChecks::MANUAL_ONLY);
        assert_eq!(UpdateChecks::MANUAL_ONLY.toggle_manual_only(), UpdateChecks::DEFAULT);
        let daily = UpdateChecks { on_start: false, every: UpdateInterval::Day };
        assert_eq!(daily.toggle_manual_only(), UpdateChecks::MANUAL_ONLY, "any automatic check counts");
    }

    #[test]
    fn an_impossible_saved_interface_version_falls_back_to_the_shipped_one() {
        let with = |interface: Option<&str>| Config { interface: interface.map(String::from), ..Config::default() }.interface();
        assert_eq!(with(None), DEFAULT_INTERFACE);
        assert_eq!(with(Some("16002")), "16002");
        assert_eq!(with(Some("1607186")), DEFAULT_INTERFACE);
        assert_eq!(with(Some("")), DEFAULT_INTERFACE);
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
        let stored = protect("secret").unwrap();
        #[cfg(windows)]
        assert!(stored.starts_with(PROTECTED));
        assert_eq!(unprotect(&stored).as_deref(), Some("secret"));
        assert_eq!(unprotect("plain").as_deref(), Some("plain"));
        assert_eq!(unprotect("dpapi:AAAA"), None);
    }
}
