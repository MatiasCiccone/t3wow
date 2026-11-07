//! The bridge between the game and T3: runs the game's commands against T3, reads T3's
//! threads, and publishes what the game shows into slot files, raising the signal files
//! that tell the game to read them.

use crate::codec::decode_cells;
use crate::config::Trust;
use crate::lua::{plain, to_lua, wow_text};
use crate::slots::{SlotWriter, SIGNAL_COUNT, SLOT_COUNT};
use crate::t3::{T3Error, T3};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

const WORKING: [&str; 4] = ["preparing", "queued", "starting", "running"];
const MIN_PUBLISH_GAP_MS: i64 = 1500;
/// Routine changes while the window is open: one update at most this often (200 slots
/// then last about 35 minutes of watching a busy thread, not 15).
const ROUTINE_GAP_MS: i64 = 10_000;
/// While waiting, the game moves to the next signal file every SIGNAL_PERIOD seconds.
/// Same clock, same arithmetic: keep in step with addon/T3WoW/Link.lua.
pub const SIGNAL_PERIOD: i64 = 25;
const VIEW_MESSAGES: usize = 24;
const VIEW_CHARS: usize = 40_000;
/// The open thread is read in full at least this often; in between, only what's new.
const VIEW_FULL_READ_MS: i64 = 300_000;
/// An open thread that isn't in the lists (no updatedAt to compare) is read again this often.
const UNLISTED_VIEW_TTL_MS: i64 = 20_000;

/// Why the game was told to stop watching signals.
const SIGNALS_OUT: &str = "this WoW session used all its signal files; restart WoW for updates within a second";

/// Failed reads in a row before an active thread off the lists stops being watched.
const UNLISTED_MISSES: u32 = 5;

/// How long a working thread's pending-question count is trusted without a re-read.
const PENDING_RECHECK_MS: i64 = 20_000;

fn working(status: &str) -> bool {
    WORKING.contains(&status)
}

/// Waiting on a question or an approval.
fn needs_you(status: &str) -> bool {
    status == "waiting" || status == "approval"
}

/// T3's sidebar files subagent threads, and threads an agent launched, under the thread
/// they came from. Linked machines don't always apply includeSubagents, so check here too.
/// Subagents never show; threads an agent launched show under the thread that launched
/// them, as in T3's sidebar, and don't count in their group.
fn is_subagent(thread: &Value) -> bool {
    thread["parentThreadId"].is_string()
}

fn on_sidebar(thread: &Value) -> bool {
    !is_subagent(thread) && thread["createdBy"] != "agent"
}

fn hash_of(value: &Value) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.to_string().hash(&mut hasher);
    hasher.finish()
}

fn str_of(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "This PC".into())
}

/// ISO 8601 ("2026-11-07T00:22:41.923Z", or with an offset) to epoch seconds; 0 if unreadable.
pub fn iso_seconds(iso: &str) -> i64 {
    fn num(s: &str, range: std::ops::Range<usize>) -> Option<i64> {
        s.get(range)?.parse().ok()
    }
    let parse = || -> Option<i64> {
        let (y, mo, d) = (num(iso, 0..4)?, num(iso, 5..7)?, num(iso, 8..10)?);
        let (h, mi, s) = (num(iso, 11..13)?, num(iso, 14..16)?, num(iso, 17..19)?);
        // Days from civil (Howard Hinnant's algorithm).
        let y = if mo <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        let mut total = days * 86_400 + h * 3600 + mi * 60 + s;
        let rest = iso.get(19..)?;
        let zone = rest.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
        if let Some(sign) = zone.chars().next().filter(|c| *c == '+' || *c == '-') {
            // "+03:00", "+0300" or "+03".
            let digits: String = zone[1..].chars().filter(char::is_ascii_digit).collect();
            let (oh, om) = (num(&digits, 0..2)?, num(&digits, 2..4).unwrap_or(0));
            let offset = oh * 3600 + om * 60;
            total -= if sign == '+' { offset } else { -offset };
        }
        Some(total)
    };
    if iso.is_empty() {
        0
    } else {
        parse().unwrap_or(0)
    }
}

/// Saved between runs in state.json (same shape as the earlier Node companion's).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LinkState {
    pub wow_key: Option<String>,
    pub ui_session: Option<String>,
    pub slot: u32,
    /// Where the game's current wait started: signal file number and epoch seconds.
    pub signal: u32,
    pub signal_time: i64,
    pub acked: BTreeMap<String, u64>,
    pub client_key: Option<String>,
    pub polling: Option<bool>,
}

impl Default for LinkState {
    fn default() -> LinkState {
        LinkState { wow_key: None, ui_session: None, slot: 1, signal: 1, signal_time: 0, acked: BTreeMap::new(), client_key: None, polling: None }
    }
}

pub enum CaptureMessage {
    /// The WoW client process changed: Some(key) when one runs, None when none does.
    Wow(Option<String>),
    /// Cell values read from the strip.
    Strip(Vec<u8>),
}

#[derive(Clone)]
struct Env {
    id: String,
    name: String,
    ok: bool,
}

#[derive(Clone)]
struct View {
    env: String,
    id: String,
    depth: usize,
}

/// The open thread's messages as last read, with what decides whether reading on from
/// the last one is enough.
struct ViewCache {
    env: String,
    id: String,
    depth: usize,
    item_count: usize,
    status: String,
    /// When the last full read was.
    read_at: i64,
    messages: Vec<Value>,
    /// There are older messages than these.
    earlier: bool,
}

/// What the tray shows.
#[derive(Clone, Debug, Default)]
pub struct BridgeStatus {
    pub t3_ok: bool,
    pub t3_problem: Option<String>,
    pub auth_needed: bool,
    pub wow_running: bool,
    pub in_game: bool,
    pub signals: bool,
    pub machines: usize,
    /// The next slot the game loads (1..=200; past 200 it has used them all).
    pub slot: u32,
    /// This game process used up its signal files: updates come on a timer until WoW restarts.
    pub signals_out: bool,
    /// Writing the game's update files keeps failing.
    pub write_problem: Option<String>,
}

/// After this many failed publishes in a row, the tray shows the error.
const WRITE_FAILURES_SHOWN: u32 = 3;

pub struct Bridge {
    client: Arc<dyn T3>,
    slots: Box<dyn SlotWriter>,
    state_file: Option<PathBuf>,
    log: Box<dyn Fn(&str) + Send>,
    now: Box<dyn Fn() -> i64 + Send>,
    /// Opens a link with Windows (T3's t3code:// links); replaced in tests.
    pub opener: Box<dyn Fn(&str) + Send>,
    pub link: LinkState,
    window_open: bool,
    view: Option<View>,
    envs: Vec<Env>,
    local_env_id: Option<String>,
    /// Per machine: its sidebar threads, its projects and how many threads it has settled.
    threads: BTreeMap<String, Vec<Value>>,
    projects: BTreeMap<String, Vec<Value>>,
    settled_totals: BTreeMap<String, u64>,
    /// Per machine: which of its three lists have more threads than were listed.
    list_more: BTreeMap<String, [bool; LIST_COUNT]>,
    /// How many threads each of the three lists asks for; the game's Show more raises them
    /// for the UI session.
    list_limits: [usize; LIST_COUNT],
    view_data: Option<Value>,
    view_key: Option<String>,
    view_cache: Option<ViewCache>,
    /// Counts commands that changed what the open thread shows (see `view_changed`).
    view_gen: u64,
    notes: Vec<Value>,
    events: Vec<Value>,
    event_seq: i64,
    last_status: HashMap<String, String>,
    /// Active threads off the lists whose last reads failed, and how many times in a row.
    unlisted_misses: HashMap<String, u32>,
    /// What working threads wait on, keyed by thread: T3's list says "running" while an
    /// agent waits on a question or an approval, and only a thread read has the count.
    pending_counts: HashMap<String, Pending>,
    /// The thread that launched each agent-launched thread (None: unknown), read once.
    launchers: HashMap<String, Option<String>>,
    /// Machines whose threads were listed at least once since the companion started.
    listed_envs: HashSet<String>,
    /// Commands waiting to run: client key, the UI session they came in, the command.
    commands: VecDeque<(String, String, Value)>,
    /// Per client key, the highest command queued to run but not yet run (not saved: a
    /// command only counts as done once it ran).
    queued: HashMap<String, u64>,
    last_publish_at: i64,
    last_published_hash: Option<u64>,
    last_urgent_hash: Option<u64>,
    /// When the next poll starts, and the one under way (if any).
    poll_at: i64,
    poll_run: Option<PollRun>,
    /// A command ran during a poll: poll again once it ends.
    repoll: bool,
    /// Failed polls in a row, which space the next ones out.
    poll_failures: u32,
    /// The last thread-list error per machine, logged once.
    list_errors: HashMap<String, String>,
    slow_at: i64,
    auth_error: Option<String>,
    poll_error: Option<String>,
    polled_ok: bool,
    /// The question shown in the game for the open thread, with the ids T3 needs back.
    pending_ask: Option<PendingAsk>,
    /// Outcomes of this UI session's recent commands, with when they ran.
    outcomes: Vec<(i64, Value)>,
    /// Why signals stopped (this game process used every signal file), until WoW restarts.
    signals_out: Option<String>,
    /// The UI session whose running out of slots was logged.
    slots_out_logged: Option<String>,
    /// The last failure writing update files, and how many publishes in a row failed.
    write_error: Option<String>,
    write_failures: u32,
    /// The tray's "Commands from the game": what the game may make T3 do.
    trust: Trust,
}

/// A working thread's pending requests, as last read.
#[derive(Clone, Debug)]
struct Pending {
    /// The thread's `updatedAt` when read, and when that was.
    updated: String,
    read_at: i64,
    /// What it waits on: None, or the status the game shows ("waiting" for a question,
    /// "approval" for a permission prompt, which T3's pending list leaves out).
    status: Option<&'static str>,
}

