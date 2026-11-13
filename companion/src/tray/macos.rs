//! The menu bar on a Mac: the run loop on the main thread (NSApplication, as an accessory
//! app: a menu bar icon, no Dock icon), notifications, Start at login (a LaunchAgent),
//! alerts, the address box, and the texts that name the Mac. Also the two items the Mac's
//! permissions call for, which appear only while they apply: Screen Recording settings when
//! macOS won't let the companion see the game, and another try at the Keychain after it
//! refused.

use super::{Tick, Tray};
use crate::app::{App, Level, Notice};
use crate::config::parse_origin_input;
use crate::keychain::Problem;
use crate::log;
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{sel, MainThreadMarker};
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSApplication, NSApplicationActivationPolicy, NSTextField};
use objc2_foundation::{NSBundle, NSPoint, NSRect, NSSize, NSString, NSTimer};
use std::cell::RefCell;
use std::io;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tray_icon::menu::{Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon};

/// The server menu's item for the T3 on this machine.
pub const THIS_MACHINE: &str = "This Mac";
/// The autostart item, and the start of its notifications' titles.
pub const AUTOSTART: &str = "Start at login";
pub const AUTOSTART_ON: &str = "T3 WoW starts by itself when you log in to this Mac.";
pub const AUTOSTART_OFF: &str = "T3 WoW no longer starts when you log in to this Mac. Start it yourself before you play.";
/// After the chosen folder's path, when it isn't a WoW client.
pub const NOT_A_CLIENT: &str = "doesn't look like a WoW client folder (no World of Warcraft.app or Interface folder).";

/// The Screen Recording pane of System Settings (Privacy & Security).
const SCREEN_SETTINGS: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";
const SCREEN_ITEM: &str = "Open Screen Recording settings…";
const KEYCHAIN_ITEM: &str = "Try the Keychain again";

/// The menu bar icon: "T3" in black on clear, 44x44 pixels with the glyph 36 pixels tall,
/// which tray-icon shows at 22 points (its cap), so the glyph stands 18 points tall at 2x.
/// A template image: macOS draws it from the alpha alone, in the menu bar's own colour.
pub fn icon() -> Icon {
    Icon::from_rgba(include_bytes!("../../assets/menubar-44.rgba").to_vec(), 44, 44).expect("menu bar icon")
}

// ---- The run loop ----

/// Runs the menu bar app on the main thread until Quit is chosen, then exits. The tray is
/// built in the timer's first fire, once NSApplication runs (tray-icon needs it up), and
/// `tick` runs every half second after that. The timer fires in the default run loop mode
/// only, so while a menu, an alert or the folder picker is open (each runs the loop in its
/// own mode) it waits, and `tick` never runs inside itself.
pub fn run(app: Arc<App>) -> ! {
    let mtm = MainThreadMarker::new().expect("the menu bar runs on the main thread");
    let ns_app = NSApplication::sharedApplication(mtm);
    ns_app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    edit_menu();
    let pending = RefCell::new(Some(app));
    let tray: Rc<RefCell<Option<Tray>>> = Rc::new(RefCell::new(None));
    let block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
        let Ok(mut tray) = tray.try_borrow_mut() else { return };
        if tray.is_none() {
            if let Some(app) = pending.borrow_mut().take() {
                *tray = Some(Tray::new(app));
            }
        }
        if let Some(tray) = tray.as_mut() {
            if tray.tick() == Tick::Quit {
                std::process::exit(0);
            }
        }
    });
    // SAFETY: the block isn't Send, but a scheduled timer fires on the run loop of the
    // thread that scheduled it: this one, the main thread, and nowhere else.
    let _timer = unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.5, true, &block) };
    ns_app.run();
    std::process::exit(0)
}

/// The app's main menu, never shown (an accessory app has no menu bar of its own), but where
/// macOS looks up Cmd+C, Cmd+V, Cmd+X, Cmd+A and Cmd+Z: without it, nothing could be pasted
/// into the address box.
fn edit_menu() {
    let edit = Submenu::new("Edit", true);
    let _ = edit.append_items(&[
        &PredefinedMenuItem::undo(None),
        &PredefinedMenuItem::redo(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::cut(None),
        &PredefinedMenuItem::copy(None),
        &PredefinedMenuItem::paste(None),
        &PredefinedMenuItem::select_all(None),
    ]);
    let menu = Menu::new();
    let _ = menu.append(&edit);
    menu.init_for_nsapp();
    // The menu must outlive the app; it lives as long as the process.
    std::mem::forget(menu);
}

/// Makes T3 WoW the active app, so the alert or panel about to open comes up in front
/// (of the game, say) instead of behind the active app.
pub fn bring_to_front() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let ns_app = NSApplication::sharedApplication(mtm);
    // Before the run loop starts (a message at start-up) nothing has set it yet.
    ns_app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    // `activate` from macOS 14; before that only the older call exists.
    if ns_app.respondsToSelector(sel!(activate)) {
        ns_app.activate();
    } else {
        #[allow(deprecated)]
        ns_app.activateIgnoringOtherApps(true);
    }
}

