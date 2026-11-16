//! Proof that a paste came from you. Any addon can draw commands for the companion, so a
//! "paste" from the game is only taken right after you pressed Ctrl+V (Cmd+V on a Mac) in
//! WoW, which no addon can fake: the game gives addons no way to press keys.
//!
//! While T3 WoW's window is open in the game and WoW is the window in front, a thread looks at
//! two keys (Ctrl and V, on a Mac Cmd and V) about 60 times a second and counts each Ctrl+V.
//! It doesn't look at any other key, and doesn't look at all otherwise. (Only while typing in
//! the message box was too late: the game's news of it takes up to a third of a second, so a
//! Ctrl+V right after the click came before the look.)
//!
//! Windows' clipboard history (Win+V) pastes the item you pick by pressing Ctrl+V for you,
//! holding Ctrl about 50 ms and tapping V in less than one look's time: so on Windows a key
//! counts as down when it's held now or was pressed since the last look.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};

/// The WoW client's process id (0: none found), from the screen capture.
static WOW_PID: AtomicU32 = AtomicU32::new(0);
/// T3 WoW's window is open in the game.
static ARMED: AtomicBool = AtomicBool::new(false);
/// Ctrl+V presses seen in WoW, and when the last one was (epoch ms).
static PRESSES: AtomicU64 = AtomicU64::new(0);
static LAST_AT: AtomicI64 = AtomicI64::new(0);

/// The WoW client the presses must be in (0: none runs).
pub fn watch_process(pid: u32) {
    WOW_PID.store(pid, Ordering::Relaxed);
}

/// Look at keys only while T3 WoW's window is open in the game.
pub fn arm(open: bool) {
    ARMED.store(open, Ordering::Relaxed);
}

/// How many Ctrl+V presses were seen in WoW, and when the last one was (epoch ms).
pub fn presses() -> (u64, i64) {
    (PRESSES.load(Ordering::SeqCst), LAST_AT.load(Ordering::SeqCst))
}

/// The V key was seen down in WoW (a Mac): macOS lets the companion see the keys.
#[cfg(target_os = "macos")]
static V_SEEN: AtomicBool = AtomicBool::new(false);

/// Tells a Ctrl+V pressed in WoW from one already held when WoW came to the front (or
/// when the watch began): only a press that starts while WoW is in front counts, so
/// holding the keys elsewhere and switching to WoW isn't one.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
#[derive(Default)]
struct Chord {
    /// Seen released while WoW was in front, since the last press.
    ready: bool,
}

#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
impl Chord {
    /// One look at the keys: whether Ctrl and V are both down, and whether WoW is in
    /// front. True for a new press.
    fn look(&mut self, down: bool, in_front: bool) -> bool {
        if !in_front || !down {
            self.ready = in_front;
            return false;
        }
        std::mem::take(&mut self.ready)
    }
}

/// Whether Ctrl+V is held. The keys aren't read at all unless WoW is in front; then both are
/// read on every look, so neither keeps a press from long ago (`pressed_since`) for the other
/// to meet: a Ctrl+A a minute ago and a v typed now aren't a Ctrl+V.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
fn ctrl_v_held(in_front: bool, v: impl Fn() -> bool, ctrl: impl Fn() -> bool) -> bool {
    if !in_front {
        return false;
    }
    let (v, ctrl) = (v(), ctrl());
    v && ctrl
}

/// A key's state from GetAsyncKeyState: the high bit, held down now; the low bit, pressed
/// since the last look. The low bit catches a press quicker than a look: Win+V's Ctrl+V, or
/// a fast tap. (It's shared with any other program that reads the same key, so it can come
/// up empty; a held press still counts by the high bit.)
#[cfg_attr(not(windows), allow(dead_code))]
fn pressed_since(state: u16) -> bool {
    state & 0x8001 != 0
}

/// Starts the thread that counts Ctrl+V presses (once, however often it is called).
#[cfg(any(windows, target_os = "macos"))]
pub fn spawn() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(start);
}

#[cfg(windows)]
fn start() {
    use std::time::Duration;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_V};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    std::thread::Builder::new()
        .name("keys".into())
        .spawn(move || {
            let mut chord = Chord::default();
            loop {
                let pid = WOW_PID.load(Ordering::Relaxed);
                if !ARMED.load(Ordering::Relaxed) || pid == 0 {
                    chord = Chord::default();
                    // Short, so a Ctrl+V right after clicking in the box isn't missed. Reads
                    // no keys: this only waits to be armed.
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                }
                let in_front = unsafe {
                    let mut owner = 0u32;
                    GetWindowThreadProcessId(GetForegroundWindow(), &mut owner);
                    owner == pid
                };
                let held = |key: u16| pressed_since(unsafe { GetAsyncKeyState(key as i32) } as u16);
                if chord.look(ctrl_v_held(in_front, || held(VK_V), || held(VK_CONTROL)), in_front) {
                    LAST_AT.store(crate::config::now_ms(), Ordering::SeqCst);
                    PRESSES.fetch_add(1, Ordering::SeqCst);
                }
                std::thread::sleep(Duration::from_millis(15));
            }
        })
        .expect("keys thread");
}

