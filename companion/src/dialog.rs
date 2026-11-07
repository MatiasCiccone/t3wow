//! The tray's one input box: the address of a T3 on another machine. A plain Win32 dialog
//! built from an in-memory template (no resource file, no GUI framework): a label, an edit
//! box, a note on plain HTTP, a line for what's wrong with the address, Connect and Cancel.
//! The address is checked (`config::parse_origin_input`) before the dialog closes.

use crate::config::parse_origin_input;
use std::cell::RefCell;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{GetSysColorBrush, SetBkMode, SetTextColor, COLOR_BTNFACE, HDC, TRANSPARENT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DialogBoxIndirectParamW, EndDialog, GetDlgItem, GetDlgItemTextW, SendDlgItemMessageW, SetDlgItemTextW, SetForegroundWindow, DLGTEMPLATE, IDCANCEL,
    IDOK, WM_COMMAND, WM_CTLCOLORSTATIC, WM_INITDIALOG,
};

// Window and control styles, as the template wants them (u32 throughout).
const WS_POPUP: u32 = 0x8000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_BORDER: u32 = 0x0080_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_TABSTOP: u32 = 0x0001_0000;
const DS_SETFONT: u32 = 0x40;
const DS_MODALFRAME: u32 = 0x80;
const DS_CENTER: u32 = 0x800;
const ES_AUTOHSCROLL: u32 = 0x80;
const BS_DEFPUSHBUTTON: u32 = 0x1;
const SS_LEFT: u32 = 0x0;
// Edit control messages (in Win32_UI_Controls, not otherwise needed).
const EM_SETSEL: u32 = 0x00B1;
const EM_SETCUEBANNER: u32 = 0x1501;
const EM_LIMITTEXT: u32 = 0x00C5;

const ID_EDIT: i32 = 1001;
const ID_ERROR: i32 = 1002;
/// Static text that never changes.
const ID_STATIC: u16 = 0xFFFF;

const BUTTON: u16 = 0x0080;
const EDIT: u16 = 0x0081;
const STATIC: u16 = 0x0082;

struct Item {
    style: u32,
    /// Dialog units: x, y, width, height.
    rect: [i16; 4],
    id: u16,
    class: u16,
    text: &'static str,
}

fn push_text(out: &mut Vec<u16>, text: &str) {
    out.extend(text.encode_utf16());
    out.push(0);
}

fn push_u32(out: &mut Vec<u16>, value: u32) {
    out.push(value as u16);
    out.push((value >> 16) as u16);
}

/// A DLGTEMPLATE followed by its items, in 16-bit units; each item starts on a 4-byte
/// boundary, as DialogBoxIndirect requires.
fn template(title: &str, size: [i16; 2], items: &[Item]) -> Vec<u16> {
    let mut out = Vec::new();
    push_u32(&mut out, WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_SETFONT | DS_MODALFRAME | DS_CENTER);
    push_u32(&mut out, 0);
    out.push(items.len() as u16);
    out.extend([0, 0, size[0] as u16, size[1] as u16]);
    out.extend([0, 0]); // no menu, the standard dialog class
    push_text(&mut out, title);
    out.push(9); // point size, then the font
    push_text(&mut out, "Segoe UI");
    for item in items {
        if out.len() % 2 == 1 {
            out.push(0);
        }
        push_u32(&mut out, item.style | WS_CHILD | WS_VISIBLE);
        push_u32(&mut out, 0);
        out.extend(item.rect.map(|v| v as u16));
        out.push(item.id);
        out.extend([0xFFFF, item.class]);
        push_text(&mut out, item.text);
        out.push(0); // no creation data
    }
    out
}