// ---- Notifications ----

/// The app's bundle identifier; None for a bare binary (started from Terminal, or from the
/// LaunchAgent). UNUserNotificationCenter needs one: it throws without a bundle.
fn bundle_id() -> Option<String> {
    NSBundle::mainBundle().bundleIdentifier().map(|id| id.to_string())
}

/// How a notification reaches Notification Center.
#[derive(Debug, PartialEq, Eq)]
enum Route {
    /// osascript's `display notification`: works for a bare binary, needs no permission
    /// (it targets no app, so Automation isn't involved), and shows as Script Editor's.
    Script,
}

/// The route for this process. The app bundle (the next phase) adds the
/// UNUserNotificationCenter route here, for when `bundle_id` is Some.
fn route(_bundle_id: Option<&str>) -> Route {
    Route::Script
}

/// osascript's arguments: the title and text go in as the script's arguments, never into
/// its source, so no quote or backslash in them can change what runs.
fn notification_args(notice: &Notice) -> Vec<String> {
    let title = match notice.level {
        Level::Info => notice.title.clone(),
        Level::Warning | Level::Error => format!("⚠️ {}", notice.title),
    };
    ["-e", "on run argv", "-e", "display notification (item 2 of argv) with title (item 1 of argv)", "-e", "end run"]
        .into_iter()
        .map(str::to_string)
        .chain([title, notice.text.clone()])
        .collect()
}

/// Shows a notification (App::notify has logged it already). Never waits on the main thread:
/// osascript runs, and is waited for, on a thread of its own.
pub fn notify(_tray: &TrayIcon, notice: &Notice) {
    match route(bundle_id().as_deref()) {
        Route::Script => {
            let args = notification_args(notice);
            let _ = std::thread::Builder::new().name("notification".into()).spawn(move || {
                match std::process::Command::new("/usr/bin/osascript").args(&args).stdin(std::process::Stdio::null()).output() {
                    Ok(out) if out.status.success() => {}
                    Ok(out) => log::line(&format!("Couldn't show the notification: osascript said {}", String::from_utf8_lossy(&out.stderr).trim())),
                    Err(error) => log::line(&format!("Couldn't show the notification: {error}")),
                }
            });
        }
    }
}

// ---- Alerts ----

fn alert(mtm: MainThreadMarker, title: &str, text: &str, style: NSAlertStyle, buttons: &[&str]) -> Retained<NSAlert> {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(text));
    alert.setAlertStyle(style);
    for button in buttons {
        alert.addButtonWithTitle(&NSString::from_str(button));
    }
    alert
}

/// A message with an OK button, for `crate::message_box` while the menu bar runs. Off the
/// main thread there is no alert (AppKit allows none there): the caller has printed it.
pub fn message(title: &str, text: &str) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    bring_to_front();
    alert(mtm, title, text, NSAlertStyle::Informational, &["OK"]).runModal();
}

/// A Yes/No question; true for Yes.
pub fn confirm(title: &str, text: &str) -> bool {
    let Some(mtm) = MainThreadMarker::new() else { return false };
    bring_to_front();
    alert(mtm, title, text, NSAlertStyle::Informational, &["Yes", "No"]).runModal() == NSAlertFirstButtonReturn
}

/// The address box's text, under its title: what to type, the note on plain HTTP the
/// Windows box shows, and, on a retry, what was wrong with the last address first.
fn origin_box_text(problem: Option<&str>) -> String {
    let body = "host:port, like 192.168.1.20:3773 or gaming-pc.lan:3773, or a full http:// or https:// URL. T3 on that machine must accept connections from other machines.\n\n\
        Plain http:// sends the sign-in token across your network unencrypted: fine on a home network you trust; anywhere else, use https://. T3 WoW switches once the sign-in succeeds and stays on its current T3 until then.";
    match problem {
        Some(problem) => format!("⚠️ {problem}\n\n{body}"),
        None => body.to_string(),
    }
}

