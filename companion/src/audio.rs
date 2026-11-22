//! Audio a thread embeds, played out loud on this PC when it's clicked in the game. The game
//! plays only Ogg and MP3 files that were there when it started, so the companion plays it:
//! with Windows' own MCI player, or `afplay` on a Mac.

use std::path::Path;

/// Plays one audio file at a time; a trait so tests can stand in for the speakers.
pub trait Player: Send {
    /// Plays `path` from the start, in place of anything playing.
    fn play(&mut self, path: &Path) -> Result<(), String>;
    /// Stops what's playing, if anything is.
    fn stop(&mut self);
    /// Whether something is still playing (it hasn't ended or been stopped).
    fn playing(&mut self) -> bool;
}

/// This PC's player. Test builds never make a sound: a test that plays something without a
/// stand-in fails.
pub fn system() -> Box<dyn Player> {
    #[cfg(test)]
    return Box::new(Unplayable);
    #[cfg(all(not(test), windows))]
    return Box::new(windows::Mci { open: false });
    #[cfg(all(not(test), target_os = "macos"))]
    return Box::new(mac::Afplay { child: None });
    #[cfg(all(not(test), not(any(windows, target_os = "macos"))))]
    return Box::new(Unplayable);
}

#[cfg(any(test, not(any(windows, target_os = "macos"))))]
struct Unplayable;

#[cfg(any(test, not(any(windows, target_os = "macos"))))]
impl Player for Unplayable {
    fn play(&mut self, path: &Path) -> Result<(), String> {
        #[cfg(test)]
        panic!("a test tried to play {} for real; stub the player instead", path.display());
        #[cfg(not(test))]
        Err(format!("{} can't be played on this system", path.display()))
    }
    fn stop(&mut self) {}
    fn playing(&mut self) -> bool {
        false
    }
}

#[cfg(windows)]
mod windows {
    use super::Player;
    use std::path::Path;
    use windows_sys::Win32::Media::Multimedia::{mciGetErrorStringW, mciSendStringW};

    const ALIAS: &str = "t3wow_audio";

    /// Windows' MCI player. Its device lives on the thread that opened it, which is the
    /// bridge's: every call comes from there.
    pub struct Mci {
        pub open: bool,
    }

    /// Sends an MCI command string; its answer, or MCI's reason it failed.
    fn send(command: &str) -> Result<String, String> {
        let wide: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
        let mut answer = [0u16; 128];
        let error = unsafe { mciSendStringW(wide.as_ptr(), answer.as_mut_ptr(), answer.len() as u32, std::ptr::null_mut()) };
        if error != 0 {
            let mut text = [0u16; 256];
            let known = unsafe { mciGetErrorStringW(error, text.as_mut_ptr(), text.len() as u32) } != 0;
            let reason = if known { String::from_utf16_lossy(&text[..text.iter().position(|&c| c == 0).unwrap_or(0)]) } else { format!("MCI error {error}") };
            return Err(reason.trim().trim_end_matches('.').to_string());
        }
        Ok(String::from_utf16_lossy(&answer[..answer.iter().position(|&c| c == 0).unwrap_or(0)]))
    }

    impl Player for Mci {
        fn play(&mut self, path: &Path) -> Result<(), String> {
            self.stop();
            // "mpegvideo" is Windows' media player behind MCI: on Windows 11 MP3, WAV, FLAC,
            // Ogg, Opus, AAC, M4A and AIFF all play through it (Ogg and Opus through the Web
            // Media Extensions it comes with). A path can't hold a quote.
            send(&format!("open \"{}\" type mpegvideo alias {ALIAS}", path.display())).map_err(|e| format!("Windows couldn't open it: {e}"))?;
            self.open = true;
            if let Err(error) = send(&format!("play {ALIAS}")) {
                self.stop();
                return Err(format!("Windows couldn't play it: {error}"));
            }
            Ok(())
        }

        fn stop(&mut self) {
            if std::mem::take(&mut self.open) {
                let _ = send(&format!("close {ALIAS}"));
            }
        }

