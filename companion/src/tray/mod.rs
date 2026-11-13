//! The system tray icon and its menu. The menu is the same everywhere; each platform's shell
//! (windows.rs, macos.rs) runs the main thread's loop and calls `Tray::tick` twice a second,
//! and the work happens in app::run on a background thread. Every action chosen here says
//! how it went: a notification (start, success, failure), the menu's activity line and
//! tooltip while it runs, or a message box where an answer is needed.
//!
//! What differs by platform comes from `platform`, one module per system with the same
//! names in each: `run` (the loop), `notify`, `confirm`, `ask_origin`, `open_path`,
//! `autostart_enabled` and `set_autostart`, and the texts that name the system
//! (`THIS_MACHINE`, `AUTOSTART`, `AUTOSTART_ON`, `AUTOSTART_OFF`, `NOT_A_CLIENT`). A module
//! rather than a trait: there is one shell per build, so nothing would ever choose between
//! two, and the texts can stay plain constants. A Mac's shell also draws the menu bar icon
//! (`icon`, a template image), adds the Screen Recording item (`Help`) below the WoW line, and
//! keeps the latest notice above the T3 line (`NoticeLine`).

use crate::app::{self, App, Control, Level, Notice, Status, Update};
use crate::config::{canonical_origin, server_label, Config, Trust, UpdateChecks, UpdateInterval};
use crate::log;
use crate::slots::Slots;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
#[cfg(windows)]
use tray_icon::Icon;
use tray_icon::{TrayIcon, TrayIconBuilder};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;
#[cfg(target_os = "macos")]
use self::macos as platform;
#[cfg(windows)]
use self::windows as platform;

/// A message box while the menu bar runs (main.rs's `message_box`).
#[cfg(target_os = "macos")]
pub use platform::message;
pub use platform::run;

/// The log's line for Quit, naming where it was chosen.
const QUIT_LINE: &str = if cfg!(target_os = "macos") { "Quit from the menu bar." } else { "Quit from the tray." };

/// Between two notifications, so one doesn't replace the other before it's read.
const NOTICE_GAP: Duration = Duration::from_secs(2);

/// Starts a fresh copy of the companion (after the single-instance lock is released) and quits.
/// On a Mac in the app, through Launch Services (`open -n`), as if opened from Finder: a new
/// copy of the app (the running one is still there), the one now on disk after an update.
fn relaunch() -> ! {
    crate::single_instance::release();
    #[cfg(target_os = "macos")]
    if let Some(app) = crate::updater::running_app() {
        match std::process::Command::new("/usr/bin/open").arg("-n").arg(&app).status() {
            Ok(status) if status.success() => std::process::exit(0),
            other => log::line(&format!("Couldn't reopen {} ({other:?}); starting its program instead.", app.display())),
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).spawn();
    }
    std::process::exit(0)
}

/// Reads WoW's interface version and, when the addon declares another, rewrites the
/// `## Interface:` lines (nothing else) and restarts the companion so it uses the new number
/// from then on. Slot addons show as out of date when WoW's patch raised the number.
fn match_interface(app: &App) {
    let Some(dir) = app.wow_dir.lock().unwrap().clone() else {
        crate::message_box("T3 WoW", "T3 WoW hasn't found the WoW folder yet: choose it first.");
        return;
    };
    let current = Config::load().interface();
    let Some(found) = crate::wow::interface_version(&dir) else {
        crate::message_box(
            "T3 WoW",
            &format!(
                "Couldn't read WoW's interface version in {}.\n\nIn the game, /dump select(4, GetBuildInfo()) prints it; then run this in a terminal:\n{} install --interface <number>",
                dir.display(),
                crate::BIN
            ),
        );
        return;
    };
    let wow = found.wow_version.map(|v| format!("WoW {v}")).unwrap_or_else(|| "WoW".into());
    if found.interface == current {
        crate::message_box("T3 WoW", &format!("The addon already matches {wow} (interface {current})."));
        return;
    }
    let question = format!(
        "{wow} uses interface {}; the addon says {current}.\n\nSwitch the addon to {}? Your sign-in and settings stay. WoW needs a full restart afterwards.",
        found.interface, found.interface
    );
    if !platform::confirm("T3 WoW", &question) {
        return;
    }
    match Slots::for_wow(&dir, &found.interface).set_interface() {
        Ok(_) => {
            Config::update(|c| c.interface = Some(found.interface.clone()));
            log::line(&format!("Interface version set to {} (was {current}); restarting the companion.", found.interface));
            crate::message_box("T3 WoW", &format!("Done. {}", app::RESTART_WOW));
            app::notice_after_restart(&Notice::new(
                Level::Info,
                format!("Addon set to interface {}", found.interface),
                format!("T3 WoW restarted with the new interface version. {}", app::RESTART_WOW),
            ));
            relaunch();
        }
        Err(error) => crate::message_box("T3 WoW", &format!("Couldn't rewrite the addon files: {error}")),
    }
}

