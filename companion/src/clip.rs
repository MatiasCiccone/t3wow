//! Images pasted in the game: the game can't read the clipboard, so when you press Ctrl+V
//! in its message box with an image copied, the companion reads it here (a screenshot from
//! Snipping Tool, an image copied in a browser or an editor) and keeps it as a PNG to send
//! with the message, plus a small picture for the game to show above the box.

use crate::render::{decode_png, Bitmap};

/// T3 takes images up to this size.
pub const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;
/// Images larger than this (in pixels) aren't read: no screen copies anything this big.
const MAX_PIXELS: u64 = 64 * 1024 * 1024;
/// The small picture's height, in pixels (the game draws it about half that tall).
pub const THUMB_HEIGHT: u32 = 80;
const THUMB_MAX_WIDTH: u32 = 240;

/// An image read from the clipboard: the PNG to send and the pixels to draw it from.
pub struct Clipped {
    pub png: Vec<u8>,
    pub bitmap: Bitmap,
}

/// The image on the clipboard, None if it holds none.
#[cfg(windows)]
pub fn read_image() -> Result<Option<Clipped>, String> {
    let win::Copied { png, dib } = win::read()?;
    // A PNG as it was copied keeps its transparency and needs no encoding; one that doesn't
    // decode falls back to the bitmap Windows keeps next to it.
    if let Some(png) = png {
        if let Ok(bitmap) = decode_png(&png) {
            return fit_upload(Some(png), bitmap).map(Some);
        }
    }
    match dib {
        Some(dib) => fit_upload(None, dib_bitmap(&dib)?).map(Some),
        None => Ok(None),
    }
}

#[cfg(not(windows))]
pub fn read_image() -> Result<Option<Clipped>, String> {
    Ok(None)
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
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW};
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

    const CF_DIB: u32 = 8;

    /// Closes the clipboard when dropped.
    struct Open;
    impl Drop for Open {
        fn drop(&mut self) {
            unsafe { CloseClipboard() };
        }
    }

    /// The clipboard's data in `format`, copied out.
    unsafe fn data(format: u32) -> Option<Vec<u8>> {
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

    /// What the clipboard holds as a PNG and as a bitmap (neither: no image).
    pub struct Copied {
        pub png: Option<Vec<u8>>,
        pub dib: Option<Vec<u8>>,
    }

    pub fn read() -> Result<Copied, String> {
        unsafe {
            // Another program may hold the clipboard for a moment.
            let mut opened = false;
            for _ in 0..10 {
                if OpenClipboard(null_mut()) != 0 {
                    opened = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            if !opened {
                return Err("another program is using the clipboard: try again".into());
            }
            let _open = Open;
            let name: Vec<u16> = "PNG".encode_utf16().chain(Some(0)).collect();
            let png_format = RegisterClipboardFormatW(name.as_ptr());
            let png = if png_format != 0 { data(png_format) } else { None };
            Ok(Copied { png, dib: data(CF_DIB) })
        }
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
    #[cfg(windows)]
    #[ignore = "reads the real clipboard: copy an image of T3WOW_CLIP_SIZE (\"WxH\") first"]
    fn reads_an_image_copied_on_this_pc() {
        let want = std::env::var("T3WOW_CLIP_SIZE").expect("set T3WOW_CLIP_SIZE to the copied image's size, like 120x80");
        let clipped = read_image().unwrap().expect("no image on the clipboard");
        assert_eq!(format!("{}x{}", clipped.bitmap.width, clipped.bitmap.height), want);
        assert_eq!(decode_png(&clipped.png).unwrap().width, clipped.bitmap.width);
        println!("read {want}: {} bytes of PNG, top-left pixel {:?}", clipped.png.len(), &clipped.bitmap.rgba[..4]);
    }
}
