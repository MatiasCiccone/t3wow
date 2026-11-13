//! macOS: the key that encrypts saved sign-ins. A file in the config folder,
//! `~/Library/Application Support/t3wow/token.key`, holds a random 256-bit key that only this
//! user can read (mode 0600); the tokens stay in config.json, encrypted with it (AES-256-GCM).
//!
//! Why a file: it gives the same protection as DPAPI does on Windows (other users of the Mac
//! can't read a token, this user's programs can), and reading it never brings up a prompt.
//! The login keychain did the same until an update: a build that isn't signed with an Apple
//! Developer ID gets a keychain partition of its own, so macOS asked for the password again
//! after every update. The key stays out of config.json, so sharing config.json to get help
//! doesn't share the sign-ins.
//!
//! The key file is created when it's missing and never replaced: a file that can't be read
//! or doesn't hold a key is reported (`Problem`), since a new key would make every saved
//! token undecryptable.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// How a token encrypted here starts in config.json (Windows' are `dpapi:`).
pub const SEALED: &str = "keyfile-aes256gcm:";
/// How a token sealed with a key from the login keychain started. Only test builds of the Mac
/// app saved those (no release did); they read as signed out, and the Keychain isn't asked.
pub const KEYCHAIN_SEALED: &str = "keychain-aes256gcm:";

/// The key file's name in the config folder. Never renamed: a new name would mean a new key,
/// and every saved sign-in lost.
const FILE_NAME: &str = "token.key";

const KEY_LEN: usize = 32;
type Key = [u8; KEY_LEN];

/// Why there is no key although the file is there. Signing in again wouldn't help: a new
/// sign-in couldn't be saved either, and the file is left as it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// The file is there but couldn't be read or created (the error says why).
    Unreadable(PathBuf, String),
    /// The file is there but doesn't hold a key.
    Malformed(PathBuf),
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Problem::Unreadable(path, error) => write!(f, "T3 WoW couldn't read its sign-in key, {} ({error})", path.display()),
            Problem::Malformed(path) => write!(
                f,
                "{} doesn't hold a T3 WoW sign-in key, so saved sign-ins can't be read. Move it aside (T3 WoW makes a new one) and sign in again",
                path.display()
            ),
        }
    }
}

/// The key, read from or written to one file.
pub struct Keyring {
    path: PathBuf,
    /// Once read, kept for the run: `t3::target` decrypts every 30 seconds.
    key: Mutex<Option<Key>>,
    /// The last problem logged, so a broken file is logged once and not every 30 seconds.
    logged: Mutex<Option<Problem>>,
}

impl Keyring {
    pub fn new(path: PathBuf) -> Keyring {
        Keyring { path, key: Mutex::new(None), logged: Mutex::new(None) }
    }

    /// The key, read or created now if this run doesn't have it yet.
    fn key(&self) -> Result<Key, Problem> {
        let mut known = self.key.lock().unwrap();
        if let Some(key) = *known {
            return Ok(key);
        }
        let answer = load_or_create(&self.path);
        match &answer {
            Ok(key) => *known = Some(*key),
            Err(problem) => {
                let mut logged = self.logged.lock().unwrap();
                if logged.as_ref() != Some(problem) {
                    crate::log::line(&format!("Saved T3 sign-ins can't be read: {problem}."));
                    *logged = Some(problem.clone());
                }
            }
        }
        answer
    }

    pub fn problem(&self) -> Option<Problem> {
        self.key().err()
    }

    /// `token` as it is stored; None when there is no key.
    pub fn seal(&self, token: &str) -> Option<String> {
        seal_with(&self.key().ok()?, token)
    }

    /// A stored token (without its prefix) back as text.
    pub fn open(&self, encoded: &str) -> Option<String> {
        open_with(&self.key().ok()?, encoded)
    }
}

/// Reads the key file, or creates it when it's missing. Anything else, a file that can't be
/// read or doesn't hold a key, is a problem and the file stays as it is.
fn load_or_create(path: &Path) -> Result<Key, Problem> {
    match read_key(path) {
        Err(ReadError::Missing) => create_key(path),
        other => other.map_err(|e| e.problem(path)),
    }
}

enum ReadError {
    Missing,
    Io(io::Error),
    Malformed,
}

impl ReadError {
    fn problem(self, path: &Path) -> Problem {
        match self {
            ReadError::Missing => Problem::Unreadable(path.to_path_buf(), "it went missing".into()),
            ReadError::Io(error) => Problem::Unreadable(path.to_path_buf(), error.to_string()),
            ReadError::Malformed => Problem::Malformed(path.to_path_buf()),
        }
    }
}

fn read_key(path: &Path) -> Result<Key, ReadError> {
    let text = fs::read_to_string(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => ReadError::Missing,
        io::ErrorKind::InvalidData => ReadError::Malformed,
        _ => ReadError::Io(e),
    })?;
    let bytes = STANDARD.decode(text.trim()).map_err(|_| ReadError::Malformed)?;
    let key = Key::try_from(bytes.as_slice()).map_err(|_| ReadError::Malformed)?;
    // Same-user protection is the point: a file others can read is narrowed, its key kept.
    if let Ok(meta) = fs::metadata(path) {
        if meta.permissions().mode() & 0o077 != 0 {
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        }
    }
    Ok(key)
}