/// Asks for the address of a T3 on another machine, starting with `initial`. The origin it
/// stands for, checked (`config::parse_origin_input`): a bad address brings the box back
/// with what's wrong and what was typed. None when cancelled.
pub fn ask_origin(initial: &str) -> Option<String> {
    let mtm = MainThreadMarker::new()?;
    let mut typed = initial.to_string();
    let mut problem: Option<String> = None;
    loop {
        let alert = alert(mtm, "Connect to T3 on another machine", &origin_box_text(problem.as_deref()), NSAlertStyle::Informational, &["Connect", "Cancel"]);
        let field = NSTextField::textFieldWithString(&NSString::from_str(&typed), mtm);
        field.setPlaceholderString(Some(&NSString::from_str("192.168.1.20:3773")));
        field.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(300.0, 24.0)));
        alert.setAccessoryView(Some(&field));
        alert.layout();
        alert.window().setInitialFirstResponder(Some(&field));
        bring_to_front();
        if alert.runModal() != NSAlertFirstButtonReturn {
            return None;
        }
        typed = field.stringValue().to_string();
        match parse_origin_input(&typed) {
            Ok(origin) => return Some(origin),
            Err(wrong) => problem = Some(wrong),
        }
    }
}

// ---- Opening things ----

pub fn open_path(path: &Path) {
    let _ = std::process::Command::new("/usr/bin/open").arg(path).spawn();
}

/// Opens a web page or a System Settings pane.
pub fn open_url(url: &str) {
    let _ = std::process::Command::new("/usr/bin/open").arg(url).spawn();
}

// ---- Start at login ----

/// The LaunchAgent's label, and its file's name.
const AGENT_LABEL: &str = "com.t3wow.companion";

fn agent_path(home: &Path) -> PathBuf {
    home.join("Library").join("LaunchAgents").join(format!("{AGENT_LABEL}.plist"))
}

fn home() -> io::Result<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME isn't set"))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// The LaunchAgent that starts `exe` when the user logs in (Aqua: a login to the desktop,
/// not over SSH). AbandonProcessGroup, because the companion restarts itself (after an
/// update of the WoW folder or the interface version) by starting a new copy and exiting,
/// and launchd would otherwise end the new copy along with the old.
fn agent_plist(exe: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{AGENT_LABEL}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{}</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>LimitLoadToSessionType</key>
	<string>Aqua</string>
	<key>ProcessType</key>
	<string>Interactive</string>
	<key>AbandonProcessGroup</key>
	<true/>
</dict>
</plist>
"#,
        escape(&exe.to_string_lossy())
    )
}

/// The program a LaunchAgent's text starts: the first string of its ProgramArguments.
fn agent_program(plist: &str) -> Option<String> {
    let (_, after) = plist.split_once("<key>ProgramArguments</key>")?;
    let after = after.trim_start().strip_prefix("<array>")?;
    let (_, rest) = after.split_once("<string>")?;
    let (program, _) = rest.split_once("</string>")?;
    Some(unescape(program))
}

fn agent_enabled_at(path: &Path, exe: &Path) -> bool {
    std::fs::read_to_string(path).ok().and_then(|text| agent_program(&text)).is_some_and(|program| Path::new(&program) == exe)
}

