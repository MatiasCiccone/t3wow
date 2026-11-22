//! Images pasted in the game: the game can't read the clipboard, so when you press Ctrl+V
//! (Cmd+V on a Mac) in its message box with an image copied, the companion reads it here (a
//! screenshot from Snipping Tool or macOS's Cmd+Ctrl+Shift+4, an image copied in a browser or
//! an editor, or on Windows image files copied in Explorer) and keeps it as a PNG to send with
//! the message, plus a small picture for the game to show above the box.
//!
//! The other way, text copied from the conversation: the game can't write to the clipboard
//! either, so its copy icon asks the companion to put a message there (`write_text`).

use crate::render::{decode_png, Bitmap};
#[cfg(any(windows, test))]
use std::path::{Path, PathBuf};

/// T3 takes images up to this size.
pub const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;
/// Images larger than this (in pixels) aren't read: no screen copies anything this big.
const MAX_PIXELS: u64 = 64 * 1024 * 1024;
/// The small picture's height, in pixels (the game draws it about half that tall).
pub const THUMB_HEIGHT: u32 = 80;
const THUMB_MAX_WIDTH: u32 = 240;
/// The widest large picture of a pasted image (a 1024-wide texture loads in the game).
const PREVIEW_MAX_WIDTH: u32 = 1024;
/// Images read at most from copied files at once (a message takes 8).
#[cfg(any(windows, test))]
const MAX_FILES: usize = 8;
/// A copied file larger than this isn't read.
#[cfg(any(windows, test))]
const MAX_FILE_BYTES: u64 = 24 * 1024 * 1024;

/// An image read from the clipboard: the PNG to send and the pixels to draw it from.
pub struct Clipped {
    pub png: Vec<u8>,
    pub bitmap: Bitmap,
}

/// The images on the clipboard: the one image copied, or the image files copied in Explorer
/// (none: it holds no image).
#[cfg(windows)]
pub fn read_images() -> Result<Vec<Clipped>, String> {
    let win::Copied { png, dib, files } = win::read()?;
    // A PNG as it was copied keeps its transparency and needs no encoding; one that doesn't
    // decode falls back to the bitmap Windows keeps next to it.
    if let Some(png) = png {
        if let Ok(bitmap) = decode_png(&png) {
            return fit_upload(Some(png), bitmap).map(|c| vec![c]);
        }
    }
    match dib {
        Some(dib) => fit_upload(None, dib_bitmap(&dib)?).map(|c| vec![c]),
        None => image_files(&files, win::decode_file),
    }
}

