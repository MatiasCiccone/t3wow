//! The menu bar on a Mac: the run loop on the main thread (NSApplication, as an accessory
//! app: a menu bar icon, no Dock icon), notifications, Start at login (a LaunchAgent),
//! alerts, the address box, and the texts that name the Mac. Also the item the Mac's
//! Screen Recording permission calls for, which appears only while macOS won't let the
//! companion see the game.

use super::{Tick, Tray};
use crate::app::{App, Level, Notice};
use crate::config::parse_origin_input;
use crate::log;
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSApplication, NSApplicationActivationOptions, NSApplicationActivationPolicy, NSMenu, NSModalResponse,
    NSRunningApplication, NSTextField, NSWorkspace,
};
use objc2_foundation::{NSBundle, NSPoint, NSRect, NSSize, NSString, NSTimer};
use std::cell::{Cell, RefCell};
use std::io;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tray_icon::menu::{ContextMenu, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};
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

/// Asked before an update of the app: macOS ties the Keychain's Always Allow and Screen
/// Recording to the app's exact code, so the updated app must be allowed again. The menu's
/// permission items and the Keychain's own prompt take it from there.
pub const UPDATE_REGRANT: &str = "T3 WoW restarts by itself once it's updated. macOS then treats it as a new app and asks again:\n\n\
    • Keychain: type your Mac login password and choose Always Allow, so T3 WoW can read your saved T3 sign-in.\n\
    • Screen Recording: turn T3 WoW on in System Settings (if it shows as on already, turn it off and on again). The menu offers the settings while it's needed.\n\n\
    If WoW is in fullscreen, leave fullscreen first so you can see these prompts.";

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

/// Makes T3 WoW the active app, taking the keyboard from the app in front (Terminal, the
/// game), so the alert or panel about to open comes up in front and what is typed, Return
/// and Esc go to it. macOS 14's `activate` asks rather than takes: the app in front keeps
/// the keyboard unless it yields, which Terminal and WoW never do. So this uses the older
/// calls that take it, deprecated but still working.
///
/// Keep what it returns until the alert or panel has closed: dropping it gives the keyboard
/// back (see `InFront`). An accessory app has no window once its alert is gone, so without
/// that it would stay the active app and keys would go nowhere until the user clicked.
pub fn bring_to_front() -> InFront {
    let Some(mtm) = MainThreadMarker::new() else { return InFront { previous: None, outermost: false } };
    let outermost = FRONT_DEPTH.with(|depth| {
        depth.set(depth.get() + 1);
        depth.get() == 1
    });
    // Only the outermost one remembers: inside it, the app in front is T3 WoW itself.
    let previous = if outermost { NSWorkspace::sharedWorkspace().frontmostApplication() } else { None };
    let ns_app = NSApplication::sharedApplication(mtm);
    // Before the run loop starts (a message at start-up) nothing has set it yet.
    ns_app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    #[allow(deprecated)]
    {
        ns_app.activateIgnoringOtherApps(true);
        NSRunningApplication::currentApplication().activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps);
    }
    InFront { previous, outermost }
}