#[derive(Clone, Debug)]
struct PendingQuestion {
    key: String,
    labels: Vec<String>,
    multi: bool,
}

#[derive(Clone, Debug)]
struct PendingAsk {
    env: String,
    thread: String,
    token: String,
    request_id: String,
    questions: Vec<PendingQuestion>,
}

/// The machine part of a "{env}:{thread}" key.
fn env_of(key: &str) -> &str {
    key.split_once(':').map_or(key, |(env, _)| env)
}

fn with_args(base: Value, extra: &Map<String, Value>) -> Value {
    let mut base = base;
    if let Value::Object(map) = &mut base {
        for (k, v) in extra {
            map.insert(k.clone(), v.clone());
        }
    }
    base
}

/// A machine's sidebar threads, its settled count, and which lists have more.
type ThreadLists = Result<(Vec<Value>, u64, [bool; LIST_COUNT]), T3Error>;

/// A whole poll gets this long; past it, optional reads wait for the next one.
const POLL_DEADLINE_MS: i64 = 20_000;
/// Command outcomes stay in the updates this long (the game takes each once), at most this many.
const OUTCOME_KEEP_MS: i64 = 60_000;
const OUTCOMES_KEPT: usize = 8;
/// How long one tick runs poll steps before it returns.
const TICK_BUDGET_MS: i64 = 300;
/// Mid-poll, publish only once the poll has run this long.
const PUBLISH_MIDPOLL_MS: i64 = 2000;
/// The open thread's messages are read at most this many pages a pass, and widened at
/// most this many passes.
const VIEW_PAGES: usize = 10;
const VIEW_PASSES: usize = 4;

/// A poll of T3, one step at a time. A step makes one request at most (a list step makes
/// one per machine, all at once), so between two requests the caller gets a turn: the
/// game's commands, the tray's controls and the capture never wait for more than one.
struct PollRun {
    started: i64,
    steps: VecDeque<Step>,
    /// Each machine's list replies so far, while the lists are read.
    lists: Vec<(String, Result<Vec<Value>, T3Error>)>,
    /// The machine whose lists are being taken in.
    listed: Option<Listed>,
    /// The open thread, while it's read.
    reading: Option<ViewRead>,
}

enum Step {
    /// The linked machines, then the server's own name (then each machine's projects).
    Machines,
    ServerName(Value),
    Projects(String),
    /// A page of one of the three thread lists, every machine's at once (a list longer
    /// than T3's page takes a step per page).
    List(usize),
    /// One machine's lists, as read. Then, for it: each working thread's question check
    /// (and what it waits on), each launcher, the status changes, each active thread that
    /// left the lists, and keeping the lists.
    Machine(String, ThreadLists),
    Question(usize),
    QuestionKind(usize, i64),
    Launcher(usize),
    Track,
    Unlisted(String, String),
    Keep,
    /// The open thread: its head, its messages a page at a time, its question.
    View,
    ViewPage,
    ViewAsk,
    ViewAskRead(String),
}

/// One machine's lists while they're taken in.
struct Listed {
    env: String,
    threads: Vec<Value>,
    settled_total: u64,
    more: [bool; LIST_COUNT],
    /// The working threads looked at: their pending counts stay, the machine's others go.
    seen: Vec<String>,
}

/// The open thread while it's read.
struct ViewRead {
    view: View,
    key: String,
    /// `view_gen` when the read began: a command that changed the view since drops it.
    gen: u64,
    thread: Value,
    /// When the head was read (a full read's time).
    began: i64,
    /// Reading on from this cache (same thread, safe to); None: a full read.
    cache: Option<ViewCache>,
    /// This pass's messages so far, where the next page starts, and pages read.
    messages: Vec<Value>,
    after: Option<u64>,
    pages: usize,
    /// A full read: the first item it reads, how many items back that is, passes done.
    start: usize,
    window: usize,
    passes: usize,
    /// The view for the game, once the messages are in, waiting on its question.
    built: Option<Value>,
}

/// T3's sidebar is made of three lists per machine (open, snoozed, settled), read in turn.
const LIST_COUNT: usize = 3;
/// The lists as the game names them.
const LIST_NAMES: [&str; LIST_COUNT] = ["open", "snoozed", "settled"];
/// How many threads each list shows at first, and at most after Show more (each one
/// doubles it), which keeps an update's size reasonable.
const LIST_LIMITS: [usize; LIST_COUNT] = [80, 25, 25];
const LIST_CAPS: [usize; LIST_COUNT] = [320, 200, 200];
/// T3 lists at most this many threads a request; past it, a list is read a page at a time.
const LIST_PAGE: usize = 100;

/// The page a machine's list needs: its machine argument, where it starts, how many threads.
type ListPage = (Map<String, Value>, Option<u64>, usize);

/// A page of one of a machine's three lists: `limit` threads from `cursor` on.
fn fetch_list(client: &dyn T3, n: usize, cursor: Option<u64>, limit: usize, env_arg: &Map<String, Value>) -> Result<Value, T3Error> {
    let which = match n {
        0 => json!({ "settled": false, "snoozed": false }),
        1 => json!({ "settled": false, "snoozed": true }),
        _ => json!({ "settled": true }),
    };
    let mut base = with_args(json!({ "allProjects": true, "includeSubagents": false, "limit": limit }), which.as_object().unwrap());
    if let Some(cursor) = cursor {
        base["cursor"] = json!(cursor);
    }
    client.call("t3_thread_list", with_args(base, env_arg))
}

/// How many threads a list read so far holds.
fn listed_count(list: &Value) -> usize {
    list["threads"].as_array().map_or(0, Vec::len)
}

/// Where the next page of a list read so far starts, if it needs one to reach `limit`.
fn next_page(list: &Value, limit: usize) -> Option<u64> {
    list["nextCursor"].as_u64().filter(|_| listed_count(list) < limit)
}

/// A list has more threads than it holds: by its total, or, without one, by its cursor.
fn has_more(list: &Value) -> bool {
    match list["total"].as_u64() {
        Some(total) => total > listed_count(list) as u64,
        None => list["nextCursor"].is_u64(),
    }
}

/// A machine's sidebar threads from its three lists, its settled count, and which lists
/// have more.
fn combine_lists(lists: &[Value]) -> (Vec<Value>, u64, [bool; LIST_COUNT]) {
    let items = |n: usize| lists.get(n).and_then(|v| v["threads"].as_array()).cloned().unwrap_or_default();
    let mut threads: Vec<Value> = items(0).into_iter().filter(|t| t["snoozed"] != true).collect();
    threads.extend(items(1).into_iter().filter(|t| t["snoozed"] == true));
    threads.extend(items(2));
    threads.retain(|t| !is_subagent(t));
    let settled_listed = items(2).iter().filter(|t| on_sidebar(t)).count() as u64;
    let total = lists.get(2).and_then(|v| v["total"].as_u64()).unwrap_or(0);
    let more = [0, 1, 2].map(|n| lists.get(n).is_some_and(has_more));
    (threads, total.max(settled_listed), more)
}
impl Bridge {
    pub fn new(
        client: Arc<dyn T3>,
        slots: Box<dyn SlotWriter>,
        state_file: Option<PathBuf>,
        log: Box<dyn Fn(&str) + Send>,
        now: Box<dyn Fn() -> i64 + Send>,
    ) -> Bridge {
        let link = state_file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Bridge {
            client,
            slots,
            state_file,
            log,
            now,
            opener: Box::new(crate::t3::open_browser),
            link,
            window_open: false,
            view: None,
            envs: vec![Env { id: String::new(), name: hostname(), ok: true }],
            local_env_id: None,
            threads: BTreeMap::new(),
            projects: BTreeMap::new(),
            settled_totals: BTreeMap::new(),
            list_more: BTreeMap::new(),
            list_limits: LIST_LIMITS,
            view_data: None,
            view_key: None,
            view_cache: None,
            view_gen: 0,
            notes: Vec::new(),
            events: Vec::new(),
            event_seq: 0,
            last_status: HashMap::new(),
            unlisted_misses: HashMap::new(),
            pending_counts: HashMap::new(),
            listed_envs: HashSet::new(),
            launchers: HashMap::new(),
            commands: VecDeque::new(),
            queued: HashMap::new(),
            last_publish_at: 0,
            last_published_hash: None,
            last_urgent_hash: None,
            poll_at: 0,
            poll_run: None,
            repoll: false,
            poll_failures: 0,
            list_errors: HashMap::new(),
            slow_at: 0,
            auth_error: None,
            poll_error: None,
            polled_ok: false,
            pending_ask: None,
            outcomes: Vec::new(),
            signals_out: None,
            slots_out_logged: None,
            write_error: None,
            write_failures: 0,
            trust: Trust::Full,
        }
    }

    /// Applies the tray's "Commands from the game" from the next command on.
    pub fn set_trust(&mut self, trust: Trust) {
        if trust != self.trust {
            self.log(&format!("Commands from the game: {}", trust.label()));
            self.trust = trust;
        }
    }

    pub fn set_client(&mut self, client: Arc<dyn T3>) {
        self.client = client;
        self.auth_error = None;
        self.poll_run = None;
        self.poll_failures = 0;
        self.poll_at = 0;
        self.slow_at = 0;
    }

    /// T3 answers as a different server than before (`login --origin` pointed the companion
    /// elsewhere): drop what was read from the old one (its machines, threads, projects, the
    /// open thread), which the new one may not have.
    fn forget_server(&mut self) {
        self.view = None;
        self.view_cache = None;
        self.view_data = None;
        self.view_key = None;
        self.pending_ask = None;
        self.threads.clear();
        self.projects.clear();
        self.settled_totals.clear();
        self.list_more.clear();
        self.last_status.clear();
        self.unlisted_misses.clear();
        self.pending_counts.clear();
        self.launchers.clear();
        self.listed_envs.clear();
    }