/// The image on a Mac's clipboard (the general pasteboard), none if it holds none.
#[cfg(target_os = "macos")]
pub fn read_images() -> Result<Vec<Clipped>, String> {
    let copied = mac::read();
    pick(copied, mac::tiff_png).map(|c| c.into_iter().collect())
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn read_images() -> Result<Vec<Clipped>, String> {
    Ok(Vec::new())
}

/// Puts text on the clipboard, in place of what it held: a message copied from the game's
/// conversation. Windows keeps its line breaks as CR LF.
#[cfg(windows)]
pub fn write_text(text: &str) -> Result<(), String> {
    win::write_text(&crlf(text))
}

/// Puts text on a Mac's clipboard (the general pasteboard), in place of what it held.
#[cfg(target_os = "macos")]
pub fn write_text(text: &str) -> Result<(), String> {
    mac::write_text(text)
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn write_text(_text: &str) -> Result<(), String> {
    Err("T3 WoW can't copy to the clipboard on this system".into())
}

/// Line breaks as Windows programs expect them on the clipboard: CR LF.
#[cfg(any(windows, test))]
fn crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n").replace('\n', "\r\n")
}

/// The paths in a CF_HDROP (files copied in Explorer): a DROPFILES header, whose first field
/// is where the list starts and whose fifth says whether it's UTF-16 (always, from Explorer)
/// or ANSI; then the paths, each ending in a 0, and a 0 after the last.
#[cfg(any(windows, test))]
fn dropped_files(drop: &[u8]) -> Vec<PathBuf> {
    let u32_at = |i: usize| drop.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let (Some(start), Some(wide)) = (u32_at(0), u32_at(16)) else {
        return Vec::new();
    };
    let list = drop.get(start as usize..).unwrap_or_default();
    let names: Vec<String> = if wide != 0 {
        let units: Vec<u16> = list.as_chunks::<2>().0.iter().map(|&b| u16::from_le_bytes(b)).collect();
        units.split(|&u| u == 0).take_while(|name| !name.is_empty()).map(String::from_utf16_lossy).collect()
    } else {
        list.split(|&b| b == 0).take_while(|name| !name.is_empty()).map(|name| String::from_utf8_lossy(name).into_owned()).collect()
    };
    names.into_iter().map(PathBuf::from).collect()
}

/// Copied files as images, in the order copied: each image file T3 WoW can read (a PNG as it
/// is; JPEG, GIF, BMP and TIFF by `decode`, the system's decoder), other files left out.
/// Files of which none is such an image are an error saying why, not "no image".
#[cfg(any(windows, test))]
fn image_files(files: &[PathBuf], decode: impl Fn(&Path) -> Result<Bitmap, String>) -> Result<Vec<Clipped>, String> {
    let mut images = Vec::new();
    let mut refused = None;
    for path in files {
        if images.len() == MAX_FILES {
            break;
        }
        match image_file(path, &decode) {
            Ok(image) => images.push(image),
            Err(error) => refused = refused.or(Some(error)),
        }
    }
    match refused {
        Some(error) if images.is_empty() => Err(error),
        _ => Ok(images),
    }
}

#[cfg(any(windows, test))]
fn image_file(path: &Path, decode: impl Fn(&Path) -> Result<Bitmap, String>) -> Result<Clipped, String> {
    let size = std::fs::metadata(path).map_err(|_| "the copied file is gone".to_string())?.len();
    if size > MAX_FILE_BYTES {
        return Err("the copied file is too large".into());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("couldn't read the copied file: {e}"))?;
    let tiff = bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*");
    let size = crate::render::sniff(&bytes).map(|(_, w, h)| (w, h));
    if size.is_none() && !tiff {
        return Err("the copied file isn't an image (PNG, JPEG, GIF, BMP or TIFF)".into());
    }
    if size.is_some_and(|(w, h)| w as u64 * h as u64 > MAX_PIXELS) {
        return Err("the copied image is too large".into());
    }
    if let Ok(bitmap) = decode_png(&bytes) {
        return fit_upload(Some(bytes), bitmap);
    }
    fit_upload(None, decode(path)?)
}

/// What a Mac's clipboard holds: an image as a PNG and as a TIFF (screenshots copy both,
/// browsers often only the TIFF), and whether a file was copied (in Finder).
#[cfg(any(target_os = "macos", test))]
#[derive(Default)]
struct MacCopied {
    png: Option<Vec<u8>>,
    tiff: Option<Vec<u8>>,
    file: bool,
}

/// The image to send from a Mac's clipboard: the PNG as it was copied when it reads (it
/// keeps its transparency and needs no encoding), else the TIFF, made a PNG by `tiff_png`
/// (macOS's own decoder). A file copied in Finder brings only its icon as a TIFF, which isn't
/// the image you meant: that is no image (on Windows, `image_files` reads the files copied).
#[cfg(any(target_os = "macos", test))]
fn pick(copied: MacCopied, tiff_png: impl FnOnce(&[u8]) -> Result<Vec<u8>, String>) -> Result<Option<Clipped>, String> {
    if let Some(png) = copied.png {
        if let Ok(bitmap) = decode_png(&png) {
            return fit_upload(Some(png), bitmap).map(Some);
        }
    }
    match copied.tiff.filter(|_| !copied.file) {
        Some(tiff) => {
            let png = tiff_png(&tiff)?;
            let bitmap = decode_png(&png).map_err(|_| "the copied image is in a format T3 WoW can't read".to_string())?;
            fit_upload(Some(png), bitmap).map(Some)
        }
        None => Ok(None),
    }
}

/// A PNG small enough for T3: the one copied if it fits, else the pixels encoded, at half
/// the size each time until they fit.
pub fn fit_upload(png: Option<Vec<u8>>, bitmap: Bitmap) -> Result<Clipped, String> {
    if let Some(png) = png.filter(|p| p.len() <= MAX_UPLOAD_BYTES) {
        return Ok(Clipped { png, bitmap });
    }
    let mut scaled = bitmap.clone();
    for _ in 0..4 {
        let png = encode_png(&scaled)?;
        if png.len() <= MAX_UPLOAD_BYTES {
            return Ok(Clipped { png, bitmap });
        }
        scaled = halved(&scaled);
    }
    Err("the image is too large to send (T3 takes up to 10 MB)".into())
}

pub fn encode_png(bitmap: &Bitmap) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, bitmap.width, bitmap.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|e| format!("couldn't encode the image: {e}"))?;
    writer.write_image_data(&bitmap.rgba).map_err(|e| format!("couldn't encode the image: {e}"))?;
    writer.finish().map_err(|e| format!("couldn't encode the image: {e}"))?;
    Ok(out)
}

