//! The files the companion writes in the game's AddOns folder.
//!
//! The game reads an addon's files only when it loads that addon, and finds only files
//! that existed when the client started. So replies travel through a pool of
//! load-on-demand "slot" addons (each loads once per UI session), and "a reply is ready"
//! travels through pre-made font files: a blank one never loads, a real one does.
//! Pictures travel the same way, through pre-made blank image files (`img/NNNN.tga`): the
//! game loads one filled while it runs, as long as it never asked for it while blank, and
//! keeps what it loaded until it quits, so each holds one picture per game process.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const SLOT_COUNT: u32 = 200;
pub const SIGNAL_COUNT: u32 = 4000; // keep in step with addon/T3WoW/Link.lua
pub const ADDON: &str = "T3WoW";
/// Image slot files, each one picture per game process.
pub const IMAGE_COUNT: u32 = 120;
/// A blank file next to them that the game asks for at login: known (it existed when the
/// game started) means the image slots are there too.
pub const IMAGE_SENTINEL: &str = "here.tga";

pub fn image_file(n: u32) -> String {
    format!("{n:04}.tga")
}

pub fn slot_name(n: u32) -> String {
    format!("{ADDON}_S{n:03}")
}

fn signal_file(n: u32) -> String {
    format!("{n:04}.ttf")
}

/// Loads the slot's `Inbox.lua`. Addon managers fingerprint a folder from its toc and the
/// files those list, but follow only self-closing `<Script/>` tags. With this open tag they
/// never reach `Inbox.lua`, so every slot keeps one fixed fingerprint whatever it holds.
const SLOT_XML: &str = "<Ui xmlns=\"http://www.blizzard.com/wow/ui/\">\n\t<Script file=\"Inbox.lua\"></Script>\n</Ui>\n";

/// A slot's toc: the same for every slot, which is why it doesn't say the slot's number.
fn slot_toc(interface: &str) -> String {
    [
        format!("## Interface: {interface}"),
        "## Title: |cff808080T3 WoW slot|r".into(),
        "## Notes: Carries T3 Code updates into the game. Keep enabled.".into(),
        "## LoadOnDemand: 1".into(),
        format!("## Dependencies: {ADDON}"),
        "Slot.xml".into(),
        String::new(),
    ]
    .join("\n")
}

/// The addon, embedded in the companion so the exe is all anyone needs.
pub const ADDON_FILES: &[(&str, &[u8])] = &[
    ("T3WoW.toc", include_bytes!("../../addon/T3WoW/T3WoW.toc")),
    ("Bindings.xml", include_bytes!("../../addon/T3WoW/Bindings.xml")),
    ("Codec.lua", include_bytes!("../../addon/T3WoW/Codec.lua")),
    ("Key.xml", include_bytes!("../../addon/T3WoW/Key.xml")),
    ("Theme.lua", include_bytes!("../../addon/T3WoW/Theme.lua")),
    ("UI.lua", include_bytes!("../../addon/T3WoW/UI.lua")),
    ("Link.lua", include_bytes!("../../addon/T3WoW/Link.lua")),
    ("T3WoW.lua", include_bytes!("../../addon/T3WoW/T3WoW.lua")),
    ("Media/logo.tga", include_bytes!("../../addon/T3WoW/Media/logo.tga")),
    ("Media/round.tga", include_bytes!("../../addon/T3WoW/Media/round.tga")),
    ("Media/ring.tga", include_bytes!("../../addon/T3WoW/Media/ring.tga")),
    ("Media/classic.tga", include_bytes!("../../addon/T3WoW/Media/classic.tga")),
    ("Media/classic-ring.tga", include_bytes!("../../addon/T3WoW/Media/classic-ring.tga")),
    ("Media/close.tga", include_bytes!("../../addon/T3WoW/Media/close.tga")),
    ("Media/chevron.tga", include_bytes!("../../addon/T3WoW/Media/chevron.tga")),
    ("Media/check.tga", include_bytes!("../../addon/T3WoW/Media/check.tga")),
    ("Media/grip.tga", include_bytes!("../../addon/T3WoW/Media/grip.tga")),
    ("Media/search.tga", include_bytes!("../../addon/T3WoW/Media/search.tga")),
    ("Media/clock.tga", include_bytes!("../../addon/T3WoW/Media/clock.tga")),
    ("Media/alarm.tga", include_bytes!("../../addon/T3WoW/Media/alarm.tga")),
    ("Media/question.tga", include_bytes!("../../addon/T3WoW/Media/question.tga")),
    ("Media/refresh.tga", include_bytes!("../../addon/T3WoW/Media/refresh.tga")),
    ("Media/compose.tga", include_bytes!("../../addon/T3WoW/Media/compose.tga")),
    ("Media/gear.tga", include_bytes!("../../addon/T3WoW/Media/gear.tga")),
    ("Media/spinner.tga", include_bytes!("../../addon/T3WoW/Media/spinner.tga")),
    ("Media/done.tga", include_bytes!("../../addon/T3WoW/Media/done.tga")),
    ("Media/shield.tga", include_bytes!("../../addon/T3WoW/Media/shield.tga")),
    ("Media/lock.tga", include_bytes!("../../addon/T3WoW/Media/lock.tga")),
    ("Media/pencil.tga", include_bytes!("../../addon/T3WoW/Media/pencil.tga")),
    ("Media/sparkle.tga", include_bytes!("../../addon/T3WoW/Media/sparkle.tga")),
    ("Media/shield-off.tga", include_bytes!("../../addon/T3WoW/Media/shield-off.tga")),
    ("Media/image-off.tga", include_bytes!("../../addon/T3WoW/Media/image-off.tga")),
    ("Media/message.tga", include_bytes!("../../addon/T3WoW/Media/message.tga")),
    ("Media/hammer.tga", include_bytes!("../../addon/T3WoW/Media/hammer.tga")),
    ("Media/terminal.tga", include_bytes!("../../addon/T3WoW/Media/terminal.tga")),
    ("Media/brain.tga", include_bytes!("../../addon/T3WoW/Media/brain.tga")),
    ("Media/eye.tga", include_bytes!("../../addon/T3WoW/Media/eye.tga")),
    ("Media/globe.tga", include_bytes!("../../addon/T3WoW/Media/globe.tga")),
    ("Media/notification-completion.mp3", include_bytes!("../../addon/T3WoW/Media/notification-completion.mp3")),
    ("Media/notification-input.mp3", include_bytes!("../../addon/T3WoW/Media/notification-input.mp3")),
    ("Media/Sounds.txt", include_bytes!("../../addon/T3WoW/Media/Sounds.txt")),
    ("Media/Inter-Regular.ttf", include_bytes!("../../addon/T3WoW/Media/Inter-Regular.ttf")),
    ("Media/Inter-SemiBold.ttf", include_bytes!("../../addon/T3WoW/Media/Inter-SemiBold.ttf")),
    ("Media/Inter-OFL.txt", include_bytes!("../../addon/T3WoW/Media/Inter-OFL.txt")),
    ("Media/JetBrainsMono-Regular.ttf", include_bytes!("../../addon/T3WoW/Media/JetBrainsMono-Regular.ttf")),
    ("Media/JetBrainsMono-OFL.txt", include_bytes!("../../addon/T3WoW/Media/JetBrainsMono-OFL.txt")),
    ("LICENSE", include_bytes!("../../addon/T3WoW/LICENSE")),
];

