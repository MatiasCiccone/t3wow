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

/// The addon, embedded in the companion so the exe is all anyone needs.
pub const ADDON_FILES: &[(&str, &[u8])] = &[
    ("T3WoW.toc", include_bytes!("../../addon/T3WoW/T3WoW.toc")),
    ("Bindings.xml", include_bytes!("../../addon/T3WoW/Bindings.xml")),
    ("Codec.lua", include_bytes!("../../addon/T3WoW/Codec.lua")),
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

/// The `## Version:` of the addon this companion ships.
pub fn shipped_version() -> String {
    parse_version(&String::from_utf8_lossy(ADDON_FILES[0].1)).unwrap_or_else(|| "0".into())
}

pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |v: &str| v.split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (pa, pb) = (parse(a), parse(b));
    for i in 0..pa.len().max(pb.len()) {
        let order = pa.get(i).unwrap_or(&0).cmp(pb.get(i).unwrap_or(&0));
        if order.is_ne() {
            return order;
        }
    }
    std::cmp::Ordering::Equal
}

/// A 2x2 white 32-bit TGA, the smallest texture the game is sure to accept.
fn tiny_tga() -> Vec<u8> {
    let mut out = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, 32, 0x28];
    out.extend_from_slice(&[0xff; 16]);
    out
}

/// A Windows text font, or None. Only copied on this PC, never shipped. Symbol fonts
/// (Marlett and the like) don't load in the game, so these are plain text fonts.
fn windows_font(names: &[&str]) -> Option<Vec<u8>> {
    let dir = PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into())).join("Fonts");
    names.iter().find_map(|name| fs::read(dir.join(name)).ok())
}

/// Every signal is a copy of this font (Lucida Console is about 110 KB), so small wins.
fn small_font() -> Option<Vec<u8>> {
    windows_font(&["lucon.ttf", "verdana.ttf", "arial.ttf"])
}

pub struct Repair {
    /// "installed 0.3.0" / "updated 0.2.7 to 0.3.0" when the addon itself was written.
    pub addon: Option<String>,
    /// Files that did not exist before (the game only sees them after a full restart).
    pub created: u32,
}

pub struct Slots {
    pub addons_dir: PathBuf,
    pub addon_dir: PathBuf,
    interface: String,
    font: Option<Vec<u8>>,
    /// A second font for the probe's valid2.ttf.
    alt_font: Option<Vec<u8>>,
}

impl Slots {
    pub fn new(addons_dir: impl Into<PathBuf>, interface: &str) -> Slots {
        Slots::with_fonts(addons_dir, interface, small_font(), windows_font(&["arial.ttf", "segoeui.ttf"]))
    }

    /// With the given fonts instead of this PC's (tests use fixture bytes, so they run anywhere).
    fn with_fonts(addons_dir: impl Into<PathBuf>, interface: &str, font: Option<Vec<u8>>, alt_font: Option<Vec<u8>>) -> Slots {
        let addons_dir = addons_dir.into();
        Slots { addon_dir: addons_dir.join(ADDON), addons_dir, interface: interface.to_string(), font, alt_font }
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
                    .map(|line| if line.starts_with("## Interface:") { format!("## Interface: {}", self.interface) } else { line.to_string() })
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
    /// ones are blanked too (a fresh install); without, only missing ones are created.
    /// Returns how many files did not exist before.
    pub fn write_support(&self, reset: bool) -> io::Result<u32> {
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
        for n in 1..=SLOT_COUNT {
            let dir = self.addons_dir.join(slot_name(n));
            fs::create_dir_all(&dir)?;
            let toc = [
                format!("## Interface: {}", self.interface),
                format!("## Title: |cff808080T3 WoW slot {n:03}|r"),
                "## Notes: Carries T3 Code updates into the game. Keep enabled.".into(),
                "## LoadOnDemand: 1".into(),
                format!("## Dependencies: {ADDON}"),
                "Inbox.lua".into(),
                String::new(),
            ]
            .join("\n");
            put(dir.join(format!("{}.toc", slot_name(n))), toc.as_bytes())?;
            put(dir.join("Inbox.lua"), b"")?;
        }
        Ok(created)
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
                std::cmp::Ordering::Equal => {
                    let restored = self.write_addon_files(true)?;
                    created += restored.len() as u32;
                    (!restored.is_empty()).then(|| format!("{shipped} repaired (put back {})", restored.join(", ")))
                }
                std::cmp::Ordering::Less => None,
            },
        };
        created += self.write_support(false)?;
        Ok(Repair { addon, created })
    }

    /// Whether the addon's toc or the files the companion makes (signal, probe and image
    /// files, slot addons) are gone: something replaced or cleaned the folder. Looks at a
    /// few of them only, so it can run often; `repair` puts back the rest.
    pub fn missing_files(&self) -> bool {
        let sig = self.addon_dir.join("sig");
        let img = self.addon_dir.join("img");
        let slot_toc = |n: u32| self.addons_dir.join(slot_name(n)).join(format!("{}.toc", slot_name(n)));
        [
            self.addon_dir.join(format!("{ADDON}.toc")),
            sig.join(signal_file(1)),
            sig.join(signal_file(SIGNAL_COUNT)),
            self.addon_dir.join("ctl").join("probe").join("valid.ttf"),
            img.join(IMAGE_SENTINEL),
            img.join(image_file(IMAGE_COUNT)),
            slot_toc(1),
            slot_toc(SLOT_COUNT),
        ]
        .iter()
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
        assert_eq!(second.created, 3);
        assert!(dir.join(slot_name(7)).join("Inbox.lua").exists());

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

    #[test]
    fn versions_compare_numerically() {
        use std::cmp::Ordering::*;
        assert_eq!(compare_versions("0.10.0", "0.9.9"), Greater);
        assert_eq!(compare_versions("0.3", "0.3.0"), Equal);
        assert_eq!(compare_versions("0.2.7", "0.3.0"), Less);
    }
}
