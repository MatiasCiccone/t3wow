//! Proof that a paste came from you. Any addon can draw commands for the companion, so a
//! "paste" from the game is only taken right after you pressed Ctrl+V in WoW, which no
//! addon can fake: the game gives addons no way to press keys.
//!
//! While you're typing in T3 WoW's message box and WoW is the window in front, a thread
//! looks at two keys (Ctrl and V) about 60 times a second and counts each Ctrl+V. It
//! doesn't look at any other key, and doesn't look at all otherwise.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};

/// The WoW client's process id (0: none found), from the screen capture.
static WOW_PID: AtomicU32 = AtomicU32::new(0);
/// You're typing in T3 WoW's message box.
static ARMED: AtomicBool = AtomicBool::new(false);
/// Ctrl+V presses seen in WoW, and when the last one was (epoch ms).
static PRESSES: AtomicU64 = AtomicU64::new(0);
static LAST_AT: AtomicI64 = AtomicI64::new(0);

/// The WoW client the presses must be in (0: none runs).
pub fn watch_process(pid: u32) {
    WOW_PID.store(pid, Ordering::Relaxed);
}

/// Look at keys only while you're typing in T3 WoW's message box.
pub fn arm(typing: bool) {
    ARMED.store(typing, Ordering::Relaxed);
}

/// How many Ctrl+V presses were seen in WoW, and when the last one was (epoch ms).
pub fn presses() -> (u64, i64) {
    (PRESSES.load(Ordering::SeqCst), LAST_AT.load(Ordering::SeqCst))
}

/// Tells a Ctrl+V pressed in WoW from one already held when WoW came to the front (or
/// when the watch began): only a press that starts while WoW is in front counts, so
/// holding the keys elsewhere and switching to WoW isn't one.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Default)]
struct Chord {
    /// Seen released while WoW was in front, since the last press.
    ready: bool,
}

#[cfg_attr(not(windows), allow(dead_code))]
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

/// Whether Ctrl+V is held. The keys aren't read at all unless WoW is in front.
#[cfg_attr(not(windows), allow(dead_code))]
fn ctrl_v_held(in_front: bool, v: impl Fn() -> bool, ctrl: impl Fn() -> bool) -> bool {
    in_front && v() && ctrl()
}

/// Starts the thread that counts Ctrl+V presses (once, however often it is called).
#[cfg(windows)]
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
                // The high bit: held down now.
                let held = |key: u16| unsafe { GetAsyncKeyState(key as i32) } as u16 & 0x8000 != 0;
                if chord.look(ctrl_v_held(in_front, || held(VK_V), || held(VK_CONTROL)), in_front) {
                    LAST_AT.store(crate::config::now_ms(), Ordering::SeqCst);
                    PRESSES.fetch_add(1, Ordering::SeqCst);
                }
                std::thread::sleep(Duration::from_millis(15));
            }
        })
        .expect("keys thread");
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

    #[test]
    #[cfg(windows)]
    #[ignore = "watches real keys: press Ctrl+V in the window of process T3WOW_KEYS_PID within 20 seconds"]
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