    fn now_ms(&self) -> i64 {
        (self.now)()
    }

    fn now_seconds(&self) -> i64 {
        self.now_ms().div_euclid(1000)
    }

    /// The signal file the game watches at `now_s`, by the same arithmetic as Link.lua.
    fn watched_signal(&self, now_s: i64) -> u32 {
        let start = if self.link.signal_time != 0 { self.link.signal_time } else { now_s };
        self.link.signal + (now_s - start).max(0).div_euclid(SIGNAL_PERIOD) as u32
    }

    fn save_state(&self) {
        if let Some(file) = &self.state_file {
            let _ = std::fs::write(file, serde_json::to_string(&self.link).unwrap_or_default());
        }
    }

    fn log(&self, message: &str) {
        (self.log)(message);
    }

    fn note(&mut self, text: &str) {
        let at = self.now_seconds();
        self.notes.push(json!({ "at": at, "x": plain(text, 300) }));
        let excess = self.notes.len().saturating_sub(4);
        self.notes.drain(..excess);
    }

    pub fn status(&self) -> BridgeStatus {
        BridgeStatus {
            t3_ok: self.polled_ok && self.auth_error.is_none() && self.poll_error.is_none(),
            t3_problem: self.auth_error.clone().or_else(|| self.poll_error.clone()),
            auth_needed: self.auth_error.is_some(),
            wow_running: self.link.wow_key.as_deref().is_some_and(|k| !k.is_empty()),
            in_game: self.link.ui_session.is_some(),
            signals: self.link.polling == Some(false) && self.signals_out.is_none(),
            machines: self.envs.iter().filter(|e| e.ok).count(),
            slot: self.link.slot,
            signals_out: self.signals_out.is_some(),
            write_problem: self.write_error.clone().filter(|_| self.write_failures >= WRITE_FAILURES_SHOWN),
        }
    }

    // ---- input from the capture ----

    pub fn on_capture(&mut self, message: CaptureMessage) {
        match message {
            CaptureMessage::Wow(key) => {
                let key = key.unwrap_or_default();
                if Some(&key) != self.link.wow_key.as_ref() {
                    self.log(&if key.is_empty() { "WoW not running".to_string() } else { format!("WoW running ({key})") });
                    // A new client process hasn't loaded any signal font: all start blank again.
                    self.slots.clear_signals();
                    self.slots.flip_probe(false);
                    self.signals_out = None;
                    self.link.signal = 1;
                    self.link.signal_time = self.now_seconds();
                    self.link.wow_key = Some(key);
                    self.link.ui_session = None;
                    self.save_state();
                }
            }
            CaptureMessage::Strip(cells) => {
                if let Some(payload) = decode_cells(&cells).and_then(|frame| serde_json::from_str::<Value>(&frame.payload).ok()) {
                    self.on_frame(&payload);
                }
            }
        }
    }

    pub fn on_frame(&mut self, frame: &Value) {
        let (Some(k), Some(u)) = (frame["k"].as_str(), frame["u"].as_str()) else { return };
        let l = frame["l"].as_u64().unwrap_or(0) as u32;
        // Until the game watches signals (it learns the starting file from its first update,
        // and some clients can't use them) it loads slots on a timer and reports where it is.
        let polling = frame["p"] == 1;
        if Some(u) != self.link.ui_session.as_deref() || Some(k) != self.link.client_key.as_deref() {
            // New UI session (login or /reload): every slot can load again. Signal files raised
            // earlier in this game process stay real, so the new wait starts past all of them.
            let now_s = self.now_seconds();
            self.link.ui_session = Some(u.to_string());
            self.link.client_key = Some(k.to_string());
            self.link.slot = l.max(1);
            self.link.signal = self.watched_signal(now_s) + 2;
            self.link.signal_time = now_s;
            self.last_published_hash = None;
            self.last_urgent_hash = None;
            self.link.polling = None;
            // The game's command numbers live in its saved variables, which a crash rolls
            // back: start counting from what this session sends. A command run twice this
            // way reaches T3 with the same clientRequestId, which T3 ignores.
            if let Some(lowest) = frame["q"].as_array().into_iter().flatten().filter_map(|c| c["s"].as_u64()).min() {
                self.link.acked.insert(k.to_string(), lowest.saturating_sub(1));
            }
            self.queued.remove(k);
            self.outcomes.clear();
            // Show more lasts for the UI session it was asked in.
            self.list_limits = LIST_LIMITS;
            self.log(&format!("Game UI connected (session {u}, next slot {})", self.link.slot));
        } else if l > 0 {
            // The game draws a frame with its current slot count, so it is the authority on
            // where it loads next (it may have loaded extra slots on its own).
            self.link.slot = l;
        }
        if Some(polling) != self.link.polling {
            if !polling {
                self.log(&format!("Game watching signals (file {}): updates arrive in about a second", frame["n"]));
            } else if self.link.polling == Some(false) {
                self.log("Game stopped watching signals: it checks for updates on a timer");
            }
            self.link.polling = Some(polling);
        }
        let seen = self.link.acked.get(k).copied().unwrap_or(0).max(self.queued.get(k).copied().unwrap_or(0));
        for command in frame["q"].as_array().into_iter().flatten() {
            let s = command["s"].as_u64().unwrap_or(0);
            if s <= seen {
                continue;
            }
            let entry = self.queued.entry(k.to_string()).or_insert(0);
            *entry = (*entry).max(s);
            self.commands.push_back((k.to_string(), u.to_string(), command.clone()));
        }
        self.save_state();
    }

    // ---- commands from the game ----

    fn env_arg(&self, env: &str) -> Map<String, Value> {
        let mut map = Map::new();
        if !env.is_empty() && Some(env) != self.local_env_id.as_deref() {
            map.insert("environmentId".into(), json!(env));
        }
        map
    }

    /// Runs one command from the game. Returns its outcome for the game, if it has one to
    /// know: a failure (so the window can put the text back), or the thread a "new" started.
    fn run(&mut self, client_key: &str, command: Value) -> Option<Value> {
        // No machine named: the server's own.
        let env = match str_of(&command["e"]) {
            "" => self.local_env_id.clone().unwrap_or_default(),
            env => env.to_string(),
        };
        let id = str_of(&command["id"]).to_string();
        let message = str_of(&command["m"]).to_string();
        let kind = str_of(&command["t"]).to_string();
        // The command's number plus a hash of it: the same command sent again (after a
        // /reload) is ignored by T3, while a new one that reuses a number after a crash isn't.
        let request_id = format!("t3wow:{client_key}:{}:{:08x}", command["s"], hash_of(&command) as u32);
        // Any addon can draw commands, so the tray setting decides what the game may do.
        if !self.trust.allows(&kind) {
            let reason = format!("blocked by the T3 WoW tray setting \"Commands from the game\" ({})", self.trust.label());
            self.log(&format!("{kind} from the game {reason}"));
            self.note(&format!("{kind}: {reason}"));
            return Some(json!({ "s": command["s"], "ok": false, "x": reason }));
        }
        let env_arg = self.env_arg(&env);
        let mut started: Option<String> = None;
        let result: Result<(), T3Error> = (|| {
            match kind.as_str() {
                "hello" | "ui" => self.window_open = command["o"] == true,
                "open" => {
                    self.view = (!id.is_empty()).then(|| View { env: env.clone(), id: id.clone(), depth: 1 });
                    self.view_changed();
                }
                "more" => {
                    if let Some(view) = &mut self.view {
                        view.depth += 1;
                    }
                    self.view_changed();
                }
                "send" if !id.is_empty() && !message.is_empty() => {
                    let args = json!({ "threadId": id, "message": message, "mode": "auto", "clientRequestId": request_id });
                    self.client.call("t3_thread_send", with_args(args, &env_arg))?;
                    self.note("Sent.");
                    let depth = self.view.as_ref().filter(|v| v.id == id).map(|v| v.depth).unwrap_or(1);
                    self.view = Some(View { env: env.clone(), id: id.clone(), depth });
                    self.view_changed();
                }
                "new" if !message.is_empty() => {
                    let title: String = message.split('\n').next().unwrap_or("").chars().take(60).collect();
                    let mut args = json!({ "title": title, "message": message, "clientRequestId": request_id });
                    match command["p"].as_str() {
                        Some(project) => args["projectId"] = json!(project),
                        None => args["scratch"] = json!(true),
                    }
                    let launched = self.client.call("t3_thread_launch", with_args(args, &env_arg))?;
                    let new_id = launched["threadId"].as_str().or(launched["thread"]["threadId"].as_str()).map(str::to_string);
                    self.note(&format!("Started \"{title}\"."));
                    if let Some(new_id) = new_id {
                        self.view = Some(View { env: env.clone(), id: new_id.clone(), depth: 1 });
                        started = Some(new_id);
                    }
                    self.view_changed();
                }
                "settle" | "unsettle" if !id.is_empty() => {
                    self.client.call("t3_thread_organize", with_args(json!({ "action": kind, "threadId": id }), &env_arg))?;
                    self.note(if kind == "settle" { "Settled." } else { "Moved back to open." });
                }
                "stop" if !id.is_empty() => {
                    self.client.call("t3_thread_interrupt", with_args(json!({ "threadId": id, "clientRequestId": request_id }), &env_arg))?;
                    self.note("Stopping.");
                }
                // Brings T3 to the front on that thread (T3's own t3code:// link), for what only
                // T3 can do, like approving a command.
                "t3" => {
                    let machine = if env.is_empty() { self.local_env_id.clone().unwrap_or_default() } else { env.clone() };
                    let link = if id.is_empty() || machine.is_empty() {
                        "t3code://open".to_string()
                    } else {
                        format!("t3code://threads/{}/{}", crate::t3::percent_encode(&machine), crate::t3::percent_encode(&id))
                    };
                    (self.opener)(&link);
                    self.note("Opened in T3.");
                }
                "probe" => {
                    let changed = self.slots.flip_probe(true);
                    self.log(&if changed.is_empty() { "probe: no probe files; reinstall the addon".into() } else { format!("probe: made {} files real", changed.join(", ")) });
                }
                "answer" if !id.is_empty() => {
                    let ask = self
                        .pending_ask
                        .clone()
                        .filter(|a| a.thread == id && a.env == env && a.token == str_of(&command["k"]))
                        .ok_or_else(|| T3Error::Other("that question was already answered or has changed".into()))?;
                    let answers = Self::answers_for(&ask, &command["a"]);
                    if answers.is_empty() {
                        return Err(T3Error::Other("pick an answer first".into()));
                    }
                    let args = json!({ "action": "respond", "threadId": id, "requestId": ask.request_id, "answers": answers });
                    self.client.call("t3_pending_request", with_args(args, &env_arg))?;
                    self.pending_counts.remove(&format!("{env}:{id}"));
                    self.note("Answered.");
                    self.pending_ask = None;
                    self.view_changed();
                }
                "refresh" => self.view_changed(),
                // Show more: the next poll lists more of that group, up to its cap.
                "list" => {
                    if let Some(n) = LIST_NAMES.iter().position(|g| *g == str_of(&command["g"])) {
                        self.list_limits[n] = (self.list_limits[n] * 2).min(LIST_CAPS[n]);
                    }
                }
                _ => return Ok(()),
            }
            Ok(())
        })();
        // Look at T3 again now (or as soon as the poll under way ends).
        self.poll_at = 0;
        self.repoll = self.poll_run.is_some();
        match result {
            Err(error) => {
                self.log(&format!("{kind} failed: {error}"));
                self.note(&format!("{kind} failed: {error}"));
                Some(json!({ "s": command["s"], "ok": false, "x": plain(&error.to_string(), 200) }))
            }
            Ok(()) => started.map(|id| json!({ "s": command["s"], "ok": true, "env": env, "id": id })),
        }
    }