/// Writes (on) or removes (off) the LaunchAgent. Turning it on doesn't load it into launchd:
/// that would start a second companion now; macOS loads it at the next login.
fn set_agent_at(path: &Path, exe: &Path, on: bool) -> io::Result<()> {
    if on {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, agent_plist(exe))
    } else {
        match std::fs::remove_file(path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}

/// This program, as an absolute path with no links in it: what launchd needs, and what a
/// LaunchAgent written by an earlier run is compared with.
fn this_exe() -> io::Result<PathBuf> {
    std::env::current_exe()?.canonicalize()
}

/// The LaunchAgent is there and starts this program.
pub fn autostart_enabled() -> bool {
    match (home(), this_exe()) {
        (Ok(home), Ok(exe)) => agent_enabled_at(&agent_path(&home), &exe),
        _ => false,
    }
}

pub fn set_autostart(on: bool) -> io::Result<()> {
    set_agent_at(&agent_path(&home()?), &this_exe()?, on)
}

// ---- Permission help ----

/// Which permission items the menu shows: Screen Recording settings while macOS refuses the
/// capture, and another try at the Keychain after it refused (not while its prompt may still
/// be up, nor while a retry runs). An item that doesn't apply is left out, not greyed.
fn help_shown(screen_refused: bool, keychain: Option<Problem>, retrying: bool) -> (bool, bool) {
    let keychain_refused = matches!(keychain, Some(Problem::Denied | Problem::Locked | Problem::Malformed | Problem::Failed(_)));
    (screen_refused, keychain_refused && !retrying)
}

/// What a retry of the Keychain says when it is done.
fn keychain_retry_notice(problem: Option<Problem>) -> Notice {
    match problem {
        None => Notice::new(Level::Info, "Keychain", "T3 WoW can read its saved sign-ins again."),
        Some(problem) => Notice::new(Level::Warning, "The Keychain still says no", format!("Saved T3 sign-ins can't be read: {problem}")),
    }
}

/// The permission items, below the WoW line while they apply.
pub struct Help {
    screen: MenuItem,
    keychain: MenuItem,
    retrying: Arc<AtomicBool>,
    shown: (bool, bool),
}

impl Help {
    pub fn new() -> Help {
        Help { screen: MenuItem::new(SCREEN_ITEM, true, None), keychain: MenuItem::new(KEYCHAIN_ITEM, true, None), retrying: Arc::new(AtomicBool::new(false)), shown: (false, false) }
    }

    /// Shows the items that apply now, right after `after` (the WoW line).
    pub fn refresh(&mut self, menu: &Menu, after: &MenuItem) {
        let now = help_shown(crate::capture::screen_refused(), crate::keychain::blocked(), self.retrying.load(Ordering::Relaxed));
        if now == self.shown {
            return;
        }
        let _ = menu.remove(&self.screen);
        let _ = menu.remove(&self.keychain);
        let Some(at) = menu.items().iter().position(|item| item.id() == after.id()).map(|i| i + 1) else { return };
        // Inserted last first, each at the same place.
        if now.1 {
            let _ = menu.insert(&self.keychain, at);
        }
        if now.0 {
            let _ = menu.insert(&self.screen, at);
        }
        self.shown = now;
    }

    /// Does what the item `id` stands for; false when it isn't one of these.
    pub fn choose(&self, id: &MenuId, app: &Arc<App>) -> bool {
        if id == self.screen.id() {
            open_url(SCREEN_SETTINGS);
            true
        } else if id == self.keychain.id() {
            // The Keychain's prompt blocks until it is answered: not on the main thread.
            if !self.retrying.swap(true, Ordering::Relaxed) {
                log::line("Asking the macOS Keychain again, from the menu.");
                let (app, retrying) = (app.clone(), self.retrying.clone());
                let _ = std::thread::Builder::new().name("keychain".into()).spawn(move || {
                    crate::keychain::retry();
                    retrying.store(false, Ordering::Relaxed);
                    app.notify(keychain_retry_notice(crate::keychain::blocked()));
                });
            }
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::server_label;

    #[test]
    fn the_texts_name_the_mac() {
        assert_eq!(format!("T3 server: {}", server_label(None)), "T3 server: this Mac");
        assert_eq!(THIS_MACHINE, "This Mac");
        assert_eq!(format!("{AUTOSTART}: on"), "Start at login: on");
        assert_eq!(format!("Couldn't change {AUTOSTART}"), "Couldn't change Start at login");
        assert!(AUTOSTART_ON.contains("log in to this Mac") && AUTOSTART_OFF.contains("log in to this Mac"));
        assert_eq!(
            format!("{} {NOT_A_CLIENT}", Path::new("/Applications/Games").display()),
            "/Applications/Games doesn't look like a WoW client folder (no World of Warcraft.app or Interface folder)."
        );
        assert!(!format!("{AUTOSTART_ON}{AUTOSTART_OFF}{NOT_A_CLIENT}").contains("Windows"));
    }

    #[test]
    fn the_icon_is_a_template_glyph_inside_the_menu_bar_height() {
        let rgba = include_bytes!("../../assets/menubar-44.rgba");
        assert_eq!(rgba.len(), 44 * 44 * 4);
        let rows: Vec<usize> = (0..44).filter(|y| (0..44).any(|x| rgba[(y * 44 + x) * 4 + 3] != 0)).collect();
        // 36 pixels of glyph: 18 points at 2x, with room above and below in the 22-point bar.
        assert_eq!(rows.len(), 36, "{rows:?}");
        assert!(rows[0] >= 4 && *rows.last().unwrap() <= 40, "{rows:?}");
        // Black with alpha only: a template image takes its shape from the alpha alone.
        assert!(rgba.chunks(4).all(|px| px[..3] == [0, 0, 0] && (px[3] == 0 || px[3] == 255)));
        let _ = icon();
    }

    #[test]
    fn notifications_carry_their_texts_as_arguments_not_as_script() {
        let notice = Notice::new(Level::Info, "Signed in to T3", r#"He said "hi" & left\ "#);
        let args = notification_args(&notice);
        assert_eq!(&args[..6], ["-e", "on run argv", "-e", "display notification (item 2 of argv) with title (item 1 of argv)", "-e", "end run"]);
        assert_eq!(&args[6..], ["Signed in to T3", r#"He said "hi" & left\ "#]);
        let warning = notification_args(&Notice::new(Level::Error, "Update failed", "x"));
        assert_eq!(warning[6], "⚠️ Update failed");
        assert_eq!(route(None), Route::Script, "a bare binary has no bundle for UNUserNotificationCenter");
    }

    #[test]
    fn the_address_box_explains_plain_http_and_says_what_was_wrong() {
        let first = origin_box_text(None);
        assert!(first.starts_with("host:port, like 192.168.1.20:3773"), "{first}");
        assert!(first.contains("Plain http:// sends the sign-in token across your network unencrypted"), "{first}");
        let again = origin_box_text(Some("Add the port T3 listens on, like box:3773."));
        assert!(again.starts_with("⚠️ Add the port T3 listens on, like box:3773.\n\n"), "{again}");
        assert!(again.ends_with(&first));
    }

    fn temp_home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("t3wow-agent-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_launch_agent_starts_this_program_at_login_and_only_that() {
        let exe = Path::new("/Users/me/Apps/T3 & WoW <beta>/t3wow");
        let plist = agent_plist(exe);
        assert!(plist.contains("<key>Label</key>\n\t<string>com.t3wow.companion</string>"), "{plist}");
        assert!(plist.contains("<string>/Users/me/Apps/T3 &amp; WoW &lt;beta&gt;/t3wow</string>"), "{plist}");
        assert!(plist.contains("<key>RunAtLoad</key>\n\t<true/>"), "{plist}");
        assert!(plist.contains("<key>LimitLoadToSessionType</key>\n\t<string>Aqua</string>"), "{plist}");
        assert!(plist.contains("<key>AbandonProcessGroup</key>\n\t<true/>"), "{plist}");
        assert!(!plist.contains("KeepAlive"), "Quit means quit");
        assert_eq!(agent_program(&plist).as_deref(), Some("/Users/me/Apps/T3 & WoW <beta>/t3wow"));
        assert_eq!(agent_program("<plist><dict></dict></plist>"), None);
        assert_eq!(agent_path(Path::new("/Users/me")), Path::new("/Users/me/Library/LaunchAgents/com.t3wow.companion.plist"));
    }

    #[test]
    fn start_at_login_is_on_only_while_the_agent_points_here() {
        let home = temp_home("toggle");
        let path = agent_path(&home);
        let exe = Path::new("/opt/t3wow/t3wow");
        assert!(!agent_enabled_at(&path, exe), "no file yet");
        set_agent_at(&path, exe, true).unwrap();
        assert!(agent_enabled_at(&path, exe));
        assert!(!agent_enabled_at(&path, Path::new("/elsewhere/t3wow")), "another copy's agent isn't this one's");
        std::fs::write(&path, "not a plist").unwrap();
        assert!(!agent_enabled_at(&path, exe), "a file that names no program");
        set_agent_at(&path, exe, true).unwrap();
        set_agent_at(&path, exe, false).unwrap();
        assert!(!path.exists() && !agent_enabled_at(&path, exe));
        set_agent_at(&path, exe, false).expect("off twice is still off");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn permission_items_show_only_while_they_apply() {
        assert_eq!(help_shown(false, None, false), (false, false));
        assert_eq!(help_shown(true, None, false), (true, false));
        assert_eq!(help_shown(false, Some(Problem::NotRead), false), (false, false), "the prompt may still be up");
        for problem in [Problem::Denied, Problem::Locked, Problem::Malformed, Problem::Failed(-1)] {
            assert_eq!(help_shown(false, Some(problem), false), (false, true), "{problem:?}");
            assert_eq!(help_shown(true, Some(problem), true), (true, false), "hidden while a retry runs");
        }
    }

    #[test]
    fn a_keychain_retry_says_how_it_went() {
        assert_eq!(keychain_retry_notice(None).level, Level::Info);
        let refused = keychain_retry_notice(Some(Problem::Denied));
        assert_eq!(refused.level, Level::Warning);
        assert!(refused.text.contains("denied"), "{}", refused.text);
    }
}