/// Writes a new random key to a temporary file and links it into place, which fails if the
/// file is already there: a `t3wow login` in Terminal may have created it first, and then its
/// key is the key. Never replaces a file.
fn create_key(path: &Path) -> Result<Key, Problem> {
    let unreadable = |error: io::Error| Problem::Unreadable(path.to_path_buf(), error.to_string());
    let mut key = [0u8; KEY_LEN];
    getrandom::fill(&mut key).map_err(|e| unreadable(io::Error::other(e.to_string())))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(unreadable)?;
    }
    let temp = path.with_extension(format!("key.{}.tmp", std::process::id()));
    let written = (|| {
        let mut out = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&temp)?;
        out.set_permissions(fs::Permissions::from_mode(0o600))?;
        out.write_all(format!("{}\n", STANDARD.encode(key)).as_bytes())?;
        out.sync_all()
    })()
    .and_then(|()| fs::hard_link(&temp, path));
    let _ = fs::remove_file(&temp);
    match written {
        Ok(()) => {
            crate::log::line(&format!("Made a new key for saved sign-ins: {}", path.display()));
            Ok(key)
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => read_key(path).map_err(|e| e.problem(path)),
        Err(e) => Err(unreadable(e)),
    }
}

fn sealing_key(key: &Key) -> Option<LessSafeKey> {
    UnboundKey::new(&AES_256_GCM, key).ok().map(LessSafeKey::new)
}

/// `SEALED` then base64 of the nonce, the ciphertext and its tag. A new random nonce every
/// time: the key stays the same for good, and GCM falls apart if a nonce repeats under it.
fn seal_with(key: &Key, token: &str) -> Option<String> {
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).ok()?;
    let mut data = token.as_bytes().to_vec();
    sealing_key(key)?.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(SEALED.as_bytes()), &mut data).ok()?;
    let mut blob = nonce.to_vec();
    blob.append(&mut data);
    Some(format!("{SEALED}{}", STANDARD.encode(blob)))
}

fn open_with(key: &Key, encoded: &str) -> Option<String> {
    let blob = STANDARD.decode(encoded).ok()?;
    let (nonce, sealed) = blob.split_at_checked(NONCE_LEN)?;
    let nonce = Nonce::try_assume_unique_for_key(nonce).ok()?;
    let mut data = sealed.to_vec();
    let plain = sealing_key(key)?.open_in_place(nonce, Aad::from(SEALED.as_bytes()), &mut data).ok()?;
    String::from_utf8(plain.to_vec()).ok()
}

#[cfg(not(test))]
fn keyring() -> Arc<Keyring> {
    static KEYRING: std::sync::OnceLock<Arc<Keyring>> = std::sync::OnceLock::new();
    KEYRING.get_or_init(|| Arc::new(Keyring::new(crate::config::config_dir().join(FILE_NAME)))).clone()
}

#[cfg(test)]
fn keyring() -> Arc<Keyring> {
    test_support::ACTIVE.with(|active| active.borrow().clone())
}

/// Why a sign-in can't be saved or read although the user may well have one: the key file is
/// there but unusable. None when the key is there or was just made.
pub fn problem() -> Option<Problem> {
    keyring().problem()
}

pub fn seal(token: &str) -> Option<String> {
    keyring().seal(token)
}

pub fn open(encoded: &str) -> Option<String> {
    keyring().open(encoded)
}