    // ---- reading T3 ----

    /// Starts a poll: its steps run from `tick`, a few at a time.
    fn start_poll(&mut self) {
        let t = self.now_ms();
        let mut steps = VecDeque::new();
        if t >= self.slow_at {
            self.slow_at = t + 60_000;
            steps.push_back(Step::Machines);
        }
        steps.push_back(Step::List(0));
        steps.push_back(Step::View);
        self.poll_run = Some(PollRun { started: t, steps, lists: Vec::new(), listed: None, reading: None });
        // This poll covers whatever asked for one.
        self.repoll = false;
    }

    /// Past the poll's deadline, optional reads (question checks, launchers, projects) wait
    /// for the next poll.
    fn out_of_time(&self) -> bool {
        self.poll_run.as_ref().is_some_and(|run| self.now_ms() - run.started > POLL_DEADLINE_MS)
    }

    /// Puts steps next in the poll, in this order.
    fn push_steps(&mut self, steps: Vec<Step>) {
        if let Some(run) = self.poll_run.as_mut() {
            for step in steps.into_iter().rev() {
                run.steps.push_front(step);
            }
        }
    }

    /// Runs the poll's next step; the last one, or a failed one, ends the poll.
    fn poll_step(&mut self) {
        let Some(step) = self.poll_run.as_mut().and_then(|run| run.steps.pop_front()) else {
            return self.finish_poll(Ok(()));
        };
        let result = match step {
            Step::Machines => self.read_links(),
            Step::ServerName(links) => self.read_machines(links),
            Step::Projects(env) => {
                self.refresh_projects(&env);
                Ok(())
            }
            Step::List(n) => {
                self.read_lists(n);
                Ok(())
            }
            Step::Machine(env, result) => self.take_lists(env, result),
            Step::Question(i) => self.check_question(i),
            Step::QuestionKind(i, read_at) => self.check_waiting_on(i, read_at),
            Step::Launcher(i) => self.find_launcher(i),
            Step::Track => {
                self.track_listed();
                Ok(())
            }
            Step::Unlisted(key, status) => self.check_unlisted(key, status),
            Step::Keep => {
                self.keep_lists();
                Ok(())
            }
            Step::View => self.refresh_view(),
            Step::ViewPage => self.read_view_page(),
            Step::ViewAsk => self.read_view_ask(),
            Step::ViewAskRead(request_id) => self.read_view_request(request_id),
        };
        match result {
            Err(error) => self.finish_poll(Err(error)),
            Ok(()) if self.poll_run.as_ref().is_some_and(|run| run.steps.is_empty()) => self.finish_poll(Ok(())),
            Ok(()) => {}
        }
    }

    /// Ends the poll and schedules the next one from now: sooner with the window open,
    /// later after each failure in a row.
    fn finish_poll(&mut self, result: Result<(), T3Error>) {
        self.poll_run = None;
        let t = self.now_ms();
        let every = if self.window_open { 4000 } else { 12_000 };
        match result {
            Ok(()) => {
                self.polled_ok = true;
                self.poll_error = None;
                self.auth_error = None;
                self.poll_failures = 0;
                self.poll_at = t + every;
            }
            Err(T3Error::Auth(message)) => {
                if self.auth_error.as_ref() != Some(&message) {
                    self.log(&message);
                }
                self.auth_error = Some(message);
                self.poll_at = t + 30_000;
            }
            Err(T3Error::Other(message)) => {
                if self.poll_error.as_ref() != Some(&message) {
                    self.log(&format!("poll: {message}"));
                }
                self.poll_error = Some(message);
                self.poll_failures += 1;
                self.poll_at = t + (every << self.poll_failures.min(4)).min(60_000);
            }
        }
        // A command ran meanwhile: look again right away.
        if std::mem::take(&mut self.repoll) {
            self.poll_at = 0;
        }
    }