pub fn parse_version(toc: &str) -> Option<String> {
    toc.lines()
        .find_map(|line| line.strip_prefix("## Version:"))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// The `## Version:` of the addon this companion ships: the companion's own version, so a
/// test build's addon names its commit too ("0.9.1-dev+57ee2ec").
pub fn shipped_version() -> String {
    crate::VERSION.to_string()
}

/// Orders versions by their numbers, part by part. A build label ("-dev+57ee2ec") doesn't
/// count: a test build of 0.9.1 is no older or newer than 0.9.1 itself.
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |v: &str| v.split(['-', '+']).next().unwrap_or_default().split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (pa, pb) = (parse(a), parse(b));
    for i in 0..pa.len().max(pb.len()) {
        let order = pa.get(i).unwrap_or(&0).cmp(pb.get(i).unwrap_or(&0));
        if order.is_ne() {
            return order;
        }
    }
    std::cmp::Ordering::Equal
}

/// Written next to the addon's files with this install's key (Key.xml loads it): the addon
/// signs its frames with it, so the companion can tell them from another addon's strip.
/// Not shipped, so each install has its own.
pub const KEY_FILE: &str = "Key.lua";

fn key_lua(key: &str) -> String {
    [
        "-- Written by the T3 WoW companion: the key this addon signs its commands with, so the",
        "-- companion can tell them from another addon's. Don't share it.",
        "local _, ns = ...",
        &format!("ns.key = \"{key}\""),
        "",
    ]
    .join("\n")
}

/// A 2x2 white 32-bit TGA, the smallest texture the game is sure to accept.
fn tiny_tga() -> Vec<u8> {
    let mut out = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, 32, 0x28];
    out.extend_from_slice(&[0xff; 16]);
    out
}

/// A Windows text font, or None. Only copied on this PC, never shipped. Symbol fonts
/// (Marlett and the like) don't load in the game, so these are plain text fonts.
#[cfg(not(target_os = "macos"))]
fn windows_font(names: &[&str]) -> Option<Vec<u8>> {
    let dir = PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into())).join("Fonts");
    names.iter().find_map(|name| fs::read(dir.join(name)).ok())
}

/// Every signal is a copy of this font (Lucida Console is about 110 KB), so small wins.
#[cfg(not(target_os = "macos"))]
fn small_font() -> Option<Vec<u8>> {
    windows_font(&["lucon.ttf", "verdana.ttf", "arial.ttf"])
}

/// A font the addon ships. macOS keeps most text fonts in .ttc collections, and the game's
/// own folder has only .slug files, neither of which the game can load through a signal: so
/// signals copy a real single-face TrueType that the game already loads for the addon's text.
#[cfg(target_os = "macos")]
fn addon_font(name: &str) -> Option<Vec<u8>> {
    ADDON_FILES.iter().find(|(file, _)| *file == name).map(|(_, bytes)| bytes.to_vec())
}

pub struct Repair {
    /// "installed 0.3.0" / "updated 0.2.7 to 0.3.0" when the addon itself was written.
    pub addon: Option<String>,
    /// Files that did not exist before (the game only sees them after a full restart).
    pub created: u32,
    /// Slot tocs rewritten from an older layout (these too need a full restart).
    pub slots_moved: u32,
}

pub struct Slots {
    pub addons_dir: PathBuf,
    pub addon_dir: PathBuf,
    interface: String,
    font: Option<Vec<u8>>,
    /// A second font for the probe's valid2.ttf.
    alt_font: Option<Vec<u8>>,
    /// The key for Key.lua, as 64 hex digits (None: Key.lua is left as it is).
    key: Option<String>,
}

impl Slots {
    #[cfg(not(target_os = "macos"))]
    pub fn new(addons_dir: impl Into<PathBuf>, interface: &str) -> Slots {
        Slots::with_fonts(addons_dir, interface, small_font(), windows_font(&["arial.ttf", "segoeui.ttf"]))
    }

    /// JetBrains Mono is the smallest font the addon ships (135 KB): a signal is a copy of it.
    #[cfg(target_os = "macos")]
    pub fn new(addons_dir: impl Into<PathBuf>, interface: &str) -> Slots {
        Slots::with_fonts(addons_dir, interface, addon_font("Media/JetBrainsMono-Regular.ttf"), addon_font("Media/Inter-Regular.ttf"))
    }

