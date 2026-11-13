//! macOS: the key that encrypts saved sign-ins. One generic-password item in the login
//! keychain holds a random 256-bit key; the tokens stay in config.json, encrypted with it
//! (AES-256-GCM), so macOS asks at most once per update of the app however many T3 servers
//! are saved.
//!
//! `Config::load` runs all over the tray and the worker, and `t3::target` decrypts every
//! 30 seconds, while a Keychain call blocks until it is answered. So the store is asked only
//! when the answer isn't known yet and something needs it (`read_at_startup`, `ensure`), or
//! when the user asks again (`retry`). Encrypting and decrypting only ever use what is
//! remembered here, and a refusal is remembered too: otherwise a denied prompt would come
//! back every 30 seconds, possibly over the game.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use core_foundation::data::CFData;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use security_framework::item::{ItemAddOptions, ItemAddValue, ItemClass, Location};
use security_framework::passwords::{generic_password, PasswordOptions};
use std::fmt;
use std::sync::{Arc, Mutex};

/// How a token encrypted here starts in config.json (Windows' are `dpapi:`).
pub const SEALED: &str = "keychain-aes256gcm:";

// Never renamed: a new name would mean a new key, and every saved sign-in lost.
#[cfg_attr(test, allow(dead_code))]
const SERVICE: &str = "t3wow.companion";
const ACCOUNT: &str = "token-encryption-key";
/// What Keychain Access and the macOS prompt call the item.
const LABEL: &str = "T3 WoW sign-in key";

const KEY_LEN: usize = 32;
type Key = [u8; KEY_LEN];

// Security.framework's result codes.
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
const ERR_SEC_DUPLICATE_ITEM: i32 = -25299;
const ERR_SEC_USER_CANCELED: i32 = -128;
const ERR_SEC_AUTH_FAILED: i32 = -25293;
const ERR_SEC_INTERACTION_NOT_ALLOWED: i32 = -25308;

/// Why saved sign-ins can't be read right now. Unlike "not signed in", signing in again
/// wouldn't help, and what is stored stays as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    /// Nothing has asked the Keychain yet in this run, or its prompt is still up.
    NotRead,
    Denied,
    Locked,
    /// The item is there but doesn't hold a key.
    Malformed,
    Failed(i32),
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Problem::NotRead => f.write_str("waiting for the macOS Keychain; saved sign-ins aren't available yet"),
            Problem::Denied => f.write_str("macOS Keychain access was denied, so saved sign-ins can't be read (they are still saved). Restart T3 WoW to be asked again"),
            Problem::Locked => f.write_str("macOS couldn't open the login keychain: it's locked, or its permission dialog can't be shown here (over SSH, for example). Saved sign-ins are still saved. Unlock it or run T3 WoW on the Mac itself, then restart it"),
            Problem::Malformed => f.write_str("the T3 WoW item in the macOS Keychain doesn't hold a key, so saved sign-ins can't be read (they are still saved)"),
            Problem::Failed(code) => write!(f, "the macOS Keychain answered with error {code}, so saved sign-ins can't be read (they are still saved). Restart T3 WoW to try again"),
        }
    }
}

/// What the key's store can answer. Only `NotFound` ever leads to a new key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreError {
    NotFound,
    /// Creating: another process created the item first.
    Duplicate,
    Blocked(Problem),
}

impl StoreError {
    fn from_code(code: i32) -> StoreError {
        match code {
            ERR_SEC_ITEM_NOT_FOUND => StoreError::NotFound,
            ERR_SEC_DUPLICATE_ITEM => StoreError::Duplicate,
            ERR_SEC_USER_CANCELED | ERR_SEC_AUTH_FAILED => StoreError::Blocked(Problem::Denied),
            // Covers both a locked keychain and a permission dialog macOS can't show.
            ERR_SEC_INTERACTION_NOT_ALLOWED => StoreError::Blocked(Problem::Locked),
            other => StoreError::Blocked(Problem::Failed(other)),
        }
    }
}

/// Where the key lives; a trait so tests never reach the real Keychain.
pub trait KeyStore: Send + Sync {
    fn read(&self) -> Result<Key, StoreError>;
    /// Adds the item, never replacing one that exists.
    fn create(&self, key: &Key) -> Result<(), StoreError>;
}