    /// A page of the `n`th of the three thread lists from every machine still answering,
    /// all at once. A machine whose list needs another page to reach its limit gets it in
    /// the next step; after the last list, each machine's lists are taken in, one machine
    /// after another.
    fn read_lists(&mut self, n: usize) {
        let mut lists = match self.poll_run.as_mut() {
            Some(run) if n > 0 || !run.lists.is_empty() => std::mem::take(&mut run.lists),
            Some(_) => self.envs.iter().filter(|e| e.ok).map(|e| (e.id.clone(), Ok(Vec::new()))).collect(),
            None => return,
        };
        let limit = self.list_limits[n];
        // Per machine, the page it needs now, if any: where it starts and how many threads.
        let args: Vec<Option<ListPage>> = lists
            .iter()
            .map(|(env, so_far)| {
                let page = match so_far {
                    Ok(replies) if replies.len() == n => Some((None, limit.min(LIST_PAGE))),
                    Ok(replies) => replies.get(n).and_then(|list| next_page(list, limit).map(|c| (Some(c), (limit - listed_count(list)).min(LIST_PAGE)))),
                    Err(_) => None,
                };
                page.map(|(cursor, count)| (self.env_arg(env), cursor, count))
            })
            .collect();
        let client = Arc::clone(&self.client);
        let replies: Vec<Option<Result<Value, T3Error>>> = std::thread::scope(|scope| {
            let handles: Vec<_> = args
                .iter()
                .map(|arg| {
                    let client = &client;
                    arg.as_ref().map(|(arg, cursor, count)| scope.spawn(move || fetch_list(client.as_ref(), n, *cursor, *count, arg)))
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.map(|h| h.join().unwrap_or_else(|_| Err(T3Error::Other("thread list panicked".into())))))
                .collect()
        });
        for ((_, so_far), reply) in lists.iter_mut().zip(replies) {
            match (reply, so_far.as_mut()) {
                // A later page: its threads join the list; its cursor and total replace.
                (Some(Ok(page)), Ok(replies)) if replies.len() > n => {
                    let list = &mut replies[n];
                    let threads = page["threads"].as_array().cloned().unwrap_or_default();
                    if let Some(all) = list["threads"].as_array_mut() {
                        all.extend(threads);
                    }
                    list["nextCursor"] = page["nextCursor"].clone();
                    list["total"] = page["total"].clone();
                }
                (Some(Ok(page)), Ok(replies)) => replies.push(page),
                (Some(Err(error)), _) => *so_far = Err(error),
                _ => {}
            }
        }
        let another_page = lists.iter().any(|(_, so_far)| so_far.as_ref().is_ok_and(|replies| replies.get(n).is_some_and(|list| next_page(list, limit).is_some())));
        if another_page || n + 1 < LIST_COUNT {
            if let Some(run) = self.poll_run.as_mut() {
                run.lists = lists;
                run.steps.push_front(Step::List(if another_page { n } else { n + 1 }));
            }
        } else {
            self.push_steps(lists.into_iter().map(|(env, so_far)| Step::Machine(env, so_far.map(|lists| combine_lists(&lists)))).collect());
        }
    }

    /// One machine's lists, as read: its question checks, launchers, status changes and
    /// threads that left the lists come next, a request at a time.
    fn take_lists(&mut self, env: String, result: ThreadLists) -> Result<(), T3Error> {
        let name = if env.is_empty() { "local".to_string() } else { env.clone() };
        match result {
            Ok((threads, settled_total, more)) => {
                let mut steps: Vec<Step> = (0..threads.len()).filter(|i| working(str_of(&threads[*i]["status"]))).map(Step::Question).collect();
                steps.extend((0..threads.len()).filter(|i| threads[*i]["createdBy"] == "agent").map(Step::Launcher));
                steps.extend([Step::Track, Step::Keep]);
                if let Some(run) = self.poll_run.as_mut() {
                    run.listed = Some(Listed { env, threads, settled_total, more, seen: Vec::new() });
                }
                self.push_steps(steps);
                Ok(())
            }
            Err(T3Error::Auth(message)) => Err(T3Error::Auth(message)),
            Err(T3Error::Other(message)) => {
                let local = self.local_env_id.clone();
                if let Some(target) = self.envs.iter_mut().find(|e| e.id == env && Some(&e.id) != local.as_ref()) {
                    target.ok = false;
                }
                // The server itself failing fails the poll; a linked machine is only marked.
                if Some(&env) == local.as_ref() || env.is_empty() {
                    return Err(T3Error::Other(message));
                }
                // Logged when it changes, not on every poll.
                if self.list_errors.insert(env.clone(), message.clone()).as_ref() != Some(&message) {
                    self.log(&format!("threads ({name}): {message}"));
                }
                Ok(())
            }
        }
    }

    /// The machine being taken in, its `i`th thread's id and its updatedAt.
    fn listed_thread(&self, i: usize) -> Option<(String, String, String)> {
        let listed = self.poll_run.as_ref()?.listed.as_ref()?;
        let thread = listed.threads.get(i)?;
        Some((listed.env.clone(), str_of(&thread["threadId"]).to_string(), str_of(&thread["updatedAt"]).to_string()))
    }

    fn listed_mut(&mut self) -> Option<&mut Listed> {
        self.poll_run.as_mut()?.listed.as_mut()
    }

    /// A working thread was looked at; with a status, it waits on that.
    fn mark_waiting(&mut self, i: usize, key: String, status: Option<&'static str>) {
        if let Some(listed) = self.listed_mut() {
            listed.seen.push(key);
            if let (Some(status), Some(thread)) = (status, listed.threads.get_mut(i)) {
                thread["status"] = json!(status);
            }
        }
    }

    /// A working thread whose agent waits on a question shows as "waiting" (Needs you),
    /// as in T3's sidebar, so it notifies and leaves the Working group. The thread's
    /// pending count is read again only when it changed or the count is old.
    fn check_question(&mut self, i: usize) -> Result<(), T3Error> {
        let Some((env, id, updated)) = self.listed_thread(i) else { return Ok(()) };
        let key = format!("{env}:{id}");
        let now = self.now_ms();
        let known = match self.pending_counts.get(&key) {
            Some(p) if p.updated == updated && now - p.read_at < PENDING_RECHECK_MS => Some(p.status),
            // No time left in this poll: the last count stands until the next one.
            Some(p) if self.out_of_time() => Some(p.status),
            None if self.out_of_time() => Some(None),
            _ => None,
        };
        if let Some(status) = known {
            self.mark_waiting(i, key, status);
            return Ok(());
        }
        let args = with_args(json!({ "threadId": id, "limit": 1, "maxCharsPerItem": 1 }), &self.env_arg(&env));
        match self.client.call("t3_thread_read", args) {
            Ok(read) if read["thread"]["pendingRequestCount"].as_u64().unwrap_or(0) == 0 => {
                self.pending_counts.insert(key.clone(), Pending { updated, read_at: now, status: None });
                self.mark_waiting(i, key, None);
            }
            // Something's pending: what it is comes next.
            Ok(_) => self.push_steps(vec![Step::QuestionKind(i, now)]),
            Err(T3Error::Auth(message)) => return Err(T3Error::Auth(message)),
            Err(T3Error::Other(message)) => self.log(&format!("pending check ({key}): {message}")),
        }
        Ok(())
    }

    /// What a working thread with pending requests waits on: its count includes approvals,
    /// which T3's pending list leaves out, so a count with nothing listed is an approval.
    fn check_waiting_on(&mut self, i: usize, read_at: i64) -> Result<(), T3Error> {
        let Some((env, id, updated)) = self.listed_thread(i) else { return Ok(()) };
        let key = format!("{env}:{id}");
        let args = with_args(json!({ "action": "list", "threadId": id }), &self.env_arg(&env));
        match self.client.call("t3_pending_request", args) {
            Ok(list) => {
                let questions = list["requestIds"].as_array().is_some_and(|ids| !ids.is_empty());
                let status = Some(if questions { "waiting" } else { "approval" });
                self.pending_counts.insert(key.clone(), Pending { updated, read_at, status });
                self.mark_waiting(i, key, status);
            }
            Err(T3Error::Auth(message)) => return Err(T3Error::Auth(message)),
            Err(T3Error::Other(message)) => self.log(&format!("pending check ({key}): {message}")),
        }
        Ok(())
    }

    /// Notes which thread launched an agent-launched thread: the sender of its first message.
    fn find_launcher(&mut self, i: usize) -> Result<(), T3Error> {
        let Some((env, id, _)) = self.listed_thread(i) else { return Ok(()) };
        let key = format!("{env}:{id}");
        if !self.launchers.contains_key(&key) && !self.out_of_time() {
            let args = with_args(json!({ "threadId": id, "view": "messages", "limit": 1, "maxCharsPerItem": 1 }), &self.env_arg(&env));
            match self.client.call("t3_thread_read", args) {
                Ok(read) => {
                    let sender = read["items"][0]["senderThreadId"].as_str().filter(|s| *s != id).map(str::to_string);
                    self.launchers.insert(key.clone(), sender);
                }
                Err(T3Error::Auth(message)) => return Err(T3Error::Auth(message)),
                Err(T3Error::Other(message)) => self.log(&format!("launcher ({key}): {message}")),
            }
        }
        if let Some(Some(launcher)) = self.launchers.get(&key).cloned() {
            if let Some(thread) = self.listed_mut().and_then(|l| l.threads.get_mut(i)) {
                thread["launcher"] = json!(launcher);
            }
        }
        Ok(())
    }

    /// The machine's status changes notify, the pending counts of working threads not
    /// looked at go, and each active thread that left its lists is read next.
    fn track_listed(&mut self) {
        let Some(listed) = self.poll_run.as_mut().and_then(|run| run.listed.take()) else { return };
        for thread in &listed.threads {
            self.track_status(&listed.env, thread);
        }
        let prefix = format!("{}:", listed.env);
        self.pending_counts.retain(|k, _| !k.starts_with(&prefix) || listed.seen.contains(k));
        let ids: HashSet<String> = listed.threads.iter().map(|t| format!("{prefix}{}", str_of(&t["threadId"]))).collect();
        self.unlisted_misses.retain(|k, _| !k.starts_with(&prefix) || !ids.contains(k));
        let gone: Vec<(String, String)> =
            self.last_status.iter().filter(|(k, _)| k.starts_with(&prefix) && !ids.contains(*k)).map(|(k, s)| (k.clone(), s.clone())).collect();
        let mut steps = Vec::new();
        for (key, status) in gone {
            if working(&status) || needs_you(&status) {
                steps.push(Step::Unlisted(key, status));
            } else {
                self.last_status.remove(&key);
                self.unlisted_misses.remove(&key);
            }
        }
        if let Some(run) = self.poll_run.as_mut() {
            run.listed = Some(listed);
        }
        self.push_steps(steps);
    }

    /// A thread that was working or waiting on you and dropped off its machine's lists
    /// (settled past the list's limits, say) is read once more each poll until it ends, so
    /// its end still notifies. Every other thread no longer listed is forgotten. A poll out
    /// of time skips the read and keeps watching; a failed read keeps watching too, until
    /// UNLISTED_MISSES in a row (a deleted thread).
    fn check_unlisted(&mut self, key: String, status: String) -> Result<(), T3Error> {
        let Some(env) = self.poll_run.as_ref().and_then(|run| run.listed.as_ref()).map(|l| l.env.clone()) else { return Ok(()) };
        let keep = if self.out_of_time() {
            true
        } else {
            let id = key.get(env.len() + 1..).unwrap_or("");
            let args = with_args(json!({ "threadId": id, "limit": 1, "maxCharsPerItem": 1 }), &self.env_arg(&env));
            match self.client.call("t3_thread_read", args) {
                Ok(read) if read["thread"].is_object() => {
                    self.unlisted_misses.remove(&key);
                    let mut thread = read["thread"].clone();
                    if working(str_of(&thread["status"])) && thread["pendingRequestCount"].as_u64().unwrap_or(0) > 0 {
                        thread["status"] = json!(if needs_you(&status) { status.as_str() } else { "waiting" });
                    }
                    self.track_status(&env, &thread);
                    working(str_of(&thread["status"])) || needs_you(str_of(&thread["status"]))
                }
                Err(T3Error::Auth(message)) => return Err(T3Error::Auth(message)),
                // Unreadable now (T3 busy, or the thread deleted): try again next poll, a
                // few times.
                _ => {
                    let misses = self.unlisted_misses.entry(key.clone()).or_insert(0);
                    *misses += 1;
                    *misses < UNLISTED_MISSES
                }
            }
        };
        if !keep {
            self.last_status.remove(&key);
            self.unlisted_misses.remove(&key);
        }
        Ok(())
    }

    /// The machine's lists, taken in: they show from now on.
    fn keep_lists(&mut self) {
        let Some(Listed { env, threads, settled_total, more, .. }) = self.poll_run.as_mut().and_then(|run| run.listed.take()) else { return };
        self.listed_envs.insert(env.clone());
        self.threads.insert(env.clone(), threads);
        self.settled_totals.insert(env.clone(), settled_total);
        self.list_more.insert(env.clone(), more);
        if let Some(target) = self.envs.iter_mut().find(|e| e.id == env) {
            target.ok = true;
        }
        if self.list_errors.remove(&env).is_some() {
            let name = if env.is_empty() { "local" } else { &env };
            self.log(&format!("threads ({name}): reachable again"));
        }
    }

    /// The machines T3 links (the server's own name is read next).
    fn read_links(&mut self) -> Result<(), T3Error> {
        let result = self.client.call("t3_environment_links", json!({}))?;
        let local_env_id = result["environmentId"].as_str().map(str::to_string);
        if self.local_env_id.is_some() && local_env_id != self.local_env_id {
            self.log(&format!("T3 is now {}: forgetting the previous server's threads", local_env_id.as_deref().unwrap_or("another server")));
            self.forget_server();
        }
        self.local_env_id = local_env_id;
        self.push_steps(vec![Step::ServerName(result)]);
        Ok(())
    }

    /// The server's own name (not this PC's when `login --origin` points at another
    /// machine), then the machines as T3 links them now; each one's projects come next.
    fn read_machines(&mut self, links: Value) -> Result<(), T3Error> {
        let name = match self.client.call("t3_environment_read", json!({})) {
            Ok(read) => read["label"].as_str().filter(|l| !l.is_empty()).map(str::to_string),
            Err(T3Error::Auth(message)) => return Err(T3Error::Auth(message)),
            Err(T3Error::Other(_)) => None,
        };
        let mut envs = vec![Env { id: self.local_env_id.clone().unwrap_or_default(), name: name.unwrap_or_else(hostname), ok: true }];
        for link in links["links"].as_array().into_iter().flatten() {
            let id = str_of(&link["environmentId"]).to_string();
            let name = link["label"].as_str().map(str::to_string).unwrap_or_else(|| id.chars().take(8).collect());
            envs.push(Env { id, name, ok: link["status"] == "reachable" });
        }
        self.envs = envs;
        self.forget_unlinked();
        let steps = self.envs.iter().filter(|e| e.ok).map(|e| Step::Projects(e.id.clone())).collect();
        self.push_steps(steps);
        Ok(())
    }

    /// Drops what was read from machines T3 no longer links (their threads would otherwise
    /// stay listed forever, under another machine's name).
    fn forget_unlinked(&mut self) {
        let known: HashSet<String> = self.envs.iter().map(|e| e.id.clone()).collect();
        let linked = |env: &str| known.contains(env);
        // A thread opened before the server's own id was known.
        if let Some(view) = self.view.as_mut().filter(|v| v.env.is_empty()) {
            view.env = self.local_env_id.clone().unwrap_or_default();
        }
        self.threads.retain(|env, _| linked(env));
        self.projects.retain(|env, _| linked(env));
        self.settled_totals.retain(|env, _| linked(env));
        self.list_more.retain(|env, _| linked(env));
        self.listed_envs.retain(|env| linked(env));
        self.last_status.retain(|key, _| linked(env_of(key)));
        self.unlisted_misses.retain(|key, _| linked(env_of(key)));
        self.pending_counts.retain(|key, _| linked(env_of(key)));
        self.launchers.retain(|key, _| linked(env_of(key)));
        self.list_errors.retain(|env, _| linked(env));
        if self.view.as_ref().is_some_and(|v| !linked(&v.env)) {
            self.view = None;
            self.view_data = None;
            self.view_key = None;
            self.pending_ask = None;
        }
    }

    fn refresh_projects(&mut self, env: &str) {
        if self.out_of_time() {
            return;
        }
        let args = with_args(json!({ "limit": 100 }), &self.env_arg(env));
        match self.client.call("t3_project_list", args) {
            Ok(page) => {
                self.projects.insert(env.to_string(), page["projects"].as_array().cloned().unwrap_or_default());
            }
            Err(error) => self.log(&format!("projects ({}): {error}", if env.is_empty() { "local" } else { env })),
        }
    }

    fn track_status(&mut self, env: &str, thread: &Value) {
        let key = format!("{env}:{}", str_of(&thread["threadId"]));
        let status = str_of(&thread["status"]).to_string();
        let before = self.last_status.insert(key, status.clone());
        let notify = match before.as_deref() {
            // A thread new since the last list that already waits on you (an agent that
            // asked straight away) notifies too; on the first list everything is old news.
            None => needs_you(&status) && self.listed_envs.contains(env),
            Some(before) if before == status => false,
            // Stopped working: finished, failed, stopped, or now waits on you.
            Some(before) if working(before) => !working(&status),
            // Answered and done before the next look (it never showed as working again).
            Some(before) if needs_you(before) => !working(&status) && !needs_you(&status),
            Some(_) => false,
        };
        if notify {
            self.event_seq = (self.event_seq + 1).max(self.now_ms());
            self.events.push(json!({
                "n": self.event_seq,
                "e": env,
                "id": str_of(&thread["threadId"]),
                "t": plain(str_of(&thread["title"]), 80),
                "s": status,
            }));
            let excess = self.events.len().saturating_sub(10);
            self.events.drain(..excess);
        }
    }

    /// A command changed what the open thread shows: read it again, and drop a read of it
    /// under way (it would bring back what the command just changed).
    fn view_changed(&mut self) {
        self.view_key = None;
        self.view_gen += 1;
    }

    /// The open thread's head, when it may have changed; its messages and question are read
    /// next, a request at a time. Its messages: only what came after the last one read when
    /// that's safe (same thread and depth, no items gone, a run still going or just started,
    /// read recently), re-reading that last one, which may still be growing. Otherwise a
    /// full read of the last 80×depth items, widened until there are enough.
    fn refresh_view(&mut self) -> Result<(), T3Error> {
        let Some(view) = self.view.clone() else {
            self.view_data = None;
            self.view_cache = None;
            return Ok(());
        };
        let listed = self.threads.get(&view.env).and_then(|list| list.iter().find(|t| t["threadId"] == view.id.as_str())).cloned();
        // A thread past the lists' limits has no updatedAt to compare: look again now and then.
        let freshness = match &listed {
            Some(t) => format!("{}:{}", str_of(&t["updatedAt"]), str_of(&t["status"])),
            None => format!("unlisted:{}", self.now_ms().div_euclid(UNLISTED_VIEW_TTL_MS)),
        };
        let key = format!("{}:{}:{}:{freshness}", view.env, view.id, view.depth);
        if Some(&key) == self.view_key.as_ref() && self.view_data.is_some() {
            return Ok(());
        }
        let args = with_args(json!({ "threadId": view.id, "limit": 1, "maxCharsPerItem": 1 }), &self.env_arg(&view.env));
        let head = self.client.call("t3_thread_read", args)?;
        let thread = head["thread"].clone();
        let item_count = thread["itemCount"].as_u64().unwrap_or(0) as usize;
        let status = str_of(&thread["status"]);
        let now = self.now_ms();
        let cache = self.view_cache.take().filter(|c| {
            c.env == view.env
                && c.id == view.id
                && c.depth == view.depth
                && item_count >= c.item_count
                && now - c.read_at < VIEW_FULL_READ_MS
                // A run that ended may have tidied its messages.
                && (!working(&c.status) || working(status))
        });
        let last = cache.as_ref().and_then(|c| c.messages.last()).and_then(|m| m["position"].as_u64());
        let (cache, after, start, window) = match last {
            Some(last) => (cache, last.checked_sub(1), 0, 0),
            None => {
                let window = 80 * view.depth;
                let start = item_count.saturating_sub(window);
                (None, (start > 0).then_some(start as u64), start, window)
            }
        };
        let gen = self.view_gen;
        if let Some(run) = self.poll_run.as_mut() {
            run.reading = Some(ViewRead {
                view,
                key,
                gen,
                thread,
                began: now,
                cache,
                messages: Vec::new(),
                after,
                pages: 0,
                start,
                window,
                passes: 0,
                built: None,
            });
        }
        self.push_steps(vec![Step::ViewPage]);
        Ok(())
    }

    /// The open thread's read under way, unless a command changed the view since it began:
    /// that read is dropped (the poll after this one reads the view again).
    fn view_read(&mut self) -> Option<&mut ViewRead> {
        let gen = self.view_gen;
        let run = self.poll_run.as_mut()?;
        if run.reading.as_ref().is_some_and(|r| r.gen != gen) {
            let dropped = run.reading.take().and_then(|r| r.cache);
            if self.view_cache.is_none() {
                self.view_cache = dropped;
            }
            return None;
        }
        self.poll_run.as_mut()?.reading.as_mut()
    }

    /// One page of the open thread's messages, VIEW_PAGES at most a pass. A full read that
    /// found too few widens, VIEW_PASSES at most; then the view for the game is built.
    fn read_view_page(&mut self) -> Result<(), T3Error> {
        let Some(read) = self.view_read() else { return Ok(()) };
        let mut args = json!({ "threadId": read.view.id, "view": "messages", "limit": 100, "maxCharsPerItem": 6000 });
        if let Some(after) = read.after {
            args["afterPosition"] = json!(after);
        }
        let env = read.view.env.clone();
        let result = self.client.call("t3_thread_read", with_args(args, &self.env_arg(&env)))?;
        let Some(read) = self.view_read() else { return Ok(()) };
        read.messages.extend(result["items"].as_array().cloned().unwrap_or_default());
        read.pages += 1;
        let next = match (result["hasMore"] == true, result["nextPosition"].as_u64()) {
            (true, Some(next)) if read.pages < VIEW_PAGES => Some(next),
            _ => None,
        };
        let widen = next.is_none() && read.cache.is_none() && {
            read.passes += 1;
            read.messages.len() < VIEW_MESSAGES * read.view.depth && read.start > 0 && read.passes < VIEW_PASSES
        };
        if let Some(next) = next {
            read.after = Some(next);
        } else if widen {
            read.window *= 4;
            read.start = (read.thread["itemCount"].as_u64().unwrap_or(0) as usize).saturating_sub(read.window);
            read.after = (read.start > 0).then_some(read.start as u64);
            read.messages.clear();
            read.pages = 0;
        } else {
            self.build_view();
            return Ok(());
        }
        self.push_steps(vec![Step::ViewPage]);
        Ok(())
    }

    /// The view for the game, from the messages read; its question is read next, if it has one.
    fn build_view(&mut self) {
        let Some(mut read) = self.poll_run.as_mut().and_then(|run| run.reading.take()) else { return };
        let View { env, id, depth } = read.view.clone();
        let newer = std::mem::take(&mut read.messages);
        let (messages, earlier, read_at) = match read.cache.take() {
            Some(cache) => {
                let last = cache.messages.last().and_then(|m| m["position"].as_u64()).unwrap_or(0);
                let mut messages: Vec<Value> = cache.messages.into_iter().filter(|m| m["position"].as_u64().is_some_and(|p| p < last)).collect();
                messages.extend(newer);
                (messages, cache.earlier, cache.read_at)
            }
            None => (newer, read.start > 0, read.began),
        };
        // Keep a few views' worth: older ones only matter after "show earlier".
        let keep = VIEW_MESSAGES * depth * 4;
        let trimmed = messages.len() > keep;
        let messages: Vec<Value> = messages[messages.len().saturating_sub(keep)..].to_vec();
        let earlier = earlier || trimmed;
        let thread = &read.thread;
        self.view_cache = Some(ViewCache {
            env: env.clone(),
            id: id.clone(),
            depth,
            item_count: thread["itemCount"].as_u64().unwrap_or(0) as usize,
            status: str_of(&thread["status"]).to_string(),
            read_at,
            messages: messages.clone(),
            earlier,
        });
        let want = VIEW_MESSAGES * depth;
        let mut items: Vec<Value> = Vec::new();
        let mut chars = 0;
        for item in messages.iter().rev().take(want) {
            let kind = str_of(&item["type"]);
            if kind != "user_message" && kind != "assistant_message" {
                continue;
            }
            let text = wow_text(str_of(&item["text"]), 6000) + if item["textTruncated"] == true { "\n…" } else { "" };
            let length = text.encode_utf16().count();
            if chars + length > VIEW_CHARS && !items.is_empty() {
                break;
            }
            chars += length;
            items.insert(0, json!({ "r": if kind == "user_message" { "u" } else { "a" }, "x": text }));
        }
        let more = messages.len() > items.len() || earlier;
        let pending = thread["pendingRequestCount"].as_u64().unwrap_or(0) > 0;
        let status = if pending && working(str_of(&thread["status"])) { "waiting" } else { str_of(&thread["status"]) };
        read.built = Some(json!({
            "e": env,
            "id": id,
            "t": plain(str_of(&thread["title"]), 120),
            "s": status,
            "set": thread["settled"] == true,
            "at": iso_seconds(str_of(&thread["updatedAt"])),
            "more": more,
            "items": items,
        }));
        if let Some(run) = self.poll_run.as_mut() {
            run.reading = Some(read);
        }
        if pending {
            self.push_steps(vec![Step::ViewAsk]);
        } else {
            self.finish_view(None);
        }
    }

    /// The question the open thread waits on: which request it is (it's read next).
    fn read_view_ask(&mut self) -> Result<(), T3Error> {
        let Some(read) = self.view_read() else { return Ok(()) };
        let View { env, id, .. } = read.view.clone();
        let args = with_args(json!({ "action": "list", "threadId": id }), &self.env_arg(&env));
        match self.client.call("t3_pending_request", args) {
            Ok(list) => match list["requestIds"].as_array().and_then(|ids| ids.first()).and_then(Value::as_str) {
                Some(request_id) => self.push_steps(vec![Step::ViewAskRead(request_id.to_string())]),
                None => self.finish_view(Some(Ok(None))),
            },
            Err(error) => self.finish_view(Some(Err(error))),
        }
        Ok(())
    }

    /// The question the open thread waits on, read; the view is then done.
    fn read_view_request(&mut self, request_id: String) -> Result<(), T3Error> {
        let Some(read) = self.view_read() else { return Ok(()) };
        let View { env, id, .. } = read.view.clone();
        let args = with_args(json!({ "action": "read", "threadId": id, "requestId": request_id }), &self.env_arg(&env));
        let ask = self.client.call("t3_pending_request", args).map(|request| Self::pending_ask(&env, &id, &request_id, &request));
        self.finish_view(Some(ask));
        Ok(())
    }

    /// The open thread's view goes to the game, with its question (`ask`: None when nothing
    /// is pending). A question the agent is waiting on goes with the conversation.
    /// Approvals can't be answered through T3's tools: the window points to T3 instead.
    /// If the question can't be read this time, the one shown stays.
    fn finish_view(&mut self, ask: Option<Result<Option<(PendingAsk, Value)>, T3Error>>) {
        let Some(read) = self.poll_run.as_mut().and_then(|run| run.reading.take()) else { return };
        let View { env, id, .. } = read.view;
        let thread = read.thread;
        let Some(mut view) = read.built else { return };
        let pending = thread["pendingRequestCount"].as_u64().unwrap_or(0) > 0;
        let shown_ask = self.view_data.as_ref().filter(|v| v["e"] == env.as_str() && v["id"] == id.as_str()).map(|v| v["ask"].clone()).filter(|a| !a.is_null());
        let previous = self.pending_ask.take().filter(|a| a.env == env && a.thread == id);
        match ask {
            Some(Ok(Some((ask, value)))) => {
                view["ask"] = value;
                self.pending_ask = Some(ask);
            }
            Some(Ok(None)) | None => {}
            Some(Err(error)) => {
                self.log(&format!("pending question: {error}"));
                if let (Some(ask), Some(value)) = (previous, shown_ask) {
                    view["ask"] = value;
                    self.pending_ask = Some(ask);
                }
            }
        }
        // Waiting on something that isn't a question: an approval, which only T3 can give.
        if self.pending_ask.is_none() && (pending || str_of(&thread["status"]) == "waiting") {
            view["approval"] = json!(true);
            if working(str_of(&thread["status"])) {
                view["s"] = json!("approval");
            }
        }
        self.view_data = Some(view);
        self.view_key = Some(read.key);
    }

    /// The first question of a pending request, for the game: a short token, and each
    /// question with its options. The full ids stay here; the game answers by index.
    fn pending_ask(env: &str, thread: &str, request_id: &str, request: &Value) -> Option<(PendingAsk, Value)> {
        let mut questions = Vec::new();
        let mut shown = Vec::new();
        for q in request["questions"].as_array().into_iter().flatten() {
            let text = str_of(&q["question"]);
            let key = q["id"].as_str().unwrap_or(text).to_string();
            let options: Vec<&Value> = q["options"].as_array().map(|o| o.iter().collect()).unwrap_or_default();
            let multi = q["multiSelect"] == true;
            questions.push(PendingQuestion { key, labels: options.iter().map(|o| str_of(&o["label"]).to_string()).collect(), multi });
            shown.push(json!({
                "h": plain(str_of(&q["header"]), 40),
                "q": wow_text(text, 1000),
                "m": multi,
                "o": options.iter().map(|o| json!({ "l": plain(str_of(&o["label"]), 60), "d": plain(str_of(&o["description"]), 160) })).collect::<Vec<_>>(),
            }));
        }
        if questions.is_empty() {
            return None;
        }
        let token = format!("{:08x}", hash_of(&json!(request_id)) as u32);
        let ask = PendingAsk { env: env.to_string(), thread: thread.to_string(), token: token.clone(), request_id: request_id.to_string(), questions };
        Some((ask, json!({ "k": token, "qs": shown })))
    }

    /// Turns the game's answer (per question: a 1-based option index, a list of them, or
    /// typed text) into T3's answers, keyed by question id with option labels.
    fn answers_for(ask: &PendingAsk, given: &Value) -> Map<String, Value> {
        let mut answers = Map::new();
        for (i, q) in ask.questions.iter().enumerate() {
            let pick = |v: &Value| v.as_u64().and_then(|n| q.labels.get((n as usize).wrapping_sub(1))).cloned();
            let value = match &given[i] {
                Value::String(text) if !text.trim().is_empty() => Some(json!(text.trim())),
                Value::Number(_) if q.multi => pick(&given[i]).map(|l| json!([l])),
                Value::Number(_) => pick(&given[i]).map(Value::String),
                Value::Array(list) => {
                    let labels: Vec<String> = list.iter().filter_map(pick).collect();
                    (!labels.is_empty()).then(|| if q.multi { json!(labels) } else { json!(labels[0]) })
                }
                _ => None,
            };
            if let Some(value) = value {
                answers.insert(q.key.clone(), value);
            }
        }
        answers
    }

    // ---- writing into the game ----

    pub fn snapshot(&self) -> Value {
        // 1-based, as the game indexes inbox.envs; 0 (left out) for a machine no longer linked.
        let env_index = |id: &str| self.envs.iter().position(|e| e.id == id).map_or(0, |i| i + 1);
        let mut project_title: HashMap<String, String> = HashMap::new();
        for (env, list) in &self.projects {
            for p in list {
                project_title.insert(format!("{env}:{}", str_of(&p["id"])), str_of(&p["title"]).to_string());
            }
        }
        let mut threads: Vec<Value> = Vec::new();
        for (env, list) in self.threads.iter().filter(|(env, _)| env_index(env) > 0) {
            for t in list {
                let mut entry = json!({
                    "e": env_index(env),
                    "id": str_of(&t["threadId"]),
                    "t": plain(str_of(&t["title"]), 90),
                    "s": str_of(&t["status"]),
                    "set": t["settled"] == true,
                });
                if t["snoozed"] == true {
                    entry["sn"] = json!(true);
                }
                if let Some(launcher) = t["launcher"].as_str() {
                    entry["par"] = json!(launcher);
                }
                entry["at"] = json!(iso_seconds(str_of(&t["updatedAt"])));
                let project = project_title.get(&format!("{env}:{}", str_of(&t["projectId"]))).map(String::as_str).unwrap_or("");
                entry["p"] = json!(plain(project, 40));
                threads.push(entry);
            }
        }
        threads.sort_by_key(|t| std::cmp::Reverse(t["at"].as_i64().unwrap_or(0)));
        let mut projects = Vec::new();
        for (env, list) in self.projects.iter().filter(|(env, _)| env_index(env) > 0) {
            for p in list.iter().filter(|p| !p["archivedAt"].is_string() && !p["deletedAt"].is_string()) {
                projects.push(json!({ "e": env_index(env), "id": str_of(&p["id"]), "t": plain(str_of(&p["title"]), 40) }));
            }
        }
        let view = self.view_data.as_ref().map(|data| {
            let mut data = data.clone();
            data["e"] = json!(env_index(str_of(&data["e"])));
            data
        });
        let client_key = self.link.client_key.clone();
        let mut out = Map::new();
        out.insert("v".into(), json!(1));
        out.insert("k".into(), json!(client_key));
        // The UI session this is for: an older session's acks mean nothing to a newer one.
        out.insert("u".into(), json!(self.link.ui_session));
        out.insert("ack".into(), json!(client_key.as_ref().and_then(|k| self.link.acked.get(k)).copied().unwrap_or(0)));
        out.insert("envs".into(), json!(self.envs.iter().map(|e| json!({ "id": e.id, "name": plain(&e.name, 40), "ok": e.ok })).collect::<Vec<_>>()));
        out.insert("threads".into(), json!(threads));
        out.insert("settledTotal".into(), json!(self.settled_totals.values().sum::<u64>()));
        // The groups with more threads than listed that Show more can still raise (left
        // out when there are none).
        let mut more = Map::new();
        for (n, name) in LIST_NAMES.iter().enumerate() {
            let unlisted = self.list_more.iter().any(|(env, m)| m[n] && env_index(env) > 0);
            if unlisted && self.list_limits[n] < LIST_CAPS[n] {
                more.insert((*name).into(), json!(true));
            }
        }
        if !more.is_empty() {
            out.insert("more".into(), Value::Object(more));
        }
        out.insert("projects".into(), json!(projects));
        out.insert("view".into(), view.unwrap_or(Value::Null));
        out.insert("notes".into(), json!(self.notes));
        out.insert("events".into(), json!(self.events));
        if !self.outcomes.is_empty() {
            out.insert("out".into(), json!(self.outcomes.iter().map(|(_, o)| o).collect::<Vec<_>>()));
        }
        out.insert("auth".into(), json!(self.auth_error));
        // What the tray setting blocks, so the window can say so (left out when nothing is).
        if let Some(code) = self.trust.code() {
            out.insert("trust".into(), json!(code));
        }
        Value::Object(out)
    }

    pub fn publish(&mut self) {
        if self.link.wow_key.as_deref().unwrap_or("").is_empty() || self.link.ui_session.is_none() {
            return;
        }
        let t = self.now_ms();
        if t - self.last_publish_at < MIN_PUBLISH_GAP_MS {
            return;
        }
        let data = self.snapshot();
        // Each update spends one of the UI session's 200 slots. Urgent: what answers you
        // (acks, notes, a thread you just opened), alerts and errors; with the window closed
        // nothing else is worth a slot. Everything else waits until ROUTINE_GAP_MS after the
        // last update.
        let urgent = json!({
            "ack": data["ack"],
            "events": data["events"],
            "notes": data["notes"],
            "out": data["out"],
            "auth": data["auth"],
            "trust": data["trust"],
            "open": self.window_open,
            // A question appearing (or going away) in the open thread is urgent too.
            "view": if data["view"].is_null() {
                Value::Null
            } else {
                json!([data["view"]["id"], data["view"]["more"], data["view"]["ask"]["k"], data["view"]["approval"]])
            },
        });
        let hash = hash_of(if self.window_open { &data } else { &urgent });
        if Some(hash) == self.last_published_hash {
            return;
        }
        let urgent_hash = hash_of(&urgent);
        if Some(urgent_hash) == self.last_urgent_hash && t - self.last_publish_at < ROUTINE_GAP_MS {
            return;
        }
        if self.link.slot > SLOT_COUNT {
            if self.slots_out_logged != self.link.ui_session {
                self.slots_out_logged = self.link.ui_session.clone();
                self.log("The game used all 200 update slots of this UI session: Reload UI in the game for more");
            }
            return;
        }
        let mut polling = self.link.polling == Some(true) || self.signals_out.is_some();
        let now_s = t.div_euclid(1000);
        // ns/nt: where and when the game's next wait starts. Polling doesn't move it.
        let mut wait = (self.link.signal, self.link.signal_time);
        let mut raise = Vec::new();
        if !polling {
            // The game may be a moment either side of moving to its next file, so raise the
            // file it should be watching now and both neighbours.
            let watched = self.watched_signal(now_s);
            if watched + 1 > SIGNAL_COUNT {
                // Past the last file: the game notices that too and loads slots on a timer.
                self.signals_out = Some(SIGNALS_OUT.into());
                self.log("This WoW session used all its signal files: updates come on a timer until WoW restarts");
                polling = true;
            } else {
                raise.extend(self.link.signal.max(watched.saturating_sub(1))..=watched + 1);
                wait = (watched + 2, now_s);
                if wait.0 + 1 > SIGNAL_COUNT {
                    // The next wait has no files left: this update (the last one a signal
                    // brings) tells the game to load slots on a timer from now on.
                    self.signals_out = Some(SIGNALS_OUT.into());
                    self.log("This WoW session used all its signal files: updates come on a timer until WoW restarts");
                }
            }
        }
        let mut inbox = data;
        inbox["now"] = json!(now_s);
        if let Some(reason) = &self.signals_out {
            inbox["sigoff"] = json!(reason);
        }
        // Write a few slots ahead so a missed count on either side still finds fresh data,
        // and only then raise the signals that send the game to read them. Nothing counts
        // as published unless every write worked.
        let mut written = Ok(());
        for slot in self.link.slot..(self.link.slot + 3).min(SLOT_COUNT + 1) {
            inbox["slot"] = json!(slot);
            inbox["ns"] = json!(wait.0);
            inbox["nt"] = json!(wait.1);
            written = written.and_then(|_| self.slots.write_slot(slot, &format!("T3WoW_Inbox = {}\n", to_lua(&inbox))));
        }
        for n in raise {
            written = written.and_then(|_| self.slots.raise_signal(n));
        }
        self.last_publish_at = t;
        if let Err(error) = written {
            let message = format!("Couldn't write the game's update files: {error}");
            if self.write_error.as_ref() != Some(&message) {
                self.log(&message);
            }
            self.write_error = Some(message);
            self.write_failures += 1;
            return;
        }
        if self.write_error.take().is_some() {
            self.log("Writing the game's update files works again");
        }
        self.write_failures = 0;
        if !polling {
            self.link.slot += 1;
            self.link.signal = wait.0;
            self.link.signal_time = wait.1;
        }
        self.last_published_hash = Some(hash);
        self.last_urgent_hash = Some(urgent_hash);
        self.save_state();
    }

    /// Sends the current state again, as if the game had never had it (after Reinstall
    /// blanked the slot and signal files).
    pub fn republish(&mut self) {
        self.last_published_hash = None;
        self.last_urgent_hash = None;
        self.last_publish_at = 0;
    }

    /// Runs the game's queued commands. Each counts as done (acked) once it ran, and only
    /// for the UI session it came from: a newer session counts afresh.
    fn run_commands(&mut self) {
        while let Some((client_key, session, command)) = self.commands.pop_front() {
            let s = command["s"].as_u64().unwrap_or(0);
            let outcome = self.run(&client_key, command);
            if Some(session.as_str()) == self.link.ui_session.as_deref() && Some(client_key.as_str()) == self.link.client_key.as_deref() {
                let entry = self.link.acked.entry(client_key).or_insert(0);
                *entry = (*entry).max(s);
                self.save_state();
                if let Some(outcome) = outcome {
                    let at = self.now_ms();
                    self.outcomes.push((at, outcome));
                }
            }
        }
    }

    /// A poll is under way: the caller should tick again soon.
    pub fn polling(&self) -> bool {
        self.poll_run.is_some()
    }

    /// One step: run the game's commands, read T3 when it's time, publish what changed.
    /// A poll runs a step (one request) at a time for up to TICK_BUDGET_MS, then gives the
    /// caller (and the game's next commands, the tray's controls) a turn.
    pub fn tick(&mut self) {
        self.run_commands();
        let now = self.now_ms();
        self.outcomes.retain(|(at, _)| now - at < OUTCOME_KEEP_MS);
        let excess = self.outcomes.len().saturating_sub(OUTCOMES_KEPT);
        self.outcomes.drain(..excess);
        let started = self.now_ms();
        loop {
            if self.poll_run.is_none() {
                if self.now_ms() < self.poll_at {
                    break;
                }
                self.start_poll();
            }
            self.poll_step();
            if self.poll_run.is_none() || self.now_ms() - started >= TICK_BUDGET_MS {
                break;
            }
        }
        // Halfway through a poll the state is half new: publish only once it has taken a
        // while, so an answer to a command still goes out.
        let now = self.now_ms();
        if self.poll_run.as_ref().is_none_or(|run| now - run.started >= PUBLISH_MIDPOLL_MS) {
            self.publish();
        }
    }
}

#[cfg(test)]
mod tests;