#[cfg(windows)]
fn icon() -> Icon {
    Icon::from_rgba(include_bytes!("../../assets/tray-32.rgba").to_vec(), 32, 32).expect("tray icon")
}

/// The tooltip: what is under way first, then the status lines; Windows keeps 127 characters.
fn tooltip(activity: Option<&str>, status: &Status) -> String {
    let mut tip = String::from("T3 WoW");
    for line in activity.into_iter().chain([status.t3.as_str(), status.wow.as_str()]) {
        if !line.is_empty() {
            tip.push('\n');
            tip.push_str(line);
        }
    }
    tip.chars().take(127).collect()
}

/// The update item's text for each state of the update, with this companion's version;
/// `installs`: choosing it installs the release (else it opens the release's page).
fn update_text(update: &Update, current: &str, installs: bool) -> String {
    match update {
        Update::NotChecked => "Check for updates".to_string(),
        Update::Checking => "Checking for updates…".to_string(),
        Update::UpToDate => format!("Up to date ({current}): check again"),
        Update::Available(release) if installs => format!("Update to {} (now {current})", release.version),
        Update::Available(release) => format!("{} is out (now {current}): open the download page", release.version),
        Update::Installing(version) => format!("Updating to {version}…"),
        Update::Failed(_) => "Update check failed: try again".to_string(),
    }
}

/// The update item can't be chosen while a check or an install is under way.
fn update_enabled(update: &Update) -> bool {
    !matches!(update, Update::Checking | Update::Installing(_))
}

fn sign_in_text(signed_in: bool) -> &'static str {
    if signed_in {
        "Sign out of T3"
    } else {
        "Sign in to T3…"
    }
}

/// Which T3 the config says to use, and the other machine's used last.
#[derive(Clone, Debug, Default, PartialEq)]
struct Servers {
    origin: Option<String>,
    last_remote: Option<String>,
}

impl Servers {
    fn load() -> Servers {
        let config = Config::load();
        Servers { origin: config.origin.as_deref().map(canonical_origin), last_remote: config.last_remote.as_deref().map(canonical_origin) }
    }
}

/// What `Tray::tick` asks of the shell's loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum Tick {
    Stay,
    /// Quit was chosen: the loop ends and the companion exits.
    Quit,
}

/// The tray icon, its menu, and what the menu last showed. Lives on the main thread, which
/// must call `tick` twice a second.
pub struct Tray {
    app: Arc<App>,
    icon: TrayIcon,
    menu: Menu,
    activity_line: MenuItem,
    t3_line: MenuItem,
    wow_line: MenuItem,
    sign_in: MenuItem,
    server_menu: Submenu,
    this_pc: CheckMenuItem,
    remote: CheckMenuItem,
    connect_remote: MenuItem,
    wow_folder: MenuItem,
    repair: MenuItem,
    put_back: CheckMenuItem,
    reinstall: MenuItem,
    interface: MenuItem,
    trust_items: Vec<(Trust, CheckMenuItem)>,
    open_log: MenuItem,
    autostart: CheckMenuItem,
    updates: MenuItem,
    checks_menu: Submenu,
    check_on_start: CheckMenuItem,
    interval_items: Vec<(UpdateInterval, CheckMenuItem)>,
    manual_only: CheckMenuItem,
    quit: MenuItem,
    /// Choosing Update installs it (`updater::can_install`); else (a bare t3wow on a Mac) it
    /// opens the release's page.
    installs: bool,
    /// A Mac's permission items, shown below the WoW line while they apply.
    #[cfg(target_os = "macos")]
    help: platform::Help,
    /// A Mac's latest notice, above the T3 line, where it can always be read.
    #[cfg(target_os = "macos")]
    notice_line: platform::NoticeLine,
    shown_update: Option<Update>,
    shown_checks: Option<UpdateChecks>,
    checks_read: Instant,
    notices: VecDeque<Notice>,
    last_notice: Option<Instant>,
    shown_activity: Option<String>,
    servers: Option<Servers>,
    servers_read: Instant,
    shown: Status,
}

