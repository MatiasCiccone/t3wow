//! Reads the addon's strip of colored cells from the top-left of the WoW window, four
//! times a second. It never sends input to the game and never reads its memory: it finds
//! the game's window and copies a small rectangle of the screen, like a screenshot.

use crate::bridge::CaptureMessage;
use crate::codec::{cells_needed, magic_cells, CELLS_PER_ROW, CELL_PX, MAX_ROWS};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

const SEARCH: usize = 12;

#[cfg(target_os = "macos")]
mod stream;

/// What the capture knows about the client between frames.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Watch {
    /// The window it watches is still the live client's.
    Valid,
    /// It watched a window that is gone (the client quit, or made a new window).
    Lost,
    /// No client found.
    Nothing,
}

/// Whether to run the full search (a process snapshot and a walk of every window), which
/// is too heavy for four times a second: at once when the watched window is lost, every
/// 2 seconds while no client runs, and every 10 seconds anyway while one is watched.
fn search_due(watch: Watch, since_search: Duration) -> bool {
    match watch {
        Watch::Valid => since_search >= Duration::from_secs(10),
        Watch::Lost => true,
        Watch::Nothing => since_search >= Duration::from_secs(2),
    }
}

/// Cell value (0..=7) at pixel (x, y) of a BGRA image, or None outside it.
fn cell(px: &[u8], stride: usize, w: usize, h: usize, x: usize, y: usize) -> Option<u8> {
    if x >= w || y >= h {
        return None;
    }
    let i = y * stride + x * 4;
    let bit = |v: u8| (v > 127) as u8;
    Some(bit(px[i + 2]) * 4 + bit(px[i + 1]) * 2 + bit(px[i]))
}

/// Finds the strip (its first cells are one of `magics`, within SEARCH pixels of the corner) and
/// reads just the cells its frame needs, so the scene behind it doesn't change the result.
#[cfg(any(windows, test))]
pub fn sample(px: &[u8], stride: usize, w: usize, h: usize, magics: &[Vec<u8>]) -> Option<Vec<u8>> {
    let half = CELL_PX / 2;
    for dy in 0..=SEARCH {
        for dx in 0..=SEARCH {
            let found = magics.iter().any(|magic| magic.iter().enumerate().all(|(i, &m)| cell(px, stride, w, h, dx + i * CELL_PX + half, dy + half) == Some(m)));
            if !found {
                continue;
            }
            let mut need = 19;
            let mut cells = Vec::with_capacity(64);
            let mut n = 0;
            while n < need {
                let (row, col) = (n / CELLS_PER_ROW, n % CELLS_PER_ROW);
                cells.push(cell(px, stride, w, h, dx + col * CELL_PX + half, dy + row * CELL_PX + half)?);
                if n == 18 {
                    need = cells_needed(&cells)?;
                    if need > CELLS_PER_ROW * MAX_ROWS {
                        return None;
                    }
                }
                n += 1;
            }
            return Some(cells);
        }
    }
    None
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::ptr::null_mut;
    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, POINT, RECT, WAIT_TIMEOUT};
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
    use windows_sys::Win32::System::Threading::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    pub struct WowProcess {
        /// "pid@creation-filetime": stays the same for one client process.
        pub key: String,
        pub hwnd: HWND,
        pid: u32,
        /// Kept open so the pid can't be reused while it's watched, and to see it exit.
        process: HANDLE,
    }

    impl WowProcess {
        pub fn pid(&self) -> u32 {
            self.pid
        }

        /// Still the live client and its window: a few cheap calls, no snapshot. Needs a
        /// real client, so it isn't unit-tested.
        pub fn still_valid(&self) -> bool {
            unsafe {
                if WaitForSingleObject(self.process, 0) != WAIT_TIMEOUT || IsWindow(self.hwnd) == 0 || IsWindowVisible(self.hwnd) == 0 {
                    return false;
                }
                let mut pid = 0u32;
                GetWindowThreadProcessId(self.hwnd, &mut pid);
                pid == self.pid
            }
        }
    }

    impl Drop for WowProcess {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.process) };
        }
    }

    fn wide_to_string(wide: &[u16]) -> String {
        let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
        String::from_utf16_lossy(&wide[..end])
    }

    fn process_info(pid: u32) -> Option<(PathBuf, u64)> {
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if handle.is_null() {
                return None;
            }
            let mut buf = [0u16; 1024];
            let mut size = buf.len() as u32;
            let path = (QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut size) != 0)
                .then(|| PathBuf::from(String::from_utf16_lossy(&buf[..size as usize])));
            let (mut created, mut exited, mut kernel, mut user): (FILETIME, FILETIME, FILETIME, FILETIME) = (zeroed(), zeroed(), zeroed(), zeroed());
            let started = (GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) != 0)
                .then_some(((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64);
            CloseHandle(handle);
            Some((path?, started.unwrap_or(0)))
        }
    }

    struct Search {
        pid: u32,
        found: HWND,
    }

    unsafe extern "system" fn each_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = &mut *(lparam as *mut Search);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == search.pid && IsWindowVisible(hwnd) != 0 && GetWindow(hwnd, GW_OWNER).is_null() {
            search.found = hwnd;
            return 0;
        }
        1
    }

    fn main_window(pid: u32) -> Option<HWND> {
        let mut search = Search { pid, found: null_mut() };
        unsafe { EnumWindows(Some(each_window), &mut search as *mut Search as LPARAM) };
        (!search.found.is_null()).then_some(search.found)
    }

    /// Running Wow.exe processes, with their image paths.
    pub fn wow_processes() -> Vec<(u32, PathBuf, u64)> {
        let mut out = Vec::new();
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return out;
            }
            let mut entry: PROCESSENTRY32W = zeroed();
            entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
            let mut ok = Process32FirstW(snapshot, &mut entry);
            while ok != 0 {
                if wide_to_string(&entry.szExeFile).eq_ignore_ascii_case("Wow.exe") {
                    if let Some((path, started)) = process_info(entry.th32ProcessID) {
                        out.push((entry.th32ProcessID, path, started));
                    }
                }
                ok = Process32NextW(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
        }
        out
    }

    /// The WoW client running from `client_folder`, if it has a window.
    pub fn find_wow(client_folder: &Path) -> Option<WowProcess> {
        let folder = client_folder.to_string_lossy().to_lowercase();
        wow_processes().into_iter().find_map(|(pid, path, started)| {
            if !path.to_string_lossy().to_lowercase().starts_with(&folder) {
                return None;
            }
            let hwnd = main_window(pid)?;
            let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
            if process.is_null() {
                return None;
            }
            Some(WowProcess { key: format!("{pid}@{started}"), hwnd, pid, process })
        })
    }

    pub enum Sampled {
        NotCapturable,
        NoStrip,
        Cells(Vec<u8>),
    }

    /// Copies the window's top-left corner from the screen and looks for the strip.
    pub fn sample_window(hwnd: HWND, magics: &[Vec<u8>]) -> Sampled {
        unsafe {
            if IsIconic(hwnd) != 0 {
                return Sampled::NotCapturable;
            }
            let mut rect: RECT = zeroed();
            if GetClientRect(hwnd, &mut rect) == 0 {
                return Sampled::NotCapturable;
            }
            let mut origin = POINT { x: 0, y: 0 };
            if ClientToScreen(hwnd, &mut origin) == 0 {
                return Sampled::NotCapturable;
            }
            let w = ((rect.right - rect.left) as usize).min(CELLS_PER_ROW * CELL_PX + SEARCH + 4);
            let h = ((rect.bottom - rect.top) as usize).min(MAX_ROWS * CELL_PX + SEARCH + 4);
            if w < 64 || h < 16 {
                return Sampled::NotCapturable;
            }
            let screen = GetDC(null_mut());
            let memory = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, w as i32, h as i32);
            let previous = SelectObject(memory, bitmap as HGDIOBJ);
            let copied = BitBlt(memory, 0, 0, w as i32, h as i32, screen, origin.x, origin.y, SRCCOPY) != 0;
            let mut info: BITMAPINFO = zeroed();
            info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = w as i32;
            info.bmiHeader.biHeight = -(h as i32); // top-down rows
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut pixels = vec![0u8; w * h * 4];
            let lines = if copied {
                GetDIBits(memory, bitmap, 0, h as u32, pixels.as_mut_ptr() as *mut c_void, &mut info, DIB_RGB_COLORS)
            } else {
                0
            };
            SelectObject(memory, previous);
            DeleteObject(bitmap as HGDIOBJ);
            DeleteDC(memory);
            ReleaseDC(null_mut(), screen);
            if lines == 0 {
                return Sampled::NotCapturable;
            }
            match sample(&pixels, w * 4, w, h, magics) {
                Some(cells) => Sampled::Cells(cells),
                None => Sampled::NoStrip,
            }
        }
    }

    /// Per-monitor (v2) DPI awareness, so the window's position and the screen copy are both
    /// in physical pixels on every monitor, whatever its scaling; system awareness alone can
    /// offset the capture on a scaled second monitor. Call it on the capture thread: when the
    /// process's awareness is already set (the tray's windows, a manifest), the thread can
    /// still be per-monitor aware on its own. Windows before 10 1703 lacks both functions
    /// (looked up at run time, so the exe still starts there) and gets the old system-wide
    /// call. Not unit-testable: it changes process state and only matters on mixed-DPI
    /// monitors.
    pub fn set_dpi_aware() {
        use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
        const PER_MONITOR_AWARE_V2: isize = -4; // DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
        let user32: Vec<u16> = "user32.dll".encode_utf16().chain(Some(0)).collect();
        unsafe {
            let module = GetModuleHandleW(user32.as_ptr());
            if !module.is_null() {
                if let Some(function) = GetProcAddress(module, c"SetProcessDpiAwarenessContext".as_ptr().cast()) {
                    let set_process: unsafe extern "system" fn(isize) -> BOOL = std::mem::transmute(function);
                    if set_process(PER_MONITOR_AWARE_V2) != 0 {
                        return;
                    }
                }
                if let Some(function) = GetProcAddress(module, c"SetThreadDpiAwarenessContext".as_ptr().cast()) {
                    // Returns the previous context, null on failure.
                    let set_thread: unsafe extern "system" fn(isize) -> isize = std::mem::transmute(function);
                    if set_thread(PER_MONITOR_AWARE_V2) != 0 {
                        return;
                    }
                }
            }
            SetProcessDPIAware();
        }
    }
}