/// Cmd+V on a Mac. The keys are read from the keyboard's state, as Windows' GetAsyncKeyState
/// does, and which app is in front from the windows on screen.
#[cfg(target_os = "macos")]
fn start() {
    use std::time::Duration;
    std::thread::Builder::new()
        .name("keys".into())
        .spawn(move || {
            let mut chord = Chord::default();
            loop {
                let pid = WOW_PID.load(Ordering::Relaxed);
                if !ARMED.load(Ordering::Relaxed) || pid == 0 {
                    chord = Chord::default();
                    // Short, so a Cmd+V right after clicking in the box isn't missed. Reads
                    // no keys: this only waits to be armed.
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                }
                let in_front = mac::front_pid() == Some(pid);
                let v = || {
                    let down = mac::v_down();
                    if down {
                        V_SEEN.store(true, Ordering::Relaxed);
                    }
                    down
                };
                if chord.look(ctrl_v_held(in_front, v, mac::command_down), in_front) {
                    LAST_AT.store(crate::config::now_ms(), Ordering::SeqCst);
                    PRESSES.fetch_add(1, Ordering::SeqCst);
                }
                std::thread::sleep(Duration::from_millis(15));
            }
        })
        .expect("keys thread");
}

/// Whether macOS may be hiding the keys from the companion, so it can't see a Cmd+V: it
/// hasn't been given Input Monitoring and hasn't seen the V key down in WoW (typing a v in
/// the box shows the keys come through). The first time it may, macOS is asked to put T3 WoW
/// in Input Monitoring's list (and offer to turn it on): only once you've tried to paste an
/// image, never at start, since the keys may well come through without it.
#[cfg(target_os = "macos")]
pub fn keys_hidden() -> bool {
    if V_SEEN.load(Ordering::Relaxed) || mac::may_listen() {
        return false;
    }
    static ASKED: std::sync::Once = std::sync::Once::new();
    ASKED.call_once(mac::ask_to_listen);
    true
}

#[cfg(not(target_os = "macos"))]
pub fn keys_hidden() -> bool {
    false
}

#[cfg(target_os = "macos")]
mod mac {
    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::{CFString, CFStringRef};