    /// With the given fonts instead of this PC's (tests use fixture bytes, so they run anywhere).
    fn with_fonts(addons_dir: impl Into<PathBuf>, interface: &str, font: Option<Vec<u8>>, alt_font: Option<Vec<u8>>) -> Slots {
        let addons_dir = addons_dir.into();
        Slots { addon_dir: addons_dir.join(ADDON), addons_dir, interface: interface.to_string(), font, alt_font, key: None }
    }

    /// With the key the addon signs its frames with (`Config::game_key`), which installs and
    /// repairs write as Key.lua.
    pub fn with_key(mut self, key: Option<String>) -> Slots {
        self.key = key;
        self
    }

    /// For other modules' tests: they run anywhere, with or without Windows' fonts.
    #[cfg(test)]
    pub fn with_fixture_fonts(addons_dir: impl Into<PathBuf>, interface: &str) -> Slots {
        Slots::with_fonts(addons_dir, interface, Some(b"fixture font bytes".to_vec()), Some(b"fixture font bytes".to_vec()))
    }

    pub fn for_wow(wow_dir: &Path, interface: &str) -> Slots {
        Slots::new(wow_dir.join("Interface").join("AddOns"), interface)
    }

    pub fn installed(&self) -> bool {
        self.addon_dir.join(format!("{ADDON}.toc")).exists() && self.addons_dir.join(slot_name(SLOT_COUNT)).exists()
    }

    pub fn installed_version(&self) -> Option<String> {
        fs::read_to_string(self.addon_dir.join(format!("{ADDON}.toc"))).ok().and_then(|toc| parse_version(&toc))
    }

    fn need_font(&self) -> io::Result<()> {
        match self.font {
            Some(_) => Ok(()),
            None => Err(io::Error::other("No usable font in the Windows Fonts folder (looked for lucon, verdana, arial).")),
        }
    }

    /// Writes the shipped addon files; the toc gets this game's interface version.
    pub fn write_addon(&self) -> io::Result<()> {
        self.write_addon_files(false).map(|_| ())
    }