impl Tray {
    /// Builds the menu and the icon, and starts the update checks' thread. Called on the
    /// main thread once the system is ready for a tray icon.
    pub fn new(app: Arc<App>) -> Tray {
        let activity_line = MenuItem::new("", false, None);
        let t3_line = MenuItem::new("T3: starting…", false, None);
        let wow_line = MenuItem::new("WoW: starting…", false, None);
        let sign_in = MenuItem::new("Sign in to T3…", true, None);
        // The T3 in use: this machine, the other machine used last, or a new one.
        let server_menu = Submenu::new(format!("T3 server: {}", server_label(None)), true);
        let this_pc = CheckMenuItem::new(platform::THIS_MACHINE, true, true, None);
        let remote = CheckMenuItem::new("", true, false, None);
        let connect_remote = MenuItem::new("Connect to T3 on another machine…", true, None);
        let _ = server_menu.append_items(&[&this_pc, &PredefinedMenuItem::separator(), &connect_remote]);
        let wow_folder = MenuItem::new("Choose WoW folder…", true, None);
        let repair = MenuItem::new("Repair addon files", true, None);
        let put_back = CheckMenuItem::new("Put back missing addon files automatically", true, Config::load().put_back_files(), None);
        let reinstall = MenuItem::new("Reinstall addon", true, None);
        let interface = MenuItem::new("Match WoW's interface version…", true, None);
        // What the game may make T3 do: any addon can draw the strip the companion reads.
        let trust_now = Config::load().trust();
        let trust_items: Vec<(Trust, CheckMenuItem)> = Trust::ALL.iter().map(|&t| (t, CheckMenuItem::new(t.label(), true, t == trust_now, None))).collect();
        let trust_menu = Submenu::new("Commands from the game", true);
        for (_, item) in &trust_items {
            let _ = trust_menu.append(item);
        }
        let open_log = MenuItem::new("Open log", true, None);
        let autostart = CheckMenuItem::new(platform::AUTOSTART, true, platform::autostart_enabled(), None);
        let updates = MenuItem::new("Check for updates", true, None);
        // When the companion looks for updates by itself, or only when asked.
        let checks_now = Config::load().update_checks();
        let checks_menu = Submenu::new(checks_now.summary(), true);
        let check_on_start = CheckMenuItem::new("When T3 WoW starts", true, checks_now.on_start, None);
        let interval_items: Vec<(UpdateInterval, CheckMenuItem)> =
            UpdateInterval::ALL.iter().map(|&every| (every, CheckMenuItem::new(every.label(), true, every == checks_now.every, None))).collect();
        let manual_only = CheckMenuItem::new("Only when I check (no automatic checks)", true, checks_now.manual_only(), None);
        let _ = checks_menu.append_items(&[&check_on_start, &PredefinedMenuItem::separator()]);
        for (_, item) in &interval_items {
            let _ = checks_menu.append(item);
        }
        let _ = checks_menu.append_items(&[&PredefinedMenuItem::separator(), &manual_only]);
        let quit = MenuItem::new("Quit T3 WoW", true, None);
        let menu = Menu::new();
        let _ = menu.append_items(&[
            &t3_line,
            &wow_line,
            &PredefinedMenuItem::separator(),
            &sign_in,
            &server_menu,
            &wow_folder,
            &repair,
            &put_back,
            &reinstall,
            &interface,
            &trust_menu,
            &open_log,
            &autostart,
            &updates,
            &checks_menu,
            &PredefinedMenuItem::separator(),
            &quit,
        ]);
        // Look for updates as Update checks says: at start, on a timer, or not at all.
        {
            let app = app.clone();
            std::thread::Builder::new().name("update checks".into()).spawn(move || app::watch_for_updates(app)).expect("update checks thread");
        }
        let builder = TrayIconBuilder::new();
        #[cfg(windows)]
        let builder = builder.with_icon(icon());
        // Drawn from its alpha alone, in the menu bar's colour, light or dark.
        #[cfg(target_os = "macos")]
        let builder = builder.with_icon_templated(platform::icon());
        let icon = builder
            .with_tooltip("T3 WoW")
            .with_menu(Box::new(menu.clone()))
            .with_menu_on_left_click(true)
            .build()
            .expect("tray icon");

        Tray {
            app,
            icon,
            menu,
            activity_line,
            t3_line,
            wow_line,
            sign_in,
            server_menu,
            this_pc,
            remote,
            connect_remote,
            wow_folder,
            repair,
            put_back,
            reinstall,
            interface,
            trust_items,
            open_log,
            autostart,
            updates,
            checks_menu,
            check_on_start,
            interval_items,
            manual_only,
            quit,
            installs: crate::updater::can_install(),
            #[cfg(target_os = "macos")]
            help: platform::Help::new(),
            #[cfg(target_os = "macos")]
            notice_line: platform::NoticeLine::new(),
            shown_update: None,
            shown_checks: Some(checks_now),
            checks_read: Instant::now(),
            // What the previous companion said before it restarted (an update, a new WoW folder).
            notices: app::take_restart_notice().into_iter().collect(),
            last_notice: None,
            shown_activity: None,
            servers: None,
            servers_read: Instant::now(),
            shown: Status::default(),
        }
    }

