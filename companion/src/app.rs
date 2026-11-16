//! The companion's work, on a background thread: find WoW and T3, keep the addon files in
//! place, sign in when needed, and run the bridge. The tray (or the console, for `run`)
//! only shows what this reports: the status lines, what is under way (`activity`), and a
//! notice for every action's start, success or failure (`take_notices`).

use crate::bridge::{Bridge, CaptureMessage};
use crate::config::{config_dir, now_ms, server_label, unencrypted_over_network, update_check_due, Config, Trust, UpdateChecks};
use crate::log;
use crate::made_for::OlderT3;
use crate::slots::{Slots, SLOT_COUNT};

use crate::t3::{self, McpClient, T3};
use crate::updater::{self, Release, Step};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What the tray shows; filled in by the worker.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub t3: String,
    pub wow: String,
    pub attention: bool,
    /// Signed in to T3 (the menu offers Sign out) or not (Sign in).
    pub signed_in: bool,
}

pub enum Control {
    /// A new sign-in finished and is saved.
    SignedIn,
    SignedOut,
    Reinstall,
    /// The tray's "Repair addon files": put back only what's missing.
    Repair,
    /// The tray's "Commands from the game" changed (it is saved already).
    Trust(Trust),
}

/// Where the update check stands, for the tray menu.
#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    NotChecked,
    Checking,
    UpToDate,
    Available(Release),
    Installing(String),
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    Info,
    Warning,
    Error,
}

/// A notification from the tray: what an action did, or why it failed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    pub level: Level,
    pub title: String,
    pub text: String,
}

impl Notice {
    pub fn new(level: Level, title: impl Into<String>, text: impl Into<String>) -> Notice {
        Notice { level, title: title.into(), text: text.into() }
    }
}

/// Work the tray shows while it runs (one line each, in this order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Task {
    SignIn,
    Update,
}

pub struct App {
    pub status: Mutex<Status>,
    pub control: Mutex<Option<Sender<Control>>>,
    /// Counts sign-ins started: a waiting sign-in ends when a newer one starts.
    sign_in_run: AtomicU64,
    pub wow_dir: Mutex<Option<PathBuf>>,
    pub update: Mutex<Update>,
    activities: Mutex<BTreeMap<Task, String>>,
    notices: Mutex<VecDeque<Notice>>,
    /// The release a notice went out for: one per new release, not one per check.
    announced: Mutex<Option<String>>,
}

/// A Mac's menu bar is up: set once by the menu bar app, never by `t3wow run`, whose
/// companion has no menu and whose texts name its commands instead.
#[cfg(target_os = "macos")]
static MENU_BAR: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "macos")]
pub fn set_menu_bar() {
    MENU_BAR.store(true, Ordering::Relaxed);
}

#[cfg(target_os = "macos")]
pub fn menu_bar() -> bool {
    MENU_BAR.load(Ordering::Relaxed)
}

