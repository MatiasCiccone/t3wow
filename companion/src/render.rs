//! Pictures for the game: pages an agent showed with T3's html_render, and images a
//! message embeds from a file on this PC, drawn to bitmaps the game can show.
//!
//! The game can't run a browser, so pages are drawn here, with the headless Microsoft Edge
//! every Windows PC has, on T3's own dark theme (its base sheet and colors), at the width
//! T3 draws them. The game loads them from pre-made image slot files (`slots::image_file`)
//! as uncompressed 32-bit TGA, the format it was seen loading while it runs.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The width T3 draws a page at, in a desktop thread.
pub const PAGE_WIDTH: u32 = 728;
/// The tallest picture sent to the game (a 1024x2048 texture loads; taller ones are scaled
/// down to this).
pub const MAX_HEIGHT: u32 = 2048;
/// html_render takes heights from 80 to 2000.
const PAGE_HEIGHTS: (u32, u32) = (80, 2000);
/// A picture file larger than this isn't read.
const MAX_FILE_BYTES: u64 = 24 * 1024 * 1024;
/// Edge gets this long to draw a page (scripts get a few seconds of their own clock).
const DRAW_TIMEOUT: Duration = Duration::from_secs(25);

/// T3's base sheet for html_render pages (dark appearance), as T3 adds it to every page:
/// its theme variables, the 8px gutter, the page background and the font.
pub const T3_BASE_SHEET: &str = r#":root { color-scheme: dark; --background: #0a0a0a; --foreground: #f5f5f5; --muted: #111111; --muted-foreground: #818181; --card: #111111; --card-foreground: #f5f5f5; --popover: #111111; --popover-foreground: #f5f5f5; --secondary: #111111; --secondary-foreground: #f5f5f5; --border: #191919; --input: #1e1e1e; --ring: #346bf1; --primary: #346bf1; --primary-foreground: #ffffff; --accent: #346bf1; --accent-foreground: #ffffff; --accent-surface: #141414; --accent-surface-foreground: #f5f5f5; --destructive: #fb414a; --destructive-foreground: #ff6467; --destructive-surface: #301214; --warning: #fe9a00; --warning-foreground: #ffb900; --warning-surface: #312108; --success: #10b981; --success-foreground: #34d399; --info: #3b82f6; --info-foreground: #60a5fa; --code-background: #111111; --code-foreground: #f5f5f5; --chart-1: #346bf1; --chart-2: #2dd4bf; --chart-3: #fbbf24; --chart-4: #c084fc; --chart-5: #fb7185; --chart-6: #a3e635; --radius: 0.625rem; --font-sans: "Liberation Sans", Arimo, Arial, Helvetica, sans-serif; --font-mono: "Liberation Mono", Cousine, Menlo, Consolas, monospace; }
html { box-sizing: border-box; padding: 8px; background: var(--background); color: var(--foreground); font-family: var(--font-sans); font-size: 14px; line-height: 1.5; -webkit-font-smoothing: antialiased; text-size-adjust: 100%; scrollbar-width: none; }
html::-webkit-scrollbar { display: none; }
body { margin: 0px; }
code, kbd, pre, samp { font-family: var(--font-mono); }"#;

/// Bumped when pages would come out differently (another base sheet, width, browser
/// flags): pictures drawn before are drawn again.
pub const DRAWING_VERSION: u32 = 1;

/// A drawn picture: `width` x `height` pixels, RGBA, rows from the top.
#[derive(Clone, Debug, PartialEq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// What draws pictures; a trait so tests can stand in for the browser.
pub trait Painter: Send {
    /// A page as T3's html_render shows it, `height` CSS pixels tall.
    fn page(&self, html: &str, height: u32) -> Result<Bitmap, String>;
    /// An image file on this PC, at most PAGE_WIDTH wide.
    fn picture(&self, path: &Path) -> Result<Bitmap, String>;
}

/// The page's height as html_render took it.
pub fn page_height(height: u64) -> u32 {
    (height.min(u32::MAX as u64) as u32).clamp(PAGE_HEIGHTS.0, PAGE_HEIGHTS.1)
}

/// The page with T3's base sheet put first, so the page's own styles win as they do in T3.
pub fn themed(html: &str) -> String {
    let sheet = format!("<style>{T3_BASE_SHEET}</style>");
    let lower = html.to_ascii_lowercase();
    // Right after <head ...>, else after <html ...>, else in front of everything.
    for tag in ["<head", "<html"] {
        if let Some(start) = lower.find(tag) {
            let after = &lower[start + tag.len()..];
            // "<header" isn't "<head".
            if after.starts_with(|c: char| c == '>' || c.is_ascii_whitespace()) {
                if let Some(end) = after.find('>') {
                    let at = start + tag.len() + end + 1;
                    return format!("{}{sheet}{}", &html[..at], &html[at..]);
                }
            }
        }
    }
    format!("{sheet}{html}")
}

