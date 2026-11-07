//! The companion's work, on a background thread: find WoW and T3, keep the addon files in
//! place, sign in when needed, and run the bridge. The tray (or the console, for `run`)
//! only shows what this reports.

use crate::bridge::{Bridge, CaptureMessage};
use crate::config::{config_dir, now_ms, Config};
use crate::log;
use crate::slots::Slots;
use crate::t3::{self, McpClient, T3};
use crate::updater::{self, Release};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// A new sign-in finished.
    SignedIn(String),
    SignedOut,
    Reinstall,
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

pub struct App {
    pub status: Mutex<Status>,
    pub control: Mutex<Option<Sender<Control>>>,
    signing_in: AtomicBool,
    pub wow_dir: Mutex<Option<PathBuf>>,
    pub update: Mutex<Update>,
}

impl App {
    pub fn new() -> Arc<App> {
        Arc::new(App {
            status: Mutex::new(Status::default()),
            control: Mutex::new(None),
            signing_in: AtomicBool::new(false),
            wow_dir: Mutex::new(None),
            update: Mutex::new(Update::NotChecked),
        })
    }

    fn set(&self, t3: impl Into<String>, wow: impl Into<String>, attention: bool, signed_in: bool) {
        *self.status.lock().unwrap() = Status { t3: t3.into(), wow: wow.into(), attention, signed_in };
    }

    /// Forgets the T3 sign-in; the menu then offers Sign in again.
    pub fn sign_out(&self) {
        Config::update(|c| {
            c.token = None;
            c.token_expires_at = None;
        });
        log::line("Signed out of T3.");
        self.send(Control::SignedOut);
    }

    /// Looks for a newer release in the background (once at a time).
    pub fn check_updates(self: &Arc<App>) {
        {
            let mut update = self.update.lock().unwrap();
            if matches!(*update, Update::Checking | Update::Installing(_)) {
                return;
            }
            *update = Update::Checking;
        }
        let app = Arc::clone(self);
        std::thread::spawn(move || {
            let result = match updater::check() {
                Ok(Some(release)) => {
                    log::line(&format!("Update available: {}", release.version));
                    Update::Available(release)
                }
                Ok(None) => Update::UpToDate,
                Err(error) => {
                    log::line(&format!("Update check failed: {error}"));
                    Update::Failed(error)
                }
            };
            *app.update.lock().unwrap() = result;
        });
    }

    /// Downloads and swaps in the release; `then` runs on success (the tray restarts).
    pub fn install_update(self: &Arc<App>, release: Release, then: impl FnOnce() + Send + 'static) {
        *self.update.lock().unwrap() = Update::Installing(release.version.clone());
        let app = Arc::clone(self);
        std::thread::spawn(move || match updater::install(&release) {
            Ok(()) => {
                log::line(&format!("Updated to {}; restarting.", release.version));
                then();
            }
            Err(error) => {
                log::line(&format!("Update to {} failed: {error}", release.version));
                *app.update.lock().unwrap() = Update::Failed(error);
            }
        });
    }

    pub fn send(&self, control: Control) {
        if let Some(tx) = self.control.lock().unwrap().as_ref() {
            let _ = tx.send(control);
        }
    }

    /// Opens T3's sign-in page in the browser (once at a time) and hands the token over.
    pub fn sign_in(self: &Arc<App>) {
        if self.signing_in.swap(true, Ordering::SeqCst) {
            return;
        }
        let app = Arc::clone(self);
        std::thread::spawn(move || {
            match t3::origin() {
                None => log::line("Can't sign in: T3 Code is not running on this PC."),
                Some(origin) => match t3::login(&origin, &|m| log::line(m)) {
                    // `login --origin` changed where T3 is while the browser was open: this
                    // token is for the old one.
                    Ok(_) if t3::origin().as_deref() != Some(origin.as_str()) => {
                        log::line(&format!("T3 moved away from {origin} during the sign-in: sign in again."));
                    }
                    Ok(signed) => {
                        Config::update(|c| {
                            c.token = Some(signed.token.clone());
                            c.token_expires_at = signed.expires_at;
                        });
                        log::line(&format!("Signed in to {origin} ({}).", if signed.scope.is_empty() { "no scope reported" } else { &signed.scope }));
                        app.send(Control::SignedIn(signed.token));
                    }
                    Err(error) => log::line(&error),
                },
            }
            app.signing_in.store(false, Ordering::SeqCst);
        });
    }
}

fn wait(rx: &Receiver<Control>, seconds: u64) -> Option<Control> {
    rx.recv_timeout(Duration::from_secs(seconds)).ok()
}