    /// kCGEventSourceStateHIDSystemState: the keyboard itself.
    const HID_STATE: i32 = 1;
    /// kVK_ANSI_V.
    const KEY_V: u16 = 9;
    /// kCGEventFlagMaskCommand: either Command key, or a key set to act as one in System
    /// Settings (Keyboard, Modifier Keys).
    const COMMAND_FLAG: u64 = 1 << 20;
    /// kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements.
    const ON_SCREEN: u32 = 1 | 16;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        static kCGWindowOwnerPID: CFStringRef;
        static kCGWindowLayer: CFStringRef;
        fn CGEventSourceKeyState(state: i32, key: u16) -> bool;
        fn CGEventSourceFlagsState(state: i32) -> u64;
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFArrayRef;
        fn CGPreflightListenEventAccess() -> bool;
        fn CGRequestListenEventAccess() -> bool;
    }

    pub fn v_down() -> bool {
        unsafe { CGEventSourceKeyState(HID_STATE, KEY_V) }
    }

    pub fn command_down() -> bool {
        unsafe { CGEventSourceFlagsState(HID_STATE) & COMMAND_FLAG != 0 }
    }

    /// The process whose window is in front: the top one of the ordinary windows on screen
    /// (layer 0; the menu bar, the Dock and notifications sit above). NSWorkspace's
    /// frontmostApplication would be simpler, but off the main thread it keeps the app that
    /// was in front when it was first asked.
    pub fn front_pid() -> Option<u32> {
        let list = unsafe { CGWindowListCopyWindowInfo(ON_SCREEN, 0) };
        if list.is_null() {
            return None;
        }
        let list: CFArray<CFDictionary<CFString, CFType>> = unsafe { TCFType::wrap_under_create_rule(list) };
        let (pid_key, layer_key) = unsafe { (CFString::wrap_under_get_rule(kCGWindowOwnerPID), CFString::wrap_under_get_rule(kCGWindowLayer)) };
        let number = |row: &CFDictionary<CFString, CFType>, key: &CFString| row.find(key)?.downcast::<CFNumber>()?.to_i64();
        let row = list.iter().find(|row| number(row, &layer_key) == Some(0))?;
        u32::try_from(number(&row, &pid_key)?).ok()
    }

    /// Input Monitoring is on for the companion (or the app it runs in).
    pub fn may_listen() -> bool {
        unsafe { CGPreflightListenEventAccess() }
    }

    /// Puts the companion in Input Monitoring's list, with macOS's offer to open the settings.
    pub fn ask_to_listen() {
        unsafe { CGRequestListenEventAccess() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds looks (keys down, WoW in front) and returns which counted as presses.
    fn presses_in(looks: &[(bool, bool)]) -> Vec<bool> {
        let mut chord = Chord::default();
        looks.iter().map(|&(down, front)| chord.look(down, front)).collect()
    }

    #[test]
    fn the_keys_are_not_read_at_all_with_another_window_in_front() {
        let reads = std::cell::Cell::new(0);
        let read = |down: bool| {
            reads.set(reads.get() + 1);
            down
        };
        assert!(!ctrl_v_held(false, || read(true), || read(true)));
        assert_eq!(reads.get(), 0, "read a key with WoW in the background");
        assert!(ctrl_v_held(true, || read(true), || read(true)));
        assert!(!ctrl_v_held(true, || read(false), || read(true)));
        // In front, both are read on every look, V down or not.
        assert_eq!(reads.get(), 4);
    }

    #[test]
    fn only_a_ctrl_v_that_starts_with_wow_in_front_counts() {
        const UP_IN_WOW: (bool, bool) = (false, true);
        const DOWN_IN_WOW: (bool, bool) = (true, true);
        const DOWN_ELSEWHERE: (bool, bool) = (true, false);
        // Pressed, held, released, pressed again: two presses.
        assert_eq!(presses_in(&[UP_IN_WOW, DOWN_IN_WOW, DOWN_IN_WOW, UP_IN_WOW, DOWN_IN_WOW]), [false, true, false, false, true]);
        // Held when the watch starts: not until it's released.
        assert_eq!(presses_in(&[DOWN_IN_WOW, DOWN_IN_WOW, UP_IN_WOW, DOWN_IN_WOW]), [false, false, false, true]);
        // Held in another window, then WoW comes to the front with it still held: no press.
        assert_eq!(presses_in(&[UP_IN_WOW, DOWN_ELSEWHERE, DOWN_IN_WOW, DOWN_IN_WOW]), [false, false, false, false]);
        // Released in the other window doesn't count as released in WoW either.
        assert_eq!(presses_in(&[(false, false), DOWN_IN_WOW, UP_IN_WOW, DOWN_IN_WOW]), [false, false, false, true]);
    }

    /// What GetAsyncKeyState answered for Ctrl and V in each look at a Win+V paste on this PC
    /// (Windows 11, clipboard history): Ctrl held for a few looks, V only ever seen by the bit
    /// that says it was pressed since the last look. Before the low bit counted, no press.
    #[test]
    fn a_win_v_paste_counts_as_a_ctrl_v() {
        const UP: u16 = 0;
        const HELD: u16 = 0x8000;
        const TAPPED: u16 = 0x0001;
        // (V, Ctrl) per look; WoW in front throughout (the history panel doesn't take it).
        let looks = [(UP, UP), (UP, HELD), (UP, HELD), (UP, HELD), (TAPPED, HELD), (UP, HELD), (UP, UP)];
        let mut chord = Chord::default();
        let counted: Vec<bool> = looks.iter().map(|&(v, ctrl)| chord.look(ctrl_v_held(true, || pressed_since(v), || pressed_since(ctrl)), true)).collect();
        assert_eq!(counted, [false, false, false, false, true, false, false]);
        // Win+V itself (V with the Windows key, no Ctrl) is no press.
        let mut chord = Chord::default();
        let counted: Vec<bool> = [(UP, UP), (TAPPED, UP), (UP, UP)].iter().map(|&(v, ctrl)| chord.look(ctrl_v_held(true, || pressed_since(v), || pressed_since(ctrl)), true)).collect();
        assert_eq!(counted, [false, false, false]);
    }

    /// Once a v typed in WoW came through, the keys aren't hidden, and macOS isn't asked
    /// about Input Monitoring at all.
    #[test]
    #[cfg(target_os = "macos")]
    fn a_v_seen_in_wow_shows_the_keys_come_through() {
        V_SEEN.store(true, Ordering::Relaxed);
        assert!(!keys_hidden());
    }

    #[test]
    #[cfg(any(windows, target_os = "macos"))]
    #[ignore = "watches real keys: press Ctrl+V (Cmd+V on a Mac) in the window of process T3WOW_KEYS_PID within 20 seconds"]
    fn counts_ctrl_v_in_the_watched_process_only() {
        let pid: u32 = std::env::var("T3WOW_KEYS_PID").expect("set T3WOW_KEYS_PID").parse().unwrap();
        let (before, _) = presses();
        // Not armed: nothing is counted.
        watch_process(pid);
        spawn();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        arm(true);
        while presses().0 == before && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let (after, at) = presses();
        assert_eq!(after, before + 1, "no Ctrl+V counted in process {pid}");
        assert!((crate::config::now_ms() - at).abs() < 20_000);
        println!("counted a Ctrl+V in process {pid} at {at}");
    }
}