/// An image file's kind and size from its first bytes: PNG, JPEG, GIF, BMP or WebP.
pub fn sniff(bytes: &[u8]) -> Option<(&'static str, u32, u32)> {
    let be16 = |i: usize| bytes.get(i..i + 2).map(|b| u16::from_be_bytes([b[0], b[1]]) as u32);
    let le16 = |i: usize| bytes.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as u32);
    let be32 = |i: usize| bytes.get(i..i + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let le32 = |i: usize| bytes.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let le24 = |i: usize| bytes.get(i..i + 3).map(|b| u32::from_le_bytes([b[0], b[1], b[2], 0]));
    let found = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("image/png", be32(16)?, be32(20)?))
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(("image/gif", le16(6)?, le16(8)?))
    } else if bytes.starts_with(b"BM") {
        Some(("image/bmp", le32(18)?, le32(22)?.cast_signed().unsigned_abs()))
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        match bytes.get(12..16)? {
            b"VP8X" => Some(("image/webp", le24(24)? + 1, le24(27)? + 1)),
            b"VP8 " => Some(("image/webp", le16(26)? & 0x3fff, le16(28)? & 0x3fff)),
            b"VP8L" => {
                let bits = le32(21)?;
                Some(("image/webp", (bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
            }
            _ => None,
        }
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        // Walk the segments to the first frame header (SOF0..SOF15, but not DHT, JPG, DAC).
        let mut i = 2;
        loop {
            while bytes.get(i) == Some(&0xff) && bytes.get(i + 1) == Some(&0xff) {
                i += 1;
            }
            if bytes.get(i) != Some(&0xff) {
                break None;
            }
            let marker = *bytes.get(i + 1)?;
            if (0xc0..=0xcf).contains(&marker) && ![0xc4, 0xc8, 0xcc].contains(&marker) {
                break Some(("image/jpeg", be16(i + 7)?, be16(i + 5)?));
            }
            i += 2 + be16(i + 2)? as usize;
        }
    } else {
        None
    };
    found.filter(|(_, w, h)| *w > 0 && *h > 0)
}

/// The size an image is drawn at for the game: as it is, but at most PAGE_WIDTH wide and
/// MAX_HEIGHT tall, keeping its shape.
pub fn fitted(width: u32, height: u32) -> (u32, u32) {
    let scale = (PAGE_WIDTH as f64 / width as f64).min(MAX_HEIGHT as f64 / height as f64).min(1.0);
    (((width as f64 * scale).round() as u32).max(1), ((height as f64 * scale).round() as u32).max(1))
}

/// A PNG as RGBA.
pub fn decode_png(bytes: &[u8]) -> Result<Bitmap, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| format!("unreadable PNG: {e}"))?;
    let mut buffer = vec![0; reader.output_buffer_size().ok_or("PNG too large")?];
    let info = reader.next_frame(&mut buffer).map_err(|e| format!("unreadable PNG: {e}"))?;
    let (width, height) = (info.width, info.height);
    let pixels = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => pixels.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("unexpected indexed PNG".into()),
    };
    if rgba.len() != (width * height * 4) as usize {
        return Err("PNG size mismatch".into());
    }
    Ok(Bitmap { width, height, rgba })
}

/// The bitmap as an uncompressed 32-bit TGA, rows from the top.
pub fn tga(bitmap: &Bitmap) -> Vec<u8> {
    let (w, h) = (bitmap.width as u16, bitmap.height as u16);
    let mut out = Vec::with_capacity(18 + bitmap.rgba.len());
    out.extend_from_slice(&[0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    out.extend_from_slice(&[32, 0x28]);
    out.extend(bitmap.rgba.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]));
    out
}

/// Draws with Microsoft Edge (or Chrome) without a window, in a profile of its own.
pub struct Edge {
    exe: PathBuf,
    work: PathBuf,
}

impl Edge {
    /// Edge where Windows says it is, else Chrome; None without either.
    pub fn find(work: PathBuf) -> Option<Edge> {
        browser_exe().map(|exe| Edge { exe, work })
    }