    /// Writes the shipped addon files, or with `only_missing` just the ones not there.
    /// Returns the names written.
    fn write_addon_files(&self, only_missing: bool) -> io::Result<Vec<&'static str>> {
        let mut written = Vec::new();
        for (name, data) in ADDON_FILES {
            let path = self.addon_dir.join(name);
            if only_missing && path.exists() {
                continue;
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            if *name == "T3WoW.toc" {
                let toc = String::from_utf8_lossy(data)
                    .lines()
                    .map(|line| {
                        if line.starts_with("## Interface:") {
                            format!("## Interface: {}", self.interface)
                        } else if line.starts_with("## Version:") {
                            format!("## Version: {}", shipped_version())
                        } else {
                            line.to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\r\n");
                fs::write(path, toc + "\r\n")?;
            } else {
                fs::write(path, data)?;
            }
            written.push(*name);
        }
        Ok(written)
    }

    /// Points the addon and its slots at this game's interface version in place: only the
    /// `## Interface:` lines change, so slots already written, the signals, the sign-in and
    /// the addon's saved settings all stay. Returns how many files changed.
    pub fn set_interface(&self) -> io::Result<u32> {
        let slot_tocs = (1..=SLOT_COUNT).map(|n| self.addons_dir.join(slot_name(n)).join(format!("{}.toc", slot_name(n))));
        let mut changed = 0;
        for toc in std::iter::once(self.addon_dir.join(format!("{ADDON}.toc"))).chain(slot_tocs) {
            let Ok(text) = fs::read_to_string(&toc) else { continue };
            let updated: String = text
                .split_inclusive('\n')
                .map(|line| match line.strip_prefix("## Interface:") {
                    Some(rest) => format!("## Interface: {}{}", self.interface, &rest[rest.trim_end_matches(['\r', '\n']).len()..]),
                    None => line.to_string(),
                })
                .collect();
            if updated != text {
                fs::write(&toc, updated)?;
                changed += 1;
            }
        }
        Ok(changed)
    }

    /// Creates the slot addons, blank signal files and probe files. With `reset`, existing
    /// ones are blanked too (a fresh install); without, only missing ones are created and
    /// slot tocs from an older layout are brought up to date.
    /// Returns how many files did not exist before and how many slot tocs were brought up to date.
    pub fn write_support(&self, reset: bool) -> io::Result<(u32, u32)> {
        let mut created = 0;
        let mut put = |path: PathBuf, data: &[u8]| -> io::Result<()> {
            let exists = path.exists();
            if !exists {
                created += 1;
            }
            if reset || !exists {
                fs::write(path, data)?;
            }
            Ok(())
        };
        let sig = self.addon_dir.join("sig");
        let probe = self.addon_dir.join("ctl").join("probe");
        if reset {
            // Older installs used .wav signals; start those folders clean.
            let _ = fs::remove_dir_all(&sig);
            for old in ["empty.wav", "valid.wav"] {
                let _ = fs::remove_file(self.addon_dir.join("ctl").join(old));
            }
        }
        let img = self.addon_dir.join("img");
        fs::create_dir_all(&sig)?;
        fs::create_dir_all(&probe)?;
        fs::create_dir_all(&img)?;
        for n in 1..=SIGNAL_COUNT {
            put(sig.join(signal_file(n)), b"")?;
        }
        for n in 1..=IMAGE_COUNT {
            put(img.join(image_file(n)), b"")?;
        }
        put(img.join(IMAGE_SENTINEL), b"")?;
        let font = self.font.clone().unwrap_or_default();
        put(probe.join("empty.ttf"), b"")?;
        put(probe.join("valid.ttf"), &font)?;
        put(probe.join("flip.ttf"), b"")?;
        put(probe.join("empty.tga"), b"")?;
        put(probe.join("valid.tga"), &tiny_tga())?;
        put(probe.join("flip.tga"), b"")?;
        put(probe.join("valid2.ttf"), &self.alt_font.clone().unwrap_or_default())?;
        let mut moved = 0;
        for n in 1..=SLOT_COUNT {
            let dir = self.addons_dir.join(slot_name(n));
            fs::create_dir_all(&dir)?;
            put(dir.join("Inbox.lua"), b"")?;
            // Before the toc, so a write that fails can't leave a toc listing a file that isn't there.
            put(dir.join("Slot.xml"), SLOT_XML.as_bytes())?;
            let toc = dir.join(format!("{}.toc", slot_name(n)));
            put(toc.clone(), slot_toc(&self.interface).as_bytes())?;
            if self.bring_slot_toc_up_to_date(&toc)? {
                moved += 1;
            }
        }
        // The key, written again when it isn't this one (a key made anew): an existing file
        // is read again on /reload, a new one only once WoW restarts.
        if let Some(key) = &self.key {
            let file = self.addon_dir.join(KEY_FILE);
            let wanted = key_lua(key);
            if fs::read_to_string(&file).ok().as_deref() != Some(wanted.as_str()) {
                if !file.exists() {
                    created += 1;
                }
                write_whole(&file, wanted.as_bytes())?;
            }
        }
        Ok((created, moved))
    }

    /// Rewrites a slot's toc from an older layout, keeping the interface version it has.
    /// True when it changed.
    fn bring_slot_toc_up_to_date(&self, toc: &Path) -> io::Result<bool> {
        let Ok(text) = fs::read_to_string(toc) else { return Ok(false) };
        let interface = text.lines().find_map(|line| line.strip_prefix("## Interface:")).map_or(self.interface.as_str(), str::trim);
        let current = slot_toc(interface);
        if text == current {
            return Ok(false);
        }
        write_whole(toc, current.as_bytes())?;
        Ok(true)
    }

    pub fn install(&self) -> io::Result<()> {
        self.need_font()?;
        fs::create_dir_all(&self.addons_dir)?;
        self.write_addon()?;
        self.write_support(true)?;
        Ok(())
    }

    /// For every companion start: puts back whatever an addon manager or a cleanup removed,
    /// and installs or updates the addon when this companion ships a newer one. A newer
    /// addon (from CurseForge, say) is left as it is: never mixed with older files. Files
    /// the game didn't have when it started only show up after a full restart of WoW.
    pub fn repair(&self) -> io::Result<Repair> {
        self.need_font()?;
        fs::create_dir_all(&self.addons_dir)?;
        let shipped = shipped_version();
        let mut created = 0;
        let addon = match self.installed_version() {
            None => {
                self.write_addon()?;
                Some(format!("installed {shipped}"))
            }
            Some(installed) => match compare_versions(&shipped, &installed) {
                std::cmp::Ordering::Greater => {
                    self.write_addon()?;
                    Some(format!("updated {installed} to {shipped}"))
                }
                // The same release, another build (a test build, or back from one): this
                // companion's files, so the game runs the addon made for it.
                std::cmp::Ordering::Equal if installed != shipped => {
                    self.write_addon()?;
                    Some(format!("replaced {installed} with {shipped}"))
                }
                std::cmp::Ordering::Equal => {
                    let restored = self.write_addon_files(true)?;
                    created += restored.len() as u32;
                    (!restored.is_empty()).then(|| format!("{shipped} repaired (put back {})", restored.join(", ")))
                }
                std::cmp::Ordering::Less => None,
            },
        };
        let (made, slots_moved) = self.write_support(false)?;
        created += made;
        Ok(Repair { addon, created, slots_moved })
    }

    /// Whether the addon's toc or the files the companion makes (signal, probe and image
    /// files, slot addons) are gone: something replaced or cleaned the folder. Looks at a
    /// few of them only, so it can run often; `repair` puts back the rest.
    pub fn missing_files(&self) -> bool {
        let sig = self.addon_dir.join("sig");
        let img = self.addon_dir.join("img");
        let toc_file = |n: u32| self.addons_dir.join(slot_name(n)).join(format!("{}.toc", slot_name(n)));
        let xml_file = |n: u32| self.addons_dir.join(slot_name(n)).join("Slot.xml");
        let key = self.key.as_ref().map(|_| self.addon_dir.join(KEY_FILE));
        [
            self.addon_dir.join(format!("{ADDON}.toc")),
            sig.join(signal_file(1)),
            sig.join(signal_file(SIGNAL_COUNT)),
            self.addon_dir.join("ctl").join("probe").join("valid.ttf"),
            img.join(IMAGE_SENTINEL),
            img.join(image_file(IMAGE_COUNT)),
            toc_file(1),
            toc_file(SLOT_COUNT),
            xml_file(1),
            xml_file(SLOT_COUNT),
        ]
        .iter()
        .chain(key.iter())
        .any(|path| !path.exists())
    }

    pub fn uninstall(&self) -> io::Result<()> {
        let _ = fs::remove_dir_all(&self.addon_dir);
        for entry in fs::read_dir(&self.addons_dir)?.flatten() {
            if entry.file_name().to_string_lossy().starts_with(&format!("{ADDON}_S")) {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
        Ok(())
    }
}

/// What the bridge does with the files; a trait so tests can watch it.
pub trait SlotWriter: Send {
    fn write_slot(&mut self, n: u32, lua: &str) -> io::Result<()>;
    fn raise_signal(&mut self, n: u32) -> io::Result<()>;
    fn clear_signals(&mut self);
    /// Blanks every slot's update: what they hold was written for a T3 login that is gone.
    fn clear_slots(&mut self);
    /// Makes the probe's flip files real (or blank again); returns the types changed.
    fn flip_probe(&mut self, real: bool) -> Vec<String>;
    /// Puts a picture (a TGA) in image slot `n`.
    fn write_image(&mut self, n: u32, tga: &[u8]) -> io::Result<()>;
    /// Blanks image slot `n`, to give its disk space back.
    fn blank_image(&mut self, n: u32);
    /// Whether image slot `n` holds a picture (an empty or missing file holds none).
    fn has_image(&self, n: u32) -> bool;
    /// The `## Version:` of the addon on disk, if known: the game compares it with the one
    /// it loaded, and says to restart WoW when the one on disk is newer.
    fn addon_version(&self) -> Option<String> {
        None
    }
}

/// Writes through a temporary file, so the game never reads a half-written one.
fn write_whole(file: &Path, data: &[u8]) -> io::Result<()> {
    let mut tmp = file.as_os_str().to_owned();
    tmp.push(".tmp");
    fs::write(&tmp, data)?;
    fs::rename(&tmp, file).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

impl SlotWriter for Slots {
    /// What is in the AddOns folder now: the shipped addon after an install or repair, or
    /// a newer one (from CurseForge, say) that `repair` left alone.
    fn addon_version(&self) -> Option<String> {
        self.installed_version()
    }

    fn write_slot(&mut self, n: u32, lua: &str) -> io::Result<()> {
        if !(1..=SLOT_COUNT).contains(&n) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("no slot {n}")));
        }
        write_whole(&self.addons_dir.join(slot_name(n)).join("Inbox.lua"), lua.as_bytes())
    }

    fn raise_signal(&mut self, n: u32) -> io::Result<()> {
        if !(1..=SIGNAL_COUNT).contains(&n) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("no signal file {n}")));
        }
        let font = self.font.as_ref().ok_or_else(|| io::Error::other("no font to raise a signal with"))?;
        write_whole(&self.addon_dir.join("sig").join(signal_file(n)), font)
    }