        fn playing(&mut self) -> bool {
            let on = self.open && send(&format!("status {ALIAS} mode")).is_ok_and(|mode| mode == "playing");
            // Ended: closed, so the file isn't held open.
            if !on {
                self.stop();
            }
            on
        }
    }

    impl Drop for Mci {
        fn drop(&mut self) {
            self.stop();
        }
    }
}

/// Stops what's playing as the app quits: on a Mac `afplay` would play on after it.
#[cfg(target_os = "macos")]
pub fn quit() {
    #[cfg(all(not(test), target_os = "macos"))]
    mac::kill_playing();
}

#[cfg(all(not(test), target_os = "macos"))]
mod mac {
    use super::Player;
    use std::path::Path;
    use std::process::{Child, Command, Stdio};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// The `afplay` playing, for `quit`, which runs on another thread (0: none).
    static PLAYING: AtomicU32 = AtomicU32::new(0);

    pub fn kill_playing() {
        let pid = PLAYING.swap(0, Ordering::SeqCst);
        if pid != 0 {
            unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        }
    }

    /// macOS's `afplay`, one at a time: MP3, WAV, AIFF, AAC, M4A and FLAC (not Ogg or Opus).
    pub struct Afplay {
        pub child: Option<Child>,
    }

    impl Player for Afplay {
        fn play(&mut self, path: &Path) -> Result<(), String> {
            self.stop();
            let child = Command::new("/usr/bin/afplay").arg(path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e| format!("couldn't start afplay: {e}"))?;
            PLAYING.store(child.id(), Ordering::SeqCst);
            self.child = Some(child);
            // One it can't read ends at once, and says so; a short clip may have played already.
            std::thread::sleep(std::time::Duration::from_millis(150));
            match self.child.as_mut().map(|c| c.try_wait()) {
                Some(Ok(Some(status))) => {
                    self.stop();
                    if !status.success() {
                        return Err("macOS can't play this kind of file".into());
                    }
                }
                Some(Err(e)) => {
                    self.stop();
                    return Err(format!("afplay: {e}"));
                }
                _ => {}
            }
            Ok(())
        }

        fn stop(&mut self) {
            if let Some(mut child) = self.child.take() {
                PLAYING.store(0, Ordering::SeqCst);
                let _ = child.kill();
                let _ = child.wait();
            }
        }

        fn playing(&mut self) -> bool {
            match self.child.as_mut().map(|c| c.try_wait()) {
                Some(Ok(None)) => true,
                Some(_) => {
                    // Ended: reaped, so it isn't left as a zombie.
                    self.stop();
                    false
                }
                None => false,
            }
        }
    }

    impl Drop for Afplay {
        fn drop(&mut self) {
            self.stop();
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::windows::Mci;
    use super::Player;
    use std::path::Path;

    /// Plays with Windows for real (silent: a WAV of zeros): `cargo test -- --ignored`.
    #[test]
    #[ignore = "plays through the PC's sound device"]
    fn windows_plays_a_file_until_its_stopped_and_says_why_it_cant() {
        let dir = std::env::temp_dir().join(format!("t3wow-audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("quiet one.wav");
        // Three seconds of 8 kHz 8-bit mono silence.
        let samples = 8000 * 3;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&(36 + samples as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&[16, 0, 0, 0, 1, 0, 1, 0, 0x40, 0x1f, 0, 0, 0x40, 0x1f, 0, 0, 1, 0, 8, 0]);
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(samples as u32).to_le_bytes());
        bytes.extend(std::iter::repeat_n(128u8, samples));
        std::fs::write(&wav, bytes).unwrap();
        let mut player = Mci { open: false };
        player.play(&wav).unwrap();
        assert!(player.playing());
        player.stop();
        assert!(!player.playing());
        let error = player.play(Path::new(r"C:\nowhere\gone.mp3")).unwrap_err();
        assert!(error.starts_with("Windows couldn't open it: "), "{error}");
        assert!(!player.playing());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