fn items() -> Vec<Item> {
    let text = |rect, text| Item { style: SS_LEFT, rect, id: ID_STATIC, class: STATIC, text };
    vec![
        text([7, 7, 266, 9], "Address of the PC running T3 Code:"),
        Item { style: WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL, rect: [7, 18, 266, 13], id: ID_EDIT as u16, class: EDIT, text: "" },
        text([7, 35, 266, 18], "host:port, like 192.168.1.20:3773 or gaming-pc.lan:3773, or a full http:// or https:// URL. T3 on that PC must accept connections from other machines."),
        text(
            [7, 57, 266, 26],
            "Plain http:// sends the sign-in token across your network unencrypted: fine on a home network you trust; anywhere else, use https://. T3 WoW switches once the sign-in succeeds and stays on its current T3 until then.",
        ),
        Item { style: SS_LEFT, rect: [7, 86, 266, 18], id: ID_ERROR as u16, class: STATIC, text: "" },
        Item { style: WS_TABSTOP | BS_DEFPUSHBUTTON, rect: [169, 108, 50, 14], id: IDOK as u16, class: BUTTON, text: "Connect" },
        Item { style: WS_TABSTOP, rect: [223, 108, 50, 14], id: IDCANCEL as u16, class: BUTTON, text: "Cancel" },
    ]
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

thread_local! {
    /// The modal dialog's state: the text it opens with, then the origin it closes with.
    static STATE: RefCell<(String, Option<String>)> = const { RefCell::new((String::new(), None)) };
}

unsafe extern "system" fn dialog_proc(dialog: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> isize {
    match message {
        WM_INITDIALOG => {
            let initial = STATE.with(|s| s.borrow().0.clone());
            SetDlgItemTextW(dialog, ID_EDIT, wide(&initial).as_ptr());
            SendDlgItemMessageW(dialog, ID_EDIT, EM_LIMITTEXT, 300, 0);
            SendDlgItemMessageW(dialog, ID_EDIT, EM_SETCUEBANNER, 1, wide("192.168.1.20:3773").as_ptr() as LPARAM);
            SendDlgItemMessageW(dialog, ID_EDIT, EM_SETSEL, 0, -1);
            // Opened from the tray menu: come to the front rather than behind the game (not
            // in tests, which mustn't take the desktop's focus).
            if !cfg!(test) {
                SetForegroundWindow(dialog);
            }
            1 // focus the first tab stop: the edit box
        }
        WM_COMMAND => {
            let id = (wparam & 0xFFFF) as i32;
            if id == IDOK {
                let mut buffer = [0u16; 320];
                let length = GetDlgItemTextW(dialog, ID_EDIT, buffer.as_mut_ptr(), buffer.len() as i32);
                let typed = String::from_utf16_lossy(&buffer[..(length as usize).min(buffer.len())]);
                match parse_origin_input(&typed) {
                    Ok(origin) => {
                        STATE.with(|s| s.borrow_mut().1 = Some(origin));
                        EndDialog(dialog, 1);
                    }
                    Err(problem) => {
                        SetDlgItemTextW(dialog, ID_ERROR, wide(&problem).as_ptr());
                    }
                }
                1
            } else if id == IDCANCEL {
                EndDialog(dialog, 0);
                1
            } else {
                0
            }
        }
        // What's wrong with the address, in red.
        WM_CTLCOLORSTATIC if lparam as HWND == GetDlgItem(dialog, ID_ERROR) => {
            let dc = wparam as HDC;
            SetTextColor(dc, 0x0000_20C0); // 0x00BBGGRR
            SetBkMode(dc, TRANSPARENT as i32);
            GetSysColorBrush(COLOR_BTNFACE) as isize
        }
        _ => 0,
    }
}

/// The box's template, in a buffer on a 4-byte boundary, as DialogBoxIndirect needs.
fn aligned_template() -> Vec<u32> {
    let units = template("Connect to T3 on another machine", [280, 129], &items());
    let mut aligned = vec![0u32; units.len().div_ceil(2)];
    for (i, unit) in units.iter().enumerate() {
        aligned[i / 2] |= (*unit as u32) << (16 * (i % 2));
    }
    aligned
}

/// Asks for the address of a T3 on another machine, starting with `initial`. The origin it
/// stands for, checked, or None when cancelled. Blocks the calling (tray) thread until the
/// box closes, as a message box does.
pub fn ask_origin(initial: &str) -> Option<String> {
    STATE.with(|s| *s.borrow_mut() = (initial.to_string(), None));
    let aligned = aligned_template();
    let result = unsafe { DialogBoxIndirectParamW(GetModuleHandleW(null_mut()), aligned.as_ptr() as *const DLGTEMPLATE, null_mut(), Some(dialog_proc), 0) };
    if result == -1 {
        crate::log::line("Couldn't open the address box.");
    }
    STATE.with(|s| s.borrow_mut().1.take())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_template_counts_its_items_and_aligns_each() {
        let items = items();
        let units = template("Title", [280, 129], &items);
        assert_eq!(units[4] as usize, items.len(), "cdit");
        assert_eq!(&units[7..9], &[280, 129]);
        // Walk the items the way Windows does: each starts on a 4-byte boundary, with its
        // id and class where the layout puts them.
        let header_end = 11 + "Title".len() + 1 + 1 + "Segoe UI".len() + 1;
        let mut at = header_end;
        for item in &items {
            at += at % 2;
            assert_eq!(units[at + 8], item.id);
            assert_eq!(&units[at + 9..at + 11], &[0xFFFF, item.class]);
            at += 11 + item.text.encode_utf16().count() + 1 + 1;
        }
        assert_eq!(at, units.len());
    }

    fn text_of(dialog: HWND, id: i32) -> String {
        let mut buffer = [0u16; 320];
        let length = unsafe { GetDlgItemTextW(dialog, id, buffer.as_mut_ptr(), buffer.len() as i32) } as usize;
        String::from_utf16_lossy(&buffer[..length])
    }

    /// Windows builds the box from the template, and Connect checks the address: a bad one
    /// keeps the box open and says why, a good one closes it with the origin.
    #[test]
    fn the_box_opens_and_connect_checks_the_address() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{CreateDialogIndirectParamW, DestroyWindow, SendMessageW};
        STATE.with(|s| *s.borrow_mut() = ("http://box:3773".into(), None));
        let aligned = aligned_template();
        let dialog = unsafe { CreateDialogIndirectParamW(GetModuleHandleW(null_mut()), aligned.as_ptr() as *const DLGTEMPLATE, null_mut(), Some(dialog_proc), 0) };
        assert!(!dialog.is_null(), "Windows refused the template");
        for id in [ID_EDIT, ID_ERROR, IDOK, IDCANCEL] {
            assert!(!unsafe { GetDlgItem(dialog, id) }.is_null(), "control {id}");
        }
        assert_eq!(text_of(dialog, ID_EDIT), "http://box:3773", "it opens with the given address");

        unsafe { SetDlgItemTextW(dialog, ID_EDIT, wide("box").as_ptr()) };
        unsafe { SendMessageW(dialog, WM_COMMAND, IDOK as WPARAM, 0) };
        assert!(text_of(dialog, ID_ERROR).contains("box:3773"), "{}", text_of(dialog, ID_ERROR));
        assert_eq!(STATE.with(|s| s.borrow().1.clone()), None);

        unsafe { SetDlgItemTextW(dialog, ID_EDIT, wide(" 192.168.1.20:3773 ").as_ptr()) };
        unsafe { SendMessageW(dialog, WM_COMMAND, IDOK as WPARAM, 0) };
        assert_eq!(STATE.with(|s| s.borrow_mut().1.take()).as_deref(), Some("http://192.168.1.20:3773"));
        unsafe { DestroyWindow(dialog) };
    }

    /// Shows the box for a few seconds, off-screen and without taking the focus, so a
    /// screenshot tool can capture it (PrintWindow on its title) without anyone's click or
    /// keypress landing in it: `cargo test -- --ignored render_the_box_off_screen`.
    #[test]
    #[ignore = "shows a window for a few seconds, for a screenshot"]
    fn render_the_box_off_screen() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CreateDialogIndirectParamW, DestroyWindow, DispatchMessageW, PeekMessageW, SetWindowPos, ShowWindow, TranslateMessage, MSG, PM_REMOVE, SWP_NOACTIVATE,
            SWP_NOSIZE, SWP_NOZORDER, SW_SHOWNOACTIVATE,
        };
        STATE.with(|s| *s.borrow_mut() = ("192.168.1.20:3773".into(), None));
        let aligned = aligned_template();
        let dialog = unsafe { CreateDialogIndirectParamW(GetModuleHandleW(null_mut()), aligned.as_ptr() as *const DLGTEMPLATE, null_mut(), Some(dialog_proc), 0) };
        assert!(!dialog.is_null());
        unsafe {
            SetDlgItemTextW(dialog, ID_ERROR, wide("Add the port T3 listens on, like box:3773.").as_ptr());
            SetWindowPos(dialog, null_mut(), -3000, -3000, 0, 0, SWP_NOACTIVATE | SWP_NOSIZE | SWP_NOZORDER);
            ShowWindow(dialog, SW_SHOWNOACTIVATE);
        }
        let until = std::time::Instant::now() + std::time::Duration::from_secs(4);
        let mut msg: MSG = unsafe { std::mem::zeroed() };
        while std::time::Instant::now() < until {
            while unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) } != 0 {
                unsafe {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        unsafe { DestroyWindow(dialog) };
    }
}