    fn clear_signals(&mut self) {
        let dir = self.addon_dir.join("sig");
        for n in 1..=SIGNAL_COUNT {
            let file = dir.join(signal_file(n));
            if fs::metadata(&file).map(|m| m.len() > 0).unwrap_or(false) {
                let _ = fs::write(&file, b"");
            }
        }
    }

    fn clear_slots(&mut self) {
        for n in 1..=SLOT_COUNT {
            let file = self.addons_dir.join(slot_name(n)).join("Inbox.lua");
            if fs::metadata(&file).map(|m| m.len() > 0).unwrap_or(false) {
                let _ = write_whole(&file, b"");
            }
        }
    }

    fn write_image(&mut self, n: u32, tga: &[u8]) -> io::Result<()> {
        if !(1..=IMAGE_COUNT).contains(&n) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("no image slot {n}")));
        }
        write_whole(&self.addon_dir.join("img").join(image_file(n)), tga)
    }

    fn has_image(&self, n: u32) -> bool {
        fs::metadata(self.addon_dir.join("img").join(image_file(n))).is_ok_and(|m| m.len() > 0)
    }

    fn blank_image(&mut self, n: u32) {
        let file = self.addon_dir.join("img").join(image_file(n));
        if fs::metadata(&file).map(|m| m.len() > 0).unwrap_or(false) {
            let _ = write_whole(&file, b"");
        }
    }

    fn flip_probe(&mut self, real: bool) -> Vec<String> {
        let mut changed = Vec::new();
        for (ext, make) in [("ttf", self.font.clone().unwrap_or_default()), ("tga", tiny_tga())] {
            let file = self.addon_dir.join("ctl").join("probe").join(format!("flip.{ext}"));
            if file.exists() && fs::write(&file, if real { make } else { Vec::new() }).is_ok() {
                changed.push(ext.to_string());
            }
        }
        changed
    }
}