/// The login keychain. Not the data protection keychain: that needs entitlements an
/// ad-hoc-signed build can't carry.
struct LoginKeychain {
    service: &'static str,
    account: &'static str,
}

impl KeyStore for LoginKeychain {
    fn read(&self) -> Result<Key, StoreError> {
        let bytes = generic_password(PasswordOptions::new_generic_password(self.service, self.account)).map_err(|e| StoreError::from_code(e.code()))?;
        Key::try_from(bytes.as_slice()).map_err(|_| StoreError::Blocked(Problem::Malformed))
    }

    fn create(&self, key: &Key) -> Result<(), StoreError> {
        let mut options = ItemAddOptions::new(ItemAddValue::Data { class: ItemClass::generic_password(), data: CFData::from_buffer(key) });
        options.set_service(self.service).set_account_name(self.account).set_label(LABEL).set_location(Location::DefaultFileKeychain);
        options.add().map_err(|e| StoreError::from_code(e.code()))
    }
}

#[derive(Clone, Copy)]
enum State {
    Unread,
    Ready(Key),
    Blocked(Problem),
}

/// The key as far as this run knows it.
pub struct Keyring {
    store: Box<dyn KeyStore>,
    /// Held only to look or to publish, never across a store call: a prompt left open must
    /// not stop another thread's `Config::load`.
    state: Mutex<State>,
    /// One store call at a time, so two threads can't both ask or both create a key.
    asking: Mutex<()>,
}

impl Keyring {
    pub fn new(store: Box<dyn KeyStore>) -> Keyring {
        Keyring { store, state: Mutex::new(State::Unread), asking: Mutex::new(()) }
    }

    fn state(&self) -> State {
        *self.state.lock().unwrap()
    }

    /// Asks the store unless this run already has an answer, whichever it was.
    pub fn ensure(&self) {
        if !matches!(self.state(), State::Unread) {
            return;
        }
        let _asking = self.asking.lock().unwrap();
        // Another thread may have got the answer while this one waited.
        if matches!(self.state(), State::Unread) {
            let answer = self.fetch();
            *self.state.lock().unwrap() = answer;
        }
    }

    /// Asks again after a refusal. The old answer stands until the new one arrives, so
    /// nothing mistakes the wait for "not signed in".
    pub fn retry(&self) {
        let _asking = self.asking.lock().unwrap();
        if !matches!(self.state(), State::Ready(_)) {
            let answer = self.fetch();
            *self.state.lock().unwrap() = answer;
        }
    }

    fn fetch(&self) -> State {
        match self.store.read() {
            Ok(key) => State::Ready(key),
            // The one case for a new key: on denied, locked or anything else, a new key
            // would make every saved token undecryptable.
            Err(StoreError::NotFound) => self.create_key(),
            Err(StoreError::Duplicate) => State::Blocked(Problem::Failed(ERR_SEC_DUPLICATE_ITEM)),
            Err(StoreError::Blocked(problem)) => State::Blocked(problem),
        }
    }

    fn create_key(&self) -> State {
        let mut key = [0u8; KEY_LEN];
        if getrandom::fill(&mut key).is_err() {
            return State::Blocked(Problem::Failed(0));
        }
        match self.store.create(&key) {
            Ok(()) => State::Ready(key),
            // A `login` in a terminal got there first: its key is the key.
            Err(StoreError::Duplicate) => match self.store.read() {
                Ok(theirs) => State::Ready(theirs),
                Err(StoreError::Blocked(problem)) => State::Blocked(problem),
                Err(_) => State::Blocked(Problem::Failed(ERR_SEC_DUPLICATE_ITEM)),
            },
            Err(StoreError::NotFound) => State::Blocked(Problem::Failed(ERR_SEC_ITEM_NOT_FOUND)),
            Err(StoreError::Blocked(problem)) => State::Blocked(problem),
        }
    }

    pub fn problem(&self) -> Option<Problem> {
        match self.state() {
            State::Ready(_) => None,
            State::Unread => Some(Problem::NotRead),
            State::Blocked(problem) => Some(problem),
        }
    }

    /// `token` as it is stored, with the remembered key. Never asks the store.
    pub fn seal(&self, token: &str) -> Option<String> {
        let State::Ready(key) = self.state() else { return None };
        seal_with(&key, token)
    }