/// `bitmap` scaled to `width` x `height`, each pixel the average of the ones it covers.
fn scaled(bitmap: &Bitmap, width: u32, height: u32) -> Bitmap {
    let (sw, sh) = (bitmap.width as usize, bitmap.height as usize);
    let (w, h) = (width.max(1) as usize, height.max(1) as usize);
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let (y0, y1) = (y * sh / h, ((y + 1) * sh / h).max(y * sh / h + 1).min(sh));
        for x in 0..w {
            let (x0, x1) = (x * sw / w, ((x + 1) * sw / w).max(x * sw / w + 1).min(sw));
            let mut sum = [0u64; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = (sy * sw + sx) * 4;
                    for (c, total) in sum.iter_mut().enumerate() {
                        *total += bitmap.rgba[i + c] as u64;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u64;
            rgba.extend(sum.iter().map(|total| (total / n) as u8));
        }
    }
    Bitmap { width: w as u32, height: h as u32, rgba }
}

fn halved(bitmap: &Bitmap) -> Bitmap {
    scaled(bitmap, bitmap.width.div_ceil(2), bitmap.height.div_ceil(2))
}

/// The small picture the game shows above the box: THUMB_HEIGHT tall (or less for a small
/// image), at most THUMB_MAX_WIDTH wide, keeping its shape.
pub fn thumbnail(bitmap: &Bitmap) -> Bitmap {
    let (w, h) = (bitmap.width as f64, bitmap.height as f64);
    let scale = (THUMB_HEIGHT as f64 / h).min(THUMB_MAX_WIDTH as f64 / w).min(1.0);
    scaled(bitmap, ((w * scale).round() as u32).max(1), ((h * scale).round() as u32).max(1))
}

/// The large picture: what the game enlarges on a click and shows in the conversation once
/// the image is sent. An image that fits a texture stays as it is.
pub fn preview(bitmap: &Bitmap) -> Bitmap {
    let (w, h) = (bitmap.width as f64, bitmap.height as f64);
    let scale = (PREVIEW_MAX_WIDTH as f64 / w).min(crate::render::MAX_HEIGHT as f64 / h).min(1.0);
    scaled(bitmap, ((w * scale).round() as u32).max(1), ((h * scale).round() as u32).max(1))
}

/// A device-independent bitmap (CF_DIB: a BITMAPINFOHEADER, color masks, then the rows) as
/// RGBA. Screens copy 24 or 32 bits per pixel; 32-bit ones from most programs leave alpha
/// at 0, which means opaque.
pub fn dib_bitmap(dib: &[u8]) -> Result<Bitmap, String> {
    let u32_at = |i: usize| dib.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let u16_at = |i: usize| dib.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let bad = || "the copied image is in a format T3 WoW can't read".to_string();
    let header = u32_at(0).ok_or_else(bad)? as usize;
    let width = u32_at(4).ok_or_else(bad)?.cast_signed();
    let height = u32_at(8).ok_or_else(bad)?.cast_signed();
    let bits = u16_at(14).ok_or_else(bad)?;
    let compression = u32_at(16).ok_or_else(bad)?;
    let colors_used = u32_at(32).ok_or_else(bad)? as usize;
    if header < 40 || width <= 0 || height == 0 || !matches!(bits, 24 | 32) {
        return Err(bad());
    }
    let (w, h) = (width as usize, height.unsigned_abs() as usize);
    if (w as u64) * (h as u64) > MAX_PIXELS {
        return Err("the copied image is too large".into());
    }
    // BI_RGB (0), or BI_BITFIELDS (3) with its masks: in the header from version 2 (52 bytes)
    // on, else just after it.
    let (masks, after_masks) = match compression {
        0 => (None, header),
        3 if header >= 52 => (Some((u32_at(40), u32_at(44), u32_at(48), if header >= 56 { u32_at(52) } else { Some(0) })), header),
        3 => (Some((u32_at(header), u32_at(header + 4), u32_at(header + 8), Some(0))), header + 12),
        _ => return Err(bad()),
    };
    let masks = match masks {
        Some((Some(r), Some(g), Some(b), Some(a))) => Some([r, g, b, a]),
        Some(_) => return Err(bad()),
        None => None,
    };
    let start = after_masks + colors_used * 4;
    let stride = (w * bits as usize).div_ceil(32) * 4;
    let rows = dib.get(start..start + stride * h).ok_or_else(bad)?;
    let channel = |pixel: u32, mask: u32| -> u8 {
        if mask == 0 {
            return 0;
        }
        let value = (pixel & mask) >> mask.trailing_zeros();
        let max = mask >> mask.trailing_zeros();
        (value * 255 / max) as u8
    };
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        // Rows run from the bottom unless the height is negative.
        let row = if height > 0 { h - 1 - y } else { y };
        let line = &rows[row * stride..row * stride + stride];
        for x in 0..w {
            match bits {
                24 => {
                    let p = &line[x * 3..x * 3 + 3];
                    rgba.extend_from_slice(&[p[2], p[1], p[0], 255]);
                }
                _ => {
                    let p = &line[x * 4..x * 4 + 4];
                    let pixel = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
                    let [r, g, b, a] = masks.unwrap_or([0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0xff00_0000]);
                    rgba.extend_from_slice(&[channel(pixel, r), channel(pixel, g), channel(pixel, b), channel(pixel, a)]);
                }
            }
        }
    }
    // No pixel with any alpha: the program didn't use it, so it's all opaque.
    if bits == 32 && rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 0) {
        rgba.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p[3] = 255);
    }
    Ok(Bitmap { width: w as u32, height: h as u32, rgba })
}