/// The worker loop. Never returns.
pub fn run(app: Arc<App>) {
    let (control_tx, control_rx) = channel::<Control>();
    *app.control.lock().unwrap() = Some(control_tx);

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
        let _ = wait(&control_rx, 5);
    };
    *app.wow_dir.lock().unwrap() = Some(wow_dir.clone());
    let interface = Config::load().interface();

    // 2. The addon files: put back what's missing, install or update the addon.
    let slots = Slots::for_wow(&wow_dir, &interface);
    let mut restart_note = None;
    match slots.repair() {
        Ok(repair) => {
            if let Some(addon) = &repair.addon {
                log::line(&format!("Addon {addon} in {}", slots.addon_dir.display()));
            }
            if repair.created > 0 {
                log::line(&format!("Created {} missing addon files", repair.created));
            }
            if repair.addon.is_some() || repair.created > 0 {
                restart_note = Some("restart WoW to load the new addon files".to_string());
            }
        }
        Err(error) => log::line(&format!("Couldn't write the addon files: {error}")),
    }

    // 3. T3 on this PC.
    let origin = loop {
        if let Some(origin) = t3::origin() {
            break origin;
        }
        app.set("T3 Code isn't running on this PC", "WoW: waiting for T3", true, Config::load().token().is_some());
        let _ = wait(&control_rx, 5);
    };

    // 4. Signed in (the first time, the browser opens by itself).
    let token = loop {
        if let Some(token) = Config::load().token() {
            break token;
        }
        app.set("T3: sign-in needed (approve it in your browser)", "WoW: waiting for the sign-in", true, false);
        app.sign_in();
        if let Some(Control::SignedIn(token)) = wait(&control_rx, 600) {
            break token;
        }
    };

    // 5. The bridge, fed by the screen capture.
    let mut client = Arc::new(McpClient::new(&origin, &token));
    let mut bridge = Bridge::new(
        client.clone() as Arc<dyn T3>,
        Box::new(Slots::for_wow(&wow_dir, &interface)),
        Some(config_dir().join("state.json")),
        Box::new(log::line),
        Box::new(now_ms),
    );
    log::line(&format!("T3 WoW companion {}: T3 at {origin}, WoW at {}", env!("CARGO_PKG_VERSION"), wow_dir.display()));
    let (capture_tx, capture_rx) = channel::<CaptureMessage>();
    #[cfg(windows)]
    crate::capture::spawn(wow_dir.clone(), capture_tx);
    #[cfg(not(windows))]
    drop(capture_tx);

    let mut origin = origin;
    let mut checked_origin = Instant::now();
    let mut signed_out = false;
    loop {
        while let Ok(message) = capture_rx.try_recv() {
            bridge.on_capture(message);
        }
        while let Ok(control) = control_rx.try_recv() {
            match control {
                Control::SignedIn(token) => {
                    signed_out = false;
                    client.set_token(&token);
                    bridge.set_client(client.clone());
                }
                Control::SignedOut => {
                    // T3 now refuses the empty token, which the game window shows too.
                    signed_out = true;
                    client.set_token("");
                    bridge.set_client(client.clone());
                }
                Control::Reinstall => match slots.install() {
                    Ok(()) => {
                        log::line("Reinstalled the addon. Restart WoW to load it.");
                        restart_note = Some("restart WoW to load the reinstalled addon".into());
                    }
                    Err(error) => log::line(&format!("Reinstall failed: {error}")),
                },
            }
        }
        bridge.tick();
        let s = bridge.status();
        // Look again now and then: `t3wow.exe login` may have changed (or cleared) the saved
        // origin or signed in, or the T3 on this PC restarted on another port. A saved origin
        // always wins.
        if checked_origin.elapsed() > Duration::from_secs(30) {
            checked_origin = Instant::now();
            let token = Config::load().token().unwrap_or_default();
            if let Some(found) = t3::origin().filter(|o| *o != origin) {
                log::line(&format!("T3 moved to {found}"));
                origin = found;
                client = Arc::new(McpClient::new(&origin, &token));
                bridge.set_client(client.clone());
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
        } else if s.t3_ok {
            format!("T3: connected ({} machine{})", s.machines, if s.machines == 1 { "" } else { "s" })
        } else {
            "T3: connecting…".to_string()
        };
        let wow_line = match (s.wow_running, s.in_game, s.signals) {
            (false, _, _) => "WoW: not running".to_string(),
            (true, false, _) => "WoW: running, waiting for the addon (log in, or /reload)".to_string(),
            (true, true, true) => "WoW: in game, updates arrive in about a second".to_string(),
            (true, true, false) => "WoW: in game, updates on a timer".to_string(),
        };
        let wow_line = match &restart_note {
            Some(note) if s.wow_running => format!("{wow_line} ({note})"),
            _ => wow_line,
        };
        if !s.wow_running {
            restart_note = None;
        }
        let attention = signed_out || s.auth_needed || (!s.t3_ok && s.t3_problem.is_some());
        app.set(t3_line, wow_line, attention, !signed_out && !s.auth_needed);
        std::thread::sleep(Duration::from_millis(250));
    }
}