    /// A stored token (without its prefix) back as text. Never asks the store.
    pub fn open(&self, encoded: &str) -> Option<String> {
        let State::Ready(key) = self.state() else { return None };
        open_with(&key, encoded)
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
    KEYRING.get_or_init(|| Arc::new(Keyring::new(Box::new(LoginKeychain { service: SERVICE, account: ACCOUNT })))).clone()
}

#[cfg(test)]
fn keyring() -> Arc<Keyring> {
    test_support::ACTIVE.with(|active| active.borrow().clone())
}

/// `T3WOW_TOKEN` stands in for storage, so with it the Keychain stays out of the way.
fn bypassed() -> bool {
    std::env::var("T3WOW_TOKEN").is_ok()
}

/// Reads the key before the companion's main loop starts: the call blocks until the prompt
/// is answered, and `Bridge::tick` runs on the same thread.
pub fn read_at_startup() {
    if bypassed() {
        return;
    }
    let keyring = keyring();
    keyring.ensure();
    if let Some(problem) = keyring.problem() {
        crate::log::line(&format!("Saved T3 sign-ins can't be read: {problem}"));
    }
}

/// Makes sure this run has an answer, for a sign-in about to be saved.
pub fn ensure() {
    keyring().ensure();
}

/// Asks the Keychain again after a refusal: the menu bar's Try the Keychain again.
pub fn retry() {
    keyring().retry();
}

/// Why sign-ins are unavailable although they may well be saved; None when they can be read,
/// or `T3WOW_TOKEN` makes that beside the point. What "signed out" must not be mistaken for.
pub fn blocked() -> Option<Problem> {
    if bypassed() {
        return None;
    }
    keyring().problem()
}

/// Like `blocked`, even with `T3WOW_TOKEN`: saving a sign-in still needs the key.
pub fn problem() -> Option<Problem> {
    keyring().problem()
}

pub fn seal(token: &str) -> Option<String> {
    keyring().seal(token)
}

pub fn open(encoded: &str) -> Option<String> {
    keyring().open(encoded)
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Memory {
        key: Option<Key>,
        read_error: Option<StoreError>,
        create_error: Option<StoreError>,
        miss_next_read: bool,
        reads: u32,
        creates: u32,
    }

    /// The store tests use, cloned so a test can look at it after handing it over.
    #[derive(Clone, Default)]
    pub struct MemoryStore(Arc<Mutex<Memory>>);

    impl MemoryStore {
        pub fn failing(problem: Problem) -> MemoryStore {
            let store = MemoryStore::default();
            store.0.lock().unwrap().read_error = Some(StoreError::Blocked(problem));
            store
        }

        pub fn with_key(key: Key) -> MemoryStore {
            let store = MemoryStore::default();
            store.0.lock().unwrap().key = Some(key);
            store
        }

        pub fn reads(&self) -> u32 {
            self.0.lock().unwrap().reads
        }

        pub fn creates(&self) -> u32 {
            self.0.lock().unwrap().creates
        }

        pub fn key(&self) -> Option<Key> {
            self.0.lock().unwrap().key
        }

        pub fn set_read_error(&self, error: Option<StoreError>) {
            self.0.lock().unwrap().read_error = error;
        }

        pub fn set_create_error(&self, error: Option<StoreError>) {
            self.0.lock().unwrap().create_error = error;
        }

        /// The next read finds nothing even if a key is there, as when another process
        /// creates the item just after this one looked.
        pub fn miss_next_read(&self) {
            self.0.lock().unwrap().miss_next_read = true;
        }
    }

    impl KeyStore for MemoryStore {
        fn read(&self) -> Result<Key, StoreError> {
            let mut memory = self.0.lock().unwrap();
            memory.reads += 1;
            if std::mem::take(&mut memory.miss_next_read) {
                return Err(StoreError::NotFound);
            }
            if let Some(error) = memory.read_error {
                return Err(error);
            }
            memory.key.ok_or(StoreError::NotFound)
        }

        fn create(&self, key: &Key) -> Result<(), StoreError> {
            let mut memory = self.0.lock().unwrap();
            memory.creates += 1;
            if let Some(error) = memory.create_error {
                return Err(error);
            }
            if memory.key.is_some() {
                return Err(StoreError::Duplicate);
            }
            memory.key = Some(*key);
            Ok(())
        }
    }

    fn granted() -> Arc<Keyring> {
        let keyring = Keyring::new(Box::new(MemoryStore::default()));
        keyring.ensure();
        Arc::new(keyring)
    }

    thread_local! {
        /// Each test runs on its own thread, with its own key.
        pub static ACTIVE: RefCell<Arc<Keyring>> = RefCell::new(granted());
    }

    /// Makes `keyring` the one this test's `seal`, `open` and `blocked` use.
    pub fn use_keyring(keyring: Arc<Keyring>) {
        ACTIVE.with(|active| *active.borrow_mut() = keyring);
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::MemoryStore;
    use super::*;

    fn keyring(store: &MemoryStore) -> Keyring {
        Keyring::new(Box::new(store.clone()))
    }

    fn ready(store: &MemoryStore) -> Keyring {
        let keyring = keyring(store);
        keyring.ensure();
        keyring
    }

    fn encoded(stored: &str) -> &str {
        stored.strip_prefix(SEALED).expect("carries the prefix")
    }

    #[test]
    fn a_token_comes_back_and_each_sealing_looks_different() {
        let keyring = ready(&MemoryStore::default());
        let one = keyring.seal("the token").unwrap();
        let two = keyring.seal("the token").unwrap();
        assert_ne!(one, two, "a new nonce every time");
        assert!(!one.contains("the token"));
        assert_eq!(keyring.open(encoded(&one)).as_deref(), Some("the token"));
        assert_eq!(keyring.open(encoded(&two)).as_deref(), Some("the token"));
    }

    #[test]
    fn tampered_or_damaged_ciphertext_fails_cleanly() {
        let keyring = ready(&MemoryStore::default());
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
        let stored = ready(&MemoryStore::default()).seal("the token").unwrap();
        assert_eq!(ready(&MemoryStore::default()).open(encoded(&stored)), None);
    }

    #[test]
    fn several_servers_share_one_key_lookup() {
        let store = MemoryStore::default();
        let keyring = ready(&store);
        let sealed: Vec<String> = (0..5).map(|n| keyring.seal(&format!("token-{n}")).unwrap()).collect();
        for _ in 0..3 {
            keyring.ensure();
            for (n, stored) in sealed.iter().enumerate() {
                assert_eq!(keyring.open(encoded(stored)), Some(format!("token-{n}")));
            }
        }
        assert_eq!(store.reads(), 1);
    }

    #[test]
    fn a_denial_is_remembered_until_a_retry() {
        let store = MemoryStore::failing(Problem::Denied);
        let keyring = keyring(&store);
        keyring.ensure();
        assert_eq!(keyring.problem(), Some(Problem::Denied));
        for _ in 0..5 {
            keyring.ensure();
            assert_eq!(keyring.seal("t"), None);
            assert_eq!(keyring.open("AAAA"), None);
        }
        assert_eq!(store.reads(), 1, "the prompt doesn't come back");
        assert_eq!(store.creates(), 0);

        keyring.retry();
        assert_eq!(store.reads(), 2);
        assert_eq!(keyring.problem(), Some(Problem::Denied), "still denied");

    }

    #[test]
    fn a_retry_that_is_allowed_gets_the_key_that_was_already_there() {
        let key = [7; KEY_LEN];
        let store = MemoryStore::with_key(key);
        store.set_read_error(Some(StoreError::Blocked(Problem::Denied)));
        let keyring = ready(&store);
        assert_eq!(keyring.problem(), Some(Problem::Denied));

        store.set_read_error(None);
        keyring.retry();
        assert_eq!(keyring.problem(), None);
        assert_eq!((store.reads(), store.creates()), (2, 0));
        assert_eq!(store.key(), Some(key));

        // Once there is a key, a retry leaves it alone.
        keyring.retry();
        assert_eq!(store.reads(), 2);
    }

    #[test]
    fn a_read_error_never_creates_a_new_key() {
        for problem in [Problem::Denied, Problem::Locked, Problem::Malformed, Problem::Failed(-1), Problem::Failed(ERR_SEC_ITEM_NOT_FOUND + 1)] {
            let store = MemoryStore::failing(problem);
            let keyring = ready(&store);
            assert_eq!(keyring.problem(), Some(problem));
            assert_eq!(store.creates(), 0, "{problem:?}");
            assert_eq!(store.key(), None, "{problem:?}");
        }
    }

    #[test]
    fn keychain_results_map_to_the_right_answers() {
        assert_eq!(StoreError::from_code(ERR_SEC_ITEM_NOT_FOUND), StoreError::NotFound);
        assert_eq!(StoreError::from_code(ERR_SEC_DUPLICATE_ITEM), StoreError::Duplicate);
        assert_eq!(StoreError::from_code(ERR_SEC_USER_CANCELED), StoreError::Blocked(Problem::Denied));
        assert_eq!(StoreError::from_code(ERR_SEC_AUTH_FAILED), StoreError::Blocked(Problem::Denied));
        assert_eq!(StoreError::from_code(ERR_SEC_INTERACTION_NOT_ALLOWED), StoreError::Blocked(Problem::Locked));
        assert_eq!(StoreError::from_code(-1), StoreError::Blocked(Problem::Failed(-1)));
    }

    #[test]
    fn not_found_creates_exactly_one_key_and_the_next_run_finds_it() {
        let store = MemoryStore::default();
        let first = ready(&store);
        first.ensure();
        assert_eq!((store.reads(), store.creates()), (1, 1));
        let stored = first.seal("the token").unwrap();

        let second = ready(&store);
        assert_eq!(store.creates(), 1, "the next run reads it");
        assert_eq!(second.open(encoded(&stored)).as_deref(), Some("the token"));
    }

    #[test]
    fn a_key_another_process_just_created_is_the_one_used() {
        let theirs = [9; KEY_LEN];
        let store = MemoryStore::with_key(theirs);
        // This process looks first and finds nothing; the other one then creates the item.
        store.miss_next_read();
        let keyring = ready(&store);
        assert_eq!(keyring.problem(), None);
        assert_eq!(store.key(), Some(theirs), "theirs wasn't replaced");
        let stored = keyring.seal("the token").unwrap();
        assert_eq!(ready(&MemoryStore::with_key(theirs)).open(encoded(&stored)).as_deref(), Some("the token"), "it uses their key");
    }

    #[test]
    fn a_failed_create_is_a_problem_and_not_a_key_only_in_memory() {
        let store = MemoryStore::default();
        store.set_create_error(Some(StoreError::Blocked(Problem::Locked)));
        let keyring = ready(&store);
        assert_eq!(keyring.problem(), Some(Problem::Locked));
        assert_eq!(keyring.seal("t"), None, "a key nothing kept would lose the token at the next start");
    }

    #[test]
    fn sealing_and_opening_never_ask_the_store_themselves() {
        let store = MemoryStore::default();
        let keyring = keyring(&store);
        assert_eq!(keyring.seal("t"), None);
        assert_eq!(keyring.open("AAAA"), None);
        assert_eq!(keyring.problem(), Some(Problem::NotRead));
        assert_eq!((store.reads(), store.creates()), (0, 0));
    }

    #[test]
    #[ignore = "uses the real login keychain (its own test item, deleted afterwards); run by hand"]
    fn the_real_login_keychain_keeps_a_key() {
        use security_framework::passwords::delete_generic_password;
        const TEST_SERVICE: &str = "t3wow.companion.manual-test";
        let store = || Box::new(LoginKeychain { service: TEST_SERVICE, account: ACCOUNT });
        let _ = delete_generic_password(TEST_SERVICE, ACCOUNT);

        assert_eq!(store().read().err(), Some(StoreError::NotFound));
        let first = Keyring::new(store());
        first.ensure();
        assert_eq!(first.problem(), None);
        let sealed = first.seal("the token").unwrap();

        let second = Keyring::new(store());
        second.ensure();
        assert_eq!(second.problem(), None);
        assert_eq!(second.open(encoded(&sealed)).as_deref(), Some("the token"), "a second run reads the same key");
        assert_eq!(store().create(&[0; KEY_LEN]).err(), Some(StoreError::Duplicate), "an existing key is never replaced");

        delete_generic_password(TEST_SERVICE, ACCOUNT).unwrap();
    }
}