#[cfg(windows)]
mod win {
    use std::ptr::null_mut;
    use windows_sys::Win32::Graphics::GdiPlus::{
        BitmapData, GdipBitmapLockBits, GdipBitmapUnlockBits, GdipCreateBitmapFromFile, GdipDisposeImage, GdipGetImageHeight, GdipGetImageWidth, GdipGetPropertyItem,
        GdipGetPropertyItemSize, GdipImageRotateFlip, GdiplusShutdown, GdiplusStartup, GdiplusStartupInput, GpBitmap, GpImage, ImageLockModeRead, Ok as GDIPLUS_OK, PropertyItem, Rect,
    };
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData};
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};

    const CF_DIB: u32 = 8;
    const CF_HDROP: u32 = 15;
    const CF_UNICODETEXT: u32 = 13;

    /// Closes the clipboard when dropped.
    pub(super) struct Open;
    impl Drop for Open {
        fn drop(&mut self) {
            unsafe { CloseClipboard() };
        }
    }

    /// The clipboard's data in `format`, copied out.
    pub(super) unsafe fn data(format: u32) -> Option<Vec<u8>> {
        unsafe {
            if IsClipboardFormatAvailable(format) == 0 {
                return None;
            }
            let handle = GetClipboardData(format);
            if handle.is_null() {
                return None;
            }
            let size = GlobalSize(handle);
            let ptr = GlobalLock(handle) as *const u8;
            if ptr.is_null() {
                return None;
            }
            let bytes = std::slice::from_raw_parts(ptr, size).to_vec();
            GlobalUnlock(handle);
            Some(bytes)
        }
    }

    /// What the clipboard holds as a PNG, as a bitmap and as copied files (none: no image).
    pub struct Copied {
        pub png: Option<Vec<u8>>,
        pub dib: Option<Vec<u8>>,
        pub files: Vec<std::path::PathBuf>,
    }

    /// Opens the clipboard, closed when the result is dropped. Another program may hold it
    /// for a moment.
    pub(super) fn open() -> Result<Open, String> {
        for _ in 0..10 {
            if unsafe { OpenClipboard(null_mut()) } != 0 {
                return Ok(Open);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        Err("another program is using the clipboard: try again".into())
    }

    pub fn read() -> Result<Copied, String> {
        unsafe {
            let _open = open()?;
            let name: Vec<u16> = "PNG".encode_utf16().chain(Some(0)).collect();
            let png_format = RegisterClipboardFormatW(name.as_ptr());
            let png = if png_format != 0 { data(png_format) } else { None };
            let files = data(CF_HDROP).map(|drop| super::dropped_files(&drop)).unwrap_or_default();
            Ok(Copied { png, dib: data(CF_DIB), files })
        }
    }

    /// Puts `text` on the clipboard as Unicode text, in place of everything it held.
    pub fn write_text(text: &str) -> Result<(), String> {
        let units: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
        let failed = || "Windows didn't take the text onto the clipboard: try again".to_string();
        unsafe {
            // The text is ready before the clipboard is emptied, so a failure to make it or to
            // open the clipboard leaves what it held. Once set, the clipboard owns the memory;
            // until then it's ours to free.
            let memory = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2);
            if memory.is_null() {
                return Err(failed());
            }
            let ptr = GlobalLock(memory) as *mut u16;
            if ptr.is_null() {
                GlobalFree(memory);
                return Err(failed());
            }
            std::ptr::copy_nonoverlapping(units.as_ptr(), ptr, units.len());
            GlobalUnlock(memory);
            let set = open().and_then(|_open| {
                // Opened with no window, as clipboard-win does: Windows takes the text all the
                // same (the ignored test below reads it back).
                if EmptyClipboard() == 0 || SetClipboardData(CF_UNICODETEXT, memory).is_null() {
                    return Err(failed());
                }
                Ok(())
            });
            if set.is_err() {
                GlobalFree(memory);
            }
            set
        }
    }

    /// GDI+ (on every Windows), started for one decode and shut down when dropped.
    struct GdiPlus(usize);
    impl Drop for GdiPlus {
        fn drop(&mut self) {
            unsafe { GdiplusShutdown(self.0) };
        }
    }

    /// A GDI+ image, disposed of (and its file let go) when dropped.
    struct Image(*mut GpBitmap);
    impl Drop for Image {
        fn drop(&mut self) {
            unsafe { GdipDisposeImage(self.0.cast()) };
        }
    }

    /// PixelFormat32bppARGB: rows of B, G, R, A bytes.
    const ARGB: i32 = 0x0026_200a;
    /// PropertyTagOrientation: how the camera was held (EXIF's 1 to 8).
    const ORIENTATION: u32 = 0x0112;

    /// An image file as RGBA, by GDI+: JPEG, GIF (its first frame), BMP, TIFF and PNG, turned
    /// the way the camera says it was held.
    pub fn decode_file(path: &std::path::Path) -> Result<super::Bitmap, String> {
        use std::os::windows::ffi::OsStrExt;
        let bad = || "the copied image is in a format T3 WoW can't read".to_string();
        unsafe {
            let mut token = 0usize;
            let input = GdiplusStartupInput { GdiplusVersion: 1, ..Default::default() };
            if GdiplusStartup(&mut token, &input, null_mut()) != GDIPLUS_OK {
                return Err(bad());
            }
            let _gdiplus = GdiPlus(token);
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let mut bitmap = null_mut();
            if GdipCreateBitmapFromFile(wide.as_ptr(), &mut bitmap) != GDIPLUS_OK || bitmap.is_null() {
                return Err(bad());
            }
            // Dropped before `_gdiplus`, so disposed of before GDI+ shuts down.
            let image = Image(bitmap);
            upright(image.0.cast());
            let (mut width, mut height) = (0u32, 0u32);
            GdipGetImageWidth(image.0.cast(), &mut width);
            GdipGetImageHeight(image.0.cast(), &mut height);
            if width == 0 || height == 0 {
                return Err(bad());
            }
            if width as u64 * height as u64 > super::MAX_PIXELS {
                return Err("the copied image is too large".into());
            }
            let rect = Rect { X: 0, Y: 0, Width: width as i32, Height: height as i32 };
            let mut locked = BitmapData::default();
            if GdipBitmapLockBits(image.0, &rect, ImageLockModeRead as u32, ARGB, &mut locked) != GDIPLUS_OK || locked.Scan0.is_null() {
                return Err(bad());
            }
            let (w, h) = (width as usize, height as usize);
            let mut rgba = Vec::with_capacity(w * h * 4);
            for y in 0..h {
                let row = (locked.Scan0 as *const u8).offset(y as isize * locked.Stride as isize);
                for p in std::slice::from_raw_parts(row, w * 4).as_chunks::<4>().0 {
                    rgba.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
                }
            }
            GdipBitmapUnlockBits(image.0, &mut locked);
            Ok(super::Bitmap { width, height, rgba })
        }
    }

    /// Turns a photo the way its EXIF orientation says (GDI+ doesn't on its own).
    unsafe fn upright(image: *mut GpImage) {
        unsafe {
            let mut size = 0u32;
            if GdipGetPropertyItemSize(image, ORIENTATION, &mut size) != GDIPLUS_OK || (size as usize) < std::mem::size_of::<PropertyItem>() + 2 {
                return;
            }
            // u64s, so the item lands aligned.
            let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
            let item = buffer.as_mut_ptr().cast::<PropertyItem>();
            if GdipGetPropertyItem(image, ORIENTATION, size, item) != GDIPLUS_OK || (*item).value.is_null() || (*item).length < 2 {
                return;
            }
            // EXIF's 2 to 8, as GDI+'s RotateFlipType.
            let turn = match *(*item).value.cast::<u16>() {
                2 => 4,
                3 => 2,
                4 => 6,
                5 => 5,
                6 => 1,
                7 => 7,
                8 => 3,
                _ => return,
            };
            GdipImageRotateFlip(image, turn);
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use super::MacCopied;
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSPasteboard, NSPasteboardTypeFileURL, NSPasteboardTypePNG, NSPasteboardTypeString, NSPasteboardTypeTIFF};
    use objc2_foundation::{NSData, NSDictionary, NSString};

    pub fn read() -> MacCopied {
        autoreleasepool(|_| {
            let board = NSPasteboard::generalPasteboard();
            // SAFETY: AppKit's constant type names, valid for the life of the process.
            let (png, tiff, file) = unsafe { (NSPasteboardTypePNG, NSPasteboardTypeTIFF, NSPasteboardTypeFileURL) };
            let data = |kind| board.dataForType(kind).map(|d| d.to_vec());
            let file = board.types().is_some_and(|types| types.containsObject(file));
            MacCopied { png: data(png), tiff: data(tiff), file }
        })
    }

    /// Puts `text` on the pasteboard as plain text, in place of everything it held.
    pub fn write_text(text: &str) -> Result<(), String> {
        autoreleasepool(|_| {
            let board = NSPasteboard::generalPasteboard();
            board.clearContents();
            // SAFETY: AppKit's constant type name, valid for the life of the process.
            let kind = unsafe { NSPasteboardTypeString };
            if board.setString_forType(&NSString::from_str(text), kind) {
                Ok(())
            } else {
                Err("macOS didn't take the text onto the clipboard: try again".into())
            }
        })
    }

    /// A TIFF as a PNG, by macOS's own image classes.
    pub fn tiff_png(tiff: &[u8]) -> Result<Vec<u8>, String> {
        let bad = || "the copied image is in a format T3 WoW can't read".to_string();
        autoreleasepool(|_| {
            let rep = NSBitmapImageRep::imageRepWithData(&NSData::with_bytes(tiff)).ok_or_else(bad)?;
            if (rep.pixelsWide() as u64) * (rep.pixelsHigh() as u64) > super::MAX_PIXELS {
                return Err("the copied image is too large".into());
            }
            // SAFETY: no properties, so none of the wrong type.
            let png = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }.ok_or_else(bad)?;
            Ok(png.to_vec())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A CF_DIB of `w` x `h` pixels at `bits` per pixel, rows from the bottom (or the top
    /// with a negative height), each pixel given as RGBA.
    fn dib(w: usize, h: usize, bits: u16, top_down: bool, pixels: &[[u8; 4]]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(w as i32).to_le_bytes());
        out.extend_from_slice(&(if top_down { -(h as i32) } else { h as i32 }).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(&[0u8; 24]);
        let stride = (w * bits as usize).div_ceil(32) * 4;
        for y in 0..h {
            let row = if top_down { y } else { h - 1 - y };
            let mut line = Vec::new();
            for p in &pixels[row * w..row * w + w] {
                line.extend_from_slice(&[p[2], p[1], p[0]]);
                if bits == 32 {
                    line.push(p[3]);
                }
            }
            line.resize(stride, 0);
            out.extend_from_slice(&line);
        }
        out
    }

    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const WHITE: [u8; 4] = [255, 255, 255, 255];

    #[test]
    fn bitmaps_from_the_clipboard_read_right_side_up_whatever_their_layout() {
        let pixels = [RED, GREEN, BLUE, WHITE, RED, GREEN];
        let want: Vec<u8> = pixels.iter().flatten().copied().collect();
        for (bits, top_down) in [(24, false), (24, true), (32, false), (32, true)] {
            let bitmap = dib_bitmap(&dib(3, 2, bits, top_down, &pixels)).unwrap();
            assert_eq!((bitmap.width, bitmap.height), (3, 2));
            assert_eq!(bitmap.rgba, want, "{bits} bits, top down {top_down}");
        }
    }

    #[test]
    fn a_32_bit_bitmap_that_uses_alpha_keeps_it_and_one_that_doesnt_is_opaque() {
        let clear = [[10, 20, 30, 0], [40, 50, 60, 0]];
        assert_eq!(dib_bitmap(&dib(2, 1, 32, false, &clear)).unwrap().rgba, vec![10, 20, 30, 255, 40, 50, 60, 255]);
        let half = [[10, 20, 30, 128], [40, 50, 60, 0]];
        assert_eq!(dib_bitmap(&dib(2, 1, 32, false, &half)).unwrap().rgba, vec![10, 20, 30, 128, 40, 50, 60, 0]);
    }

    #[test]
    fn bitfield_masks_after_the_header_are_read() {
        let mut bytes = dib(1, 1, 32, false, &[[0, 0, 0, 0]]);
        bytes[16] = 3; // BI_BITFIELDS
        let pixel = bytes.split_off(40);
        for mask in [0x00ff_0000u32, 0x0000_ff00, 0x0000_00ff] {
            bytes.extend_from_slice(&mask.to_le_bytes());
        }
        // Blue 0x11, green 0x22, red 0x33.
        bytes.extend_from_slice(&[0x11, 0x22, 0x33, 0]);
        assert_eq!(pixel.len(), 4);
        assert_eq!(dib_bitmap(&bytes).unwrap().rgba, vec![0x33, 0x22, 0x11, 255]);
    }

    #[test]
    fn truncated_or_odd_bitmaps_are_refused_not_read_past_their_end() {
        let full = dib(4, 4, 24, false, &[RED; 16]);
        assert!(dib_bitmap(&full[..full.len() - 1]).is_err());
        assert!(dib_bitmap(&full[..20]).is_err());
        let mut eight_bit = full.clone();
        eight_bit[14] = 8;
        assert!(dib_bitmap(&eight_bit).is_err());
    }

    /// Writes this PC's clipboard, so it runs only when asked (`cargo test -- --ignored`).
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn text_written_to_the_windows_clipboard_reads_back_as_unicode_text() {
        let text = "copied from T3 WoW ✓\n\tline two | pipe";
        write_text(text).unwrap();
        let _open = win::open().unwrap();
        let units = unsafe { win::data(13) }.expect("no text on the clipboard");
        let read: Vec<u16> = units.as_chunks::<2>().0.iter().map(|&b| u16::from_le_bytes(b)).take_while(|&u| u != 0).collect();
        assert_eq!(String::from_utf16(&read).unwrap(), "copied from T3 WoW ✓\r\n\tline two | pipe");
    }

    #[test]
    fn text_copied_on_windows_breaks_its_lines_with_cr_lf() {
        assert_eq!(crlf("one\ntwo\r\nthree\rfour\n"), "one\r\ntwo\r\nthree\r\nfour\r\n");
        assert_eq!(crlf("no breaks"), "no breaks");
    }

    #[test]
    fn a_copied_png_goes_as_it_is_and_pixels_are_encoded() {
        let bitmap = Bitmap { width: 2, height: 1, rgba: vec![1, 2, 3, 4, 5, 6, 7, 8] };
        let kept = fit_upload(Some(b"as copied".to_vec()), bitmap.clone()).unwrap();
        assert_eq!(kept.png, b"as copied");
        let encoded = fit_upload(None, bitmap.clone()).unwrap();
        assert_eq!(decode_png(&encoded.png).unwrap(), bitmap);
    }

    #[test]
    fn thumbnails_keep_the_shape_within_their_box_and_average_pixels() {
        let wide = Bitmap { width: 1920, height: 1080, rgba: vec![200; 1920 * 1080 * 4] };
        let t = thumbnail(&wide);
        assert_eq!((t.width, t.height), (142, 80));
        assert!(t.rgba.iter().all(|&v| v == 200));
        let banner = Bitmap { width: 3000, height: 100, rgba: vec![0; 3000 * 100 * 4] };
        assert_eq!((thumbnail(&banner).width, thumbnail(&banner).height), (240, 8));
        let tiny = Bitmap { width: 2, height: 2, rgba: [RED, BLUE, RED, BLUE].concat() };
        assert_eq!(thumbnail(&tiny), tiny);
        assert_eq!(halved(&tiny).rgba, vec![127, 0, 127, 255]);
    }

    #[test]
    fn previews_fit_a_texture_keeping_the_shape_and_small_images_stay_as_they_are() {
        let wide = Bitmap { width: 1920, height: 1080, rgba: vec![200; 1920 * 1080 * 4] };
        assert_eq!((preview(&wide).width, preview(&wide).height), (1024, 576));
        let tall = Bitmap { width: 400, height: 4096, rgba: vec![0; 400 * 4096 * 4] };
        assert_eq!((preview(&tall).width, preview(&tall).height), (200, 2048));
        let tiny = Bitmap { width: 2, height: 2, rgba: [RED, BLUE, RED, BLUE].concat() };
        assert_eq!(preview(&tiny), tiny);
    }

    fn png_of(bitmap: &Bitmap) -> Vec<u8> {
        encode_png(bitmap).unwrap()
    }

    #[test]
    fn a_macs_png_is_preferred_and_sent_as_it_was_copied() {
        let bitmap = Bitmap { width: 2, height: 1, rgba: [RED, BLUE].concat() };
        let png = png_of(&bitmap);
        let copied = MacCopied { png: Some(png.clone()), tiff: Some(b"a tiff".to_vec()), file: false };
        let clipped = pick(copied, |_| panic!("the TIFF was read with a PNG there")).unwrap().unwrap();
        assert_eq!(clipped.png, png);
        assert_eq!(clipped.bitmap, bitmap);
    }

    #[test]
    fn a_macs_tiff_is_read_when_there_is_no_png_or_it_doesnt_decode() {
        let bitmap = Bitmap { width: 1, height: 2, rgba: [GREEN, WHITE].concat() };
        let converted = png_of(&bitmap);
        for png in [None, Some(b"not a png".to_vec())] {
            let copied = MacCopied { png, tiff: Some(b"a tiff".to_vec()), file: false };
            let clipped = pick(copied, |tiff| {
                assert_eq!(tiff, b"a tiff");
                Ok(converted.clone())
            })
            .unwrap()
            .unwrap();
            assert_eq!(clipped.bitmap, bitmap);
            assert_eq!(clipped.png, converted);
        }
    }

    #[test]
    fn a_file_copied_in_finder_is_no_image_and_neither_is_text() {
        let copied = MacCopied { png: None, tiff: Some(b"the file's icon".to_vec()), file: true };
        assert!(pick(copied, |_| panic!("read the icon")).unwrap().is_none());
        assert!(pick(MacCopied::default(), |_| panic!("read a TIFF that isn't there")).unwrap().is_none());
        // A PNG copied with a file (an app that puts both) still goes.
        let bitmap = Bitmap { width: 1, height: 1, rgba: RED.to_vec() };
        let copied = MacCopied { png: Some(png_of(&bitmap)), tiff: None, file: true };
        assert_eq!(pick(copied, |_| unreachable!()).unwrap().unwrap().bitmap, bitmap);
    }

    #[test]
    fn a_tiff_macos_cant_read_says_so() {
        let copied = MacCopied { png: None, tiff: Some(vec![0; 8]), file: false };
        assert!(pick(copied, |_| Err("the copied image is in a format T3 WoW can't read".into())).is_err());
        let copied = MacCopied { png: None, tiff: Some(vec![0; 8]), file: false };
        assert!(pick(copied, |_| Ok(b"still not a png".to_vec())).is_err());
    }

    /// A CF_HDROP as Explorer puts it on the clipboard: DROPFILES (where the list starts,
    /// a point, two flags: the second, UTF-16), then each path and a 0, and a 0 to end.
    fn drop_list(paths: &[&str], wide: bool) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&20u32.to_le_bytes());
        out.extend_from_slice(&[0; 12]);
        out.extend_from_slice(&u32::from(wide).to_le_bytes());
        for path in paths {
            if wide {
                path.encode_utf16().chain(Some(0)).for_each(|u| out.extend_from_slice(&u.to_le_bytes()));
            } else {
                out.extend(path.bytes().chain(Some(0)));
            }
        }
        out.extend_from_slice(if wide { &[0, 0] } else { &[0] });
        out
    }

    #[test]
    fn files_copied_in_explorer_are_read_from_the_drop_list() {
        let paths = ["C:\\Users\\you\\Pictures\\shot 1.png", "D:\\été.jpg"];
        let want: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        assert_eq!(dropped_files(&drop_list(&paths, true)), want);
        assert_eq!(dropped_files(&drop_list(&["C:\\a.png"], false)), vec![PathBuf::from("C:\\a.png")]);
        // Cut short, or a list that starts past the end: no files, nothing read past it.
        assert!(dropped_files(&[20, 0, 0]).is_empty());
        let mut far = drop_list(&paths, true);
        far[0] = 200;
        assert!(dropped_files(&far).is_empty());
    }

    /// A folder of its own for one test's files.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("t3wow-clip-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn copied_image_files_paste_as_images_and_other_files_are_left_out() {
        let dir = scratch("files");
        let shot = Bitmap { width: 2, height: 1, rgba: [RED, BLUE].concat() };
        let photo = Bitmap { width: 1, height: 1, rgba: GREEN.to_vec() };
        let (png, jpeg, notes) = (dir.join("shot.png"), dir.join("photo.jpg"), dir.join("notes.txt"));
        std::fs::write(&png, png_of(&shot)).unwrap();
        // A JPEG's start, to the frame header that gives its size (1 x 1).
        std::fs::write(&jpeg, [0xff, 0xd8, 0xff, 0xc0, 0, 17, 8, 0, 1, 0, 1, 3]).unwrap();
        std::fs::write(&notes, "not a picture").unwrap();
        let decode = |path: &Path| {
            assert_eq!(path, jpeg, "a PNG went to the system's decoder");
            Ok(photo.clone())
        };
        let images = image_files(&[png.clone(), notes.clone(), jpeg.clone()], decode).unwrap();
        assert_eq!(images.len(), 2);
        // The PNG goes as it is; the JPEG as a PNG of what the decoder read.
        assert_eq!(images[0].png, std::fs::read(&png).unwrap());
        assert_eq!(images[0].bitmap, shot);
        assert_eq!(decode_png(&images[1].png).unwrap(), photo);
        // Only files that aren't images, or are gone: say why, not "no image".
        let error = image_files(std::slice::from_ref(&notes), |_| unreachable!()).err().unwrap();
        assert!(error.contains("isn't an image"), "{error}");
        assert!(image_files(&[dir.join("gone.png")], |_| unreachable!()).is_err());
        // A file the decoder can't read says so.
        let error = image_files(std::slice::from_ref(&jpeg), |_| Err("the copied image is in a format T3 WoW can't read".into())).err().unwrap();
        assert!(error.contains("can't read"), "{error}");
        // No more images than a message takes are read, and other files don't count.
        let many = vec![png.clone(); MAX_FILES + 3];
        assert_eq!(image_files(&many, |_| unreachable!()).unwrap().len(), MAX_FILES);
        let mixed: Vec<PathBuf> = std::iter::repeat_n(notes.clone(), MAX_FILES).chain(std::iter::repeat_n(png.clone(), MAX_FILES)).collect();
        assert_eq!(image_files(&mixed, |_| unreachable!()).unwrap().len(), MAX_FILES);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A small JPEG (4 x 2, all one green), as Windows' own encoder writes it.
    #[cfg(windows)]
    const GREEN_JPEG: &[u8] = include_bytes!("../tests/fixtures/green-4x2.jpg");

    /// GDI+ reads a JPEG, turns it the way an EXIF orientation says, and reads a BMP file.
    #[test]
    #[cfg(windows)]
    fn windows_reads_jpeg_and_bmp_files() {
        let dir = scratch("gdiplus");
        let jpeg = dir.join("green.jpg");
        std::fs::write(&jpeg, GREEN_JPEG).unwrap();
        let bitmap = win::decode_file(&jpeg).unwrap();
        assert_eq!((bitmap.width, bitmap.height), (4, 2));
        for p in bitmap.rgba.as_chunks::<4>().0 {
            assert!(p[0] < 16 && (184..=216).contains(&p[1]) && p[2] < 16 && p[3] == 255, "not green: {p:?}");
        }

        // The same photo with EXIF orientation 6 (taken with the camera turned): 2 x 4.
        let mut exif = vec![0xff, 0xe1, 0, 34];
        exif.extend_from_slice(b"Exif\0\0II*\0\x08\0\0\0\x01\0");
        exif.extend_from_slice(&[0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0]);
        let turned: Vec<u8> = [&GREEN_JPEG[..2], &exif, &GREEN_JPEG[2..]].concat();
        std::fs::write(&jpeg, turned).unwrap();
        let bitmap = win::decode_file(&jpeg).unwrap();
        assert_eq!((bitmap.width, bitmap.height), (2, 4));

        // A BMP file: a file header before the same bitmap the clipboard holds.
        let pixels = [RED, GREEN, BLUE, WHITE, RED, GREEN];
        let body = dib(3, 2, 24, false, &pixels);
        let mut bmp = b"BM".to_vec();
        bmp.extend_from_slice(&(14 + body.len() as u32).to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&body);
        let file = dir.join("pixels.bmp");
        std::fs::write(&file, bmp).unwrap();
        let images = image_files(&[file], win::decode_file).unwrap();
        assert_eq!(decode_png(&images[0].png).unwrap().rgba, pixels.concat());

        // Not an image GDI+ knows: an error, not a crash.
        let odd = dir.join("odd.jpg");
        std::fs::write(&odd, [0xff, 0xd8, 0xff, 0xc0, 0, 17, 8, 0, 1, 0, 1, 3]).unwrap();
        assert!(win::decode_file(&odd).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// macOS turns a real TIFF into a PNG with the same pixels.
    #[test]
    #[cfg(target_os = "macos")]
    fn macos_reads_a_tiff() {
        let tiff = tiff_of(3, 2, &[RED, GREEN, BLUE, WHITE, RED, GREEN]);
        let bitmap = decode_png(&mac::tiff_png(&tiff).unwrap()).unwrap();
        assert_eq!((bitmap.width, bitmap.height), (3, 2));
        assert_eq!(bitmap.rgba, [RED, GREEN, BLUE, WHITE, RED, GREEN].concat());
        assert!(mac::tiff_png(b"not a tiff").is_err());
    }

    /// An uncompressed RGBA TIFF, little-endian, one strip.
    #[cfg(target_os = "macos")]
    fn tiff_of(w: u32, h: u32, pixels: &[[u8; 4]]) -> Vec<u8> {
        let data: Vec<u8> = pixels.iter().flatten().copied().collect();
        let entries: [(u16, u16, u32, u32); 11] = [
            (256, 4, 1, w),                 // width
            (257, 4, 1, h),                 // height
            (258, 3, 4, 0),                 // bits per sample: offset filled below
            (259, 3, 1, 1),                 // no compression
            (262, 3, 1, 2),                 // RGB
            (273, 4, 1, 0),                 // strip offset: filled below
            (277, 3, 1, 4),                 // samples per pixel
            (278, 4, 1, h),                 // rows per strip
            (279, 4, 1, data.len() as u32), // strip byte count
            (284, 3, 1, 1),                 // chunky
            (338, 3, 1, 2),                 // unassociated alpha
        ];
        let ifd_len = 2 + entries.len() * 12 + 4;
        let bits_at = 8 + ifd_len as u32;
        let data_at = bits_at + 8;
        let mut out = b"II*\0".to_vec();
        out.extend_from_slice(&8u32.to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, kind, count, value) in entries {
            let value = match tag {
                258 => bits_at,
                273 => data_at,
                _ => value,
            };
            out.extend_from_slice(&tag.to_le_bytes());
            out.extend_from_slice(&kind.to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
            if kind == 3 && count == 1 {
                out.extend_from_slice(&(value as u16).to_le_bytes());
                out.extend_from_slice(&[0, 0]);
            } else {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        out.extend_from_slice(&0u32.to_le_bytes());
        for _ in 0..4 {
            out.extend_from_slice(&8u16.to_le_bytes());
        }
        out.extend_from_slice(&data);
        out
    }

    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "reads the real clipboard: copy an image of T3WOW_CLIP_SIZE (\"WxH\") first"]
    fn reads_an_image_copied_on_this_mac() {
        let want = std::env::var("T3WOW_CLIP_SIZE").expect("set T3WOW_CLIP_SIZE to the copied image's size, like 120x80");
        let copied = mac::read();
        println!("on the clipboard: PNG {:?} bytes, TIFF {:?} bytes, file {}", copied.png.as_ref().map(Vec::len), copied.tiff.as_ref().map(Vec::len), copied.file);
        let clipped = read_images().unwrap().into_iter().next().expect("no image on the clipboard");
        assert_eq!(format!("{}x{}", clipped.bitmap.width, clipped.bitmap.height), want);
        assert_eq!(decode_png(&clipped.png).unwrap().width, clipped.bitmap.width);
        println!("read {want}: {} bytes of PNG, top-left pixel {:?}", clipped.png.len(), &clipped.bitmap.rgba[..4]);
    }

    #[test]
    #[cfg(windows)]
    #[ignore = "reads the real clipboard: copy an image, or image files in Explorer, of T3WOW_CLIP_SIZE (\"WxH\", several with commas) first"]
    fn reads_an_image_copied_on_this_pc() {
        let want = std::env::var("T3WOW_CLIP_SIZE").expect("set T3WOW_CLIP_SIZE to the copied image's size, like 120x80");
        let images = read_images().unwrap();
        let sizes: Vec<String> = images.iter().map(|c| format!("{}x{}", c.bitmap.width, c.bitmap.height)).collect();
        assert_eq!(sizes.join(","), want, "no image on the clipboard, or not the ones copied");
        for clipped in images {
            assert_eq!(decode_png(&clipped.png).unwrap().width, clipped.bitmap.width);
            println!("read {}x{}: {} bytes of PNG, top-left pixel {:?}", clipped.bitmap.width, clipped.bitmap.height, clipped.png.len(), &clipped.bitmap.rgba[..4]);
        }
    }
}