/// A Mac's text, for the menu bar app or for `t3wow run` in Terminal (no menu); elsewhere the
/// tray's one text.
#[cfg(target_os = "macos")]
pub(crate) fn shown(texts: (&'static str, &'static str)) -> &'static str {
    if menu_bar() {
        texts.0
    } else {
        texts.1
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn shown(text: &'static str) -> &'static str {
    text
}

/// "T3 on this PC" / "T3 at http://box:3773", from `config::server_label`.
pub fn t3_named(label: &str) -> String {
    if label == server_label(None) {
        format!("T3 on {label}")
    } else {
        format!("T3 at {label}")
    }
}

/// The notification when a new release is out: on Windows the tray installs it, on a Mac
/// the app's menu bar installs it too, and a bare t3wow's opens the download page.
#[cfg(not(target_os = "macos"))]
fn update_out_text(version: &str, current: &str) -> String {
    format!("T3 WoW {version} is out (you have {current}). Choose Update to {version} in the tray menu.")
}

#[cfg(target_os = "macos")]
fn update_out_text(version: &str, current: &str) -> String {
    if updater::running_translocated() {
        return translocated_update_out_text(version, current);
    }
    mac_update_out_text(version, current, updater::can_install())
}

/// A new release while macOS runs the app from a temporary copy: how to get it.
#[cfg(target_os = "macos")]
fn translocated_update_out_text(version: &str, current: &str) -> String {
    format!("T3 WoW {version} is out (you have {current}). To update, quit T3 WoW, drag T3 WoW into Applications in Finder, and open it from there.")
}

/// `installs`: the menu's item installs the release (in the app), as the item's own text
/// says (tray::update_text), or opens its download page (a bare t3wow).
#[cfg(target_os = "macos")]
fn mac_update_out_text(version: &str, current: &str, installs: bool) -> String {
    if installs {
        format!("T3 WoW {version} is out (you have {current}). Choose Update to {version} in the menu bar.")
    } else {
        format!("T3 WoW {version} is out (you have {current}). Choose “{version} is out” in the menu bar to open its download page.")
    }
}

/// The notice when an update fails: why, then how to try again. An app macOS runs from a
/// temporary copy (`updater::TRANSLOCATED`) can't, until it is moved: that text says how.
fn update_failed_text(error: &str, version: &str) -> String {
    let error = error.trim_end_matches('.');
    #[cfg(target_os = "macos")]
    if error == updater::TRANSLOCATED.trim_end_matches('.') {
        return updater::TRANSLOCATED.to_string();
    }
    format!("{error}. Choose Update to {version} in {MENU_NAME} to try again.")
}

/// The notice at startup when macOS runs the app from a temporary, read-only copy: the update
/// item says the same, rather than trying.
#[cfg(target_os = "macos")]
pub fn translocated_notice() -> Notice {
    Notice::new(Level::Warning, "Move T3 WoW to Applications", updater::TRANSLOCATED)
}

/// The tray's line while an update runs.
/// What choosing Update installs, given the release on offer and a fresh look at GitHub:
/// whatever GitHub says is newest now, or nothing if the release was withdrawn. A failed
/// look doesn't stand in the way: the offered release is still checked when it downloads.
fn release_to_install(offered: Release, newest: Result<Option<Release>, String>) -> Option<Release> {
    newest.unwrap_or(Some(offered))
}

pub fn update_step_text(version: &str, step: Step) -> String {
    match step {
        Step::Downloading(Some(percent)) => format!("Downloading update {version}… {percent}%"),
        Step::Downloading(None) => format!("Downloading update {version}…"),
        Step::Checking => format!("Checking the download of {version}…"),
        Step::Installing => format!("Installing update {version}…"),
    }
}

/// Said whenever the game has to restart to see new addon files.
pub const RESTART_WOW: &str = "Quit WoW completely and start it again: /reload doesn't load new addon files.";

impl App {
    pub fn new() -> Arc<App> {
        Arc::new(App {
            status: Mutex::new(Status::default()),
            control: Mutex::new(None),
            sign_in_run: AtomicU64::new(0),
            wow_dir: Mutex::new(None),
            update: Mutex::new(Update::NotChecked),
            activities: Mutex::new(BTreeMap::new()),
            notices: Mutex::new(VecDeque::new()),
            announced: Mutex::new(None),
        })
    }

    fn set(&self, t3: impl Into<String>, wow: impl Into<String>, attention: bool, signed_in: bool) {
        *self.status.lock().unwrap() = Status { t3: t3.into(), wow: wow.into(), attention, signed_in };
    }

    /// Logs a notice and queues it for the tray to show.
    pub fn notify(&self, notice: Notice) {
        log::line(&format!("{}: {}", notice.title, notice.text));
        let mut notices = self.notices.lock().unwrap();
        // A tray that isn't there (`run`) never takes them: keep the newest few.
        if notices.len() >= 20 {
            notices.pop_front();
        }
        notices.push_back(notice);
    }

    pub fn take_notices(&self) -> Vec<Notice> {
        self.notices.lock().unwrap().drain(..).collect()
    }

    /// Sets (or, with None, ends) the line shown while `task` runs.
    pub fn set_activity(&self, task: Task, line: Option<String>) {
        let mut activities = self.activities.lock().unwrap();
        match line {
            Some(line) => activities.insert(task, line),
            None => activities.remove(&task),
        };
    }

    /// What is under way, one line per task; None when nothing is.
    pub fn activity(&self) -> Option<String> {
        let activities = self.activities.lock().unwrap();
        (!activities.is_empty()).then(|| activities.values().cloned().collect::<Vec<_>>().join("\n"))
    }

    /// Forgets the current T3's sign-in; the menu then offers Sign in again.
    pub fn sign_out(&self) {
        let config = Config::update(Config::sign_out);
        let name = t3_named(&server_label(config.origin.as_deref()));
        self.notify(Notice::new(Level::Info, "Signed out of T3", format!("Signed out of {name}. Choose Sign in to T3… to sign in again.")));
        self.send(Control::SignedOut);
    }

    /// Saves the tray's "Commands from the game" and applies it at once.
    pub fn set_trust(&self, trust: Trust) {
        // Saved first, so the worker's next look at config.json agrees.
        Config::update(|c| c.game_commands = Some(trust));
        self.send(Control::Trust(trust));
        self.notify(Notice::new(Level::Info, format!("Commands from the game: {}", trust.label()), trust.description()));
    }

    pub fn set_put_back_files(&self, on: bool) {
        Config::update(|c| c.put_back_files = Some(on));
        self.notify(if on {
            Notice::new(Level::Info, "Put back missing addon files: on", "When something removes T3 WoW's files from the AddOns folder, T3 WoW puts them back by itself.")
        } else {
            Notice::new(Level::Info, "Put back missing addon files: off", "T3 WoW leaves the AddOns folder as it is. Choose Repair addon files to put back what's missing.")
        });
    }

    /// Looks for a newer release in the background (once at a time). `asked`: chosen from
    /// the menu, so the result is announced whatever it is; the timed checks only announce
    /// a new release, once.
    pub fn check_updates(self: &Arc<App>, asked: bool) {
        {
            let mut update = self.update.lock().unwrap();
            if matches!(*update, Update::Checking | Update::Installing(_)) {
                return;
            }
            *update = Update::Checking;
        }
        // Timed checks count from here, across restarts too.
        Config::update(|c| c.last_update_check = Some(now_ms()));
        if asked {
            self.set_activity(Task::Update, Some("Checking for updates…".into()));
        }
        let app = Arc::clone(self);
        std::thread::spawn(move || {
            let current = updater::current_version();
            let result = match updater::check() {
                Ok(Some(release)) => {
                    let mut announced = app.announced.lock().unwrap();
                    if asked || announced.as_ref() != Some(&release.version) {
                        let v = &release.version;
                        app.notify(Notice::new(Level::Info, "T3 WoW update", update_out_text(v, current)));
                        *announced = Some(v.clone());
                    }
                    Update::Available(release)
                }
                Ok(None) => {
                    if asked {
                        app.notify(Notice::new(Level::Info, "T3 WoW is up to date", updater::up_to_date_text()));
                    }
                    Update::UpToDate
                }
                Err(error) => {
                    if asked {
                        app.notify(Notice::new(Level::Error, "Couldn't check for updates", error.clone()));
                    } else {
                        log::line(&format!("Update check failed: {error}"));
                    }
                    Update::Failed(error)
                }
            };
            *app.update.lock().unwrap() = result;
            if asked {
                app.set_activity(Task::Update, None);
            }
        });
    }

    /// Saves the tray's Update checks; the timed checks follow it within a minute.
    pub fn set_update_checks(&self, checks: UpdateChecks) {
        Config::update(|c| c.set_update_checks(checks));
        let title = if checks.manual_only() { "Automatic update checks: off".to_string() } else { checks.summary() };
        self.notify(Notice::new(Level::Info, title, checks.description()));
    }

    /// Looks once more for the newest release (the offer can be hours old), then downloads
    /// and swaps it in, showing how far it has got; `then` runs on success (the tray
    /// restarts, and the new companion says it was updated).
    pub fn install_update(self: &Arc<App>, offered: Release, then: impl FnOnce() + Send + 'static) {
        *self.update.lock().unwrap() = Update::Checking;
        self.set_activity(Task::Update, Some("Checking for updates…".into()));
        let app = Arc::clone(self);
        std::thread::spawn(move || {
            let newest = updater::check();
            if let Err(error) = &newest {
                log::line(&format!("Update re-check failed, so installing the release on offer: {error}"));
            }
            let Some(release) = release_to_install(offered, newest) else {
                app.set_activity(Task::Update, None);
                app.notify(Notice::new(Level::Info, "T3 WoW is up to date", updater::up_to_date_text()));
                *app.update.lock().unwrap() = Update::UpToDate;
                return;
            };
            let version = release.version.clone();
            *app.update.lock().unwrap() = Update::Installing(version.clone());
            app.set_activity(Task::Update, Some(update_step_text(&version, Step::Downloading(None))));
            app.notify(Notice::new(Level::Info, format!("Updating T3 WoW to {version}"), "Downloading it now. T3 WoW restarts by itself when it's done."));
            match updater::install(&release, &|step| app.set_activity(Task::Update, Some(update_step_text(&version, step)))) {
                Ok(()) => {
                    app.set_activity(Task::Update, Some("Restarting T3 WoW…".into()));
                    let text = format!("Updated from {} and restarted. If WoW is running, it needs a restart to load the new addon: {RESTART_WOW}", updater::current_version());
                    notice_after_restart(&Notice::new(Level::Info, format!("T3 WoW updated to {version}"), text));
                    log::line(&format!("Updated to {version}; restarting."));
                    then();
                }
                Err(error) => {
                    app.set_activity(Task::Update, None);
                    app.notify(Notice::new(Level::Error, format!("Update to {version} failed"), update_failed_text(&error, &version)));
                    // Still out there: the menu offers it again.
                    *app.update.lock().unwrap() = Update::Available(release);
                }
            }
        });
    }

    pub fn send(&self, control: Control) {
        if let Some(tx) = self.control.lock().unwrap().as_ref() {
            let _ = tx.send(control);
        }
    }

    /// Signs in to the T3 in use again, through the browser.
    pub fn sign_in(self: &Arc<App>) {
        self.connect(None, false);
    }

    /// Signs in, and with `switch_to` moves to that T3 (`Some(None)`: the one on this PC),
    /// in the background, through `t3::connect`, the one path for both. Asked again while a
    /// browser sign-in waits, it starts over: the old wait ends.
    pub fn connect(self: &Arc<App>, switch_to: Option<Option<String>>, reuse_saved: bool) {
        let run = self.sign_in_run.fetch_add(1, Ordering::SeqCst) + 1;
        let app = Arc::clone(self);
        std::thread::spawn(move || {
            let cancelled = || app.sign_in_run.load(Ordering::SeqCst) != run;
            let to = switch_to.as_ref().map(|origin| t3_named(&server_label(origin.as_deref())));
            app.set_activity(Task::SignIn, Some(match &to {
                Some(name) => format!("Connecting to {name}…"),
                None => "Signing in to T3…".into(),
            }));
            let opened = || {
                app.set_activity(Task::SignIn, Some("Waiting for you to approve the sign-in in your browser…".into()));
                app.notify(Notice::new(
                    Level::Info,
                    "Approve the sign-in in your browser",
                    browser_open_text(),
                ));
            };
            let result = t3::connect(switch_to, reuse_saved, &|m| log::line(m), &opened, &cancelled);
            if !cancelled() {
                app.set_activity(Task::SignIn, None);
            }
            match result {
                Ok(connected) => {
                    app.notify(signed_in_notice(&connected));
                    app.send(Control::SignedIn);
                }
                Err(_) if cancelled() => log::line("Started the sign-in over."),
                Err(error) => {
                    let (title, stays) = match &to {
                        Some(_) => ("Couldn't switch T3", format!(" T3 WoW stays on {}.", t3_named(&server_label(Config::load().origin.as_deref())))),
                        None => ("Sign-in to T3 failed", String::new()),
                    };
                    app.notify(Notice::new(Level::Error, title, format!("{error}{stays}")));
                }
            }
        });
    }
}

/// What the tray says while T3's sign-in page waits in the browser.
/// The tray's notification when the T3 runs an older T3 Code than this release is made for:
/// once for each version it runs, so it doesn't come back with every poll.
fn older_t3_notice(older: Option<&OlderT3>, said: &mut Option<String>) -> Option<Notice> {
    let older = older?;
    if said.as_deref() == Some(older.running.as_str()) {
        return None;
    }
    *said = Some(older.running.clone());
    Some(Notice::new(Level::Warning, older.title(), older.note()))
}

fn browser_open_text() -> String {
    format!("T3's sign-in page is open in your browser. {} Asked for a code? Make one in T3 under Settings → Sharing & access.", t3::PICK_ACCESS_NOTE)
}

/// What the tray says once `t3::connect` succeeded: a warning when the sign-in can only read
/// (or the token crosses the network in plain HTTP). Windows shows 255 characters of it.
fn signed_in_notice(connected: &t3::Connected) -> Notice {
    let name = t3_named(&connected.label);
    let (title, mut text) = match (connected.switched, &connected.scope) {
        (true, None) => ("Switched T3", format!("T3 WoW now uses {name}, with the sign-in saved for it.")),
        (true, Some(_)) => ("Switched T3", format!("Signed in: T3 WoW now uses {name}.")),
        (false, _) => ("Signed in to T3", format!("T3 WoW is signed in to {name}.")),
    };
    let mut level = Level::Info;
    let title = if connected.read_only {
        level = Level::Warning;
        // The warning first: the plain-HTTP note after it may not fit.
        text = format!("{} {text}", t3::READ_ONLY_NOTE.replace("Sign in again", shown(SIGN_IN_AGAIN)));
        if connected.switched { "Switched T3, but the sign-in is read only" } else { "Signed in to T3, but read only" }
    } else {
        title
    };
    if unencrypted_over_network(&connected.origin) {
        text.push_str(" The connection is plain HTTP, so the sign-in token crosses your network unencrypted.");
        level = Level::Warning;
    }
    Notice::new(level, title, text)
}

/// How a read-only sign-in is done again: the tray's Sign in to T3…, or on a Mac the menu
/// bar's, or (under `t3wow run`) `t3wow login`.
#[cfg(not(target_os = "macos"))]
const SIGN_IN_AGAIN: &str = "Choose Sign in to T3…";
#[cfg(target_os = "macos")]
const SIGN_IN_AGAIN: (&str, &str) = ("Choose Sign in to T3… in the menu bar", "Run t3wow login");

/// The tray menu's T3 line while the sign-in in use can only read.
#[cfg(not(target_os = "macos"))]
const READ_ONLY_LINE: &str = "T3: connected, but read only: choose Sign in to T3, pick Auto or Full access";
#[cfg(target_os = "macos")]
const READ_ONLY_LINE: (&str, &str) =
    ("T3: connected, but read only: choose Sign in to T3, pick Auto or Full access", "T3: connected, but read only: run t3wow login, pick Auto or Full access");

/// The automatic update checks, as the tray's Update checks set them: one when the companion
/// starts, then one whenever the interval has passed since the last check (a check from the
/// menu counts too). Reads the setting every half minute, so a change applies without a
/// restart. Never returns.
pub fn watch_for_updates(app: Arc<App>) {
    let started = now_ms();
    if Config::load().update_checks().on_start {
        app.check_updates(false);
    }
    loop {
        std::thread::sleep(Duration::from_secs(30));
        let config = Config::load();
        // Never checked: the interval counts from this start.
        let last = config.last_update_check.unwrap_or(started);
        if update_check_due(config.update_checks().every, last, now_ms()) {
            app.check_updates(false);
        }
    }
}

/// T3's settings file, when the T3 in use is the one on this PC (no saved origin).
fn local_t3_settings() -> Option<PathBuf> {
    Config::load().origin.is_none().then(|| crate::config::t3_home().join("userdata").join("settings.json"))
}

/// T3's provider caches, when the T3 in use is the one on this PC (no saved origin).
fn local_provider_caches() -> Option<PathBuf> {
    Config::load().origin.is_none().then(|| crate::config::t3_home().join("caches"))
}

fn restart_notice_file() -> PathBuf {
    config_dir().join("restart-notice.json")
}

/// Keeps a notice for the companion that starts next (an update or a new WoW folder restarts
/// it, and a notification from this one would vanish with its tray icon).
pub fn notice_after_restart(notice: &Notice) {
    save_notice_at(&restart_notice_file(), notice);
}

/// The notice the previous companion left, once.
pub fn take_restart_notice() -> Option<Notice> {
    take_notice_at(&restart_notice_file())
}

fn save_notice_at(file: &Path, notice: &Notice) {
    if let Err(error) = std::fs::write(file, serde_json::to_string(notice).unwrap_or_default()) {
        log::line(&format!("Couldn't keep the notice for after the restart: {error}"));
    }
}

fn take_notice_at(file: &Path) -> Option<Notice> {
    let text = std::fs::read_to_string(file).ok()?;
    let _ = std::fs::remove_file(file);
    serde_json::from_str(&text).ok()
}

/// New addon files the running game hasn't loaded: WoW reads addon files only when it starts.
struct RestartNote {
    /// For the tray's WoW line.
    line: String,
    /// Said when WoW is seen running with the old files (None: said already).
    notice: Option<Notice>,
}

/// Whether the restart note still applies: until a WoW that was running stops, or, when WoW
/// wasn't running, once the screen capture has had time to see it (it then starts with the
/// new files).
fn restart_note_stays(wow_running: bool, seen_running: bool, since: Duration) -> bool {
    if seen_running {
        wow_running
    } else {
        // The capture looks for WoW as soon as it starts, so a few seconds is plenty; much
        // longer and a WoW started meanwhile (with the new files) would be told to restart.
        wow_running || since < Duration::from_secs(5)
    }
}

/// Reinstalls the addon, saying how it went. True when it worked.
fn reinstall(app: &App, slots: &Slots) -> bool {
    match slots.install() {
        Ok(()) => {
            app.notify(Notice::new(
                Level::Info,
                "Addon reinstalled",
                format!("Reinstalled in {}. If WoW is running, it needs a restart to load it: {RESTART_WOW}", slots.addons_dir.display()),
            ));
            true
        }
        Err(error) => {
            app.notify(Notice::new(Level::Error, "Reinstalling the addon failed", error.to_string()));
            false
        }
    }
}

/// Puts back the addon files that are gone and updates an older addon, keeping the rest:
/// the slots' updates, the sign-in, the settings. `asked`: chosen from the tray, so it says
/// how it went whatever happened. A note to restart WoW when anything was written.
fn put_back(app: &App, slots: &Slots, asked: bool) -> Option<RestartNote> {
    let repair = match slots.repair() {
        Ok(repair) => repair,
        Err(error) => {
            log::line(&format!("Couldn't put back the addon files: {error}"));
            app.notify(Notice::new(Level::Error, "Couldn't write the addon files", error.to_string()));
            return None;
        }
    };
    if let Some(addon) = &repair.addon {
        log::line(&format!("Addon {addon} in {}", slots.addon_dir.display()));
    }
    if repair.created > 0 {
        log::line(&format!("Created {} missing addon files", repair.created));
    }
    if repair.slots_moved > 0 {
        log::line(&format!("Moved {} slot addons to Slot.xml", repair.slots_moved));
    }
    if repair.addon.is_none() && repair.created == 0 && repair.slots_moved == 0 {
        if asked {
            app.notify(Notice::new(Level::Info, "Addon files: nothing missing", format!("Every T3 WoW file is in {}.", slots.addons_dir.display())));
        }
        return None;
    }
    let what = match (&repair.addon, repair.slots_moved) {
        (Some(addon), _) => format!("The addon was {addon}"),
        (None, 0) => format!("{} missing addon files were put back", repair.created),
        (None, _) => "The update slots were changed so addon managers leave them alone".to_string(),
    };
    let notice = if asked {
        // Said now, whether WoW runs or not.
        app.notify(Notice::new(
            Level::Info,
            "Addon files put back",
            format!("{what}; your updates, sign-in and settings stay. If WoW is running, it needs a restart to load them: {RESTART_WOW}"),
        ));
        None
    } else {
        // Said once WoW is seen running with the old files.
        Some(Notice::new(Level::Warning, "Restart WoW to load the new addon", format!("{what}, but the running game still has the old files. {RESTART_WOW}")))
    };
    Some(RestartNote { line: "restart WoW to load the new addon files (not /reload)".into(), notice })
}

/// The game was patched since the addon's files were written: their interface version
/// follows it, and the setting too, or the game marks the addon out of date.
#[cfg(target_os = "macos")]
fn follow_game_version(slots: &Slots, interface: &str) -> Option<RestartNote> {
    if Config::load().interface.as_deref() != Some(interface) {
        Config::update(|c| c.interface = Some(interface.to_string()));
    }
    match slots.set_interface() {
        Ok(0) => None,
        Ok(changed) => {
            log::line(&format!("Set the addon to this game's interface version, {interface} ({changed} addon files)."));
            Some(RestartNote {
                line: "restart WoW to load the updated addon files (not /reload)".into(),
                notice: Some(Notice::new(Level::Warning, "Restart WoW to load the addon", format!("The addon now matches this game's version. {RESTART_WOW}"))),
            })
        }
        Err(error) => {
            log::line(&format!("Couldn't set the interface version to {interface}: {error}"));
            None
        }
    }
}

/// How often the companion looks for addon files gone while it runs, and how long it waits
/// after failing to put them back.
const PUT_BACK_EVERY: Duration = Duration::from_secs(10);
const PUT_BACK_RETRY: Duration = Duration::from_secs(300);

/// Waits up to `seconds` for a control before the bridge runs, acting on a Reinstall or a
/// Repair (the rest are picked up by the caller's next look at the config).
fn wait(app: &App, rx: &Receiver<Control>, seconds: u64, slots: Option<&Slots>) {
    let reinstalling = match rx.recv_timeout(Duration::from_secs(seconds)) {
        Ok(Control::Reinstall) => true,
        Ok(Control::Repair) => false,
        _ => return,
    };
    match slots {
        Some(slots) if reinstalling => {
            reinstall(app, slots);
        }
        Some(slots) => {
            put_back(app, slots, true);
        }
        None => app.notify(Notice::new(
            Level::Error,
            "Can't change the addon yet",
            format!("T3 WoW hasn't found the WoW folder: choose it with Choose WoW folder… in {MENU_NAME} first."),
        )),
    }
}

/// Where the menu's items are: Windows' tray menu, or a Mac's menu bar. Only the menu sends
/// controls and installs updates (a Mac's app, which always has it), so its name will do.
#[cfg(not(target_os = "macos"))]
const MENU_NAME: &str = "the tray menu";
#[cfg(target_os = "macos")]
const MENU_NAME: &str = "the menu bar";

// Under `t3wow run` a Mac's companion has no menu: its commands do what the menu's items do.
#[cfg(not(target_os = "macos"))]
const SIGN_IN_NEEDED: &str = "T3: sign-in needed (approve it in your browser, or choose Sign in to T3)";
#[cfg(target_os = "macos")]
const SIGN_IN_NEEDED: (&str, &str) =
    ("T3: sign-in needed (approve it in your browser, or choose Sign in to T3)", "T3: sign-in needed (approve it in your browser, or run t3wow login)");
#[cfg(not(target_os = "macos"))]
const NO_LOCAL_T3: &str = "T3 Code isn't running on this PC: start it, or choose T3 server > Connect to T3 on another machine…";
#[cfg(target_os = "macos")]
const NO_LOCAL_T3: (&str, &str) = (
    "T3 Code isn't running on this Mac: start it, or choose T3 server > Connect to T3 on another machine…",
    "T3 Code isn't running on this Mac: start it, or connect to T3 on another machine with t3wow login --origin http://<host>:<port>",
);
/// The WoW line while macOS won't let the companion see the game.
#[cfg(target_os = "macos")]
const SCREEN_OFF_LINE: (&str, &str) = (
    "WoW: running, but T3 WoW can't see it: choose Open Screen Recording settings…",
    "WoW: running, but T3 WoW can't see it: turn on Screen Recording for Terminal",
);

/// The notification when macOS refuses the capture, for the menu bar; under `t3wow run` the
/// capture's own help (capture::permission_help) is in the log already. In the app
/// (`in_app`) the switch is T3 WoW's own, and a grant applies without a restart. A switch
/// left by a build signed another way (ad hoc, or another certificate) may show as on and
/// still not count; removing it and adding the app again fixes that. A bare t3wow's menu bar
/// is allowed as t3wow, or as Terminal when started there, and restarts to see it.
#[cfg(target_os = "macos")]
fn screen_off_notice(in_app: bool) -> Notice {
    let text = if in_app {
        "macOS doesn't let T3 WoW see the game. Choose Open Screen Recording settings… in the menu bar and turn on T3 WoW. If it shows as on already, remove it with − and add it again with +."
    } else {
        "macOS doesn't let T3 WoW see the game. Choose Open Screen Recording settings… in the menu bar, turn on t3wow (or Terminal, if you started it there), then quit T3 WoW and start it again."
    };
    Notice::new(Level::Warning, "Turn on Screen Recording for T3 WoW", text)
}
#[cfg(not(target_os = "macos"))]
const NO_BROWSER: &str = "No Microsoft Edge (or Chrome) to draw pictures with: pages and images show as text in the game";
#[cfg(target_os = "macos")]
const NO_BROWSER: &str = "No Google Chrome, Microsoft Edge or Chromium in Applications to draw pictures with: pages and images show as text in the game";

/// The worker loop. Never returns.
pub fn run(app: Arc<App>) {
    let (control_tx, control_rx) = channel::<Control>();
    *app.control.lock().unwrap() = Some(control_tx);
    // Early, so it is up before the game is.
    #[cfg(target_os = "macos")]
    crate::capture::request_screen_access();
    // A copy macOS made read-only can't update: say how to move it now, not after a download.
    #[cfg(target_os = "macos")]
    if updater::running_translocated() {
        app.notify(translocated_notice());
    }

    // 1. The WoW folder.
    let wow_dir = loop {
        let config = Config::load();
        let saved = config.wow_dir.as_ref().map(PathBuf::from).filter(|d| crate::wow::looks_like_client(d));
        if let Some(dir) = saved.or_else(crate::wow::find_wow_dir) {
            if config.wow_dir.as_deref() != Some(&dir.to_string_lossy()) {
                Config::update(|c| c.wow_dir = Some(dir.to_string_lossy().into_owned()));
            }
            break dir;
        }
        app.set("T3: waiting", "WoW: Forever folder not found: choose it from this menu", true, Config::load().token().is_some());
        wait(&app, &control_rx, 5, None);
    };
    *app.wow_dir.lock().unwrap() = Some(wow_dir.clone());
    let interface = Config::load().interface();
    // What this client is (its app says, even before its first launch) over what was saved.
    #[cfg(target_os = "macos")]
    let client_interface = crate::wow::interface_version(&wow_dir).map(|found| found.interface);
    #[cfg(target_os = "macos")]
    let interface = client_interface.clone().unwrap_or(interface);

    // 2. The addon files: put back what's missing, install or update the addon.
    let slots = Slots::for_wow(&wow_dir, &interface).with_key(Config::load().game_key);
    let mut restart: Option<RestartNote> = put_back(&app, &slots, false);
    #[cfg(target_os = "macos")]
    if let Some(note) = client_interface.and_then(|found| follow_game_version(&slots, &found)) {
        restart.get_or_insert(note);
    }
    // 0.5.0's Match WoW's interface version… could save a number no WoW has (1607186, from a
    // misread Wow.exe), and the game then marked every T3 WoW addon Incompatible. Config
    // already reads that as the shipped number; put it back in the files and the settings.
    if let Some(bad) = Config::load().interface.filter(|saved| *saved != interface) {
        match slots.set_interface() {
            Ok(changed) => {
                Config::update(|c| c.interface = Some(interface.clone()));
                log::line(&format!("The saved interface version {bad} can't be WoW's; set back to {interface} ({changed} addon files)."));
                if changed > 0 && restart.is_none() {
                    restart = Some(RestartNote {
                        line: "restart WoW to load the fixed addon files (not /reload)".into(),
                        notice: Some(Notice::new(Level::Warning, "Restart WoW to load the fixed addon", format!("The addon's interface version was set back to {interface}. {RESTART_WOW}"))),
                    });
                }
            }
            Err(error) => log::line(&format!("Couldn't set the interface version back to {interface}: {error}")),
        }
    }

    // 3. T3 (on this PC, or the saved origin) and a sign-in for it. The browser opens by
    //    itself once per start; after that, the tray's Sign in to T3… opens it again. A
    //    terminal `login` or the tray's T3 server menu is picked up too.
    let mut asked = false;
    let (mut key, origin, token) = loop {
        // The key file is there but unusable: it says so, and the browser stays shut, since
        // a new sign-in couldn't be saved either.
        #[cfg(target_os = "macos")]
        if let Some(problem) = crate::token_key::problem() {
            app.set(format!("T3: {problem}"), "WoW: waiting for the sign-in", true, false);
            wait(&app, &control_rx, 5, Some(&slots));
            continue;
        }
        match t3::target() {
            Some(t3::Target { key, origin, token: Some(token) }) => break (key, origin, token),
            Some(_) => {
                app.set(shown(SIGN_IN_NEEDED), "WoW: waiting for the sign-in", true, false);
                if !asked {
                    asked = true;
                    app.sign_in();
                }
            }
            None => app.set(shown(NO_LOCAL_T3), "WoW: waiting for T3", true, false),
        }
        wait(&app, &control_rx, 5, Some(&slots));
    };

    // 4. The bridge, fed by the screen capture.
    let mut client = Arc::new(McpClient::new(&origin, &token));
    let mut bridge = Bridge::new(
        client.clone() as Arc<dyn T3>,
        Box::new(Slots::for_wow(&wow_dir, &interface)),
        Some(config_dir().join("state.json")),
        Box::new(log::line),
        Box::new(now_ms),
    );
    let config = Config::load();
    bridge.set_trust(config.trust());
    bridge.set_game_key(config.game_key_bytes());
    bridge.set_sign_in_read_only(config.sign_in_read_only(&key));
    bridge.set_provider_caches(local_provider_caches());
    bridge.set_t3_settings(local_t3_settings());
    // Another T3 than the game's updates came from last time (`login --origin` while the
    // companion was off): the game drops the previous one's threads.
    bridge.set_login(&key);
    log::line(&format!("T3 WoW companion {}: T3 at {origin}, WoW at {}", crate::VERSION, wow_dir.display()));
    // Pages and images go to the game as pictures, drawn with the browser Windows has (on a
    // Mac, Chrome, Edge or Chromium if one is installed).
    match crate::render::Edge::find(config_dir().join("render")) {
        Some(edge) => {
            #[cfg(target_os = "macos")]
            log::line(&format!("Pictures are drawn with {}", edge.exe().display()));
            bridge.set_painter(Some(Box::new(edge)));
        }
        None => log::line(NO_BROWSER),
    }
    let (capture_tx, capture_rx) = channel::<CaptureMessage>();
    #[cfg(any(windows, target_os = "macos"))]
    crate::capture::spawn(wow_dir.clone(), capture_tx);
    #[cfg(any(windows, target_os = "macos"))]
    crate::keys::spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    drop(capture_tx);

    let mut origin = origin;
    let mut checked_origin = Instant::now();
    let mut recheck = false;
    let mut signed_out = false;
    // For the restart note: the bridge only knows WoW runs once the capture has looked.
    let mut restart_since = Instant::now();
    let mut seen_running = false;
    // When to look for addon files gone next (Put back missing addon files automatically).
    let mut put_back_at = Instant::now() + PUT_BACK_EVERY;
    // The older T3 Code the tray's notification last went out for: once per start and version.
    let mut older_said: Option<String> = None;
    // The Screen Recording notification went out, until the capture works again.
    #[cfg(target_os = "macos")]
    let mut screen_told = false;
    loop {
        while let Ok(message) = capture_rx.try_recv() {
            bridge.on_capture(message);
        }
        while let Ok(control) = control_rx.try_recv() {
            match control {
                // Picked up from the config below, paired with the server it's for.
                Control::SignedIn => {
                    signed_out = false;
                    recheck = true;
                }
                Control::SignedOut => {
                    // T3 now refuses the empty token, which the game window shows too, and
                    // what was read with the sign-in goes.
                    signed_out = true;
                    client.set_token("");
                    bridge.set_client(client.clone());
                    bridge.sign_out();
                }
                Control::Reinstall => {
                    if reinstall(&app, &slots) {
                        // Its notice already said to restart WoW.
                        restart = Some(RestartNote { line: "restart WoW to load the reinstalled addon (not /reload)".into(), notice: None });
                        (restart_since, seen_running) = (Instant::now(), false);
                        // The reinstall blanked the slots and signals already written: write them again.
                        bridge.republish();
                    }
                }
                Control::Repair => {
                    if let Some(note) = put_back(&app, &slots, true) {
                        restart = Some(note);
                        (restart_since, seen_running) = (Instant::now(), false);
                        // A slot written blank again lost its update: write them again.
                        bridge.republish();
                    }
                }
                Control::Trust(trust) => bridge.set_trust(trust),
            }
        }
        // Something replaced or cleaned the addon folder (an addon manager, a copy of the
        // addon from elsewhere): put back what it took, so the next start of WoW finds it.
        // The addon itself gone means it was uninstalled: that stays as it is.
        if Instant::now() >= put_back_at {
            put_back_at = Instant::now() + PUT_BACK_EVERY;
            if slots.missing_files() && slots.installed_version().is_some() && Config::load().put_back_files() {
                log::line("Some addon files are gone from the AddOns folder: putting them back");
                match put_back(&app, &slots, false) {
                    Some(note) => {
                        restart = Some(note);
                        (restart_since, seen_running) = (Instant::now(), false);
                        bridge.republish();
                    }
                    None if slots.missing_files() => put_back_at = Instant::now() + PUT_BACK_RETRY,
                    None => {}
                }
            }
        }
        bridge.tick();
        let s = bridge.status();
        // Look again now and then: `t3wow.exe login` may have changed (or cleared) the saved
        // origin or signed in, or the T3 on this PC restarted on another port. A saved origin
        // always wins. The origin and token come from one read of the config, so a token
        // only goes to the server it was issued for.
        if recheck || checked_origin.elapsed() > Duration::from_secs(30) {
            checked_origin = Instant::now();
            recheck = false;
            // A hand edit of config.json (or `run`, which has no tray) counts too.
            let config = Config::load();
            bridge.set_trust(config.trust());
            // A key the start couldn't read (config.json busy at that moment) still arrives.
            if let Some(key) = config.game_key_bytes() {
                bridge.set_game_key(Some(key));
            }
            bridge.set_sign_in_read_only(config.sign_in_read_only(&config.server_key()));
            let found = t3::target();
            let token = found.as_ref().and_then(|t| t.token.clone()).unwrap_or_default();
            if let Some(found) = found.filter(|t| t.origin != origin || t.key != key) {
                log::line(&format!("T3 moved to {}", found.origin));
                // Another T3 (not the same one on another port): another sign-in, whose
                // threads replace the previous one's in the game. A tray sign-out was the
                // previous T3's; this one is signed in if it has a saved sign-in.
                let switched = found.key != key;
                (key, origin) = (found.key, found.origin);
                client = Arc::new(McpClient::new(&origin, &token));
                bridge.set_client(client.clone());
                bridge.set_provider_caches(local_provider_caches());
                bridge.set_t3_settings(local_t3_settings());
                if switched {
                    bridge.set_login(&key);
                    signed_out = token.is_empty();
                }
            } else if !token.is_empty() && token != client.token() {
                // A sign-in from the terminal: use its token (it also ends a tray sign-out).
                log::line("Using the new T3 sign-in.");
                signed_out = false;
                client.set_token(&token);
                bridge.set_client(client.clone());
            }
        }
        let t3_line = if signed_out {
            "T3: signed out: choose Sign in to T3".to_string()
        } else if s.auth_needed {
            "T3: sign-in expired: choose Sign in to T3".to_string()
        } else if let (false, Some(problem)) = (s.t3_ok, &s.t3_problem) {
            format!("T3: {}", problem.chars().take(70).collect::<String>())
        } else if s.t3_ok && s.sign_in_read_only {
            shown(READ_ONLY_LINE).to_string()
        } else if s.t3_ok {
            let connected = format!("T3: connected ({} machine{})", s.machines, if s.machines == 1 { "" } else { "s" });
            match &s.older_t3 {
                Some(older) => format!("{connected}. {}", older.line()),
                None => connected,
            }
        } else {
            "T3: connecting…".to_string()
        };
        if let Some(notice) = older_t3_notice(s.older_t3.as_ref(), &mut older_said) {
            app.notify(notice);
        }
        let used = s.slot.saturating_sub(1).min(SLOT_COUNT);
        let wow_line = match (s.wow_running, s.in_game, s.signals) {
            (false, _, _) => "not running".to_string(),
            (true, false, _) => "running, waiting for the addon (log in, or /reload)".to_string(),
            (true, true, _) if s.write_problem.is_some() => s.write_problem.as_deref().unwrap_or("").chars().take(70).collect::<String>(),
            (true, true, _) if used >= SLOT_COUNT => "connected, all 200 updates used: Reload UI in the game".to_string(),
            (true, true, true) => format!("connected, updates arrive in about a second ({used}/{SLOT_COUNT} updates used)"),
            (true, true, false) if s.signals_lost && slots.missing_files() => "connected, signal files missing: choose Repair addon files, then restart WoW".to_string(),
            (true, true, false) if s.signals_out => format!("connected, updates on a timer until WoW restarts ({used}/{SLOT_COUNT} updates used)"),
            (true, true, false) => format!("connected, updates on a timer ({used}/{SLOT_COUNT} updates used)"),
        };
        // The restart comes first, where the tray's tooltip still shows it.
        if s.wow_running {
            seen_running = true;
        }
        if restart.is_some() && !restart_note_stays(s.wow_running, seen_running, restart_since.elapsed()) {
            restart = None;
        }
        let wow_line = match &mut restart {
            Some(note) if s.wow_running => {
                if let Some(notice) = note.notice.take() {
                    app.notify(notice);
                }
                format!("WoW: {}; {wow_line}", note.line)
            }
            _ => format!("WoW: {wow_line}"),
        };
        // macOS refuses the capture: the game can be running and still never be seen.
        #[cfg(target_os = "macos")]
        let screen_off = s.wow_running && crate::capture::screen_refused();
        #[cfg(target_os = "macos")]
        let wow_line = if screen_off {
            if !screen_told && menu_bar() {
                app.notify(screen_off_notice(updater::running_app().is_some()));
            }
            screen_told = true;
            shown(SCREEN_OFF_LINE).to_string()
        } else {
            screen_told = false;
            wow_line
        };
        let restart_needed = restart.is_some() && s.wow_running;
        let attention = signed_out || s.auth_needed || s.sign_in_read_only || (!s.t3_ok && s.t3_problem.is_some()) || s.write_problem.is_some() || restart_needed;
        #[cfg(target_os = "macos")]
        let attention = attention || screen_off;
        app.set(t3_line, wow_line, attention, !signed_out && !s.auth_needed);
        // Mid-poll, each tick reads T3 for one request or TICK_BUDGET_MS, whichever is
        // longer, and the capture and the controls above get their turn in between: no nap.
        if !bridge.polling() {
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows shows this much of a notification's text (`szInfo`, less its 0).
    fn fits_a_toast(text: &str) -> bool {
        text.encode_utf16().count() <= 255
    }

    fn connected(origin: &str, scope: Option<&str>, read_only: bool, switched: bool) -> t3::Connected {
        t3::Connected { origin: origin.into(), label: server_label(Some(origin).filter(|o| !o.contains("127.0.0.1"))), scope: scope.map(str::to_string), read_only, switched }
    }

    #[test]
    fn a_read_only_sign_in_warns_and_says_to_pick_auto_or_full_access() {
        let notice = signed_in_notice(&connected("http://127.0.0.1:3773", Some("orchestration:read"), true, false));
        assert!(matches!(notice.level, Level::Warning));
        assert_eq!(notice.title, "Signed in to T3, but read only");
        assert!(notice.text.starts_with("The sign-in is Read only"), "{}", notice.text);
        #[cfg(not(target_os = "macos"))]
        {
            assert!(notice.text.contains("Choose Sign in to T3… and pick Auto or Full access"), "{}", notice.text);
            assert!(notice.text.contains("T3 WoW is signed in to T3 on this PC."), "{}", notice.text);
        }
        // Tests run as `t3wow run` on a Mac, which has no menu: its own command signs in again.
        #[cfg(target_os = "macos")]
        {
            assert!(notice.text.contains("Run t3wow login and pick Auto or Full access"), "{}", notice.text);
            assert!(notice.text.contains("T3 WoW is signed in to T3 on this Mac."), "{}", notice.text);
        }
        assert!(fits_a_toast(&notice.text), "{} units: {}", notice.text.encode_utf16().count(), notice.text);

        // Switched to another machine's T3, with a long name: still whole, warning first.
        let notice = signed_in_notice(&connected("https://gaming-pc.tailnet-1234.ts.net:3773", Some("orchestration:read"), true, true));
        assert!(matches!(notice.level, Level::Warning));
        assert_eq!(notice.title, "Switched T3, but the sign-in is read only");
        assert!(fits_a_toast(&notice.text), "{} units: {}", notice.text.encode_utf16().count(), notice.text);

        // A saved sign-in that can only read says so too, on a switch back to it.
        let notice = signed_in_notice(&connected("https://box.lan", None, true, true));
        assert!(notice.text.contains("Read only"), "{}", notice.text);

        // Auto (or Full access) is the plain notice it always was.
        let notice = signed_in_notice(&connected("http://127.0.0.1:3773", Some("orchestration:read orchestration:operate"), false, false));
        assert!(matches!(notice.level, Level::Info));
        #[cfg(not(target_os = "macos"))]
        assert_eq!((notice.title.as_str(), notice.text.as_str()), ("Signed in to T3", "T3 WoW is signed in to T3 on this PC."));
        #[cfg(target_os = "macos")]
        assert_eq!((notice.title.as_str(), notice.text.as_str()), ("Signed in to T3", "T3 WoW is signed in to T3 on this Mac."));

        // The page's own notice says which level to pick, whole.
        assert!(browser_open_text().contains("Pick Auto"));
        assert!(fits_a_toast(&browser_open_text()), "{}", browser_open_text());
    }

    /// The menu bar names its own items; `t3wow run` in Terminal, which has no menu, the
    /// commands that do the same.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_mac_names_the_menu_bar_items_or_the_commands() {
        for (menu, terminal) in [SIGN_IN_AGAIN, READ_ONLY_LINE, SIGN_IN_NEEDED, NO_LOCAL_T3, SCREEN_OFF_LINE] {
            assert!(!menu.contains("t3wow "), "{menu}");
            assert!(terminal.contains("t3wow") || terminal.contains("Terminal"), "{terminal}");
            assert!(!format!("{menu}{terminal}").contains("this PC"));
        }
        assert!(SIGN_IN_AGAIN.0.contains("menu bar"));
        assert!(SCREEN_OFF_LINE.0.ends_with("Open Screen Recording settings…"));
        for in_app in [true, false] {
            let notice = screen_off_notice(in_app);
            assert_eq!(notice.level, Level::Warning);
            assert!(notice.text.contains("Open Screen Recording settings…"), "{}", notice.text);
            assert!(fits_a_toast(&notice.text), "{}", notice.text);
        }
    }

    /// In the app the Screen Recording switch is T3 WoW's, and one left by another build may
    /// need removing and adding again; a bare t3wow started in Terminal is allowed as
    /// Terminal and restarts.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_mac_names_the_screen_recording_switch_it_needs() {
        let app = screen_off_notice(true).text;
        assert!(app.contains("turn on T3 WoW") && app.contains("If it shows as on already, remove it"), "{app}");
        assert!(!app.contains("update"), "a signed update keeps the switch: {app}");
        assert!(!app.contains("Terminal") && !app.contains("t3wow") && !app.contains("start it again"), "{app}");
        let bare = screen_off_notice(false).text;
        assert!(bare.contains("Terminal") && bare.contains("quit T3 WoW and start it again"), "{bare}");
    }

    /// The app's menu installs a release, a bare t3wow's opens its download page: the notice
    /// names the item each one shows (tray::update_text).
    #[cfg(target_os = "macos")]
    #[test]
    fn a_mac_names_the_update_item_its_menu_shows() {
        let app = mac_update_out_text("0.11.0", "0.10.0", true);
        assert_eq!(app, "T3 WoW 0.11.0 is out (you have 0.10.0). Choose Update to 0.11.0 in the menu bar.");
        let bare = mac_update_out_text("0.11.0", "0.10.0", false);
        assert!(bare.contains("Choose “0.11.0 is out” in the menu bar to open its download page"), "{bare}");
        assert_eq!(update_out_text("0.11.0", "0.10.0"), bare, "the tests run as a bare t3wow");
        assert!(fits_a_toast(&app) && fits_a_toast(&bare));
    }

    /// A failed update says why once (no doubled full stop) and where to try again: the
    /// tray menu on Windows, the menu bar on a Mac.
    #[test]
    fn a_failed_update_says_why_and_where_to_try_again() {
        let menu = if cfg!(target_os = "macos") { "the menu bar" } else { "the tray menu" };
        let expected = format!("the download timed out. Choose Update to 0.11.0 in {menu} to try again.");
        assert_eq!(update_failed_text("the download timed out", "0.11.0"), expected);
        assert_eq!(update_failed_text("the download timed out.", "0.11.0"), expected);
        assert!(!update_failed_text("x.", "0.11.0").contains(".."));
    }

    /// A copy macOS made read-only can't update until it is moved: the failure, the notice at
    /// startup and a new release's notice say how, and never to try again from here.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_translocated_app_says_how_to_move_it() {
        let failed = update_failed_text(updater::TRANSLOCATED, "0.11.0");
        assert_eq!(failed, updater::TRANSLOCATED);
        assert!(failed.ends_with("Quit T3 WoW, drag T3 WoW into Applications in Finder, and open it from there."), "{failed}");
        assert!(!failed.contains("try again") && !failed.contains("..") && !failed.contains("tray"), "{failed}");
        let notice = translocated_notice();
        assert_eq!((notice.level, notice.title.as_str(), notice.text.as_str()), (Level::Warning, "Move T3 WoW to Applications", updater::TRANSLOCATED));
        let out = translocated_update_out_text("0.11.0", "0.10.0");
        assert_eq!(out, "T3 WoW 0.11.0 is out (you have 0.10.0). To update, quit T3 WoW, drag T3 WoW into Applications in Finder, and open it from there.");
        assert!(fits_a_toast(&failed) && fits_a_toast(&out));
        // The tests aren't translocated: a new release's notice is a bare t3wow's.
        assert!(!updater::running_translocated());
    }

    /// Choosing an addon action before the WoW folder is known names the menu it is in.
    #[test]
    fn the_menu_is_named_as_each_system_shows_it() {
        assert_eq!(MENU_NAME, if cfg!(target_os = "macos") { "the menu bar" } else { "the tray menu" });
    }

    #[test]
    fn an_older_t3_code_is_announced_once_per_version_it_runs() {
        let older = crate::made_for::older_t3("0.1.7").unwrap();
        let mut said = None;
        let notice = older_t3_notice(Some(&older), &mut said).expect("the first time");
        assert_eq!(notice.level, Level::Warning);
        assert_eq!(notice.title, format!("Update T3 Code to {}", crate::made_for::T3_CODE));
        assert!(notice.text.contains("T3 Code 0.17.0") && notice.text.contains("Settings > About > Check for Updates"), "{}", notice.text);
        assert!(fits_a_toast(&notice.text), "{}", notice.text);
        assert_eq!(older_t3_notice(Some(&older), &mut said), None, "not with every poll");
        assert_eq!(older_t3_notice(None, &mut said), None);
        let older_still = crate::made_for::older_t3("0.1.8").unwrap();
        assert!(older_t3_notice(Some(&older_still), &mut said).is_some(), "another older version is said again");
    }

    #[test]
    fn notices_queue_for_the_tray_and_the_oldest_make_room() {
        let app = App::new();
        app.notify(Notice::new(Level::Info, "a", "1"));
        app.notify(Notice::new(Level::Error, "b", "2"));
        assert_eq!(app.take_notices(), [Notice::new(Level::Info, "a", "1"), Notice::new(Level::Error, "b", "2")]);
        assert!(app.take_notices().is_empty(), "each is taken once");
        for n in 0..25 {
            app.notify(Notice::new(Level::Info, "n", n.to_string()));
        }
        let kept = app.take_notices();
        assert_eq!(kept.len(), 20);
        assert_eq!(kept[0].text, "5");
    }

    #[test]
    fn the_activity_line_shows_each_running_task_until_it_ends() {
        let app = App::new();
        assert_eq!(app.activity(), None);
        app.set_activity(Task::Update, Some(update_step_text("0.6.0", Step::Downloading(Some(42)))));
        assert_eq!(app.activity().as_deref(), Some("Downloading update 0.6.0… 42%"));
        app.set_activity(Task::SignIn, Some("Signing in to T3…".into()));
        assert_eq!(app.activity().as_deref(), Some("Signing in to T3…\nDownloading update 0.6.0… 42%"));
        app.set_activity(Task::Update, None);
        app.set_activity(Task::SignIn, None);
        assert_eq!(app.activity(), None);
    }

    #[test]
    fn choosing_update_installs_the_newest_release_github_knows_of() {
        let release = |version: &str| Release {
            version: version.into(),
            exe_url: format!("https://x/{version}/exe"),
            sha_url: format!("https://x/{version}/sha"),
            sig_url: None,
        };
        assert_eq!(release_to_install(release("0.7.0"), Ok(Some(release("0.7.1")))), Some(release("0.7.1")), "a newer one came out since the offer");
        assert_eq!(release_to_install(release("0.7.0"), Ok(Some(release("0.7.0")))), Some(release("0.7.0")));
        assert_eq!(release_to_install(release("0.7.1"), Ok(Some(release("0.7.0")))), Some(release("0.7.0")), "the offered one was withdrawn, an older one is newest");
        assert_eq!(release_to_install(release("0.7.0"), Ok(None)), None, "withdrawn, and nothing newer than this companion is left");
        assert_eq!(release_to_install(release("0.7.0"), Err("GitHub answered 403".into())), Some(release("0.7.0")), "a failed look still installs the offer");
    }

    #[test]
    fn update_steps_read_as_progress() {
        assert_eq!(update_step_text("0.6.0", Step::Downloading(None)), "Downloading update 0.6.0…");
        assert_eq!(update_step_text("0.6.0", Step::Checking), "Checking the download of 0.6.0…");
        assert_eq!(update_step_text("0.6.0", Step::Installing), "Installing update 0.6.0…");
        #[cfg(not(target_os = "macos"))]
        assert_eq!(t3_named("this PC"), "T3 on this PC");
        #[cfg(target_os = "macos")]
        assert_eq!(t3_named("this Mac"), "T3 on this Mac");
        assert_eq!(t3_named("http://box:3773"), "T3 at http://box:3773");
    }

    #[test]
    fn a_notice_left_for_the_next_start_is_shown_once() {
        let dir = std::env::temp_dir().join(format!("t3wow-notice-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("restart-notice.json");
        assert_eq!(take_notice_at(&file), None);
        let notice = Notice::new(Level::Info, "T3 WoW updated to 0.6.0", "Updated from 0.5.0.");
        save_notice_at(&file, &notice);
        assert_eq!(take_notice_at(&file), Some(notice));
        assert_eq!(take_notice_at(&file), None);
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(take_notice_at(&file), None, "a damaged one is dropped");
        assert!(!file.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn repair_addon_files_from_the_tray_puts_back_only_whats_missing() {
        let dir = std::env::temp_dir().join(format!("t3wow-repair-tray-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let slots = Slots::with_fixture_fonts(&dir, "16001");
        slots.install().unwrap();
        let app = App::new();
        let (tx, rx) = channel();

        // Nothing missing: it says so.
        tx.send(Control::Repair).unwrap();
        wait(&app, &rx, 0, Some(&slots));
        assert_eq!(app.take_notices()[0].title, "Addon files: nothing missing");

        // The folder replaced by a plain copy of the addon; a slot holds an update.
        for made in ["sig", "ctl", "img"] {
            std::fs::remove_dir_all(slots.addon_dir.join(made)).unwrap();
        }
        let inbox = dir.join("T3WoW_S003").join("Inbox.lua");
        std::fs::write(&inbox, "T3WoW_Inbox = {}").unwrap();
        tx.send(Control::Repair).unwrap();
        wait(&app, &rx, 0, Some(&slots));
        let notices = app.take_notices();
        assert_eq!(notices[0].title, "Addon files put back");
        assert!(notices[0].text.contains("missing addon files were put back"), "{}", notices[0].text);
        assert!(!slots.missing_files());
        assert_eq!(std::fs::read_to_string(&inbox).unwrap(), "T3WoW_Inbox = {}", "the slot's update stays");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_install_from_before_slot_xml_is_told_once_to_restart_wow() {
        let dir = std::env::temp_dir().join(format!("t3wow-slot-xml-note-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let slots = Slots::new(&dir, "16001");
        slots.install().unwrap();
        crate::slots::make_slots_old(&dir, "16001");
        let app = App::new();

        let note = put_back(&app, &slots, false).expect("a note to restart WoW");
        assert!(note.line.contains("not /reload"), "{}", note.line);
        let notice = note.notice.expect("said once WoW is seen running");
        assert_eq!(notice.title, "Restart WoW to load the new addon");
        assert!(notice.text.contains("update slots were changed"), "{}", notice.text);
        assert!(notice.text.contains(RESTART_WOW), "{}", notice.text);

        assert!(put_back(&app, &slots, false).is_none(), "nothing more to say next time");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_restart_note_lasts_until_the_game_that_had_the_old_files_stops() {
        let early = Duration::from_secs(1);
        let late = Duration::from_secs(60);
        // Just started: the capture hasn't looked yet, so the note waits for it.
        assert!(restart_note_stays(false, false, early));
        // WoW wasn't running: it will start with the new files.
        assert!(!restart_note_stays(false, false, late));
        // WoW runs with the old files, however long ago they changed...
        assert!(restart_note_stays(true, true, late));
        // ...until it stops.
        assert!(!restart_note_stays(false, true, early));
    }
}