    fn draw(&self, html: &str, width: u32, height: u32) -> Result<Bitmap, String> {
        std::fs::create_dir_all(&self.work).map_err(|e| format!("no folder to draw in: {e}"))?;
        let page = self.work.join("page.html");
        let shot = self.work.join("page.png");
        let _ = std::fs::remove_file(&shot);
        std::fs::write(&page, html).map_err(|e| format!("couldn't write the page: {e}"))?;
        let mut command = std::process::Command::new(&self.exe);
        command
            .args([
                "--headless=new",
                "--disable-gpu",
                "--hide-scrollbars",
                "--force-device-scale-factor=1",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-extensions",
                "--mute-audio",
                "--virtual-time-budget=4000",
            ])
            .arg(format!("--user-data-dir={}", self.work.join("profile").display()))
            .arg(format!("--window-size={width},{height}"))
            .arg(format!("--screenshot={}", shot.display()))
            .arg(file_url(&page))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = command.spawn().map_err(|e| format!("couldn't start {}: {e}", self.exe.display()))?;
        let deadline = Instant::now() + DRAW_TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("the browser took too long to draw the page".into());
                }
            }
        }
        let png = std::fs::read(&shot).map_err(|_| "the browser drew nothing".to_string())?;
        decode_png(&png)
    }
}

impl Painter for Edge {
    fn page(&self, html: &str, height: u32) -> Result<Bitmap, String> {
        self.draw(&themed(html), PAGE_WIDTH, height)
    }

    fn picture(&self, path: &Path) -> Result<Bitmap, String> {
        let size = std::fs::metadata(path).map_err(|_| "the file isn't on this PC".to_string())?.len();
        if size > MAX_FILE_BYTES {
            return Err("the file is too large".into());
        }
        let bytes = std::fs::read(path).map_err(|e| format!("couldn't read it: {e}"))?;
        let (mime, w, h) = sniff(&bytes).ok_or("not an image the game can be shown (PNG, JPEG, GIF, BMP or WebP)")?;
        let (w, h) = fitted(w, h);
        let html = format!(
            "<!doctype html><html><head><style>html,body{{margin:0;padding:0;background:#0a0a0a}}img{{display:block;width:{w}px;height:{h}px}}</style></head><body><img src=\"data:{mime};base64,{}\"></body></html>",
            STANDARD.encode(&bytes)
        );
        self.draw(&html, w, h)
    }
}

fn file_url(path: &Path) -> String {
    let text = path.display().to_string().replace('\\', "/");
    let encoded: String = text
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-._~/:".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect();
    format!("file:///{}", encoded.trim_start_matches('/'))
}

#[cfg(windows)]
fn browser_exe() -> Option<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    let app_path = |exe: &str| {
        [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER].into_iter().find_map(|root| {
            let key = RegKey::predef(root).open_subkey(format!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{exe}")).ok()?;
            let path: String = key.get_value("").ok()?;
            let path = PathBuf::from(path.trim_matches('"'));
            path.exists().then_some(path)
        })
    };
    let program_files = |var: &str, rest: &str| std::env::var_os(var).map(|p| PathBuf::from(p).join(rest)).filter(|p| p.exists());
    app_path("msedge.exe")
        .or_else(|| program_files("ProgramFiles(x86)", "Microsoft\\Edge\\Application\\msedge.exe"))
        .or_else(|| program_files("ProgramFiles", "Microsoft\\Edge\\Application\\msedge.exe"))
        .or_else(|| app_path("chrome.exe"))
}