thread_local! {
    /// How many `InFront`s are alive on the main thread. Only the outermost gives the
    /// keyboard back, so the address box's retries don't hand it to the game in between.
    static FRONT_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// T3 WoW holds the keyboard while this lives (from `bring_to_front`). Dropped, it hands the
/// keyboard back to the app that was in front before: that app is reactivated, which puts
/// the user back where they were (the game, Terminal). If there is none to go back to (it
/// quit meanwhile, it was T3 WoW itself, or macOS refuses), T3 WoW hides instead, and macOS
/// activates the next app in window order. Hiding alone could pick a different app than the
/// one the user came from, so it is only the fallback.
#[must_use = "dropping it at once hands the keyboard back before the alert opens"]
pub struct InFront {
    previous: Option<Retained<NSRunningApplication>>,
    outermost: bool,
}

/// How to give the keyboard back once the outermost alert has closed.
#[derive(Debug, PartialEq)]
enum GiveBack {
    /// Reactivate the app that was in front.
    Previous,
    /// Hide T3 WoW, so macOS activates the next app.
    Hide,
}

/// `previous` tells about the app that was in front: whether it is T3 WoW itself, and
/// whether it has quit since.
fn give_back(previous: Option<(bool, bool)>) -> GiveBack {
    match previous {
        Some((is_self, terminated)) if !is_self && !terminated => GiveBack::Previous,
        _ => GiveBack::Hide,
    }
}

impl Drop for InFront {
    fn drop(&mut self) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        FRONT_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
        if !self.outermost {
            return;
        }
        let previous = self.previous.take();
        // isEqual: is how Apple says to compare running apps (not pids).
        let facts = previous.as_ref().map(|app| (**app == *NSRunningApplication::currentApplication(), app.isTerminated()));
        let restored = match (give_back(facts), previous) {
            // T3 WoW is the active app, so macOS lets it hand activation on.
            (GiveBack::Previous, Some(app)) => app.activateWithOptions(NSApplicationActivationOptions::empty()),
            _ => false,
        };
        if !restored {
            NSApplication::sharedApplication(mtm).hide(None);
        }
    }
}

/// Runs `alert` once T3 WoW has the keyboard: the alert's window key and in front, and its
/// first responder (the address box's field, or the default button) ready for keys. Once
/// it closes, the keyboard goes back to the app that had it (unless an outer `InFront`
/// still holds it).
fn run_in_front(alert: &NSAlert) -> NSModalResponse {
    let _front = bring_to_front();
    alert.layout();
    let window = alert.window();
    window.makeKeyAndOrderFront(None);
    alert.runModal()
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
    /// UNUserNotificationCenter, as T3 WoW's own: the app has a bundle identifier, which it
    /// needs (it throws without one). macOS asks the user once whether to allow them.
    Center,
}

/// The route for this process: the app's own notifications in the app, osascript for a bare
/// binary.
fn route(bundle_id: Option<&str>) -> Route {
    match bundle_id {
        Some(_) => Route::Center,
        None => Route::Script,
    }
}

/// The notification's title: a warning or an error marked.
fn notification_title(notice: &Notice) -> String {
    match notice.level {
        Level::Info => notice.title.clone(),
        Level::Warning | Level::Error => format!("⚠️ {}", notice.title),
    }
}

/// osascript's arguments: the title and text go in as the script's arguments, never into
/// its source, so no quote or backslash in them can change what runs.
fn notification_args(notice: &Notice) -> Vec<String> {
    ["-e", "on run argv", "-e", "display notification (item 2 of argv) with title (item 1 of argv)", "-e", "end run"]
        .into_iter()
        .map(str::to_string)
        .chain([notification_title(notice), notice.text.clone()])
        .collect()
}

