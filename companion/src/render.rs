//! Pictures for the game: pages an agent showed with T3's html_render, and images and videos
//! a message embeds from a file on this PC, drawn to bitmaps the game can show.
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
#[cfg(not(target_os = "macos"))]
const NOT_HERE: &str = "the file isn't on this PC";
#[cfg(target_os = "macos")]
const NOT_HERE: &str = "the file isn't on this Mac";
/// Edge gets this long to draw a page (scripts get a few seconds of their own clock).
const DRAW_TIMEOUT: Duration = Duration::from_secs(25);
/// How long the launcher may be gone with no browser on the profile and no picture
/// before the draw counts as failed.
const BROWSER_START_GRACE: Duration = Duration::from_secs(5);

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
    /// A video file on this PC as T3 shows it before it plays: the frame at 0.1 seconds,
    /// at most PAGE_WIDTH wide, with a play mark and its length over it.
    fn poster(&self, path: &Path) -> Result<Bitmap, String>;
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

/// The color of the line the poster page draws under a video once its frame is in: the
/// picture is cut there, to the video's size. Only the page's background is below it, so
/// it's looked for from the bottom up, and a frame with this color in it can't be taken for it.
const POSTER_MARK: [u8; 3] = [0xff, 0x00, 0xfe];

/// A page that shows a video file's frame at 0.1 seconds (where T3 shows it), at most
/// PAGE_WIDTH wide and a row short of MAX_HEIGHT, with a play mark in the middle and its
/// length in the corner. Once the frame is in, a line of POSTER_MARK as wide as the video
/// goes under it; a video the browser can't read never gets one.
pub fn poster_page(video_url: &str) -> String {
    let src = video_url.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;");
    format!(
        r#"<!doctype html><html><head><style>
html,body{{margin:0;padding:0;background:#0a0a0a;overflow:hidden}}
#box{{position:relative;width:{w}px;height:150px;overflow:hidden}}
video{{display:block;width:100%;height:100%;background:#000}}
#play{{position:absolute;left:50%;top:50%;width:56px;height:56px;margin:-28px 0 0 -28px;border-radius:50%;background:rgba(0,0,0,.6);box-shadow:inset 0 0 0 2px rgba(255,255,255,.85)}}
#play:after{{content:"";position:absolute;left:22px;top:16px;border-style:solid;border-width:12px 0 12px 19px;border-color:transparent transparent transparent #fff}}
#time{{position:absolute;right:8px;bottom:8px;padding:0 6px;border-radius:4px;background:rgba(0,0,0,.7);color:#f5f5f5;font:12px/18px "Liberation Sans",Arimo,Arial,sans-serif}}
#time:empty{{display:none}}
#mark{{display:none;height:1px;background:rgb({r},{g},{b})}}
</style></head><body><div id="box"><video id="v" muted preload="auto" src="{src}"></video><div id="play"></div><span id="time"></span></div><div id="mark"></div><script>
const v=document.getElementById("v"),box=document.getElementById("box"),mark=document.getElementById("mark");
const clock=s=>{{s=Math.floor(s);const h=Math.floor(s/3600),m=Math.floor(s/60)%60,r=String(s%60).padStart(2,"0");return h?h+":"+String(m).padStart(2,"0")+":"+r:m+":"+r}};
v.addEventListener("loadedmetadata",()=>{{
  if(!v.videoWidth||!v.videoHeight)return;
  const scale=Math.min({w}/v.videoWidth,{h}/v.videoHeight,1);
  const w=Math.max(1,Math.round(v.videoWidth*scale)),h=Math.max(1,Math.round(v.videoHeight*scale));
  box.style.width=w+"px";box.style.height=h+"px";mark.style.width=w+"px";
  if(Number.isFinite(v.duration)&&v.duration>0){{document.getElementById("time").textContent=clock(v.duration);v.currentTime=Math.min(0.1,v.duration/2)}}
  else v.addEventListener("loadeddata",()=>{{mark.style.display="block"}});
}});
v.addEventListener("seeked",()=>{{mark.style.display="block"}});
</script></body></html>"#,
        w = PAGE_WIDTH,
        h = MAX_HEIGHT - 1,
        r = POSTER_MARK[0],
        g = POSTER_MARK[1],
        b = POSTER_MARK[2],
    )
}

/// The video in a drawn poster page: everything above the mark line, as wide as the line.
/// None when there's no line (the browser couldn't read the video).
pub fn cut_at_mark(shot: &Bitmap) -> Option<Bitmap> {
    if shot.width == 0 {
        return None;
    }
    let near = |x: u32, y: u32| {
        let i = ((y * shot.width + x) * 4) as usize;
        shot.rgba[i..i + 3].iter().zip(POSTER_MARK).all(|(a, b)| a.abs_diff(b) <= 3)
    };
    let y = (1..shot.height).rev().find(|&y| near(0, y) && near(1.min(shot.width - 1), y))?;
    let width = (0..shot.width).take_while(|&x| near(x, y)).count() as u32;
    let row = (shot.width * 4) as usize;
    let rgba = (0..y as usize).flat_map(|r| shot.rgba[r * row..r * row + (width * 4) as usize].iter().copied()).collect();
    Some(Bitmap { width, height: y, rgba })
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
        png::ColorType::Rgb => pixels.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => pixels.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
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
    out.extend(bitmap.rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[2], p[1], p[0], p[3]]));
    out
}

/// Draws with Microsoft Edge (or Chrome) without a window, in a profile of its own.
pub struct Edge {
    exe: PathBuf,
    work: PathBuf,
}

impl Edge {
    /// Edge where Windows says it is, else Chrome; None without either. On a Mac: Chrome,
    /// Edge or Chromium in Applications.
    pub fn find(work: PathBuf) -> Option<Edge> {
        browser_exe().map(|exe| Edge { exe, work })
    }

    #[cfg(target_os = "macos")]
    pub fn exe(&self) -> &Path {
        &self.exe
    }

    fn draw(&self, html: &str, width: u32, height: u32) -> Result<Bitmap, String> {
        std::fs::create_dir_all(&self.work).map_err(|e| format!("no folder to draw in: {e}"))?;
        let page = self.work.join("page.html");
        let shot = self.work.join("page.png");
        let profile = self.work.join("profile");
        let deadline = Instant::now() + DRAW_TIMEOUT;
        // A browser still drawing the last picture holds the profile: one started now
        // would hand it this page and quit without drawing anything.
        while !profile_free(&profile) {
            if Instant::now() >= deadline {
                return Err("the browser is still busy with the last picture".into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
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
            .arg(format!("--user-data-dir={}", profile.display()))
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
        // Without it Chrome asks the Keychain for its "Safe Storage" key, and macOS may show
        // a prompt for it; this profile keeps nothing worth encrypting.
        #[cfg(target_os = "macos")]
        command.arg("--use-mock-keychain");
        // Chrome hard-links its own executable into a temp folder on every launch, to keep its
        // signature valid through an update. macOS counts that as modifying another app and,
        // started from Terminal, blocks it and says "Terminal was prevented from modifying
        // apps". A one-off headless render doesn't need it. Chrome reads only the last
        // --disable-features, so another disabled feature has to join this one.
        #[cfg(target_os = "macos")]
        command.arg("--disable-features=MacAppCodeSignClone");
        let mut child = command.spawn().map_err(|e| format!("couldn't start {}: {e}", self.exe.display()))?;
        // Started from Explorer (the tray app), Edge's launcher hands the page to a browser
        // of its own and quits at once, before anything is drawn. The picture is done when
        // the launcher has quit and no browser holds the profile any more.
        let mut idle_since: Option<Instant> = None;
        loop {
            let launcher_done = !matches!(child.try_wait(), Ok(None));
            if launcher_done && profile_free(&profile) {
                if shot.exists() {
                    break;
                }
                // The browser it handed off to may not have taken the profile yet.
                if idle_since.get_or_insert_with(Instant::now).elapsed() >= BROWSER_START_GRACE {
                    return Err("the browser drew nothing".into());
                }
            } else {
                idle_since = None;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err("the browser took too long to draw the page".into());
            }
            std::thread::sleep(Duration::from_millis(50));
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
        let size = std::fs::metadata(path).map_err(|_| NOT_HERE.to_string())?.len();
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

    fn poster(&self, path: &Path) -> Result<Bitmap, String> {
        if !std::fs::metadata(path).is_ok_and(|m| m.is_file()) {
            return Err(NOT_HERE.into());
        }
        let shot = self.draw(&poster_page(&file_url(path)), PAGE_WIDTH, MAX_HEIGHT)?;
        cut_at_mark(&shot).ok_or_else(|| "the browser couldn't read the video".to_string())
    }
}

/// Whether no browser is running on the profile. A running Edge holds `lockfile` there open
/// without letting it be deleted, and deletes it when it quits; one left by a browser that
/// crashed is deleted here.
#[cfg(not(target_os = "macos"))]
fn profile_free(profile: &Path) -> bool {
    match std::fs::remove_file(profile.join("lockfile")) {
        Ok(()) => true,
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

/// Whether no browser is running on the profile. On a Mac, Chromium (Chrome, Edge) marks a
/// profile it runs on with `SingletonLock`, a symlink to "<host>-<pid>", and removes it when
/// it quits. One whose process is gone was left by a browser that crashed: it is removed here.
/// The profile is T3 WoW's own (`render/profile` in its folder), never the user's browser's.
#[cfg(target_os = "macos")]
fn profile_free(profile: &Path) -> bool {
    let lock = profile.join("SingletonLock");
    let target = match std::fs::read_link(&lock) {
        Ok(target) => target,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return true,
        // Something else by that name: not a browser's lock, but it would stop one starting.
        Err(_) => return std::fs::remove_file(&lock).is_ok(),
    };
    if lock_owner(&target.to_string_lossy()).is_some_and(process_alive) {
        return false;
    }
    match std::fs::remove_file(&lock) {
        Ok(()) => true,
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

/// The browser's process id in a `SingletonLock` target, "<host>-<pid>" (the host name may
/// have dashes of its own).
#[cfg(target_os = "macos")]
fn lock_owner(target: &str) -> Option<i32> {
    let (_, pid) = target.rsplit_once('-')?;
    pid.parse().ok().filter(|&pid| pid > 0)
}

#[cfg(target_os = "macos")]
fn process_alive(pid: i32) -> bool {
    // Signal 0 only asks whether the process exists; EPERM means it does, as someone else's.
    unsafe { libc::kill(pid, 0) == 0 || *libc::__error() == libc::EPERM }
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

#[cfg(not(any(windows, target_os = "macos")))]
fn browser_exe() -> Option<PathBuf> {
    ["/usr/bin/microsoft-edge", "/usr/bin/google-chrome", "/usr/bin/chromium"].iter().map(PathBuf::from).find(|p| p.exists())
}

#[cfg(target_os = "macos")]
fn browser_exe() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    mac_browsers(&home).into_iter().find(|p| p.is_file())
}

/// Where a Mac keeps the browsers that can draw pages headless, in the order they're tried:
/// Chrome, Edge, then Chromium, each in /Applications and then in the user's ~/Applications.
#[cfg(target_os = "macos")]
fn mac_browsers(home: &Path) -> Vec<PathBuf> {
    const APPS: [(&str, &str); 3] = [("Google Chrome", "Google Chrome"), ("Microsoft Edge", "Microsoft Edge"), ("Chromium", "Chromium")];
    let roots = [PathBuf::from("/Applications"), home.join("Applications")];
    APPS.iter()
        .flat_map(|(app, binary)| roots.iter().map(move |root| root.join(format!("{app}.app")).join("Contents").join("MacOS").join(binary)))
        .collect()
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
    fn a_poster_is_cut_to_the_video_above_its_mark_line() {
        // 4x3: two rows of video 3 wide, then the mark under them, then background.
        let (v, m, bg) = ([9, 9, 9, 255], [0xff, 0x01, 0xfd, 255], [10, 10, 10, 255]);
        let rows = [[v, v, v, bg], [v, v, v, bg], [m, m, m, bg]];
        let shot = Bitmap { width: 4, height: 3, rgba: rows.iter().flatten().flatten().copied().collect() };
        assert_eq!(cut_at_mark(&shot), Some(Bitmap { width: 3, height: 2, rgba: [v; 6].concat() }));
        // A frame with the mark's color in it, at its left edge: the real mark is the lowest.
        let rows = [[v, v, v, bg], [m, m, v, bg], [v, v, v, bg], [m, m, m, bg], [bg, bg, bg, bg]];
        let shot = Bitmap { width: 4, height: 5, rgba: rows.iter().flatten().flatten().copied().collect() };
        let cut = cut_at_mark(&shot).unwrap();
        assert_eq!((cut.width, cut.height), (3, 3));
        let unread = Bitmap { width: 4, height: 2, rgba: [bg; 8].concat() };
        assert_eq!(cut_at_mark(&unread), None, "no frame came in");
        assert!(poster_page("file:///C:/a%20b/x.mp4?\"<").contains(r#"src="file:///C:/a%20b/x.mp4?&quot;&lt;""#));
    }

    /// Draws a video's frame with the real browser: `cargo test -- --ignored` on a PC with Edge.
    #[test]
    #[ignore = "needs Microsoft Edge or Chrome"]
    fn edge_draws_a_videos_frame_at_its_own_size_with_a_play_mark() {
        let work = std::env::temp_dir().join(format!("t3wow-edge-video-{}", std::process::id()));
        let edge = Edge::find(work.clone()).expect("no Edge or Chrome on this PC");
        let video = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video.webm");
        let poster = edge.poster(&video).unwrap();
        assert_eq!((poster.width, poster.height), (320, 180), "the video's own size");
        let at = |x: u32, y: u32| poster.rgba[((y * poster.width + x) * 4) as usize..][..3].to_vec();
        let blue = at(4, 4);
        assert!(blue[2] > 180 && blue[0] < 70, "the frame: {blue:?}");
        assert!(at(160 + 2, 90).iter().all(|&c| c > 200), "the play mark's white triangle");
        assert_eq!(edge.poster(&work.join("none.mp4")), Err(NOT_HERE.to_string()));
        let fake = work.join("fake.mp4");
        std::fs::write(&fake, b"not a video").unwrap();
        assert_eq!(edge.poster(&fake), Err("the browser couldn't read the video".to_string()));
        let _ = std::fs::remove_dir_all(&work);
    }

    #[cfg(windows)]
    #[test]
    fn a_profile_is_busy_while_a_browser_holds_its_lockfile() {
        use std::os::windows::fs::OpenOptionsExt;
        let profile = std::env::temp_dir().join(format!("t3wow-lock-{}", std::process::id()));
        std::fs::create_dir_all(&profile).unwrap();
        assert!(profile_free(&profile), "no lockfile");
        let held = std::fs::OpenOptions::new().write(true).create(true).truncate(true).share_mode(0).open(profile.join("lockfile")).unwrap();
        assert!(!profile_free(&profile), "a browser holds it");
        drop(held);
        assert!(profile.join("lockfile").exists());
        assert!(profile_free(&profile), "left by a browser that crashed");
        assert!(!profile.join("lockfile").exists());
        let _ = std::fs::remove_dir_all(&profile);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_mac_looks_for_chrome_edge_and_chromium_in_both_applications_folders() {
        let found = mac_browsers(Path::new("/Users/me"));
        let expected = [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Users/me/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/Users/me/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Users/me/Applications/Chromium.app/Contents/MacOS/Chromium",
        ];
        assert_eq!(found, expected.iter().map(PathBuf::from).collect::<Vec<_>>());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_mac_profile_is_busy_while_its_browser_holds_singleton_lock() {
        use std::os::unix::fs::symlink;
        let profile = std::env::temp_dir().join(format!("t3wow-singleton-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&profile);
        std::fs::create_dir_all(&profile).unwrap();
        let lock = profile.join("SingletonLock");
        assert!(profile_free(&profile), "no lock");
        // A running browser: this test's own process stands in for it.
        symlink(format!("my-mac.local-{}", std::process::id()), &lock).unwrap();
        assert!(!profile_free(&profile), "a running browser holds it");
        assert!(std::fs::symlink_metadata(&lock).is_ok(), "a live browser's lock stays");
        // One that crashed: no such process.
        std::fs::remove_file(&lock).unwrap();
        let mut gone = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let dead = gone.id();
        gone.wait().unwrap();
        symlink(format!("my-mac.local-{dead}"), &lock).unwrap();
        assert!(profile_free(&profile), "left by a browser that crashed");
        assert!(std::fs::symlink_metadata(&lock).is_err(), "the stale lock is removed");
        // Not a lock a browser wrote: removed too.
        symlink("garbage", &lock).unwrap();
        assert!(profile_free(&profile));
        let _ = std::fs::remove_dir_all(&profile);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_singleton_lock_names_its_browser_after_the_last_dash() {
        assert_eq!(lock_owner("my-mac.local-4242"), Some(4242));
        assert_eq!(lock_owner("host-0"), None);
        assert_eq!(lock_owner("no pid"), None);
        assert_eq!(lock_owner("host-12ab"), None);
    }

    #[test]
    fn file_urls_escape_what_a_url_cant_hold() {
        assert_eq!(file_url(Path::new("C:\\Users\\a b\\page.html")), "file:///C:/Users/a%20b/page.html");
    }
}