    /// Handles what was chosen in the menu since the last call, refreshes the menu, the
    /// tooltip and the update item, and shows at most one notification (one per NOTICE_GAP).
    pub fn tick(&mut self) -> Tick {
        let mut next = Tick::Stay;
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if self.choose(event.id) == Tick::Quit {
                next = Tick::Quit;
            }
        }
        self.refresh();
        self.notices.extend(self.app.take_notices());
        if self.last_notice.is_none_or(|at| at.elapsed() >= NOTICE_GAP) {
            if let Some(notice) = self.notices.pop_front() {
                platform::notify(&self.icon, &notice);
                #[cfg(target_os = "macos")]
                self.notice_line.show(&self.menu, &self.t3_line, &notice);
                self.last_notice = Some(Instant::now());
            }
        }
        next
    }

    /// Does what the menu item `id` stands for.
    fn choose(&mut self, id: MenuId) -> Tick {
        let app = &self.app;
        #[cfg(target_os = "macos")]
        if self.help.choose(&id) {
            return Tick::Stay;
        }
        if id == *self.quit.id() {
            log::line(QUIT_LINE);
            return Tick::Quit;
        } else if id == *self.sign_in.id() {
            if self.shown.signed_in {
                app.sign_out();
            } else {
                app.sign_in();
            }
        } else if id == *self.this_pc.id() || id == *self.remote.id() || id == *self.connect_remote.id() {
            let current = Servers::load();
            let wanted = if id == *self.this_pc.id() {
                Some(None)
            } else if id == *self.remote.id() {
                current.last_remote.clone().map(Some)
            } else {
                let start = current.origin.clone().or(current.last_remote.clone()).unwrap_or_default();
                platform::ask_origin(&start).map(Some)
            };
            match wanted {
                Some(origin) if origin == current.origin && self.shown.signed_in => {
                    let name = app::t3_named(&server_label(origin.as_deref()));
                    app.notify(Notice::new(Level::Info, "T3 server", format!("T3 WoW already uses {name}.")));
                }
                Some(origin) => app.connect(Some(origin), true),
                None => {}
            }
            // A check item ticks itself when chosen: show what the config says.
            self.servers = None;
        } else if id == *self.updates.id() {
            let state = app.update.lock().unwrap().clone();
            match state {
                // A bare t3wow on a Mac can't install it: the release's page, to download the app.
                #[cfg(target_os = "macos")]
                Update::Available(_) if !self.installs => platform::open_url(&crate::updater::releases_page()),
                // The Mac app: macOS will ask for the Keychain and Screen Recording again.
                #[cfg(target_os = "macos")]
                Update::Available(release) => {
                    if platform::confirm(&format!("Update T3 WoW to {}?", release.version), platform::UPDATE_REGRANT) {
                        app.install_update(release, || relaunch());
                    }
                }
                #[cfg(not(target_os = "macos"))]
                Update::Available(release) => app.install_update(release, || relaunch()),
                Update::Checking | Update::Installing(_) => {}
                _ => app.check_updates(true),
            }
        } else if id == *self.check_on_start.id() || id == *self.manual_only.id() || self.interval_items.iter().any(|(_, item)| *item.id() == id) {
            let current = Config::load().update_checks();
            let wanted = if id == *self.check_on_start.id() {
                UpdateChecks { on_start: !current.on_start, ..current }
            } else if id == *self.manual_only.id() {
                current.toggle_manual_only()
            } else {
                let every = self.interval_items.iter().find(|(_, item)| *item.id() == id).map(|(every, _)| *every).unwrap_or(current.every);
                UpdateChecks { every, ..current }
            };
            if wanted != current {
                app.set_update_checks(wanted);
            }
            // A check item ticks itself when chosen: show what the config says.
            self.shown_checks = None;
        } else if id == *self.open_log.id() {
            platform::open_path(&log::path());
        } else if id == *self.reinstall.id() {
            app.send(Control::Reinstall);
        } else if id == *self.repair.id() {
            app.send(Control::Repair);
        } else if id == *self.put_back.id() {
            // A check item ticks itself when chosen: it now shows what was asked for.
            app.set_put_back_files(self.put_back.is_checked());
            self.put_back.set_checked(Config::load().put_back_files());
        } else if id == *self.interface.id() {
            match_interface(app);
        } else if let Some(&(trust, _)) = self.trust_items.iter().find(|(_, item)| *item.id() == id) {
            app.set_trust(trust);
            for (t, item) in &self.trust_items {
                item.set_checked(*t == trust);
            }
        } else if id == *self.autostart.id() {
            let wanted = self.autostart.is_checked();
            match platform::set_autostart(wanted) {
                Ok(()) if wanted => app.notify(Notice::new(Level::Info, format!("{}: on", platform::AUTOSTART), platform::AUTOSTART_ON)),
                Ok(()) => app.notify(Notice::new(Level::Info, format!("{}: off", platform::AUTOSTART), platform::AUTOSTART_OFF)),
                Err(error) => app.notify(Notice::new(Level::Error, format!("Couldn't change {}", platform::AUTOSTART), error.to_string())),
            }
            self.autostart.set_checked(platform::autostart_enabled());
        } else if id == *self.wow_folder.id() {
            let start = app.wow_dir.lock().unwrap().clone();
            let mut dialog = rfd::FileDialog::new().set_title("Choose your World of Warcraft _forever_ folder");
            if let Some(start) = start {
                dialog = dialog.set_directory(start);
            }
            let picked = {
                // On a Mac the panel takes the keyboard, which goes back once it closes.
                #[cfg(target_os = "macos")]
                let _front = platform::bring_to_front();
                dialog.pick_folder()
            };
            if let Some(picked) = picked {
                match crate::wow::normalize(&picked.to_string_lossy()).filter(|d| crate::wow::looks_like_client(d)) {
                    Some(dir) => {
                        Config::update(|c| c.wow_dir = Some(dir.to_string_lossy().into_owned()));
                        log::line(&format!("WoW folder set to {}; restarting the companion.", dir.display()));
                        app::notice_after_restart(&Notice::new(Level::Info, "WoW folder set", format!("T3 WoW restarted to use {}.", dir.display())));
                        relaunch();
                    }
                    None => crate::message_box("T3 WoW", &format!("{} {}", picked.display(), platform::NOT_A_CLIENT)),
                }
            }
        }
        Tick::Stay
    }

    /// Brings the menu and the tooltip up to date with the app and the config.
    fn refresh(&mut self) {
        let app = &self.app;
        let update = app.update.lock().unwrap().clone();
        if self.shown_update.as_ref() != Some(&update) {
            self.updates.set_text(update_text(&update, crate::updater::current_version(), self.installs));
            self.updates.set_enabled(update_enabled(&update));
            self.shown_update = Some(update);
        }
        // Update checks, from the config (a hand edit counts too).
        if self.shown_checks.is_none() || self.checks_read.elapsed() > Duration::from_secs(2) {
            let now = Config::load().update_checks();
            if self.shown_checks != Some(now) {
                self.checks_menu.set_text(now.summary());
                self.check_on_start.set_checked(now.on_start);
                for (every, item) in &self.interval_items {
                    item.set_checked(*every == now.every);
                }
                self.manual_only.set_checked(now.manual_only());
                self.shown_checks = Some(now);
            }
            self.checks_read = Instant::now();
        }
        // The T3 in use, from the config: a terminal `login --origin` changes it too.
        if self.servers.is_none() || self.servers_read.elapsed() > Duration::from_secs(2) {
            let now = Servers::load();
            if self.servers.as_ref() != Some(&now) {
                self.server_menu.set_text(format!("T3 server: {}", server_label(now.origin.as_deref())));
                self.this_pc.set_checked(now.origin.is_none());
                let _ = self.server_menu.remove(&self.remote);
                if let Some(last) = &now.last_remote {
                    self.remote.set_text(last);
                    self.remote.set_checked(now.origin.as_ref() == Some(last));
                    let _ = self.server_menu.insert(&self.remote, 1);
                }
                self.servers = Some(now);
            }
            self.servers_read = Instant::now();
        }
        let activity = app.activity();
        let status = app.status.lock().unwrap().clone();
        if activity != self.shown_activity {
            // The activity line heads the menu while something runs.
            let _ = self.menu.remove(&self.activity_line);
            if let Some(text) = &activity {
                self.activity_line.set_text(text.replace('\n', " · "));
                let _ = self.menu.insert(&self.activity_line, 0);
            }
        }
        #[cfg(target_os = "macos")]
        {
            self.help.refresh(&self.menu, &self.wow_line);
            self.notice_line.refresh(&self.menu);
        }
        if status != self.shown || activity != self.shown_activity {
            self.t3_line.set_text(&status.t3);
            self.wow_line.set_text(&status.wow);
            self.sign_in.set_text(sign_in_text(status.signed_in));
            let _ = self.icon.set_tooltip(Some(tooltip(activity.as_deref(), &status)));
            self.shown = status;
            self.shown_activity = activity;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updater::Release;

    #[test]
    fn the_tooltip_leads_with_what_is_under_way_and_fits() {
        let status = Status { t3: "T3: connected (2 machines)".into(), wow: "WoW: not running".into(), ..Status::default() };
        assert_eq!(tooltip(None, &status), "T3 WoW\nT3: connected (2 machines)\nWoW: not running");
        assert_eq!(tooltip(Some("Downloading update 0.6.0… 42%"), &status), "T3 WoW\nDownloading update 0.6.0… 42%\nT3: connected (2 machines)\nWoW: not running");
        let long = Status { t3: "x".repeat(200), ..Status::default() };
        assert_eq!(tooltip(None, &long).chars().count(), 127);
    }

    #[test]
    fn the_tooltip_skips_empty_lines() {
        assert_eq!(tooltip(None, &Status::default()), "T3 WoW");
        let status = Status { wow: "WoW: running".into(), ..Status::default() };
        assert_eq!(tooltip(Some(""), &status), "T3 WoW\nWoW: running");
    }

    #[test]
    fn the_update_item_says_where_the_update_stands() {
        let release = Release { version: "0.11.0".into(), exe_url: String::new(), sha_url: String::new(), sig_url: None };
        let cases = [
            (Update::NotChecked, "Check for updates", true),
            (Update::Checking, "Checking for updates…", false),
            (Update::UpToDate, "Up to date (0.10.0): check again", true),
            (Update::Available(release), "Update to 0.11.0 (now 0.10.0)", true),
            (Update::Installing("0.11.0".into()), "Updating to 0.11.0…", false),
            (Update::Failed("timed out".into()), "Update check failed: try again", true),
        ];
        for (update, text, enabled) in cases {
            assert_eq!(update_text(&update, "0.10.0", true), text);
            assert_eq!(update_enabled(&update), enabled, "{update:?}");
        }
    }

    /// A bare t3wow on a Mac can't install a release: the item says it's out and opens its
    /// page; every other state reads as where it installs.
    #[test]
    fn where_updates_dont_install_the_item_says_the_release_is_out() {
        let release = Release { version: "0.11.0".into(), exe_url: String::new(), sha_url: String::new(), sig_url: None };
        assert_eq!(update_text(&Update::Available(release), "0.10.0", false), "0.11.0 is out (now 0.10.0): open the download page");
        for update in [Update::NotChecked, Update::Checking, Update::UpToDate, Update::Installing("0.11.0".into()), Update::Failed("x".into())] {
            assert_eq!(update_text(&update, "0.10.0", false), update_text(&update, "0.10.0", true));
        }
        // Tests don't run in the Mac app: there, a bare program.
        assert_eq!(crate::updater::can_install(), cfg!(windows));
    }

    #[test]
    fn the_sign_in_item_offers_the_other_one() {
        assert_eq!(sign_in_text(false), "Sign in to T3…");
        assert_eq!(sign_in_text(true), "Sign out of T3");
    }
}