/// Shows a notification (App::notify has logged it already). Never waits on the main thread:
/// osascript runs, and is waited for, on a thread of its own; Notification Center answers on
/// a queue of its own.
pub fn notify(_tray: &TrayIcon, notice: &Notice) {
    match route(bundle_id().as_deref()) {
        Route::Center => center::notify(notice),
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

/// Whether notifications may be posted yet: macOS's question is asked once, at the first
/// notice, and notices that come while it's up wait for the answer. After that each one is
/// posted; one the user turned off is dropped by macOS (the menu's notice line has it).
#[derive(Debug, PartialEq)]
enum Asking<T> {
    NotYet,
    Waiting(Vec<T>),
    Answered,
}

impl<T> Asking<T> {
    /// A notice to show: what to do with it.
    fn notice(&mut self, item: T) -> Step<T> {
        match self {
            Asking::Answered => Step::Post(item),
            Asking::Waiting(queue) => {
                queue.push(item);
                Step::Wait
            }
            Asking::NotYet => {
                *self = Asking::Waiting(vec![item]);
                Step::Ask
            }
        }
    }

    /// macOS answered: the notices that waited, to post.
    fn answered(&mut self) -> Vec<T> {
        match std::mem::replace(self, Asking::Answered) {
            Asking::Waiting(queue) => queue,
            _ => Vec::new(),
        }
    }
}

#[derive(Debug, PartialEq)]
enum Step<T> {
    Post(T),
    Wait,
    Ask,
}

/// The app's notifications, through UNUserNotificationCenter.
mod center {
    use super::{notification_title, Asking, Step};
    use crate::app::Notice;
    use crate::log;
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_user_notifications::{UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest, UNUserNotificationCenter};
    use std::sync::Mutex;

    static ASKING: Mutex<Asking<(String, String)>> = Mutex::new(Asking::NotYet);

    pub fn notify(notice: &Notice) {
        let item = (notification_title(notice), notice.text.clone());
        let step = ASKING.lock().unwrap().notice(item);
        match step {
            Step::Post((title, text)) => post(&title, &text),
            Step::Wait => {}
            Step::Ask => ask(),
        }
    }

    /// Asks macOS (and so the user, the first time ever) to allow alerts and sounds, then
    /// posts what waited for the answer.
    fn ask() {
        let block = RcBlock::new(|granted: Bool, error: *mut NSError| {
            if !granted.as_bool() {
                // SAFETY: the error macOS passes, or null.
                let why = unsafe { error.as_ref() }.map(|e| e.localizedDescription().to_string()).unwrap_or_else(|| "turned off for T3 WoW".into());
                log::line(&format!("Notifications aren't allowed ({why}): notices show in the menu only."));
            }
            for (title, text) in ASKING.lock().unwrap().answered() {
                post(&title, &text);
            }
        });
        UNUserNotificationCenter::currentNotificationCenter().requestAuthorizationWithOptions_completionHandler(UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound, &block);
    }

    fn post(title: &str, text: &str) {
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(text));
        let id = NSString::from_str(&format!("t3wow-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
        let done = RcBlock::new(|error: *mut NSError| {
            // SAFETY: the error macOS passes, or null.
            if let Some(error) = unsafe { error.as_ref() } {
                log::line(&format!("Couldn't show the notification: {}", error.localizedDescription()));
            }
        });
        UNUserNotificationCenter::currentNotificationCenter().addNotificationRequest_withCompletionHandler(&request, Some(&done));
    }
}

// ---- The latest notice, in the menu ----

/// How long the menu keeps the latest notice, unless a newer one replaces it.
const NOTICE_KEPT: Duration = Duration::from_secs(10 * 60);
/// The notice line's length in characters, "…" included: under the WoW line's longest
/// texts, so the menu doesn't grow to fit a notice (the tooltip has all of it).
const NOTICE_LINE_CHARS: usize = 90;

/// The notice line: title and text on one line, a warning or an error marked, cut to fit.
fn notice_line_text(notice: &Notice) -> String {
    let mark = match notice.level {
        Level::Info => "",
        Level::Warning | Level::Error => "⚠️ ",
    };
    let line = format!("{mark}{}: {}", notice.title, notice.text);
    let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= NOTICE_LINE_CHARS {
        return line;
    }
    let cut: String = line.chars().take(NOTICE_LINE_CHARS - 1).collect();
    // At a word's end when there is one near: no half words.
    let cut = match cut.rfind(' ') {
        Some(at) if cut[..at].chars().count() >= NOTICE_LINE_CHARS * 2 / 3 => cut[..at].trim_end_matches([' ', ',', ':', ';']).to_string(),
        _ => cut,
    };
    format!("{cut}…")
}

/// The notice line's tooltip: all of it, the title on its own line.
fn notice_tooltip(notice: &Notice) -> String {
    let mark = if notice.level == Level::Info { "" } else { "⚠️ " };
    format!("{mark}{}\n{}", notice.title, notice.text)
}

/// The notice is still worth showing at `now`.
fn notice_current(shown_at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(shown_at) < NOTICE_KEPT
}

/// The latest notice as a greyed line above the T3 line, for NOTICE_KEPT or until a newer
/// one. Notification Center doesn't show a bare binary's notifications (osascript's go to
/// Script Editor, which macOS may never have allowed), and the app's only once the user has
/// allowed them: this line is where a notice can always be read.
pub struct NoticeLine {
    item: MenuItem,
    shown_at: Option<Instant>,
}

impl NoticeLine {
    pub fn new() -> NoticeLine {
        NoticeLine { item: MenuItem::new("", false, None), shown_at: None }
    }

    /// Shows `notice` right above `before` (the T3 line), in place of the one shown.
    pub fn show(&mut self, menu: &Menu, before: &MenuItem, notice: &Notice) {
        let _ = menu.remove(&self.item);
        self.item.set_text(notice_line_text(notice));
        let Some(at) = menu.items().iter().position(|item| item.id() == before.id()) else { return };
        if menu.insert(&self.item, at).is_err() {
            return;
        }
        self.shown_at = Some(Instant::now());
        // muda has no tooltips: set it on the NSMenuItem it just made, the one at `at` in the
        // NSMenu the menu bar icon shows.
        // SAFETY: `ns_menu` is the menu's own NSMenu, alive as long as `menu`, on the main thread.
        if let Some(ns_menu) = unsafe { menu.ns_menu().cast::<NSMenu>().as_ref() } {
            if let Some(ns_item) = ns_menu.itemAtIndex(at as isize) {
                ns_item.setToolTip(Some(&NSString::from_str(&notice_tooltip(notice))));
            }
        }
    }

    /// Takes the line away once it has been up for NOTICE_KEPT.
    pub fn refresh(&mut self, menu: &Menu) {
        if self.shown_at.is_some_and(|at| !notice_current(at, Instant::now())) {
            let _ = menu.remove(&self.item);
            self.shown_at = None;
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
    run_in_front(&alert(mtm, title, text, NSAlertStyle::Informational, &["OK"]));
}

/// A Yes/No question; true for Yes.
pub fn confirm(title: &str, text: &str) -> bool {
    let Some(mtm) = MainThreadMarker::new() else { return false };
    run_in_front(&alert(mtm, title, text, NSAlertStyle::Informational, &["Yes", "No"])) == NSAlertFirstButtonReturn
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
    // Held across the retries, so the keyboard stays here until the box is done.
    let _front = bring_to_front();
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
        if run_in_front(&alert) != NSAlertFirstButtonReturn {
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

/// The Screen Recording item, below the WoW line while macOS refuses the capture. Left out,
/// not greyed, while it doesn't apply.
pub struct Help {
    screen: MenuItem,
    shown: bool,
}

impl Help {
    pub fn new() -> Help {
        Help { screen: MenuItem::new(SCREEN_ITEM, true, None), shown: false }
    }

    /// Shows the item while it applies, right after `after` (the WoW line).
    pub fn refresh(&mut self, menu: &Menu, after: &MenuItem) {
        let now = crate::capture::screen_refused();
        if now == self.shown {
            return;
        }
        let _ = menu.remove(&self.screen);
        let Some(at) = menu.items().iter().position(|item| item.id() == after.id()).map(|i| i + 1) else { return };
        if now {
            let _ = menu.insert(&self.screen, at);
        }
        self.shown = now;
    }

    /// Does what the item `id` stands for; false when it isn't this one.
    pub fn choose(&self, id: &MenuId) -> bool {
        if id == self.screen.id() {
            open_url(SCREEN_SETTINGS);
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
        assert_eq!(route(Some("com.t3wow.companion")), Route::Center);
        // Tests run as a bare binary.
        assert_eq!(bundle_id(), None);
    }

    #[test]
    fn macos_is_asked_once_and_notices_wait_for_its_answer() {
        let mut asking = Asking::NotYet;
        assert_eq!(asking.notice(1), Step::Ask);
        assert_eq!(asking.notice(2), Step::Wait);
        assert_eq!(asking.notice(3), Step::Wait);
        assert_eq!(asking.answered(), [1, 2, 3]);
        assert_eq!(asking, Asking::Answered);
        assert_eq!(asking.notice(4), Step::Post(4), "asked once: later notices go straight out");
        assert_eq!(asking.answered(), Vec::<i32>::new());
    }

    #[test]
    fn an_update_says_what_macos_will_ask_again() {
        for words in ["Keychain", "Always Allow", "Screen Recording", "fullscreen", "restarts by itself"] {
            assert!(UPDATE_REGRANT.contains(words), "{words}");
        }
        assert!(!UPDATE_REGRANT.contains("  "), "{UPDATE_REGRANT}");
    }

    #[test]
    fn the_notice_line_puts_title_and_text_on_one_line_and_marks_trouble() {
        let info = Notice::new(Level::Info, "Addon files", "nothing missing");
        assert_eq!(notice_line_text(&info), "Addon files: nothing missing");
        assert_eq!(notice_tooltip(&info), "Addon files\nnothing missing");
        let warning = Notice::new(Level::Warning, "Restart WoW to load the new addon", "Quit WoW completely\nand start it again.");
        assert_eq!(notice_line_text(&warning), "⚠️ Restart WoW to load the new addon: Quit WoW completely and start it again.");
        assert_eq!(notice_tooltip(&warning), "⚠️ Restart WoW to load the new addon\nQuit WoW completely\nand start it again.");
        assert!(notice_line_text(&Notice::new(Level::Error, "Update failed", "x")).starts_with("⚠️ Update failed: "));
    }

    #[test]
    fn a_long_notice_is_cut_at_a_word_and_the_tooltip_keeps_all_of_it() {
        let text = "Every T3 WoW file is in /Applications/World of Warcraft/_forever_/Interface/AddOns, and the slot addons are all there.";
        let notice = Notice::new(Level::Info, "Addon files: nothing missing", text);
        let line = notice_line_text(&notice);
        assert!(line.chars().count() <= NOTICE_LINE_CHARS, "{line}");
        assert!(line.ends_with('…') && !line.ends_with(" …"), "{line}");
        let shown = line.trim_end_matches('…');
        assert!(format!("Addon files: nothing missing: {text}").starts_with(shown), "{line}");
        assert!(notice_tooltip(&notice).ends_with(text));
        // One long word: cut where it must be.
        let word = Notice::new(Level::Info, "Path", "x".repeat(200));
        let line = notice_line_text(&word);
        assert_eq!(line.chars().count(), NOTICE_LINE_CHARS);
        assert!(line.starts_with("Path: xxx") && line.ends_with("x…"), "{line}");
        // Multi-byte characters near the cut don't split.
        let accents = Notice::new(Level::Warning, "Trouble", "é".repeat(200));
        assert_eq!(notice_line_text(&accents).chars().count(), NOTICE_LINE_CHARS);
    }

    #[test]
    fn the_keyboard_goes_back_to_the_app_that_had_it_or_t3_wow_hides() {
        assert_eq!(give_back(Some((false, false))), GiveBack::Previous);
        // It quit while the alert was up.
        assert_eq!(give_back(Some((false, true))), GiveBack::Hide);
        // T3 WoW was already the active app: going "back" to it would keep the keys stuck.
        assert_eq!(give_back(Some((true, false))), GiveBack::Hide);
        assert_eq!(give_back(None), GiveBack::Hide);
    }

    #[test]
    fn the_notice_line_goes_after_ten_minutes() {
        let shown = Instant::now();
        assert!(notice_current(shown, shown));
        assert!(notice_current(shown, shown + Duration::from_secs(9 * 60 + 59)));
        assert!(!notice_current(shown, shown + Duration::from_secs(10 * 60)));
        assert!(!notice_current(shown, shown + Duration::from_secs(3600)));
        assert!(notice_current(shown + Duration::from_secs(5), shown), "a clock that seems to go back keeps it");
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
}