#[cfg(not(windows))]
fn browser_exe() -> Option<PathBuf> {
    ["/usr/bin/microsoft-edge", "/usr/bin/google-chrome", "/usr/bin/chromium"].iter().map(PathBuf::from).find(|p| p.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_of(width: u32, height: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(data).unwrap();
        out
    }

    #[test]
    fn pngs_decode_to_rgba_whatever_their_color_type() {
        let rgb = png_of(2, 1, png::ColorType::Rgb, &[10, 20, 30, 40, 50, 60]);
        assert_eq!(decode_png(&rgb).unwrap(), Bitmap { width: 2, height: 1, rgba: vec![10, 20, 30, 255, 40, 50, 60, 255] });
        let gray = png_of(1, 2, png::ColorType::GrayscaleAlpha, &[7, 100, 9, 200]);
        assert_eq!(decode_png(&gray).unwrap().rgba, vec![7, 7, 7, 100, 9, 9, 9, 200]);
        assert!(decode_png(b"not a png").is_err());
    }

    #[test]
    fn a_tga_is_32_bit_bgra_with_rows_from_the_top() {
        let bitmap = Bitmap { width: 2, height: 1, rgba: vec![1, 2, 3, 4, 5, 6, 7, 8] };
        let out = tga(&bitmap);
        assert_eq!(&out[..18], &[0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 0, 32, 0x28]);
        assert_eq!(&out[18..], &[3, 2, 1, 4, 7, 6, 5, 8]);
    }

    #[test]
    fn image_files_are_sized_from_their_first_bytes() {
        assert_eq!(sniff(&png_of(3, 2, png::ColorType::Rgb, &[0; 18])), Some(("image/png", 3, 2)));
        assert_eq!(sniff(b"GIF89a\x40\x01\xf0\x00rest"), Some(("image/gif", 320, 240)));
        // A JPEG: SOI, an APP0 segment to skip, then SOF0 with height 0x01e0 and width 0x0280.
        let jpeg = [0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 17, 8, 0x01, 0xe0, 0x02, 0x80, 3];
        assert_eq!(sniff(&jpeg), Some(("image/jpeg", 640, 480)));
        let mut bmp = b"BM".to_vec();
        bmp.extend_from_slice(&[0; 16]);
        bmp.extend_from_slice(&100u32.to_le_bytes());
        bmp.extend_from_slice(&(-50i32).to_le_bytes());
        assert_eq!(sniff(&bmp), Some(("image/bmp", 100, 50)));
        let mut webp = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
        webp.extend_from_slice(&[0; 8]);
        webp.extend_from_slice(&[0x1f, 0x03, 0x00, 0xc7, 0x00, 0x00]);
        assert_eq!(sniff(&webp), Some(("image/webp", 800, 200)));
        assert_eq!(sniff(b"<svg></svg>"), None);
        assert_eq!(sniff(&[0xff, 0xd8, 0xff]), None);
    }

    #[test]
    fn pictures_fit_the_page_width_and_the_tallest_texture() {
        assert_eq!(fitted(300, 200), (300, 200));
        assert_eq!(fitted(1456, 400), (728, 200));
        assert_eq!(fitted(400, 4096), (200, 2048));
        assert_eq!(page_height(30), 80);
        assert_eq!(page_height(880), 880);
        assert_eq!(page_height(99_999), 2000);
    }

    #[test]
    fn t3s_sheet_goes_first_so_the_pages_own_styles_win() {
        let sheet = format!("<style>{T3_BASE_SHEET}</style>");
        assert_eq!(themed("<html><head><title>x</title></head></html>"), format!("<html><head>{sheet}<title>x</title></head></html>"));
        assert_eq!(themed("<HTML lang=en><body>x</body></HTML>"), format!("<HTML lang=en>{sheet}<body>x</body></HTML>"));
        assert_eq!(themed("<header>x</header>"), format!("{sheet}<header>x</header>"));
        assert_eq!(themed("<p>bare</p>"), format!("{sheet}<p>bare</p>"));
    }

    /// Draws with the real browser: `cargo test -- --ignored` on a PC with Edge.
    #[test]
    #[ignore = "needs Microsoft Edge or Chrome"]
    fn edge_draws_a_page_on_t3s_theme_and_an_image_file() {
        let work = std::env::temp_dir().join(format!("t3wow-edge-{}", std::process::id()));
        let edge = Edge::find(work.clone()).expect("no Edge or Chrome on this PC");
        let page = edge.page("<div style=\"height:40px;background:var(--chart-1)\"></div>", 120).unwrap();
        assert_eq!((page.width, page.height), (PAGE_WIDTH, 120));
        let at = |b: &Bitmap, x: u32, y: u32| b.rgba[((y * b.width + x) * 4) as usize..][..3].to_vec();
        assert_eq!(at(&page, 2, 2), vec![0x0a, 0x0a, 0x0a], "T3's page background in the gutter");
        assert_eq!(at(&page, 100, 20), vec![0x34, 0x6b, 0xf1], "--chart-1 from T3's sheet");
        let png = work.join("red.png");
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, 1000, 500);
        encoder.set_color(png::ColorType::Rgb);
        encoder.write_header().unwrap().write_image_data(&[200, 30, 30].repeat(1000 * 500)).unwrap();
        std::fs::write(&png, bytes).unwrap();
        let picture = edge.picture(&png).unwrap();
        assert_eq!((picture.width, picture.height), (PAGE_WIDTH, 364), "scaled to the page width");
        let red = at(&picture, 300, 150);
        assert!(red[0] > 190 && red[1] < 45 && red[2] < 45, "{red:?}");
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn file_urls_escape_what_a_url_cant_hold() {
        assert_eq!(file_url(Path::new("C:\\Users\\a b\\page.html")), "file:///C:/Users/a%20b/page.html");
    }
}