#[cfg(windows)]
pub use win::{find_wow, wow_processes};

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use crate::codec::decode_cells;
    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::number::CFNumber;
    use core_foundation::string::{CFString, CFStringRef};
    use std::ffi::{c_void, OsStr};
    use std::fs;
    use std::io::{self, Read as _};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    use std::process::{Command, Stdio};
    use std::time::Instant;

    pub use super::stream::Backend;

    /// How far down from the window's top the strip can be, in points: the title bar when
    /// windowed, the notch band when fullscreen.
    const POINTS_DOWN: usize = 40;
    /// Cell sizes the strip can have on screen: 4 pixels, or up to twice that when the game
    /// renders below the screen's resolution.
    const MAX_CELL: usize = 8;
    /// With the strip gone from where it was, how often to look for it elsewhere. It hides
    /// whenever it has nothing to say, so this is the usual case, not a problem.
    const WIDE_EVERY: Duration = Duration::from_secs(2);
    /// The window's position changes rarely, and the lookup walks every window on the system.
    pub const WINDOW_EVERY: Duration = Duration::from_secs(2);
    const GRAB_TIMEOUT: Duration = Duration::from_millis(1500);
    const SCREENCAPTURE: &str = "/usr/sbin/screencapture";
    /// For when the display can't be asked: every Mac with a notch, and most others, are 2.
    const UNKNOWN_SCALE: u32 = 2;
    const WINDOW_LIST_ALL: u32 = 0;

    /// What to do when macOS refuses the capture: in the app, and for a bare t3wow. The app
    /// has its own switch, named T3 WoW, and the capture picks a new grant up without a
    /// restart. Releases are signed with one certificate, so the switch carries over an update;
    /// one left by a build signed another way (ad hoc, say) may show as on and still not
    /// count until it is removed and the app added again. A bare t3wow is allowed through the
    /// app it runs in, which only sees a grant once restarted.
    const PERMISSION_HELP: (&str, &str) = (
        "T3 WoW can't see the game: macOS hasn't given it Screen Recording permission. \
        Open System Settings > Privacy & Security > Screen & System Audio Recording and turn on T3 WoW. \
        If it shows as on already, select it, remove it with the − button and add T3 WoW again with +.",
        "T3 WoW can't see the game: macOS hasn't given it Screen Recording permission. \
        Open System Settings > Privacy & Security > Screen & System Audio Recording, turn on the app you run t3wow from \
        (Terminal, when you run it there), then quit that app completely, open it again and run t3wow again.",
    );

    /// PERMISSION_HELP's text for the app (`in_app`) or for a bare t3wow.
    pub fn permission_help(in_app: bool) -> &'static str {
        if in_app {
            PERMISSION_HELP.0
        } else {
            PERMISSION_HELP.1
        }
    }
    const NO_STRIP_YET: &str = "WoW is running but T3 WoW's strip isn't in view yet: log in to a character (or /reload), and keep other windows off the game's top-left corner.";

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        static kCGWindowOwnerPID: CFStringRef;
        static kCGWindowNumber: CFStringRef;
        static kCGWindowBounds: CFStringRef;
        static kCGWindowIsOnscreen: CFStringRef;
        fn CGRequestScreenCaptureAccess() -> bool;
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFArrayRef;
        fn CGRectMakeWithDictionaryRepresentation(dictionary: CFDictionaryRef, rect: *mut CGRect) -> bool;
        fn CGGetDisplaysWithPoint(point: CGPoint, max: u32, displays: *mut u32, count: *mut u32) -> i32;
        fn CGDisplayCopyDisplayMode(display: u32) -> *mut c_void;
        fn CGDisplayModeGetWidth(mode: *mut c_void) -> usize;
        fn CGDisplayModeGetPixelWidth(mode: *mut c_void) -> usize;
        fn CGDisplayModeRelease(mode: *mut c_void);
    }

    /// Puts up macOS's Screen Recording prompt when access is missing (and nothing when it is
    /// there), early, so it isn't hidden behind the game. The answer isn't used: an actual
    /// capture is what shows whether access works.
    pub fn request_screen_access() {
        static ASKED: std::sync::Once = std::sync::Once::new();
        ASKED.call_once(|| {
            unsafe { CGRequestScreenCaptureAccess() };
        });
    }

    pub struct WowProcess {
        /// "pid@start-microseconds": stays the same for one client process. The start time
        /// is the kernel's, so a change of time zone doesn't look like a new client.
        pub key: String,
        pid: u32,
        started: u64,
    }

    impl WowProcess {
        pub fn pid(&self) -> u32 {
            self.pid
        }

        pub fn still_valid(&self) -> bool {
            process_started(self.pid) == Some(self.started)
        }
    }

    fn all_pids() -> Vec<i32> {
        unsafe {
            let count = libc::proc_listallpids(std::ptr::null_mut(), 0);
            if count <= 0 {
                return Vec::new();
            }
            // Room for processes that start between the two calls.
            let mut pids = vec![0i32; count as usize * 2];
            let got = libc::proc_listallpids(pids.as_mut_ptr().cast(), (pids.len() * size_of::<i32>()) as i32);
            pids.truncate(got.max(0) as usize);
            pids
        }
    }

    fn exe_path(pid: i32) -> Option<PathBuf> {
        let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let len = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
        (len > 0).then(|| PathBuf::from(OsStr::from_bytes(&buf[..len as usize])))
    }

    /// When the process started, in microseconds since the epoch; None once it has gone.
    fn process_started(pid: u32) -> Option<u64> {
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = size_of::<libc::proc_bsdinfo>() as i32;
        let got = unsafe { libc::proc_pidinfo(pid as i32, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size) };
        (got == size).then(|| info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
    }

    /// Running WoW: Forever clients: pid, executable path, start time.
    pub fn wow_processes() -> Vec<(u32, PathBuf, u64)> {
        all_pids()
            .into_iter()
            .filter_map(|pid| {
                let path = exe_path(pid)?;
                crate::wow::client_folder_of_exe(&path)?;
                Some((pid as u32, path, process_started(pid as u32)?))
            })
            .collect()
    }

    /// The WoW client running from `client_folder`. Unlike on Windows it doesn't need a window:
    /// off in another Space the client has none on screen but is still running.
    pub fn find_wow(client_folder: &Path) -> Option<WowProcess> {
        // The path the kernel reports is the one on disk; a typed folder may differ in case,
        // symlinks or accents.
        let folder = fs::canonicalize(client_folder).unwrap_or_else(|_| client_folder.to_path_buf());
        wow_processes().into_iter().find_map(|(pid, path, started)| {
            let client = crate::wow::client_folder_of_exe(&path)?;
            same_folder(&client, &folder).then(|| WowProcess { key: format!("{pid}@{started}"), pid, started })
        })
    }

    fn same_folder(a: &Path, b: &Path) -> bool {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct Rect {
        pub x: i32,
        pub y: i32,
        pub w: u32,
        pub h: u32,
    }

    /// One row of the system's window list: the three things read from it. Bounds need no
    /// Screen Recording permission, and window names (which would) are never read.
    #[derive(Clone, Copy, Debug)]
    struct WindowRow {
        pid: i64,
        id: u32,
        on_screen: bool,
        rect: (f64, f64, f64, f64),
    }

    /// The window the game draws in: the largest on-screen one the client owns. The rest of
    /// its windows are small helpers. On a tie, the frontmost (the list runs front to back).
    fn choose_window(rows: &[WindowRow], pid: u32) -> Option<(Rect, u32)> {
        let area = |r: &WindowRow| r.rect.2 * r.rect.3;
        let window = rows.iter().filter(|r| r.pid == i64::from(pid) && r.on_screen && r.rect.2 >= 64.0 && r.rect.3 >= 16.0).min_by(|a, b| area(b).total_cmp(&area(a)))?;
        Some((Rect { x: window.rect.0.round() as i32, y: window.rect.1.round() as i32, w: window.rect.2.round() as u32, h: window.rect.3.round() as u32 }, window.id))
    }

    fn window_rows() -> Option<Vec<WindowRow>> {
        let list = unsafe { CGWindowListCopyWindowInfo(WINDOW_LIST_ALL, 0) };
        if list.is_null() {
            return None;
        }
        let list: CFArray<CFDictionary<CFString, CFType>> = unsafe { TCFType::wrap_under_create_rule(list) };
        let (pid_key, id_key, bounds_key, on_screen_key) = unsafe {
            (
                CFString::wrap_under_get_rule(kCGWindowOwnerPID),
                CFString::wrap_under_get_rule(kCGWindowNumber),
                CFString::wrap_under_get_rule(kCGWindowBounds),
                CFString::wrap_under_get_rule(kCGWindowIsOnscreen),
            )
        };
        Some(
            list.iter()
                .filter_map(|row| {
                    let pid = row.find(&pid_key)?.downcast::<CFNumber>()?.to_i64()?;
                    let id = u32::try_from(row.find(&id_key)?.downcast::<CFNumber>()?.to_i64()?).ok()?;
                    let bounds = row.find(&bounds_key)?;
                    let mut rect = CGRect::default();
                    if !unsafe { CGRectMakeWithDictionaryRepresentation(bounds.as_CFTypeRef() as CFDictionaryRef, &mut rect) } {
                        return None;
                    }
                    // An off-screen window leaves this out altogether.
                    let on_screen = row.find(&on_screen_key).and_then(|v| v.downcast::<CFBoolean>()).is_some_and(bool::from);
                    Some(WindowRow { pid, id, on_screen, rect: (rect.origin.x, rect.origin.y, rect.size.width, rect.size.height) })
                })
                .collect(),
        )
    }

    /// The on-screen windows in front of window `id` that overlap `rect`: owner pid and
    /// bounds. For the live test, to tell a covered corner from a misplaced capture.
    #[cfg(test)]
    pub fn windows_in_front(id: u32, rect: Rect) -> Vec<(i64, Rect)> {
        let rows = window_rows().unwrap_or_default();
        let overlaps = |r: &WindowRow| {
            let (x, y, w, h) = r.rect;
            x < f64::from(rect.x) + f64::from(rect.w) && x + w > f64::from(rect.x) && y < f64::from(rect.y) + f64::from(rect.h) && y + h > f64::from(rect.y)
        };
        rows.iter()
            .take_while(|r| r.id != id)
            .filter(|r| r.on_screen && overlaps(r))
            .map(|r| (r.pid, Rect { x: r.rect.0 as i32, y: r.rect.1 as i32, w: r.rect.2 as u32, h: r.rect.3 as u32 }))
            .collect()
    }

    /// Pixels per point on the display showing the window's top-left, where the strip is.
    fn display_scale(rect: Rect) -> u32 {
        let point = CGPoint { x: f64::from(rect.x) + 1.0, y: f64::from(rect.y) + 1.0 };
        let (mut display, mut count) = (0u32, 0u32);
        unsafe {
            if CGGetDisplaysWithPoint(point, 1, &mut display, &mut count) != 0 || count == 0 {
                return UNKNOWN_SCALE;
            }
            let mode = CGDisplayCopyDisplayMode(display);
            if mode.is_null() {
                return UNKNOWN_SCALE;
            }
            let (pixels, points) = (CGDisplayModeGetPixelWidth(mode), CGDisplayModeGetWidth(mode));
            CGDisplayModeRelease(mode);
            if points == 0 {
                return UNKNOWN_SCALE;
            }
            ((pixels as f64 / points as f64).round() as u32).clamp(1, 4)
        }
    }

    /// The client's game window on screen and its display's scale; None when it has none
    /// on screen (minimized, or off in another Space).
    pub fn window_of(pid: u32) -> Option<WindowFrame> {
        let (rect, id) = choose_window(&window_rows()?, pid)?;
        Some(WindowFrame { rect, scale: display_scale(rect), id })
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct WindowFrame {
        pub rect: Rect,
        /// Pixels per point.
        pub scale: u32,
        /// The window server's number for it: a ScreenCaptureKit stream follows this window.
        pub id: u32,
    }

    /// Pixels of a captured rectangle, top row first, as `sample` and `cell` take them.
    #[derive(Clone, Debug, PartialEq)]
    pub struct Image {
        pub bgra: Vec<u8>,
        pub width: usize,
        pub height: usize,
    }

    #[derive(Clone, Debug, PartialEq)]
    pub enum GrabError {
        /// macOS refuses: no Screen Recording permission.
        Permission,
        Failed(String),
    }

    /// What `screencapture`'s end says happened. "could not create image" is how it reports
    /// missing permission; it writes no file then.
    fn grab_failure(succeeded: bool, file_written: bool, stderr: &str) -> Option<GrabError> {
        if stderr.to_lowercase().contains("could not create image") {
            return Some(GrabError::Permission);
        }
        if succeeded && file_written {
            return None;
        }
        let said = stderr.trim();
        Some(GrabError::Failed(if said.is_empty() { "screencapture exited without an image".to_string() } else { format!("screencapture: {said}") }))
    }

    /// A folder only this user can read, for the moment a capture of the screen sits on disk.
    fn private_dir(parent: &Path) -> io::Result<PathBuf> {
        let uid = unsafe { libc::geteuid() };
        let dir = parent.join(format!("t3wow-capture-{uid}"));
        match fs::DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => Ok(dir),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let meta = fs::symlink_metadata(&dir)?;
                if meta.is_dir() && meta.uid() == uid && meta.mode() & 0o077 == 0 {
                    Ok(dir)
                } else {
                    Err(io::Error::other(format!("{} isn't a private folder", dir.display())))
                }
            }
            Err(error) => Err(error),
        }
    }

    /// Where the Reader's pictures come from: `grab` (screencapture), a test's closure, or the
    /// ScreenCaptureKit stream with `grab` behind it (`stream::Capturer`).
    pub trait Grab {
        /// A rectangle of the screen, in points, as pixels.
        fn grab(&mut self, rect: Rect) -> Result<Image, GrabError>;
    }

    impl<F: FnMut(Rect) -> Result<Image, GrabError>> Grab for F {
        fn grab(&mut self, rect: Rect) -> Result<Image, GrabError> {
            self(rect)
        }
    }

    /// Takes a rectangle of the screen, in points, with a screencapture process. The one-shot
    /// way, and the fallback when the ScreenCaptureKit stream can't run: everything else only
    /// gets pixels from it.
    pub fn grab(rect: Rect) -> Result<Image, GrabError> {
        let dir = private_dir(&std::env::temp_dir()).map_err(|e| GrabError::Failed(format!("no private folder for the capture: {e}")))?;
        let file = dir.join("strip.bmp");
        let image = run_screencapture(rect, &file).and_then(|()| {
            let bytes = fs::read(&file).map_err(|e| GrabError::Failed(format!("couldn't read the capture: {e}")))?;
            decode_bmp(&bytes).map_err(GrabError::Failed)
        });
        let _ = fs::remove_file(&file);
        image
    }

    fn run_screencapture(rect: Rect, file: &Path) -> Result<(), GrabError> {
        let mut child = Command::new(SCREENCAPTURE)
            .args(["-x", "-R", &format!("{},{},{},{}", rect.x, rect.y, rect.w, rect.h), "-t", "bmp"])
            .arg(file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| GrabError::Failed(format!("couldn't start screencapture: {e}")))?;
        let deadline = Instant::now() + GRAB_TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(GrabError::Failed(format!("screencapture didn't finish within {} ms", GRAB_TIMEOUT.as_millis())));
                }
            }
        };
        let mut stderr = String::new();
        if let Some(mut pipe) = child.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        grab_failure(status.success(), file.exists(), &stderr).map_or(Ok(()), Err)
    }

    /// A BMP file as pixels, through the clipboard's bitmap reader. That reader expects the
    /// pixels straight after the header and its masks; a file says where they are (`bfOffBits`)
    /// and may leave a gap, so the header is put right against them first.
    fn decode_bmp(file: &[u8]) -> Result<Image, String> {
        const FILE_HEADER: usize = 14;
        let bad = || "the capture isn't a BMP T3 WoW can read".to_string();
        let u32_at = |bytes: &[u8], i: usize| bytes.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
        if file.get(..2) != Some(b"BM") {
            return Err(bad());
        }
        let pixels_at = u32_at(file, 10).ok_or_else(bad)?;
        let dib = file.get(FILE_HEADER..).ok_or_else(bad)?;
        let header = u32_at(dib, 0).ok_or_else(bad)?;
        let compression = u32_at(dib, 16).ok_or_else(bad)?;
        // Masks follow a 40-byte header, and are inside a longer one.
        let before_pixels = header + if compression == 3 && header < 52 { 12 } else { 0 };
        if pixels_at < FILE_HEADER + before_pixels || before_pixels < 40 {
            return Err(bad());
        }
        let mut joined = Vec::with_capacity(before_pixels + file.len().saturating_sub(pixels_at));
        joined.extend_from_slice(dib.get(..before_pixels).ok_or_else(bad)?);
        joined[32..36].fill(0); // no palette
        joined.extend_from_slice(file.get(pixels_at..).ok_or_else(bad)?);
        let bitmap = crate::clip::dib_bitmap(&joined)?;
        let mut bgra = bitmap.rgba;
        for pixel in bgra.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        Ok(Image { bgra, width: bitmap.width as usize, height: bitmap.height as usize })
    }

    /// Where the strip is in a captured image, all in pixels. Worked out once and kept: at
    /// other cell sizes, offsets and scales it takes a wide search to find again.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Geometry {
        pub dx: usize,
        pub dy: usize,
        pub cell: usize,
    }

    /// The strip's cells at exactly `geometry`, if a frame's first cells (one of `magics`) are there.
    fn read_at(image: &Image, magics: &[Vec<u8>], geometry: Geometry) -> Option<Vec<u8>> {
        let (w, h, stride) = (image.width, image.height, image.width * 4);
        let half = geometry.cell / 2;
        let at = |col: usize, row: usize| cell(&image.bgra, stride, w, h, geometry.dx + col * geometry.cell + half, geometry.dy + row * geometry.cell + half);
        if !magics.iter().any(|magic| magic.iter().enumerate().all(|(i, &m)| at(i, 0) == Some(m))) {
            return None;
        }
        let mut need = 19;
        let mut cells = Vec::with_capacity(64);
        let mut n = 0;
        while n < need {
            cells.push(at(n % CELLS_PER_ROW, n / CELLS_PER_ROW)?);
            if n == 18 {
                need = cells_needed(&cells)?;
                if need > CELLS_PER_ROW * MAX_ROWS {
                    return None;
                }
            }
            n += 1;
        }
        Some(cells)
    }

    /// Looks for the strip in a wide area. Only a frame whose checksum holds counts: with this
    /// many positions and cell sizes, something in the scene could pass the first cells alone.
    fn search(image: &Image, magics: &[Vec<u8>], scale: u32) -> Option<(Geometry, Vec<u8>)> {
        let scale = scale as usize;
        for dy in 0..=POINTS_DOWN * scale {
            for dx in 0..=SEARCH * scale {
                for cell in CELL_PX..=MAX_CELL {
                    let geometry = Geometry { dx, dy, cell };
                    if let Some(cells) = read_at(image, magics, geometry).filter(|c| decode_cells(c).is_some()) {
                        return Some((settled(image, magics, geometry, &cells), cells));
                    }
                }
            }
        }
        None
    }

    /// The strip's real corner, from a first hit. A cell is read at its middle, so the frame
    /// reads the same from several offsets and the search stops at the first, up to half a
    /// cell early. The last offset that still reads places it exactly, and reading from the
    /// corner keeps every later read at the middle of each cell, away from its edges.
    fn settled(image: &Image, magics: &[Vec<u8>], hit: Geometry, cells: &[u8]) -> Geometry {
        let reads_the_same = |g: Geometry| read_at(image, magics, g).is_some_and(|c| c == cells);
        let furthest = |step: fn(Geometry) -> Geometry| {
            let mut at = hit;
            for _ in 0..hit.cell {
                let next = step(at);
                if !reads_the_same(next) {
                    break;
                }
                at = next;
            }
            at
        };
        let corner = |last: usize| (last + hit.cell / 2 + 1).saturating_sub(hit.cell);
        let geometry = Geometry {
            dx: corner(furthest(|g| Geometry { dx: g.dx + 1, ..g }).dx),
            dy: corner(furthest(|g| Geometry { dy: g.dy + 1, ..g }).dy),
            cell: hit.cell,
        };
        if reads_the_same(geometry) {
            geometry
        } else {
            hit
        }
    }

    fn clamped(window: WindowFrame, w_px: usize, h_px: usize) -> Rect {
        let scale = window.scale as usize;
        let points = |pixels: usize, window_points: u32| (pixels.div_ceil(scale) as u32).min(window_points);
        Rect { x: window.rect.x, y: window.rect.y, w: points(w_px, window.rect.w), h: points(h_px, window.rect.h) }
    }

    /// The part of the window a ScreenCaptureKit stream covers: all the wide search looks at,
    /// so the stream serves every capture the Reader asks for.
    pub fn stream_region(window: WindowFrame) -> Rect {
        wide_rect(window)
    }

    /// Everything the wide search needs: the largest strip at the largest offset.
    fn wide_rect(window: WindowFrame) -> Rect {
        let scale = window.scale as usize;
        clamped(window, SEARCH * scale + CELLS_PER_ROW * MAX_CELL + MAX_CELL, POINTS_DOWN * scale + MAX_ROWS * MAX_CELL + MAX_CELL)
    }

    /// Just the strip's own area, when its place is known. Starts at the window's corner too,
    /// so a geometry means the same in every capture.
    fn tight_rect(window: WindowFrame, geometry: Geometry) -> Rect {
        clamped(window, geometry.dx + CELLS_PER_ROW * geometry.cell, geometry.dy + MAX_ROWS * geometry.cell)
    }

    #[derive(Debug, PartialEq)]
    pub enum Read {
        NotCapturable,
        NoStrip,
        Cells(Vec<u8>),
        Failed(GrabError),
    }

    /// Takes the captures and keeps track of where the strip is.
    pub struct Reader<G> {
        grab: G,
        magics: Vec<Vec<u8>>,
        window: Option<WindowFrame>,
        geometry: Option<Geometry>,
        /// The last capture, when it had a geometry to go by, didn't show the strip there.
        missed: bool,
        last_wide: Option<Instant>,
    }

    impl<G: Grab> Reader<G> {
        pub fn new(grab: G, magics: impl Into<Vec<Vec<u8>>>) -> Self {
            Reader { grab, magics: magics.into(), window: None, geometry: None, missed: false, last_wide: None }
        }

        pub fn geometry(&self) -> Option<Geometry> {
            self.geometry
        }

        pub fn grabber(&mut self) -> &mut G {
            &mut self.grab
        }

        pub fn forget(&mut self) {
            self.window = None;
            self.geometry = None;
            self.missed = false;
            self.last_wide = None;
        }

        /// One look at the window, and how long the capture took (None if none was taken).
        pub fn read(&mut self, window: Option<WindowFrame>, now: Instant) -> (Read, Option<Duration>) {
            let Some(window) = window else {
                return (Read::NotCapturable, None);
            };
            if self.window != Some(window) {
                // Moved or resized: where the strip was says nothing any more.
                self.forget();
                self.window = Some(window);
            }
            let due = self.last_wide.is_none_or(|at| now.saturating_duration_since(at) >= WIDE_EVERY);
            let wide = self.geometry.is_none() || (self.missed && due);
            let rect = match (wide, self.geometry) {
                (false, Some(geometry)) => tight_rect(window, geometry),
                _ => wide_rect(window),
            };
            let started = Instant::now();
            let image = self.grab.grab(rect);
            let took = started.elapsed();
            let image = match image {
                Ok(image) => image,
                Err(error) => return (Read::Failed(error), Some(took)),
            };
            if wide {
                self.last_wide = Some(now);
            }
            if let Some(cells) = self.geometry.and_then(|geometry| read_at(&image, &self.magics, geometry)) {
                self.missed = false;
                return (Read::Cells(cells), Some(took));
            }
            if wide {
                if let Some((geometry, cells)) = search(&image, &self.magics, window.scale) {
                    self.geometry = Some(geometry);
                    self.missed = false;
                    return (Read::Cells(cells), Some(took));
                }
            }
            self.missed = true;
            (Read::NoStrip, Some(took))
        }
    }

    /// What to tell the user about how the capture is going: each problem once, not once
    /// per frame, and "no strip" only until the strip has been seen (it hides all the time).
    #[derive(Default)]
    pub struct Report {
        told_permission: bool,
        last_failure: Option<String>,
        told_no_strip: bool,
        seen_strip: bool,
    }

    impl Report {
        pub fn on(&mut self, read: &Read) -> Option<String> {
            match read {
                Read::Failed(GrabError::Permission) if !self.told_permission => {
                    self.told_permission = true;
                    Some(permission_help(crate::updater::running_app().is_some()).to_string())
                }
                Read::Failed(GrabError::Failed(text)) if self.last_failure.as_ref() != Some(text) => {
                    self.last_failure = Some(text.clone());
                    Some(format!("Couldn't capture the screen: {text}"))
                }
                Read::NoStrip if !self.seen_strip && !self.told_no_strip => {
                    self.told_no_strip = true;
                    Some(NO_STRIP_YET.to_string())
                }
                Read::Cells(_) => {
                    self.seen_strip = true;
                    self.last_failure = None;
                    None
                }
                _ => None,
            }
        }
    }

    pub fn geometry_line(geometry: Geometry, window: WindowFrame, took: Option<Duration>) -> String {
        let scale = window.scale as usize;
        let took = took.map_or("not timed".to_string(), |d| format!("{:.1} ms", d.as_secs_f64() * 1000.0));
        format!(
            "Strip found: {} px across and {} px down from the window's corner ({} points down), cell {} px; window {}x{} points at ({},{}), scale {}; capture took {took}.",
            geometry.dx,
            geometry.dy,
            geometry.dy / scale,
            geometry.cell,
            window.rect.w,
            window.rect.h,
            window.rect.x,
            window.rect.y,
            window.scale
        )
    }

    #[derive(Default)]
    struct Tally {
        grabs: u32,
        total: Duration,
        slowest: Duration,
    }

    impl Tally {
        fn add(&mut self, took: Duration) {
            self.grabs += 1;
            self.total += took;
            self.slowest = self.slowest.max(took);
        }

        fn line(&self, how: &str) -> String {
            let ms = |d: Duration| d.as_secs_f64() * 1000.0;
            format!("{} in the last minute {how}, {:.1} ms on average, {:.1} ms at the slowest", self.grabs, ms(self.total) / f64::from(self.grabs), ms(self.slowest))
        }
    }

    /// How long captures have been taking, for the log, per backend. From the stream a capture
    /// is a copy out of memory, so its age (how long ago macOS sent the frame) says more than
    /// its time; frames come only when the window's content changes, so an old one can still
    /// be what is on screen.
    #[derive(Default)]
    pub struct Timing {
        stream: Tally,
        screencapture: Tally,
        frames: u32,
        aged: u32,
        age_total: Duration,
    }

    impl Timing {
        pub fn add(&mut self, backend: Backend, took: Duration, age: Option<Duration>) {
            match backend {
                Backend::Stream => self.stream.add(took),
                Backend::Screencapture => self.screencapture.add(took),
            }
            if let Some(age) = age {
                self.aged += 1;
                self.age_total += age;
            }
        }

        /// Frames the stream sent.
        pub fn frames(&mut self, count: u32) {
            self.frames += count;
        }

        /// The line for the log, and starts counting again; None when no capture was taken.
        pub fn take_line(&mut self) -> Option<String> {
            let mut parts = Vec::new();
            if self.stream.grabs > 0 {
                let mut part = self.stream.line(&format!("from the {}", Backend::Stream.name()));
                part += &format!(", out of {} frames it sent", self.frames);
                if self.aged > 0 {
                    part += &format!(", the newest {:.0} ms old on average", self.age_total.as_secs_f64() * 1000.0 / f64::from(self.aged));
                }
                parts.push(part);
            }
            if self.screencapture.grabs > 0 {
                parts.push(self.screencapture.line(&format!("with {}", Backend::Screencapture.name())));
            }
            *self = Timing::default();
            (!parts.is_empty()).then(|| format!("Captures: {}.", parts.join("; ")))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::codec::{decode_cells, encode_frame, encode_part_frame};

        const PAYLOAD: &str = r#"{"check":"T3 WoW ✓","q":[{"m":"a message that wraps onto a second row of cells, with a little more text to be sure it does","s":4,"t":"send"}]}"#;

        /// The strip on a grey background, as BGRA, like the game draws it.
        fn drawn(cells: &[u8], dx: usize, dy: usize, cell: usize, w: usize, h: usize) -> Image {
            let mut bgra = vec![0x60u8; w * h * 4];
            for (i, &v) in cells.iter().enumerate() {
                let (x0, y0) = (dx + (i % CELLS_PER_ROW) * cell, dy + (i / CELLS_PER_ROW) * cell);
                for y in y0..y0 + cell {
                    for x in x0..x0 + cell {
                        let at = (y * w + x) * 4;
                        bgra[at] = if v & 1 != 0 { 255 } else { 0 };
                        bgra[at + 1] = if v & 2 != 0 { 255 } else { 0 };
                        bgra[at + 2] = if v & 4 != 0 { 255 } else { 0 };
                    }
                }
            }
            Image { bgra, width: w, height: h }
        }

        fn frame_cells() -> Vec<u8> {
            encode_frame(42, PAYLOAD)
        }

        #[test]
        fn a_strip_is_found_under_the_title_bar_and_notch_at_any_cell_size() {
            let cells = frame_cells();
            for (dx, dy, cell, scale) in [(0, 0, 4, 2), (3, 55, 4, 2), (24, 80, 4, 2), (5, 64, 6, 2), (0, 72, 8, 2), (12, 40, 8, 1), (0, 0, 7, 1)] {
                let image = drawn(&cells, dx, dy, cell, 200 * cell + dx + 2, 8 * cell + dy + 2);
                let (found, read) = search(&image, &magic_cells(), scale).unwrap_or_else(|| panic!("not found at {dx},{dy} cell {cell}"));
                assert_eq!(found, Geometry { dx, dy, cell });
                assert_eq!(decode_cells(&read).unwrap().payload, PAYLOAD);
                assert_eq!(read_at(&image, &magic_cells(), found), Some(read), "read back at the kept geometry");
            }
        }

        #[test]
        fn a_long_message_part_is_found_too() {
            let cells = encode_part_frame(9, r#"{"k":"a"}"#, &[1, 2, 0, 255]);
            let image = drawn(&cells, 3, 56, 4, 200 * 4 + 5, 8 * 4 + 58);
            let (found, read) = search(&image, &magic_cells(), 2).expect("part frame found");
            assert_eq!(found, Geometry { dx: 3, dy: 56, cell: 4 });
            assert_eq!(decode_cells(&read).unwrap().part, Some(vec![1, 2, 0, 255]));
        }

        #[test]
        fn past_the_search_area_there_is_no_strip() {
            let cells = frame_cells();
            // A strip is still found up to half a cell past the limit, where its first cells'
            // middles are inside the area searched.
            let too_low = drawn(&cells, 0, 83, 4, 820, 140);
            assert_eq!(search(&too_low, &magic_cells(), 2), None);
            let too_far = drawn(&cells, 27, 0, 4, 830, 60);
            assert_eq!(search(&too_far, &magic_cells(), 2), None);
            let cell_nine = drawn(&cells, 0, 0, 9, 1810, 80);
            assert_eq!(search(&cell_nine, &magic_cells(), 2), None);
        }

        #[test]
        fn the_first_cells_alone_do_not_make_a_strip() {
            let mut cells = frame_cells();
            let middle = cells.len() / 2;
            cells[middle] = (cells[middle] + 1) % 8;
            assert_eq!(search(&drawn(&cells, 0, 10, 4, 820, 60), &magic_cells(), 2), None);
        }

        /// A BMP file of `image`: `bits` per pixel, V5 header with masks or a plain one, rows
        /// from the top or the bottom, and `gap` unused bytes before the pixels.
        fn bmp(image: &Image, bits: u16, v5: bool, top_down: bool, gap: usize) -> Vec<u8> {
            let (w, h) = (image.width, image.height);
            let header = if v5 { 124usize } else { 40 };
            let mut dib = vec![0u8; header];
            dib[0..4].copy_from_slice(&(header as u32).to_le_bytes());
            dib[4..8].copy_from_slice(&(w as i32).to_le_bytes());
            dib[8..12].copy_from_slice(&(if top_down { -(h as i32) } else { h as i32 }).to_le_bytes());
            dib[12..14].copy_from_slice(&1u16.to_le_bytes());
            dib[14..16].copy_from_slice(&bits.to_le_bytes());
            if v5 {
                dib[16..20].copy_from_slice(&3u32.to_le_bytes());
                dib[40..44].copy_from_slice(&0x00ff_0000u32.to_le_bytes());
                dib[44..48].copy_from_slice(&0x0000_ff00u32.to_le_bytes());
                dib[48..52].copy_from_slice(&0x0000_00ffu32.to_le_bytes());
                dib[52..56].copy_from_slice(&0xff00_0000u32.to_le_bytes());
            }
            let stride = (w * bits as usize).div_ceil(32) * 4;
            let mut pixels = vec![0u8; stride * h];
            for y in 0..h {
                let row = if top_down { y } else { h - 1 - y };
                for x in 0..w {
                    let source = &image.bgra[(y * w + x) * 4..(y * w + x) * 4 + 4];
                    let at = row * stride + x * (bits as usize / 8);
                    pixels[at..at + 3].copy_from_slice(&source[..3]);
                    if bits == 32 {
                        pixels[at + 3] = 255;
                    }
                }
            }
            let offset = 14 + header + gap;
            let mut file = Vec::new();
            file.extend_from_slice(b"BM");
            file.extend_from_slice(&((offset + pixels.len()) as u32).to_le_bytes());
            file.extend_from_slice(&[0; 4]);
            file.extend_from_slice(&(offset as u32).to_le_bytes());
            file.extend_from_slice(&dib);
            file.extend(std::iter::repeat_n(0xaa, gap));
            file.extend_from_slice(&pixels);
            file
        }

        #[test]
        fn a_bmp_reads_back_as_the_strip_whatever_its_layout() {
            let image = drawn(&frame_cells(), 2, 3, 4, 820, 40);
            for (bits, v5, top_down, gap) in [(32, true, true, 0), (32, true, false, 0), (32, true, true, 12), (32, false, false, 0), (24, false, true, 0), (24, false, false, 0), (24, false, false, 4)] {
                let decoded = decode_bmp(&bmp(&image, bits, v5, top_down, gap)).unwrap_or_else(|e| panic!("{bits} bits, V5 {v5}, top down {top_down}: {e}"));
                assert_eq!((decoded.width, decoded.height), (820, 40));
                let (found, read) = search(&decoded, &magic_cells(), 2).unwrap();
                assert_eq!(found, Geometry { dx: 2, dy: 3, cell: 4 });
                assert_eq!(decode_cells(&read).unwrap().payload, PAYLOAD);
            }
        }

        #[test]
        fn a_file_that_is_not_a_bmp_is_refused() {
            assert!(decode_bmp(b"").is_err());
            assert!(decode_bmp(b"PNGxxxxxxxxxxxxxxxxxxx").is_err());
            let mut truncated = bmp(&drawn(&[], 0, 0, 4, 2, 2), 32, true, true, 0);
            truncated.truncate(30);
            assert!(decode_bmp(&truncated).is_err());
            // pixels said to be inside the headers
            let mut file = bmp(&drawn(&[], 0, 0, 4, 2, 2), 32, true, true, 0);
            file[10..14].copy_from_slice(&20u32.to_le_bytes());
            assert!(decode_bmp(&file).is_err());
        }

        #[test]
        fn the_client_window_is_the_one_on_screen_and_full_size() {
            let row = |pid, on_screen, w, h| WindowRow { pid, id: (w + h) as u32, on_screen, rect: (0.0, 0.0, w, h) };
            const WOW: i64 = 4242;
            // The six windows measured on this Mac, front to back, and another app's.
            let mut rows = vec![
                row(77, true, 3000.0, 2000.0),
                row(WOW, false, 1728.0, 33.0),
                row(WOW, true, 1728.0, 1117.0),
                row(WOW, false, 1728.0, 33.0),
                row(WOW, false, 64.0, 64.0),
                row(WOW, false, 1728.0, 33.0),
                row(WOW, false, 1728.0, 33.0),
            ];
            assert_eq!(choose_window(&rows, WOW as u32), Some((Rect { x: 0, y: 0, w: 1728, h: 1117 }, 1728 + 1117)));
            // Nothing on screen (another Space, minimized): nothing to capture.
            rows[2].on_screen = false;
            assert_eq!(choose_window(&rows, WOW as u32), None);
            assert_eq!(choose_window(&rows, 999), None);
            // A small helper on screen doesn't pass for the game.
            rows[4].on_screen = true;
            rows[4].rect.2 = 40.0;
            assert_eq!(choose_window(&rows, WOW as u32), None);
        }

        #[test]
        fn on_a_tie_the_frontmost_window_wins() {
            let front = WindowRow { pid: 1, id: 7, on_screen: true, rect: (10.0, 0.0, 800.0, 600.0) };
            let back = WindowRow { pid: 1, id: 8, on_screen: true, rect: (20.0, 0.0, 800.0, 600.0) };
            assert_eq!(choose_window(&[front, back], 1).map(|(rect, id)| (rect.x, id)), Some((10, 7)));
        }

        const WINDOW: WindowFrame = WindowFrame { rect: Rect { x: 0, y: 0, w: 1728, h: 1117 }, scale: 2, id: 77 };

        #[test]
        fn capture_rectangles_cover_the_search_then_just_the_strip() {
            let wide = wide_rect(WINDOW);
            assert_eq!((wide.w, wide.h), ((12 * 2 + 200 * 8 + 8) as u32 / 2, (40 * 2 + 40 * 8 + 8) as u32 / 2));
            let tight = tight_rect(WINDOW, Geometry { dx: 3, dy: 55, cell: 4 });
            assert_eq!((tight.w, tight.h), (402, 108), "pixels rounded up to points");
            // A small window keeps the rectangle inside it, at its own corner.
            let small = WindowFrame { rect: Rect { x: 200, y: 100, w: 300, h: 120 }, scale: 2, id: 78 };
            assert_eq!(wide_rect(small), Rect { x: 200, y: 100, w: 300, h: 120 });
        }

        type Rects = std::rc::Rc<std::cell::RefCell<Vec<Rect>>>;

        fn reader_over(image: Image) -> (Reader<impl FnMut(Rect) -> Result<Image, GrabError>>, Rects) {
            let rects = Rects::default();
            let seen = rects.clone();
            let reader = Reader::new(
                move |rect| {
                    seen.borrow_mut().push(rect);
                    Ok(image.clone())
                },
                magic_cells(),
            );
            (reader, rects)
        }

        #[test]
        fn once_the_strip_is_found_only_its_own_area_is_captured() {
            let image = drawn(&frame_cells(), 3, 55, 4, 820, 160);
            let (mut reader, rects) = reader_over(image);
            let start = Instant::now();
            let (read, took) = reader.read(Some(WINDOW), start);
            assert!(matches!(read, Read::Cells(_)));
            assert!(took.is_some());
            assert_eq!(reader.geometry(), Some(Geometry { dx: 3, dy: 55, cell: 4 }));
            for i in 1..30 {
                assert!(matches!(reader.read(Some(WINDOW), start + Duration::from_millis(250) * i).0, Read::Cells(_)));
            }
            let rects = rects.borrow();
            assert_eq!(rects[0], wide_rect(WINDOW));
            assert!(rects[1..].iter().all(|r| *r == tight_rect(WINDOW, Geometry { dx: 3, dy: 55, cell: 4 })), "no wide capture while the strip is there");
        }

        #[test]
        fn with_the_strip_hidden_the_wide_search_stays_rare() {
            let shown = drawn(&frame_cells(), 3, 55, 4, 820, 160);
            let hidden = drawn(&[], 0, 0, 4, 820, 160);
            let images = std::rc::Rc::new(std::cell::RefCell::new(vec![shown.clone()]));
            let rects = Rects::default();
            let (images_for_grab, rects_for_grab) = (images.clone(), rects.clone());
            let mut reader = Reader::new(
                move |rect| {
                    rects_for_grab.borrow_mut().push(rect);
                    let next = images_for_grab.borrow().last().cloned().unwrap();
                    Ok(next)
                },
                magic_cells(),
            );
            let start = Instant::now();
            assert!(matches!(reader.read(Some(WINDOW), start).0, Read::Cells(_)));
            images.borrow_mut().push(hidden);
            let tight = tight_rect(WINDOW, Geometry { dx: 3, dy: 55, cell: 4 });
            let mut at = start;
            for _ in 0..12 {
                at += Duration::from_millis(250);
                assert_eq!(reader.read(Some(WINDOW), at).0, Read::NoStrip);
            }
            let wide_count = rects.borrow().iter().filter(|r| **r != tight).count();
            assert_eq!(wide_count, 2, "the first search, then one more after two seconds");
            assert_eq!(reader.geometry(), Some(Geometry { dx: 3, dy: 55, cell: 4 }), "still where it was last seen");
        }

        #[test]
        fn a_moved_window_is_searched_again_and_a_missing_one_is_not_captured() {
            let image = drawn(&frame_cells(), 3, 55, 4, 820, 160);
            let (mut reader, rects) = reader_over(image);
            let start = Instant::now();
            reader.read(Some(WINDOW), start);
            let moved = WindowFrame { rect: Rect { x: 50, ..WINDOW.rect }, ..WINDOW };
            reader.read(Some(moved), start + Duration::from_millis(250));
            let rects_now = rects.borrow().clone();
            assert_eq!(rects_now.last().copied(), Some(wide_rect(moved)));
            let (read, took) = reader.read(None, start + Duration::from_millis(500));
            assert_eq!(read, Read::NotCapturable);
            assert_eq!(took, None);
            assert_eq!(rects.borrow().len(), 2, "no capture without a window");
        }

        #[test]
        fn a_refused_capture_and_an_empty_game_are_different_states() {
            // What screencapture says with no Screen Recording permission.
            assert_eq!(grab_failure(false, false, "could not create image from display\n"), Some(GrabError::Permission));
            assert_eq!(grab_failure(true, false, "screencapture: could not create image"), Some(GrabError::Permission), "whatever its exit status");
            assert_eq!(grab_failure(true, true, ""), None);
            assert_eq!(grab_failure(false, false, "out of disk space\n"), Some(GrabError::Failed("screencapture: out of disk space".into())));
            assert_eq!(grab_failure(true, false, ""), Some(GrabError::Failed("screencapture exited without an image".into())));

            let mut refused = Reader::new(|_| Err(GrabError::Permission), magic_cells());
            assert_eq!(refused.read(Some(WINDOW), Instant::now()).0, Read::Failed(GrabError::Permission));

            let (mut empty, _) = reader_over(drawn(&[], 0, 0, 4, 820, 160));
            assert_eq!(empty.read(Some(WINDOW), Instant::now()).0, Read::NoStrip);
        }

        /// The app names its own switch, T3 WoW, and what to do with a stale one; a bare t3wow
        /// names Terminal, whose restart it needs.
        #[test]
        fn the_permission_help_names_the_app_or_terminal() {
            let app = permission_help(true);
            assert!(app.contains("System Settings > Privacy & Security > Screen & System Audio Recording"), "{app}");
            assert!(app.contains("turn on T3 WoW"), "{app}");
            assert!(app.contains("If it shows as on already") && !app.contains("update"), "{app}");
            assert!(!app.contains("Terminal") && !app.contains("t3wow") && !app.contains("again and run"), "{app}");
            let bare = permission_help(false);
            assert!(bare.contains("Screen & System Audio Recording") && bare.contains("Terminal"), "{bare}");
            assert!(bare.contains("quit that app completely"), "{bare}");
        }

        #[test]
        fn each_problem_is_told_once_and_the_strip_ends_the_no_strip_notice() {
            let mut report = Report::default();
            assert_eq!(report.on(&Read::NotCapturable), None);
            let help = report.on(&Read::Failed(GrabError::Permission)).unwrap();
            assert!(help.contains("Screen & System Audio Recording"), "{help}");
            assert_eq!(help, permission_help(false), "the tests run as a bare t3wow");
            for _ in 0..5 {
                assert_eq!(report.on(&Read::Failed(GrabError::Permission)), None, "not once per frame");
            }

            let mut report = Report::default();
            assert!(report.on(&Read::NoStrip).is_some());
            assert_eq!(report.on(&Read::NoStrip), None);
            assert_eq!(report.on(&Read::Cells(vec![1])), None);
            assert_eq!(report.on(&Read::NoStrip), None, "the strip hides all the time once working");

            let mut report = Report::default();
            let timeout = Read::Failed(GrabError::Failed("timeout".into()));
            assert!(report.on(&timeout).is_some());
            assert_eq!(report.on(&timeout), None);
            report.on(&Read::Cells(vec![1]));
            assert!(report.on(&timeout).is_some(), "said again once it comes back after working");
        }

        #[test]
        fn the_log_says_where_the_strip_is_and_what_captures_cost() {
            let line = geometry_line(Geometry { dx: 3, dy: 55, cell: 4 }, WINDOW, Some(Duration::from_millis(62)));
            assert!(line.contains("3 px across") && line.contains("55 px down") && line.contains("cell 4 px") && line.contains("scale 2") && line.contains("62.0 ms"), "{line}");
            let mut timing = Timing::default();
            assert_eq!(timing.take_line(), None);
            timing.add(Backend::Screencapture, Duration::from_millis(40), None);
            timing.add(Backend::Screencapture, Duration::from_millis(60), None);
            let line = timing.take_line().unwrap();
            assert_eq!(line, "Captures: 2 in the last minute with screencapture, 50.0 ms on average, 60.0 ms at the slowest.");
            assert_eq!(timing.take_line(), None, "counts start over");

            // From the stream: each backend on its own, with the frames' age.
            timing.add(Backend::Stream, Duration::from_micros(200), Some(Duration::from_millis(50)));
            timing.add(Backend::Stream, Duration::from_micros(600), Some(Duration::from_millis(70)));
            timing.frames(9);
            timing.add(Backend::Screencapture, Duration::from_millis(90), None);
            let line = timing.take_line().unwrap();
            assert_eq!(
                line,
                "Captures: 2 in the last minute from the ScreenCaptureKit stream, 0.4 ms on average, 0.6 ms at the slowest, out of 9 frames it sent, the newest 60 ms old on average; \
                 1 in the last minute with screencapture, 90.0 ms on average, 90.0 ms at the slowest."
            );
        }

        #[test]
        fn the_capture_folder_is_private_and_not_taken_over() {
            let parent = std::env::temp_dir().join(format!("t3wow-private-{}", std::process::id()));
            let _ = fs::remove_dir_all(&parent);
            fs::create_dir_all(&parent).unwrap();
            let dir = private_dir(&parent).unwrap();
            assert_eq!(fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
            assert_eq!(private_dir(&parent).unwrap(), dir, "reused");
            // Someone else's or anyone's folder, or a link elsewhere, isn't used.
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
            assert!(private_dir(&parent).is_err());
            fs::remove_dir_all(&dir).unwrap();
            std::os::unix::fs::symlink(&parent, &dir).unwrap();
            assert!(private_dir(&parent).is_err());
            let _ = fs::remove_dir_all(&parent);
        }

        #[test]
        fn a_process_start_time_is_steady_and_an_ended_process_has_none() {
            let me = std::process::id();
            let started = process_started(me).expect("this test's own process");
            assert_eq!(process_started(me), Some(started));
            assert!(started > 1_600_000_000_000_000, "microseconds since the epoch, got {started}");
            assert_eq!(process_started(0x7fff_fff0), None);
        }

        #[test]
        fn the_process_list_finds_this_process_and_its_path() {
            let me = std::process::id() as i32;
            let pids = all_pids();
            assert!(pids.contains(&me), "{} processes listed", pids.len());
            assert!(pids.len() < 100_000, "a count of processes, not of bytes");
            let exe = exe_path(me).expect("this test's own path");
            assert_eq!(fs::canonicalize(&exe).unwrap(), fs::canonicalize(std::env::current_exe().unwrap()).unwrap());
        }

        #[test]
        fn folders_compare_without_regard_to_case() {
            assert!(same_folder(Path::new("/Applications/World of Warcraft/_forever_"), Path::new("/applications/world of warcraft/_FOREVER_")));
            assert!(!same_folder(Path::new("/Applications/World of Warcraft/_forever_"), Path::new("/Applications/World of Warcraft/_retail_")));
        }
    }
}

#[cfg(target_os = "macos")]
pub use mac::{find_wow, request_screen_access, wow_processes};

/// Watches for the WoW client in `client_folder` and sends what changes: the process
/// starting or stopping, and each new strip.
#[cfg(windows)]
pub fn spawn(client_folder: PathBuf, tx: Sender<CaptureMessage>) {
    std::thread::Builder::new()
        .name("capture".into())
        .spawn(move || {
            win::set_dpi_aware();
            let magic = magic_cells();
            let mut last_key: Option<Option<String>> = None;
            let mut last_cells: Vec<u8> = Vec::new();
            let mut wow: Option<win::WowProcess> = None;
            let mut searched: Option<std::time::Instant> = None;
            loop {
                let watch = match &wow {
                    Some(w) if w.still_valid() => Watch::Valid,
                    Some(_) => Watch::Lost,
                    None => Watch::Nothing,
                };
                if searched.is_none_or(|at| search_due(watch, at.elapsed())) {
                    wow = win::find_wow(&client_folder);
                    searched = Some(std::time::Instant::now());
                }
                // Ctrl+V counts only in this client (see `keys`).
                crate::keys::watch_process(wow.as_ref().map_or(0, |w| w.pid()));
                let key = wow.as_ref().map(|w| w.key.clone());
                if last_key.as_ref() != Some(&key) {
                    last_key = Some(key.clone());
                    if tx.send(CaptureMessage::Wow(key)).is_err() {
                        return;
                    }
                }
                if let Some(wow) = &wow {
                    match win::sample_window(wow.hwnd, &magic) {
                        win::Sampled::NotCapturable => {}
                        win::Sampled::NoStrip => last_cells.clear(),
                        win::Sampled::Cells(cells) => {
                            if cells != last_cells {
                                last_cells = cells.clone();
                                if tx.send(CaptureMessage::Strip(cells, crate::config::now_ms())).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        })
        .expect("capture thread");
}

/// The last capture was refused for want of Screen Recording permission; cleared by the
/// next capture that works. The menu bar shows its help while this holds.
#[cfg(target_os = "macos")]
static SCREEN_REFUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// macOS refuses to let the companion see the game (no Screen Recording permission).
#[cfg(target_os = "macos")]
pub fn screen_refused() -> bool {
    SCREEN_REFUSED.load(std::sync::atomic::Ordering::Relaxed)
}

/// What a capture says about the permission: refused, granted, or nothing new.
#[cfg(target_os = "macos")]
fn screen_access(read: &mac::Read) -> Option<bool> {
    match read {
        mac::Read::Failed(mac::GrabError::Permission) => Some(true),
        mac::Read::Cells(_) | mac::Read::NoStrip => Some(false),
        mac::Read::NotCapturable | mac::Read::Failed(_) => None,
    }
}

/// Watches for the WoW client in `client_folder` and sends what changes: the process
/// starting or stopping, and each new strip. As on Windows, a look at the client and the
/// strip four times a second, from one ScreenCaptureKit stream on the game's window. When
/// the stream can't run, a screencapture each time, less often while the strip hides
/// (`stream::tick_every`).
#[cfg(target_os = "macos")]
pub fn spawn(client_folder: PathBuf, tx: Sender<CaptureMessage>) {
    use std::time::Instant;
    std::thread::Builder::new()
        .name("capture".into())
        .spawn(move || {
            let streams = stream::Streams::new(|target, window| stream::start(target, window, stream::Filter::Window));
            let mut reader = mac::Reader::new(stream::Capturer::new(streams, mac::grab), magic_cells());
            let mut report = mac::Report::default();
            let mut timing = mac::Timing::default();
            let mut timing_at = Instant::now();
            let mut last_key: Option<Option<String>> = None;
            let mut last_cells: Vec<u8> = Vec::new();
            let mut wow: Option<mac::WowProcess> = None;
            let mut searched: Option<Instant> = None;
            let mut window: Option<mac::WindowFrame> = None;
            let mut window_at: Option<Instant> = None;
            let mut logged: Option<(mac::Geometry, mac::WindowFrame)> = None;
            let mut strip_seen: Option<Instant> = None;
            loop {
                let tick = Instant::now();
                let watch = match &wow {
                    Some(w) if w.still_valid() => Watch::Valid,
                    Some(_) => Watch::Lost,
                    None => Watch::Nothing,
                };
                if searched.is_none_or(|at| search_due(watch, at.elapsed())) {
                    wow = mac::find_wow(&client_folder);
                    searched = Some(Instant::now());
                }
                // Cmd+V counts only in this client (see `keys`).
                crate::keys::watch_process(wow.as_ref().map_or(0, |w| w.pid()));
                let key = wow.as_ref().map(|w| w.key.clone());
                if last_key.as_ref() != Some(&key) {
                    last_key = Some(key.clone());
                    // Another client: where the last one drew the strip says nothing.
                    reader.forget();
                    report = mac::Report::default();
                    last_cells.clear();
                    window = None;
                    window_at = None;
                    logged = None;
                    strip_seen = None;
                    if tx.send(CaptureMessage::Wow(key)).is_err() {
                        return;
                    }
                }
                if let Some(wow) = &wow {
                    if window_at.is_none_or(|at| at.elapsed() >= mac::WINDOW_EVERY) {
                        window = mac::window_of(wow.pid());
                        window_at = Some(Instant::now());
                    }
                }
                // Opens, replaces or (with no window) stops the stream.
                for line in reader.grabber().streams.update(window, window.map(mac::stream_region), Instant::now()) {
                    crate::log::line(&line);
                }
                if wow.is_some() {
                    let (read, took) = reader.read(window, Instant::now());
                    if let (Some(took), Some((backend, age))) = (took, reader.grabber().last()) {
                        timing.add(backend, took, age);
                    }
                    if let Some(line) = report.on(&read) {
                        crate::log::line(&line);
                    }
                    if let Some(refused) = screen_access(&read) {
                        SCREEN_REFUSED.store(refused, std::sync::atomic::Ordering::Relaxed);
                    }
                    if let (Some(geometry), Some(frame)) = (reader.geometry(), window) {
                        if logged != Some((geometry, frame)) {
                            logged = Some((geometry, frame));
                            crate::log::line(&mac::geometry_line(geometry, frame, took));
                        }
                    }
                    match read {
                        mac::Read::NotCapturable | mac::Read::Failed(_) => {}
                        mac::Read::NoStrip => last_cells.clear(),
                        mac::Read::Cells(cells) => {
                            strip_seen = Some(Instant::now());
                            if cells != last_cells {
                                last_cells = cells.clone();
                                if tx.send(CaptureMessage::Strip(cells, crate::config::now_ms())).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                }
                timing.frames(reader.grabber().streams.take_frames());
                if timing_at.elapsed() >= Duration::from_secs(60) {
                    timing_at = Instant::now();
                    if let Some(line) = timing.take_line() {
                        crate::log::line(&line);
                    }
                }
                let every = stream::tick_every(reader.grabber().streams.live(), window.is_some(), strip_seen, Instant::now());
                std::thread::sleep(every.saturating_sub(tick.elapsed()));
            }
        })
        .expect("capture thread");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{decode_cells, encode_frame, encode_part_frame};

    /// A BGRA image of the strip drawn at (dx, dy) on a grey background, like the game draws it.
    fn draw(cells: &[u8], dx: usize, dy: usize) -> (Vec<u8>, usize, usize) {
        let w = CELLS_PER_ROW * CELL_PX + 20;
        let h = cells.len().div_ceil(CELLS_PER_ROW) * CELL_PX + 20;
        let mut px = vec![0x60u8; w * h * 4];
        for (i, &v) in cells.iter().enumerate() {
            let (x0, y0) = (dx + (i % CELLS_PER_ROW) * CELL_PX, dy + (i / CELLS_PER_ROW) * CELL_PX);
            for y in y0..y0 + CELL_PX {
                for x in x0..x0 + CELL_PX {
                    let at = (y * w + x) * 4;
                    px[at] = if v & 1 != 0 { 255 } else { 0 };
                    px[at + 1] = if v & 2 != 0 { 255 } else { 0 };
                    px[at + 2] = if v & 4 != 0 { 255 } else { 0 };
                }
            }
        }
        (px, w, h)
    }

    #[test]
    fn a_strip_on_screen_reads_back_as_the_frame() {
        let payload = r#"{"check":"T3 WoW ✓","q":[{"m":"a longer message that wraps onto a second row of cells, and then some more text to be sure","s":4,"t":"send"}]}"#;
        let (px, w, h) = draw(&encode_frame(42, payload), 3, 2);
        let cells = sample(&px, w * 4, w, h, &magic_cells()).expect("strip found");
        assert_eq!(decode_cells(&cells).unwrap().payload, payload);
    }

    #[test]
    fn a_part_frame_on_screen_is_found_too() {
        let (px, w, h) = draw(&encode_part_frame(9, r#"{"k":"a"}"#, &[1, 2, 0, 255]), 1, 4);
        let cells = sample(&px, w * 4, w, h, &magic_cells()).expect("strip found");
        assert_eq!(decode_cells(&cells).unwrap().part, Some(vec![1, 2, 0, 255]));
    }

    #[test]
    fn the_full_window_search_runs_only_when_needed() {
        let s = Duration::from_secs;
        assert!(!search_due(Watch::Valid, Duration::from_millis(250)));
        assert!(!search_due(Watch::Valid, s(9)));
        assert!(search_due(Watch::Valid, s(10)));
        assert!(search_due(Watch::Lost, Duration::ZERO));
        assert!(!search_due(Watch::Nothing, s(1)));
        assert!(search_due(Watch::Nothing, s(2)));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn only_a_refused_capture_or_a_working_one_changes_the_permission_state() {
        assert_eq!(screen_access(&mac::Read::Failed(mac::GrabError::Permission)), Some(true));
        assert_eq!(screen_access(&mac::Read::Cells(vec![1])), Some(false));
        assert_eq!(screen_access(&mac::Read::NoStrip), Some(false), "a capture without the strip still worked");
        assert_eq!(screen_access(&mac::Read::NotCapturable), None, "no window: nothing was asked");
        assert_eq!(screen_access(&mac::Read::Failed(mac::GrabError::Failed("timed out".into()))), None);
    }

    #[test]
    fn no_strip_means_nothing() {
        let (px, w, h) = draw(&[], 0, 0);
        assert_eq!(sample(&px, w * 4, w, h, &magic_cells()), None);
    }
}