/// Turns an install's slots back into what companions before `Slot.xml` wrote.
#[cfg(test)]
pub fn make_slots_old(addons_dir: &Path, interface: &str) {
    for n in 1..=SLOT_COUNT {
        let dir = addons_dir.join(slot_name(n));
        let _ = fs::remove_file(dir.join("Slot.xml"));
        let toc = format!(
            "## Interface: {interface}\n## Title: |cff808080T3 WoW slot {n:03}|r\n## Notes: Carries T3 Code updates into the game. Keep enabled.\n## LoadOnDemand: 1\n## Dependencies: {ADDON}\nInbox.lua\n"
        );
        fs::write(dir.join(format!("{}.toc", slot_name(n))), toc).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stands in for a Windows font: the companion only copies font bytes, never reads them.
    const FIXTURE_FONT: &[u8] = b"fixture font bytes";

    /// A file added to the addon but not to ADDON_FILES would be missing from every install.
    #[test]
    fn every_addon_file_is_embedded() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../addon/T3WoW");
        let mut on_disk = Vec::new();
        let mut dirs = vec![root.clone()];
        while let Some(dir) = dirs.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                } else {
                    on_disk.push(path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/"));
                }
            }
        }
        on_disk.sort();
        let mut embedded: Vec<String> = ADDON_FILES.iter().map(|(name, _)| name.to_string()).collect();
        embedded.sort();
        assert_eq!(on_disk, embedded);
    }

    fn fixture_slots(dir: &Path, interface: &str) -> Slots {
        Slots::with_fonts(dir, interface, Some(FIXTURE_FONT.to_vec()), Some(FIXTURE_FONT.to_vec()))
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("t3wow-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn repair_puts_back_missing_files_and_updates_an_older_addon() {
        let dir = temp_dir("repair");
        let slots = fixture_slots(&dir, "16001");
        let first = slots.repair().unwrap();
        assert!(first.addon.unwrap().starts_with("installed "));
        assert!(first.created > SIGNAL_COUNT);
        assert!(slots.installed());
        assert!(dir.join("T3WoW/Media/logo.tga").exists());
        assert_eq!(fs::read(dir.join("T3WoW/ctl/probe/valid.ttf")).unwrap(), FIXTURE_FONT);

        // An addon manager removed a slot and a signal file; nothing else changed.
        fs::remove_dir_all(dir.join(slot_name(7))).unwrap();
        fs::remove_file(dir.join("T3WoW/sig/0042.ttf")).unwrap();
        let second = slots.repair().unwrap();
        assert_eq!(second.addon, None);
        assert_eq!(second.created, 4);
        assert!(dir.join(slot_name(7)).join("Inbox.lua").exists());
        assert!(dir.join(slot_name(7)).join("Slot.xml").exists());

        // An older addon gets the shipped one, with this game's interface version.
        let toc = dir.join("T3WoW/T3WoW.toc");
        let old = fs::read_to_string(&toc).unwrap().replace(&format!("## Version: {}", shipped_version()), "## Version: 0.0.1");
        fs::write(&toc, old).unwrap();
        assert!(slots.repair().unwrap().addon.unwrap().starts_with("updated 0.0.1 to "));
        // The game is told the version it would load now.
        assert_eq!(slots.addon_version(), Some(shipped_version()));
        assert!(fs::read_to_string(&toc).unwrap().contains("## Interface: 16001"));
        let _ = fs::remove_dir_all(&dir);
    }

    fn slot_file(dir: &Path, n: u32, file: &str) -> PathBuf {
        dir.join(slot_name(n)).join(file)
    }

    fn slot_toc_file(dir: &Path, n: u32) -> PathBuf {
        slot_file(dir, n, &format!("{}.toc", slot_name(n)))
    }

    #[test]
    fn every_slot_looks_the_same_to_an_addon_manager_whatever_its_update_holds() {
        let dir = temp_dir("fingerprint");
        let mut slots = fixture_slots(&dir, "16001");
        slots.install().unwrap();
        let toc = fs::read(slot_toc_file(&dir, 1)).unwrap();
        let xml = fs::read(slot_file(&dir, 1, "Slot.xml")).unwrap();
        let toc_text = String::from_utf8_lossy(&toc).into_owned();
        assert!(toc_text.lines().any(|line| line == "Slot.xml"), "{toc_text}");
        assert!(!toc_text.contains("Inbox.lua"), "a listed Inbox.lua changes the fingerprint with every update: {toc_text}");
        assert!(toc_text.contains("## Dependencies: T3WoW"), "{toc_text}");
        assert!(toc_text.contains("## LoadOnDemand: 1"), "{toc_text}");

        // Scanners follow only self-closing <Script/> tags, so this one has to stay open.
        let xml_text = String::from_utf8_lossy(&xml).into_owned();
        assert!(xml_text.contains(r#"<Script file="Inbox.lua"></Script>"#), "{xml_text}");
        assert!(!xml_text.contains("/>"), "{xml_text}");

        for n in 2..=SLOT_COUNT {
            assert_eq!(fs::read(slot_toc_file(&dir, n)).unwrap(), toc, "slot {n}'s toc");
            assert_eq!(fs::read(slot_file(&dir, n, "Slot.xml")).unwrap(), xml, "slot {n}'s Slot.xml");
        }

        // An update rewrites Inbox.lua and nothing the scanners read.
        slots.write_slot(9, "T3WoW_Inbox = { lid = \"new\" }").unwrap();
        assert_eq!(fs::read(slot_toc_file(&dir, 9)).unwrap(), toc);
        assert_eq!(fs::read(slot_file(&dir, 9, "Slot.xml")).unwrap(), xml);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_repair_moves_an_old_install_to_slot_xml_and_keeps_its_updates_and_interface() {
        let dir = temp_dir("slot-xml-upgrade");
        let slots = fixture_slots(&dir, "16001");
        slots.install().unwrap();
        make_slots_old(&dir, "16002");
        let inbox = slot_file(&dir, 5, "Inbox.lua");
        fs::write(&inbox, "T3WoW_Inbox = { slot = 5 }").unwrap();

        let repair = slots.repair().unwrap();
        assert_eq!((repair.created, repair.slots_moved), (SLOT_COUNT, SLOT_COUNT), "one Slot.xml and one toc per slot");
        for n in [1, 5, SLOT_COUNT] {
            let toc = fs::read_to_string(slot_toc_file(&dir, n)).unwrap();
            assert!(toc.lines().any(|line| line == "Slot.xml") && !toc.contains("Inbox.lua"), "slot {n}: {toc}");
            assert!(toc.starts_with("## Interface: 16002\n"), "the install keeps the interface it had: {toc}");
            assert!(slot_file(&dir, n, "Slot.xml").exists());
        }
        assert_eq!(fs::read_to_string(&inbox).unwrap(), "T3WoW_Inbox = { slot = 5 }", "the slot's update stays");

        // Done once: the next start of the companion finds nothing to do.
        let again = slots.repair().unwrap();
        assert_eq!((again.addon, again.created, again.slots_moved), (None, 0, 0));

        // Stopped partway last time: this slot has its Slot.xml, only its toc is old.
        fs::write(slot_toc_file(&dir, 8), "## Interface: 16002\n## Title: slot 8\nInbox.lua\n").unwrap();
        let finished = slots.repair().unwrap();
        assert_eq!((finished.created, finished.slots_moved), (0, 1));
        assert_eq!(fs::read(slot_toc_file(&dir, 8)).unwrap(), fs::read(slot_toc_file(&dir, 1)).unwrap());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_key_is_written_as_key_lua_and_put_back_like_any_addon_file() {
        let dir = temp_dir("key");
        let key = "0f".repeat(32);
        let slots = fixture_slots(&dir, "16001").with_key(Some(key.clone()));
        slots.install().unwrap();
        let file = dir.join(ADDON).join(KEY_FILE);
        let lua = fs::read_to_string(&file).unwrap();
        assert!(lua.contains(&format!("ns.key = \"{key}\"")), "{lua}");
        // Key.xml loads it, with the open tag addon managers don't follow.
        let toc = fs::read_to_string(dir.join(ADDON).join("T3WoW.toc")).unwrap();
        assert!(toc.lines().any(|l| l == "Key.xml") && !toc.contains(KEY_FILE));
        assert!(fs::read_to_string(dir.join(ADDON).join("Key.xml")).unwrap().contains("<Script file=\"Key.lua\"></Script>"));
        assert!(!slots.missing_files());
        fs::remove_file(&file).unwrap();
        assert!(slots.missing_files());
        assert_eq!(slots.repair().unwrap().created, 1);
        assert_eq!(fs::read_to_string(&file).unwrap(), lua);
        // A new key replaces the old one in place.
        let other = fixture_slots(&dir, "16001").with_key(Some("a1".repeat(32)));
        assert_eq!(other.repair().unwrap().created, 0);
        assert!(fs::read_to_string(&file).unwrap().contains(&"a1".repeat(32)));
        // Without a key, Key.lua is neither needed nor touched.
        let keyless = fixture_slots(&dir, "16001");
        fs::remove_file(&file).unwrap();
        assert!(!keyless.missing_files());
        keyless.repair().unwrap();
        assert!(!file.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_slot_that_lost_its_slot_xml_counts_as_missing_files() {
        let dir = temp_dir("slot-xml-missing");
        let slots = fixture_slots(&dir, "16001");
        slots.install().unwrap();
        assert!(!slots.missing_files());
        fs::remove_file(slot_file(&dir, SLOT_COUNT, "Slot.xml")).unwrap();
        assert!(slots.missing_files());
        assert_eq!(slots.repair().unwrap().created, 1);
        assert!(!slots.missing_files());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_replaced_by_a_plain_copy_of_the_addon_counts_as_missing_files_until_repaired() {
        let dir = temp_dir("missing");
        let slots = fixture_slots(&dir, "16001");
        slots.install().unwrap();
        assert!(!slots.missing_files());

        // Copied over from the repo: the Lua files are there, sig, ctl and img are not.
        for made in ["sig", "ctl", "img"] {
            fs::remove_dir_all(dir.join("T3WoW").join(made)).unwrap();
        }
        assert!(slots.missing_files());
        let slot = dir.join(slot_name(5)).join("Inbox.lua");
        fs::write(&slot, "T3WoW_Inbox = {}").unwrap();

        let repair = slots.repair().unwrap();
        assert_eq!(repair.addon, None);
        assert_eq!(repair.created, SIGNAL_COUNT + IMAGE_COUNT + 1 + 7);
        assert!(!slots.missing_files());
        // Only what was gone: a slot's update stays.
        assert_eq!(fs::read_to_string(&slot).unwrap(), "T3WoW_Inbox = {}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn repair_puts_back_deleted_addon_files_but_leaves_a_newer_addon_alone() {
        let dir = temp_dir("repair-files");
        let slots = fixture_slots(&dir, "16001");
        slots.install().unwrap();

        // A cleanup removed some of the addon's own files; the toc survived.
        fs::remove_file(dir.join("T3WoW/UI.lua")).unwrap();
        fs::remove_file(dir.join("T3WoW/Media/logo.tga")).unwrap();
        let repair = slots.repair().unwrap();
        assert_eq!(repair.addon, Some(format!("{} repaired (put back UI.lua, Media/logo.tga)", shipped_version())));
        assert_eq!(repair.created, 2);
        let shipped = |name: &str| ADDON_FILES.iter().find(|(n, _)| *n == name).unwrap().1;
        assert_eq!(fs::read(dir.join("T3WoW/UI.lua")).unwrap(), shipped("UI.lua"));
        assert_eq!(fs::read(dir.join("T3WoW/Media/logo.tga")).unwrap(), shipped("Media/logo.tga"));
        assert_eq!(slots.repair().unwrap().addon, None);

        // A newer addon missing a file isn't patched with this companion's older one.
        let toc = dir.join("T3WoW/T3WoW.toc");
        let newer = fs::read_to_string(&toc).unwrap().replace(&format!("## Version: {}", shipped_version()), "## Version: 99.0.0");
        fs::write(&toc, &newer).unwrap();
        fs::remove_file(dir.join("T3WoW/Link.lua")).unwrap();
        let repair = slots.repair().unwrap();
        assert_eq!((repair.addon, repair.created), (None, 0));
        assert!(!dir.join("T3WoW/Link.lua").exists());
        assert_eq!(fs::read_to_string(&toc).unwrap(), newer);
        // The game is told about the newer addon, not this companion's.
        assert_eq!(slots.addon_version().as_deref(), Some("99.0.0"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_interface_version_changes_only_the_interface_lines() {
        let dir = temp_dir("interface");
        fixture_slots(&dir, "16001").install().unwrap();
        let inbox = dir.join(slot_name(5)).join("Inbox.lua");
        fs::write(&inbox, "T3WoW_Inbox = { slot = 5 }").unwrap();
        let toc = dir.join("T3WoW/T3WoW.toc");
        let before = fs::read_to_string(&toc).unwrap();

        assert_eq!(fixture_slots(&dir, "16002").set_interface().unwrap(), 1 + SLOT_COUNT);
        assert_eq!(fs::read_to_string(&toc).unwrap(), before.replace("## Interface: 16001\r\n", "## Interface: 16002\r\n"));
        let slot_toc = fs::read_to_string(dir.join(slot_name(5)).join(format!("{}.toc", slot_name(5)))).unwrap();
        assert!(slot_toc.starts_with("## Interface: 16002\n## Title:"));
        assert_eq!(fs::read_to_string(&inbox).unwrap(), "T3WoW_Inbox = { slot = 5 }");
        assert_eq!(fixture_slots(&dir, "16002").set_interface().unwrap(), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_install_with_an_impossible_interface_version_goes_back_to_the_shipped_one() {
        // The fallback for an impossible saved version is the shipped addon's own number.
        let expected = format!("## Interface: {}", crate::config::DEFAULT_INTERFACE);
        assert!(String::from_utf8_lossy(ADDON_FILES[0].1).lines().any(|line| line.trim_end() == expected));

        // What 0.5.0's Match WoW's interface version… wrote on 2026-11-07.
        let dir = temp_dir("bad-interface");
        fixture_slots(&dir, "1607186").install().unwrap();
        let mut config = crate::config::Config::default();
        config.interface = Some("1607186".into());
        let interface = config.interface();
        assert_eq!(fixture_slots(&dir, &interface).set_interface().unwrap(), 1 + SLOT_COUNT);
        assert!(fs::read_to_string(dir.join("T3WoW/T3WoW.toc")).unwrap().contains("## Interface: 16001\r\n"));
        let slot_toc = fs::read_to_string(dir.join(slot_name(SLOT_COUNT)).join(format!("{}.toc", slot_name(SLOT_COUNT)))).unwrap();
        assert!(slot_toc.starts_with("## Interface: 16001\n"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn image_slots_start_blank_take_pictures_and_give_their_space_back() {
        let dir = temp_dir("image-slots");
        let mut slots = fixture_slots(&dir, "16001");
        slots.install().unwrap();
        let file = |n: u32| dir.join("T3WoW/img").join(image_file(n));
        for n in [1, IMAGE_COUNT] {
            assert_eq!(fs::read(file(n)).unwrap(), b"", "slot {n} starts blank");
        }
        assert_eq!(fs::read(dir.join("T3WoW/img").join(IMAGE_SENTINEL)).unwrap(), b"", "the sentinel stays blank");
        slots.write_image(3, b"tga bytes").unwrap();
        assert_eq!(fs::read(file(3)).unwrap(), b"tga bytes");
        assert!(slots.write_image(0, b"x").is_err() && slots.write_image(IMAGE_COUNT + 1, b"x").is_err());
        // A repair leaves a picture alone (the game may have it loaded); a fresh install blanks it.
        slots.repair().unwrap();
        assert_eq!(fs::read(file(3)).unwrap(), b"tga bytes");
        slots.blank_image(3);
        assert_eq!(fs::read(file(3)).unwrap(), b"");
        slots.write_image(4, b"tga bytes").unwrap();
        slots.install().unwrap();
        assert_eq!(fs::read(file(4)).unwrap(), b"");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn clearing_the_slots_blanks_every_update_and_nothing_else() {
        let dir = temp_dir("clear-slots");
        let mut slots = fixture_slots(&dir, "16001");
        slots.install().unwrap();
        slots.write_slot(1, "T3WoW_Inbox = { lid = \"old\" }").unwrap();
        slots.write_slot(SLOT_COUNT, "T3WoW_Inbox = { lid = \"old\" }").unwrap();
        slots.raise_signal(7).unwrap();
        slots.clear_slots();
        for n in [1, SLOT_COUNT] {
            assert_eq!(fs::read_to_string(dir.join(slot_name(n)).join("Inbox.lua")).unwrap(), "");
        }
        assert!(slots.installed());
        assert_eq!(fs::read(dir.join("T3WoW/sig/0007.ttf")).unwrap(), FIXTURE_FONT, "signals are another matter");
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_fresh_mac_client_gets_the_addon_folder_and_signals_that_can_load() {
        let wow = temp_dir("fresh-mac");
        let slots = Slots::for_wow(&wow, "16001");
        assert!(!wow.join("Interface").exists(), "a fresh install has none");
        slots.install().unwrap();
        assert!(wow.join("Interface/AddOns/T3WoW/T3WoW.toc").exists());
        let valid = fs::read(wow.join("Interface/AddOns/T3WoW/ctl/probe/valid.ttf")).unwrap();
        // A plain single-face TrueType: an sfnt, not a collection and not OpenType/CFF.
        assert_eq!(&valid[..4], &[0, 1, 0, 0]);
        assert_eq!(valid, addon_font("Media/JetBrainsMono-Regular.ttf").unwrap());
        assert!(!fs::read(wow.join("Interface/AddOns/T3WoW/ctl/probe/valid2.ttf")).unwrap().is_empty());

        let mut slots = slots;
        let sig = wow.join("Interface/AddOns/T3WoW/sig/0007.ttf");
        assert_eq!(fs::read(&sig).unwrap(), b"", "blank until raised");
        slots.raise_signal(7).unwrap();
        assert_eq!(fs::read(&sig).unwrap(), valid);
        let _ = fs::remove_dir_all(&wow);
    }

    #[test]
    fn versions_compare_numerically() {
        use std::cmp::Ordering::*;
        assert_eq!(compare_versions("0.10.0", "0.9.9"), Greater);
        assert_eq!(compare_versions("0.3", "0.3.0"), Equal);
        assert_eq!(compare_versions("0.2.7", "0.3.0"), Less);
        // A test build's label doesn't order it: a build of 0.9.1 is 0.9.1.
        assert_eq!(compare_versions("0.9.1-dev+57ee2ec", "0.9.1"), Equal);
        assert_eq!(compare_versions("0.9.1-dev+57ee2ec.dirty", "0.9.1-dev+0123abc"), Equal);
        assert_eq!(compare_versions("0.9.2", "0.9.1-dev+57ee2ec"), Greater);
        assert_eq!(compare_versions("0.10.0-dev", "0.9.9"), Greater);
    }

    #[test]
    fn another_build_of_the_same_release_replaces_the_addon() {
        let dir = temp_dir("other-build");
        let slots = fixture_slots(&dir, "16001");
        slots.install().unwrap();
        // The installed toc names this build, whatever the repo's toc says.
        assert_eq!(slots.installed_version(), Some(shipped_version()));
        let toc = dir.join("T3WoW/T3WoW.toc");
        let base = env!("CARGO_PKG_VERSION");
        for other in [format!("{base}-dev+0123abc"), base.to_string()].into_iter().filter(|v| *v != shipped_version()) {
            let text = fs::read_to_string(&toc).unwrap().replace(&format!("## Version: {}", shipped_version()), &format!("## Version: {other}"));
            fs::write(&toc, text).unwrap();
            fs::write(dir.join("T3WoW/UI.lua"), "-- the other build's").unwrap();
            assert_eq!(slots.repair().unwrap().addon, Some(format!("replaced {other} with {}", shipped_version())));
            assert_eq!(slots.installed_version(), Some(shipped_version()));
            assert_ne!(fs::read_to_string(dir.join("T3WoW/UI.lua")).unwrap(), "-- the other build's");
        }
        // This build again: nothing to do.
        assert_eq!(slots.repair().unwrap().addon, None);
        let _ = fs::remove_dir_all(&dir);
    }
}