/// Says once per run that a token sealed with the old Keychain key is there and reads as
/// signed out.
pub fn note_keychain_token() {
    static SAID: AtomicBool = AtomicBool::new(false);
    if !SAID.swap(true, Ordering::Relaxed) {
        crate::log::line("A T3 sign-in saved by an earlier test build of T3 WoW (in the macOS Keychain) can't be read by this one: sign in to T3 again.");
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::cell::RefCell;

    /// A key file path of its own under the system temp folder (not created).
    pub fn temp_key_path(name: &str) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("t3wow-key-{}-{n}-{name}", std::process::id())).join(FILE_NAME)
    }

    thread_local! {
        /// Each test runs on its own thread, with its own key file.
        pub static ACTIVE: RefCell<Arc<Keyring>> = RefCell::new(Arc::new(Keyring::new(temp_key_path("default"))));
    }

    /// Makes `keyring` the one this test's `seal`, `open` and `problem` use.
    pub fn use_keyring(keyring: Arc<Keyring>) {
        ACTIVE.with(|active| *active.borrow_mut() = keyring);
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::temp_key_path;
    use super::*;

    fn encoded(stored: &str) -> &str {
        stored.strip_prefix(SEALED).expect("carries the prefix")
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_missing_key_file_is_created_private_and_the_next_run_reads_it() {
        let path = temp_key_path("created");
        assert!(!path.exists() && !path.parent().unwrap().exists());
        let first = Keyring::new(path.clone());
        let stored = first.seal("the token").unwrap();
        assert_eq!(mode(&path), 0o600);
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(STANDARD.decode(text.trim()).unwrap().len(), KEY_LEN);
        assert!(fs::read_dir(path.parent().unwrap()).unwrap().count() == 1, "no temporary file left");

        let second = Keyring::new(path.clone());
        assert_eq!(second.open(encoded(&stored)).as_deref(), Some("the token"), "a second run reads the same key");
        assert_eq!(fs::read_to_string(&path).unwrap(), text, "and doesn't rewrite it");
    }

    #[test]
    fn a_token_comes_back_and_each_sealing_looks_different() {
        let keyring = Keyring::new(temp_key_path("round-trip"));
        let one = keyring.seal("the token").unwrap();
        let two = keyring.seal("the token").unwrap();
        assert_ne!(one, two, "a new nonce every time");
        assert!(!one.contains("the token"));
        assert_eq!(keyring.open(encoded(&one)).as_deref(), Some("the token"));
        assert_eq!(keyring.open(encoded(&two)).as_deref(), Some("the token"));
    }

    #[test]
    fn a_malformed_key_file_is_reported_and_never_replaced() {
        for (name, contents) in [("empty", &b""[..]), ("text", b"not a key\n"), ("short", b"AAAA\n"), ("binary", &[0xff, 0xfe, 0, 1][..])] {
            let path = temp_key_path(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            let keyring = Keyring::new(path.clone());
            assert_eq!(keyring.problem(), Some(Problem::Malformed(path.clone())), "{name}");
            assert_eq!(keyring.seal("t"), None, "{name}");
            assert_eq!(keyring.open("AAAA"), None, "{name}");
            assert_eq!(fs::read(&path).unwrap(), contents, "{name}: left as it was");
        }
        let text = Problem::Malformed(PathBuf::from("/x/token.key")).to_string();
        assert!(text.contains("/x/token.key") && text.contains("sign in again"), "{text}");
    }

    #[test]
    fn an_unreadable_key_file_is_reported_and_never_replaced() {
        let path = temp_key_path("unreadable");
        let stored = Keyring::new(path.clone()).seal("the token").unwrap();
        let before = fs::read(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        let keyring = Keyring::new(path.clone());
        let problem = keyring.problem();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(problem, Some(Problem::Unreadable(..))), "{problem:?}");
        assert_eq!(fs::read(&path).unwrap(), before, "left as it was");

        // A folder where the file should be can't be read either, and isn't touched.
        let dir = temp_key_path("a-folder");
        fs::create_dir_all(&dir).unwrap();
        assert!(matches!(Keyring::new(dir.clone()).problem(), Some(Problem::Unreadable(..))));
        assert!(dir.is_dir());

        // Once it can be read again, the same key opens the token.
        assert_eq!(keyring.open(encoded(&stored)).as_deref(), Some("the token"));
    }

    #[test]
    fn a_key_file_others_can_read_is_narrowed_and_kept() {
        let path = temp_key_path("wide");
        let stored = Keyring::new(path.clone()).seal("t").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(Keyring::new(path.clone()).open(encoded(&stored)).as_deref(), Some("t"));
        assert_eq!(mode(&path), 0o600);
    }

    #[test]
    fn a_key_file_another_process_just_created_is_the_one_used() {
        let path = temp_key_path("race");
        let theirs = Keyring::new(path.clone());
        let stored = theirs.seal("t").unwrap();
        let text = fs::read_to_string(&path).unwrap();
        // This one looked first, found nothing, and now tries to create it.
        assert_eq!(create_key(&path).ok(), theirs.key().ok(), "it gets their key");
        assert_eq!(fs::read_to_string(&path).unwrap(), text, "theirs wasn't replaced");
        assert_eq!(Keyring::new(path).open(encoded(&stored)).as_deref(), Some("t"));
    }

    #[test]
    fn tampered_or_damaged_ciphertext_fails_cleanly() {
        let keyring = Keyring::new(temp_key_path("tampered"));
        let stored = keyring.seal("the token").unwrap();
        let mut blob = STANDARD.decode(encoded(&stored)).unwrap();
        for at in [0, NONCE_LEN, blob.len() - 1] {
            blob[at] ^= 1;
            assert_eq!(keyring.open(&STANDARD.encode(&blob)), None, "byte {at} changed");
            blob[at] ^= 1;
        }
        assert_eq!(keyring.open(&STANDARD.encode(&blob[..blob.len() - 1])), None, "cut short");
        assert_eq!(keyring.open(&STANDARD.encode(&blob[..NONCE_LEN - 1])), None, "shorter than a nonce");
        assert_eq!(keyring.open(""), None);
        assert_eq!(keyring.open("not base64!"), None);
        assert_eq!(keyring.open(encoded(&stored)).as_deref(), Some("the token"), "the original still opens");
    }

    #[test]
    fn another_key_cannot_open_a_token() {
        let stored = Keyring::new(temp_key_path("one")).seal("the token").unwrap();
        assert_eq!(Keyring::new(temp_key_path("other")).open(encoded(&stored)), None);
    }
}
