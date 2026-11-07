//! Reads the addon's strip of colored cells from the top-left of the WoW window, four
//! times a second. It never sends input to the game and never reads its memory: it finds
//! the Wow.exe window and copies a small rectangle of the screen, like a screenshot.

use crate::bridge::CaptureMessage;
use crate::codec::{cells_needed, magic_cells, CELLS_PER_ROW, CELL_PX, MAX_ROWS};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

const SEARCH: usize = 12;

/// Cell value (0..=7) at pixel (x, y) of a BGRA image, or None outside it.
fn cell(px: &[u8], stride: usize, w: usize, h: usize, x: usize, y: usize) -> Option<u8> {
    if x >= w || y >= h {
        return None;
    }
    let i = y * stride + x * 4;
    let bit = |v: u8| (v > 127) as u8;
    Some(bit(px[i + 2]) * 4 + bit(px[i + 1]) * 2 + bit(px[i]))
}

/// Finds the strip (its first cells are `magic`, within SEARCH pixels of the corner) and
/// reads just the cells its frame needs, so the scene behind it doesn't change the result.
pub fn sample(px: &[u8], stride: usize, w: usize, h: usize, magic: &[u8]) -> Option<Vec<u8>> {
    let half = CELL_PX / 2;
    for dy in 0..=SEARCH {
        for dx in 0..=SEARCH {
            let found = magic.iter().enumerate().all(|(i, &m)| cell(px, stride, w, h, dx + i * CELL_PX + half, dy + half) == Some(m));
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
    use windows_sys::Win32::Foundation::{CloseHandle, BOOL, FILETIME, HWND, INVALID_HANDLE_VALUE, LPARAM, POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
    use windows_sys::Win32::System::Threading::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    pub struct WowProcess {
        /// "pid@creation-filetime": stays the same for one client process.
        pub key: String,
        pub hwnd: HWND,
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
            Some(WowProcess { key: format!("{pid}@{started}"), hwnd })
        })
    }

    pub enum Sampled {
        NotCapturable,
        NoStrip,
        Cells(Vec<u8>),
    }

    /// Copies the window's top-left corner from the screen and looks for the strip.
    pub fn sample_window(hwnd: HWND, magic: &[u8]) -> Sampled {
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
            match sample(&pixels, w * 4, w, h, magic) {
                Some(cells) => Sampled::Cells(cells),
                None => Sampled::NoStrip,
            }
        }
    }

    pub fn set_dpi_aware() {
        unsafe { SetProcessDPIAware() };
    }
}

#[cfg(windows)]
pub use win::{find_wow, wow_processes};

/// Watches for the WoW client in `client_folder` and sends what changes: the process
/// starting or stopping, and each new strip.
#[cfg(windows)]
pub fn spawn(client_folder: PathBuf, tx: Sender<CaptureMessage>) {
    win::set_dpi_aware();
    std::thread::Builder::new()
        .name("capture".into())
        .spawn(move || {
            let magic = magic_cells();
            let mut last_key: Option<Option<String>> = None;
            let mut last_cells: Vec<u8> = Vec::new();
            loop {
                let wow = win::find_wow(&client_folder);
                let key = wow.as_ref().map(|w| w.key.clone());
                if last_key.as_ref() != Some(&key) {
                    last_key = Some(key.clone());
                    if tx.send(CaptureMessage::Wow(key)).is_err() {
                        return;
                    }
                }
                if let Some(wow) = wow {
                    match win::sample_window(wow.hwnd, &magic) {
                        win::Sampled::NotCapturable => {}
                        win::Sampled::NoStrip => last_cells.clear(),
                        win::Sampled::Cells(cells) => {
                            if cells != last_cells {
                                last_cells = cells.clone();
                                if tx.send(CaptureMessage::Strip(cells)).is_err() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{decode_cells, encode_frame};

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
    fn no_strip_means_nothing() {
        let (px, w, h) = draw(&[], 0, 0);
        assert_eq!(sample(&px, w * 4, w, h, &magic_cells()), None);
    }
}
