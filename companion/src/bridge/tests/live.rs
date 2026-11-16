//! Live tests: the companion's bridge against the T3 it is signed in to, with the game
//! stood in for. Commands go in as the frames the addon draws; what comes back is checked
//! in T3 itself and in the updates written for the game.
//!
//! Four checks in one run:
//! - T3's tools take the inputs the companion was made for (`tests/fixtures/t3-tools.json`;
//!   see docs/t3-update.md);
//! - every call the bridge makes is answered in the shape `FakeT3` answers it, so the unit
//!   tests above keep testing against the real T3 (`fake_and_live_differ`);
//! - every command the game can send works end to end against the real T3;
//! - each update written for the game is saved under `target/live/<test>/`, and
//!   `tests/live-replay.js` loads those into the real addon (`npm run test:live` runs both).
//!
//! They start real agent runs (the cheapest model the catalog offers, low effort, in their
//! own T3 project, "T3 WoW live tests", which they create in an empty folder) and archive
//! their threads at the end. Ignored by default; run them with `npm run test:live`.
//! `T3WOW_LIVE_MODEL=<provider>/<model>` picks another model.

use super::*;
use crate::t3::McpClient;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

const PROJECT_TITLE: &str = "T3 WoW live tests";
/// A reply from the agent; a cheap model answers in seconds, a busy machine takes longer.
const REPLY_TIMEOUT: Duration = Duration::from_secs(240);
/// T3's state reaching the game: a poll or two.
const STATE_TIMEOUT: Duration = Duration::from_secs(60);
/// Tries for the steps that hang on what a model chooses to do.
const TRIES: usize = 3;

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// The real T3 client, keeping every call the bridge makes and its answer.
struct Recorder {
    client: McpClient,
    calls: Mutex<Vec<(String, Value, Value)>>,
}

impl T3 for Recorder {
    fn call(&self, name: &str, args: Value) -> Result<Value, T3Error> {
        let result = self.client.call(name, args.clone());
        if let Ok(value) = &result {
            self.calls.lock().unwrap().push((name.to_string(), args, value.clone()));
        }
        result
    }

    fn upload(&self, relative_url: &str, content_type: &str, bytes: &[u8]) -> Result<(), T3Error> {
        self.client.upload(relative_url, content_type, bytes)
    }
}

/// The game's slots, kept in the order they were written.
#[derive(Default)]
struct Inboxes {
    written: Vec<String>,
}

struct RecordingSlots(Arc<Mutex<Inboxes>>);

impl SlotWriter for RecordingSlots {
    fn write_slot(&mut self, _n: u32, lua: &str) -> std::io::Result<()> {
        self.0.lock().unwrap().written.push(lua.to_string());
        Ok(())
    }
    fn raise_signal(&mut self, _n: u32) -> std::io::Result<()> {
        Ok(())
    }
    fn clear_signals(&mut self) {}
    fn clear_slots(&mut self) {}
    fn flip_probe(&mut self, _real: bool) -> Vec<String> {
        Vec::new()
    }
    fn write_image(&mut self, _n: u32, _tga: &[u8]) -> std::io::Result<()> {
        Ok(())
    }
    fn blank_image(&mut self, _n: u32) {}
    fn has_image(&self, _n: u32) -> bool {
        false
    }
}

/// Fields the fake always sends that T3 sends only sometimes, which the bridge already
/// handles being absent.
const OPTIONAL: &[&str] = &[
    // Codex's questions leave it out (single choice); Claude's say it.
    "t3_pending_request.questions[].multiSelect",
];

/// The JSON kind of a value, for messages.
fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a bool",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

/// Where the fake's answer has a field T3's lacks, or of another kind. Null on either side
/// matches anything (the fake uses it for "not there", T3 for "none now"). A list's items
/// are matched by their `type` when they have one (a timeline mixes kinds of item).
fn compare(path: &str, fake: &Value, live: &Value, out: &mut BTreeSet<String>) {
    if fake.is_null() || live.is_null() {
        return;
    }
    if kind(fake) != kind(live) {
        out.insert(format!("{path}: the fake has {}, T3 sends {}", kind(fake), kind(live)));
        return;
    }
    match (fake, live) {
        (Value::Object(fake), Value::Object(live)) => {
            for (key, value) in fake {
                match live.get(key) {
                    Some(theirs) => compare(&format!("{path}.{key}"), value, theirs, out),
                    None if !value.is_null() && !OPTIONAL.contains(&format!("{path}.{key}").as_str()) => {
                        out.insert(format!("{path}.{key}: the fake has it, T3 doesn't send it"));
                    }
                    None => {}
                }
            }
        }
        (Value::Array(fake), Value::Array(live)) => {
            for value in fake {
                let same_type = live.iter().find(|theirs| value.get("type").is_some() && theirs.get("type") == value.get("type"));
                if let Some(theirs) = same_type.or(live.first()) {
                    compare(&format!("{path}[]"), value, theirs, out);
                }
            }
        }
        _ => {}
    }
}

/// Each call the bridge made, asked of a fresh `FakeT3` too: where the fake answers in a
/// shape T3 no longer does, the unit tests test something T3 doesn't send.
fn fake_and_live_differ(calls: &[(String, Value, Value)]) -> Vec<String> {
    let mut out = BTreeSet::new();
    for (name, args, live) in calls {
        let fake = FakeT3::new().call(name, args.clone()).unwrap_or(Value::Null);
        compare(name, &fake, live, &mut out);
    }
    out.into_iter().collect()
}

/// A checkpoint for the replay: after loading inbox `file` (and those before it), the addon
/// shows thread `id` with each of `texts` on screen, and each of `buttons`.
#[derive(Serialize)]
struct Checkpoint {
    label: String,
    file: String,
    id: String,
    texts: Vec<String>,
    buttons: Vec<String>,
}

struct Live {
    bridge: Bridge,
    t3: Arc<Recorder>,
    inboxes: Arc<Mutex<Inboxes>>,
    session: String,
    seq: u64,
    project: String,
    /// The model the threads run on: provider, model, its efforts (lowest first).
    model: (String, String, Vec<String>),
    started: Vec<String>,
    dir: PathBuf,
    checkpoints: Vec<Checkpoint>,
    /// The image the next paste finds on the clipboard, and the Ctrl+V presses so far.
    clipboard: Arc<Mutex<Option<Clipped>>>,
    presses: Arc<Mutex<(u64, i64)>>,
}

impl Live {
    /// Signs in as the companion does, finds (or makes) the test project, and opens the
    /// game's window, so the bridge reads T3 as it would while you play.
    fn start(test: &str) -> Live {
        let target = crate::t3::target().expect("no T3 to talk to: start T3 Code (or point the companion at one with t3wow.exe login --origin)");
        let token = target.token.clone().expect("the companion isn't signed in to T3: run t3wow.exe login first");
        let t3 = Arc::new(Recorder { client: McpClient::new(&target.origin, &token), calls: Mutex::new(Vec::new()) });
        let inboxes = Arc::new(Mutex::new(Inboxes::default()));
        let mut bridge = Bridge::new(t3.clone(), Box::new(RecordingSlots(Arc::clone(&inboxes))), None, Box::new(|line| eprintln!("  bridge: {line}")), Box::new(now_ms));
        bridge.set_login(&target.key);
        // Links are never opened: a test must not bring T3 or a browser to the front.
        bridge.opener = Box::new(|link| eprintln!("  (would open {link})"));
        let clipboard: Arc<Mutex<Option<Clipped>>> = Arc::new(Mutex::new(None));
        let presses = Arc::new(Mutex::new((0u64, 0i64)));
        let (clip, read) = (Arc::clone(&clipboard), Arc::clone(&presses));
        bridge.clipboard = Box::new(move || Ok(clip.lock().unwrap().take().into_iter().collect()));
        bridge.presses = Box::new(move || *read.lock().unwrap());
        bridge.keys_hidden = Box::new(|| false);
        bridge.on_capture(CaptureMessage::Wow(Some(format!("live@{}", now_ms()))));
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("live").join(test);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("inboxes")).unwrap();
        let project = ensure_project(&t3.client);
        let mut live = Live {
            bridge,
            t3,
            inboxes,
            session: format!("live{}", now_ms()),
            seq: 0,
            project,
            model: (String::new(), String::new(), Vec::new()),
            started: Vec::new(),
            dir,
            checkpoints: Vec::new(),
            clipboard,
            presses,
        };
        live.command(json!({ "t": "ui", "o": true, "f": true }));
        live.wait("the first look at T3", STATE_TIMEOUT, |live| live.bridge.polled_ok && live.snapshot()["models"].is_array());
        live.model = live.pick_model();
        eprintln!("  model: {} / {} (efforts {:?})", live.model.0, live.model.1, live.model.2);
        live
    }

    /// The model from `T3WOW_LIVE_MODEL`, else the first whose name says it's small and cheap
    /// (haiku, then mini, flash, luna), else the first listed.
    fn pick_model(&self) -> (String, String, Vec<String>) {
        let wanted = std::env::var("T3WOW_LIVE_MODEL").ok();
        let mut all = Vec::new();
        for provider in self.snapshot()["models"].as_array().into_iter().flatten().filter(|p| p["e"] == 1) {
            for model in provider["m"].as_array().into_iter().flatten() {
                let efforts = model["ef"].as_array().map(|v| v.iter().map(|e| str_of(e).to_string()).collect()).unwrap_or_default();
                all.push((str_of(&provider["id"]).to_string(), str_of(&model["id"]).to_string(), efforts));
            }
        }
        if let Some(wanted) = wanted {
            return all.into_iter().find(|(p, m, _)| format!("{p}/{m}") == wanted).unwrap_or_else(|| panic!("T3 offers no {wanted}"));
        }
        // Haiku first: it asks a question when told to, where others may just answer.
        let cheap = ["haiku", "mini", "flash", "luna"];
        let found = cheap.iter().find_map(|c| all.iter().find(|(_, m, _)| m.contains(c))).or(all.first()).cloned();
        found.expect("T3 offers no model this machine can run")
    }

    fn snapshot(&self) -> Value {
        self.bridge.snapshot()
    }

    /// Draws one command in a frame, as the addon does, and gives the bridge a turn.
    fn command(&mut self, mut command: Value) -> u64 {
        self.seq += 1;
        command["s"] = json!(self.seq);
        // The game's client key, new each run: a command's retry key (clientRequestId) is
        // made from it, and T3 answers a key it has seen with what it did then (the thread
        // a previous run started, say). The replay gives the updates its own key.
        let frame = json!({ "k": self.session, "u": self.session, "n": 0, "l": if self.seq == 1 { 1 } else { 0 }, "dc": 1, "wk": 1, "q": [command] });
        self.bridge.on_frame(&frame, None, true, now_ms());
        self.bridge.tick();
        self.seq
    }

    /// Ticks the bridge, as the companion's loop does, until `done` holds.
    fn wait(&mut self, what: &str, timeout: Duration, done: impl Fn(&Live) -> bool) {
        let started = Instant::now();
        loop {
            self.bridge.tick();
            if done(self) {
                return;
            }
            if started.elapsed() > timeout {
                panic!("timed out ({}s) waiting for {what}; the game was last sent:\n{:#}", timeout.as_secs(), self.snapshot());
            }
            std::thread::sleep(Duration::from_millis(if self.bridge.polling() { 50 } else { 500 }));
        }
    }

    /// Like `wait`, but Err instead of a panic: for steps worth trying again.
    fn try_wait(&mut self, timeout: Duration, done: impl Fn(&Live) -> bool) -> Result<(), ()> {
        let started = Instant::now();
        loop {
            self.bridge.tick();
            if done(self) {
                return Ok(());
            }
            if started.elapsed() > timeout {
                return Err(());
            }
            std::thread::sleep(Duration::from_millis(if self.bridge.polling() { 50 } else { 500 }));
        }
    }

    /// The outcome the game hears for command `s`, once it has one.
    fn outcome(&mut self, s: u64) -> Value {
        self.wait(&format!("an outcome for command {s}"), STATE_TIMEOUT, |live| outcome_of(live, s).is_some());
        outcome_of(self, s).unwrap()
    }

    /// Commands with no outcome when they work: a failure would come back within a turn.
    fn expect_no_failure(&mut self, s: u64) {
        for _ in 0..4 {
            self.bridge.tick();
        }
        if let Some(o) = outcome_of(self, s) {
            assert_ne!(o["ok"], false, "command {s} failed: {o}");
        }
    }

    /// A thread as T3 has it now (read directly, not through the bridge).
    fn thread(&self, id: &str) -> Value {
        let read = self.t3.client.call("t3_thread_read", json!({ "threadId": id, "limit": 1, "maxCharsPerItem": 1 })).expect("thread read");
        read["thread"].clone()
    }

    /// A model selection for a command: the test's model at effort `effort` (an index into
    /// its efforts), when it has efforts.
    fn with_model(&self, mut command: Value, effort: usize) -> Value {
        command["pv"] = json!(self.model.0);
        command["md"] = json!(self.model.1);
        if let Some(e) = self.model.2.get(effort.min(self.model.2.len().saturating_sub(1))) {
            command["ef"] = json!(e);
        }
        command
    }

    /// Starts a thread from the game's New Thread in the test project, and returns its id.
    fn new_thread(&mut self, message: &str) -> String {
        let command = self.with_model(json!({ "t": "new", "p": self.project, "m": message, "rm": "approval-required" }), 0);
        let s = self.command(command);
        let o = self.outcome(s);
        assert_eq!(o["ok"], true, "New Thread failed: {o}");
        let id = str_of(&o["id"]).to_string();
        assert!(!id.is_empty(), "New Thread named no thread: {o}");
        self.started.push(id.clone());
        id
    }

    /// The open thread's messages from `role` ("u" or "a"), as the game is sent them.
    fn said(&self, id: &str, role: &str) -> Vec<String> {
        let view = self.snapshot()["view"].clone();
        if view["id"] != id {
            return Vec::new();
        }
        view["items"].as_array().into_iter().flatten().filter(|i| i["r"] == role).map(|i| str_of(&i["x"]).to_string()).collect()
    }

    fn agent_said(&self, id: &str, word: &str) -> bool {
        self.said(id, "a").iter().any(|x| x.to_lowercase().contains(&word.to_lowercase()))
    }

    /// The thread's row in the game's list.
    fn row(&self, id: &str) -> Value {
        self.snapshot()["threads"].as_array().into_iter().flatten().find(|t| t["id"] == id).cloned().unwrap_or(Value::Null)
    }

    /// Saves every update written so far for the replay, and marks this point to check there:
    /// the latest update, once it shows thread `id` open with each of `texts` (in any case)
    /// in one of the agent's messages, which is where the replay looks for them.
    fn checkpoint(&mut self, label: &str, id: &str, texts: &[&str], buttons: &[&str]) {
        // The message itself, as written for the game: a text found anywhere in the update
        // (your prompt, another thread's title) would pass before the reply is written, which
        // can be up to ROUTINE_GAP_MS after the run's end is.
        let shows = |live: &Live| {
            let inboxes = live.inboxes.lock().unwrap();
            let last = inboxes.written.last().map(String::as_str).unwrap_or("");
            let view = live.snapshot()["view"].clone();
            let replies: Vec<&Value> = view["items"].as_array().into_iter().flatten().filter(|i| i["r"] == "a").collect();
            let written = |t: &&str| replies.iter().any(|i| str_of(&i["x"]).to_lowercase().contains(&t.to_lowercase()) && last.contains(&to_lua(i)));
            view["id"] == id && last.contains("view={") && texts.iter().all(written)
        };
        self.wait(&format!("an update for the game with {label}"), STATE_TIMEOUT, shows);
        let file = self.save_inboxes();
        self.checkpoints.push(Checkpoint {
            label: label.into(),
            file,
            id: id.into(),
            texts: texts.iter().map(|t| t.to_string()).collect(),
            buttons: buttons.iter().map(|b| b.to_string()).collect(),
        });
        let json = serde_json::to_string_pretty(&self.checkpoints).unwrap();
        std::fs::write(self.dir.join("checkpoints.json"), json).unwrap();
    }

    /// Writes the updates not saved yet, numbered in order; returns the last one's name.
    fn save_inboxes(&self) -> String {
        let inboxes = self.inboxes.lock().unwrap();
        let folder = self.dir.join("inboxes");
        let saved = std::fs::read_dir(&folder).map(|d| d.count()).unwrap_or(0);
        for (n, lua) in inboxes.written.iter().enumerate().skip(saved) {
            std::fs::write(folder.join(format!("{:05}.lua", n + 1)), lua).unwrap();
        }
        format!("{:05}.lua", inboxes.written.len())
    }

    /// Ends the test: no call answered in a shape the fake doesn't know.
    fn finish(mut self) {
        self.save_inboxes();
        let calls = self.t3.calls.lock().unwrap().clone();
        let differ = fake_and_live_differ(&calls);
        let tools: BTreeSet<&str> = calls.iter().map(|(n, _, _)| n.as_str()).collect();
        eprintln!("  compared {} calls to {} tools with the fake: {tools:?}", calls.len(), tools.len());
        self.archive();
        assert!(differ.is_empty(), "T3 answers differently from FakeT3 (fix the fake, then the bridge if its tests fail):\n  {}", differ.join("\n  "));
    }

    fn archive(&mut self) {
        for id in self.started.drain(..) {
            if let Err(error) = self.t3.client.call("t3_thread_organize", json!({ "action": "archive", "threadId": id })) {
                eprintln!("  couldn't archive {id}: {error}");
            }
        }
    }
}

impl Drop for Live {
    /// A failed test leaves no threads behind either.
    fn drop(&mut self) {
        self.archive();
    }
}

fn outcome_of(live: &Live, s: u64) -> Option<Value> {
    live.snapshot()["out"].as_array().into_iter().flatten().find(|o| o["s"] == s).cloned()
}

/// The test project's id; made in an empty folder of the companion's the first time.
fn ensure_project(client: &McpClient) -> String {
    let mut cursor = 0;
    loop {
        let page = client.call("t3_project_list", json!({ "limit": 100, "cursor": cursor })).expect("project list");
        let found = page["projects"].as_array().into_iter().flatten().find(|p| p["title"] == PROJECT_TITLE && !p["archivedAt"].is_string() && !p["deletedAt"].is_string());
        if let Some(project) = found {
            return str_of(&project["id"]).to_string();
        }
        match page["nextCursor"].as_u64() {
            Some(next) if next > cursor => cursor = next,
            _ => break,
        }
    }
    let folder = crate::config::config_dir().join("live-tests");
    let made = client
        .call("t3_project_create", json!({ "title": PROJECT_TITLE, "workspaceRoot": folder.to_string_lossy(), "createWorkspaceRootIfMissing": true }))
        .expect("couldn't create the test project");
    let id = made["projectId"].as_str().or(made["project"]["id"].as_str()).or(made["id"].as_str());
    id.unwrap_or_else(|| panic!("T3 made the project but named no id: {made}")).to_string()
}

fn green_image() -> Clipped {
    let bitmap = Bitmap { width: 64, height: 64, rgba: [0u8, 200, 0, 255].repeat(64 * 64) };
    Clipped { png: crate::clip::encode_png(&bitmap).unwrap(), bitmap }
}

/// New Thread, a reply, Send, a pasted image, the model row, Snooze, Settle and their undo.
#[test]
#[ignore = "live: starts agent runs on the T3 the companion is signed in to (npm run test:live)"]
fn live_a_thread_from_the_game_round_trips() {
    let mut live = Live::start("round-trip");
    // Replies the prompts don't contain, so the replay can't find them in your own words.
    let id = live.new_thread("What is the capital of France? Reply with only the city's name, in capital letters. Do not use any tools.");
    live.wait("the reply to New Thread", REPLY_TIMEOUT, |live| live.agent_said(&id, "PARIS") && !working(str_of(&live.thread(&id)["status"])));
    live.wait("the thread in the game's list", STATE_TIMEOUT, |live| live.row(&id)["s"] == "completed");
    assert!(live.said(&id, "u").iter().any(|x| x.contains("France")), "the opening message isn't in the view");
    live.checkpoint("the reply to a new thread", &id, &["PARIS"], &[]);

    let s = live.command(json!({ "t": "send", "id": id, "m": "And of Italy? Same format." }));
    live.expect_no_failure(s);
    live.wait("the reply to Send", REPLY_TIMEOUT, |live| live.agent_said(&id, "ROME"));
    live.checkpoint("the reply to a sent message", &id, &["ROME"], &[]);

    // A pasted image goes with the next message, and the agent sees it.
    for attempt in 1..=TRIES {
        *live.clipboard.lock().unwrap() = Some(green_image());
        let pressed = live.presses.lock().unwrap().0 + 1;
        *live.presses.lock().unwrap() = (pressed, now_ms() - 100);
        let s = live.command(json!({ "t": "paste", "d": format!(":{id}") }));
        assert_eq!(live.outcome(s)["ok"], true, "the paste was refused");
        let s = live.command(json!({ "t": "send", "id": id, "m": "What single color fills the attached image? Answer with one word." }));
        live.expect_no_failure(s);
        if live.try_wait(REPLY_TIMEOUT, |live| live.agent_said(&id, "green")).is_ok() {
            break;
        }
        assert!(attempt < TRIES, "the agent never said the pasted image is green: {:?}", live.said(&id, "a"));
        eprintln!("  try {attempt}: no \"green\" yet, pasting again");
    }
    live.checkpoint("the reply about a pasted image", &id, &["reen"], &[]);

    // The model row: another effort, sent with the thread's other options.
    assert!(live.model.2.len() > 1, "{} offers no second effort to change to: pick a model that does with T3WOW_LIVE_MODEL", live.model.1);
    let s = live.command(live.with_model(json!({ "t": "model", "id": id }), 1));
    live.expect_no_failure(s);
    let config = live.t3.client.call("t3_thread_configure", json!({ "threadId": id })).unwrap();
    let effort = live.model.2[1].clone();
    assert!(
        config["modelSelection"]["options"].as_array().into_iter().flatten().any(|o| o["value"] == effort.as_str()),
        "the effort didn't change to {effort}: {config}"
    );
    assert_eq!(config["modelSelection"]["model"], live.model.1.as_str());
    live.wait("the new effort in the game's model row", STATE_TIMEOUT, |live| live.snapshot()["view"]["cfg"]["ef"] == effort.as_str());

    let s = live.command(json!({ "t": "snooze", "id": id, "mn": 30 }));
    live.expect_no_failure(s);
    assert_eq!(live.thread(&id)["snoozed"], true, "T3 didn't snooze it");
    live.wait("the snooze in the game's list", STATE_TIMEOUT, |live| live.row(&id)["sn"] == true);
    live.checkpoint("a snoozed thread", &id, &[], &[]);
    let s = live.command(json!({ "t": "unsnooze", "id": id }));
    live.expect_no_failure(s);
    assert_eq!(live.thread(&id)["snoozed"], false, "T3 didn't unsnooze it");
    live.wait("the unsnooze in the game's list", STATE_TIMEOUT, |live| live.row(&id)["sn"].is_null());

    let s = live.command(json!({ "t": "settle", "id": id }));
    live.expect_no_failure(s);
    assert_eq!(live.thread(&id)["settled"], true, "T3 didn't settle it");
    live.wait("the settle in the game's list", STATE_TIMEOUT, |live| live.row(&id)["set"] == true);
    live.checkpoint("a settled thread", &id, &[], &[]);
    let s = live.command(json!({ "t": "unsettle", "id": id }));
    live.expect_no_failure(s);
    assert_eq!(live.thread(&id)["settled"], false, "T3 didn't unsettle it");
    live.wait("the unsettle in the game's list", STATE_TIMEOUT, |live| live.row(&id)["set"] == false);
    live.finish();
}

/// Stop ends a run under way, and the game hears it's stopping.
#[test]
#[ignore = "live: starts agent runs on the T3 the companion is signed in to (npm run test:live)"]
fn live_stop_ends_a_run() {
    let mut live = Live::start("stop");
    let long = "Write the numbers from 1 to 3000 in words, one per line. Do not use any tools.";
    let id = live.new_thread(long);
    for attempt in 1..=TRIES {
        live.wait("the run to start", REPLY_TIMEOUT, |live| working(str_of(&live.thread(&id)["status"])));
        let s = live.command(json!({ "t": "stop", "id": id }));
        let o = live.outcome(s);
        assert_eq!(o["ok"], true, "Stop failed: {o}");
        if o["st"] == "stopping" {
            live.wait("the run to stop", REPLY_TIMEOUT, |live| !working(str_of(&live.thread(&id)["status"])));
            let status = str_of(&live.thread(&id)["status"]).to_string();
            assert!(matches!(status.as_str(), "interrupted" | "cancelled"), "the run ended as {status}, not stopped");
            live.wait("the stop in the game's list", STATE_TIMEOUT, |live| !working(str_of(&live.row(&id)["s"])));
            live.checkpoint("a stopped run", &id, &[], &[]);
            return live.finish();
        }
        assert!(attempt < TRIES, "every run ended before Stop reached it: {o}");
        eprintln!("  try {attempt}: the run ended before Stop, starting another");
        let s = live.command(json!({ "t": "send", "id": id, "m": long }));
        live.expect_no_failure(s);
    }
}

/// An agent's question shows in the game, and the answer picked there reaches the agent,
/// with what was typed in the box as a note on the pick.
#[test]
#[ignore = "live: starts agent runs on the T3 the companion is signed in to (npm run test:live)"]
fn live_an_answer_from_the_game_reaches_the_agent() {
    let mut live = Live::start("answer");
    let ask = "Use your tool for asking the user a question (AskUserQuestion) to ask me exactly one question, \
               \"Which color?\", with two options: Red and Blue. Once I answer, reply with only the color I picked, in capitals, and do what a note in my answer says. \
               Do not use any other tools.";
    let id = live.new_thread(ask);
    for attempt in 1..=TRIES {
        let asked = live.try_wait(REPLY_TIMEOUT, |live| {
            let view = live.snapshot()["view"].clone();
            (view["id"] == id.as_str() && view["ask"].is_object()) || (!working(str_of(&live.thread(&id)["status"])) && live.thread(&id)["status"] != "waiting")
        });
        let view = live.snapshot()["view"].clone();
        if asked.is_ok() && view["ask"].is_object() {
            let labels: Vec<String> = view["ask"]["qs"][0]["o"].as_array().into_iter().flatten().map(|o| str_of(&o["l"]).to_string()).collect();
            let blue = labels.iter().position(|l| l.eq_ignore_ascii_case("blue")).unwrap_or_else(|| panic!("no Blue among {labels:?}"));
            // Its text is in your message too: the buttons are what only the question shows.
            live.checkpoint("an agent's question", &id, &[], &["Red", "Blue", "Submit answer"]);
            // What was typed in the box goes with the pick, as the game sends it.
            let s = live.command(json!({ "t": "answer", "id": id, "k": view["ask"]["k"], "a": [[blue + 1, "Also say the word KIWI."]] }));
            live.expect_no_failure(s);
            live.wait("the reply to the answer and its note", REPLY_TIMEOUT, |live| live.agent_said(&id, "blue") && live.agent_said(&id, "kiwi"));
            live.checkpoint("the reply to an answer", &id, &["BLUE"], &[]);
            return live.finish();
        }
        assert!(view["approval"] != true, "the agent's question came as an approval, which T3's tools can't answer: {view:#}");
        assert!(attempt < TRIES, "the agent never asked: {:?}", live.said(&id, "a"));
        eprintln!("  try {attempt}: the agent didn't ask, asking it again");
        let s = live.command(json!({ "t": "send", "id": id, "m": ask }));
        live.expect_no_failure(s);
    }
}

/// The T3 tools the companion calls, and those whose calls it reads from a thread's timeline.
const WATCHED_TOOLS: &[&str] = &[
    "list_thread_pull_requests",
    "orchestrator_capabilities",
    "t3_attachment_discard",
    "t3_attachment_prepare_upload",
    "t3_environment_links",
    "t3_environment_read",
    "t3_pending_request",
    "t3_project_list",
    "t3_thread_configure",
    "t3_thread_interrupt",
    "t3_thread_launch",
    "t3_thread_list",
    "t3_thread_organize",
    "t3_thread_read",
    "t3_thread_send",
    "t3_thread_send_attachments",
    // Read from the timeline: its `html` becomes a picture (`draw_page`).
    "html_render",
];
const TOOLS_FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/t3-tools.json");

/// A tool's inputs as the fixture keeps them: each top-level input with its allowed values
/// (empty when any value goes), and which are required.
fn tool_inputs(tool: &Value) -> Value {
    let schema = &tool["inputSchema"];
    let mut inputs = serde_json::Map::new();
    for (name, input) in schema["properties"].as_object().into_iter().flatten() {
        let mut values: Vec<Value> = input["enum"].as_array().cloned().unwrap_or_default();
        values.sort_by_key(Value::to_string);
        inputs.insert(name.clone(), json!(values));
    }
    let mut required: Vec<&str> = schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    required.sort();
    json!({ "inputs": inputs, "required": required })
}

/// What changed between the inputs the companion was made for and T3's.
fn tool_changes(name: &str, was: &Value, now: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let (before, after) = (was["inputs"].as_object().cloned().unwrap_or_default(), now["inputs"].as_object().cloned().unwrap_or_default());
    for (input, values) in &after {
        match before.get(input) {
            None => out.push(format!("{name}: new input `{input}`")),
            Some(old) if old != values => out.push(format!("{name}: `{input}` takes {values} (was {old})")),
            _ => {}
        }
    }
    for input in before.keys().filter(|k| !after.contains_key(*k)) {
        out.push(format!("{name}: input `{input}` is gone"));
    }
    if was["required"] != now["required"] {
        out.push(format!("{name}: requires {} (was {})", now["required"], was["required"]));
    }
    out
}

/// T3's tools still take what the companion sends, and the ones it reads still look as it
/// expects. A change fails with what changed: read T3's release notes on it, fix the companion
/// where it needs to, then record T3's tools with `T3WOW_LIVE_UPDATE_TOOLS=1`.
#[test]
#[ignore = "live: reads the tools of the T3 the companion is signed in to (npm run test:live)"]
fn live_t3s_tools_take_what_the_companion_was_made_for() {
    let target = crate::t3::target().expect("no T3 to talk to: start T3 Code");
    let client = McpClient::new(&target.origin, &target.token.expect("the companion isn't signed in to T3: run t3wow.exe login first"));
    let tools = client.list_tools().expect("T3's tool list");
    let version = client.call("t3_environment_read", json!({})).ok().and_then(|e| e["serverVersion"].as_str().map(str::to_string)).unwrap_or_default();
    let mut now = serde_json::Map::new();
    for name in WATCHED_TOOLS {
        if let Some(tool) = tools.iter().find(|t| t["name"] == *name) {
            now.insert(name.to_string(), tool_inputs(tool));
        }
    }
    let fixture: Value = std::fs::read_to_string(TOOLS_FIXTURE).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    if std::env::var_os("T3WOW_LIVE_UPDATE_TOOLS").is_some() {
        let missing: Vec<&&str> = WATCHED_TOOLS.iter().filter(|n| !now.contains_key(**n)).collect();
        assert!(missing.is_empty(), "T3 {version} doesn't offer {missing:?}: nothing recorded");
        let recorded = json!({ "t3": version, "tools": now });
        std::fs::write(TOOLS_FIXTURE, serde_json::to_string_pretty(&recorded).unwrap() + "\n").unwrap();
        eprintln!("  recorded the tools of T3 {version} in {TOOLS_FIXTURE}");
        return;
    }
    let mut changes = Vec::new();
    for name in WATCHED_TOOLS {
        match (fixture["tools"].get(*name), now.get(*name)) {
            (Some(was), Some(is)) => changes.extend(tool_changes(name, was, is)),
            (Some(_), None) => changes.push(format!("{name}: T3 doesn't offer it any more")),
            (None, _) => changes.push(format!("{name}: not recorded yet")),
        }
    }
    assert!(
        changes.is_empty(),
        "T3 {version}'s tools changed since T3 {} (what the companion was made for):\n  {}\n\
         Read T3's release notes on each, fix the companion where it needs to, then record them: T3WOW_LIVE_UPDATE_TOOLS=1 npm run test:live",
        fixture["t3"].as_str().unwrap_or("?"),
        changes.join("\n  ")
    );
}

/// Every tool the companion's code calls is watched, so a new call can't go unchecked.
#[test]
fn every_tool_the_companion_calls_is_watched() {
    let sources = [include_str!("../../bridge.rs"), include_str!("../../app.rs"), include_str!("../../main.rs"), include_str!("../../t3.rs")];
    let called = regex::Regex::new(r#"\.call\(\s*"([a-z0-9_]+)""#).unwrap();
    let mut names: BTreeSet<&str> = BTreeSet::new();
    for source in sources {
        // The client's own tests call made-up and probe tools.
        let code = source.split("#[cfg(test)]\nmod tests").next().unwrap();
        names.extend(called.captures_iter(code).map(|c| c.get(1).unwrap().as_str()));
    }
    let unwatched: Vec<&&str> = names.iter().filter(|n| !WATCHED_TOOLS.contains(n)).collect();
    assert!(unwatched.is_empty(), "add these to WATCHED_TOOLS in live.rs: {unwatched:?}");
    assert!(names.len() >= 10, "found only {names:?}: has the call syntax changed?");
}

#[test]
fn a_tool_change_names_new_gone_and_changed_inputs() {
    let was = json!({ "inputs": { "html": [], "title": [], "action": ["a"] }, "required": ["html"] });
    let now = json!({ "inputs": { "html": [], "path": [], "action": ["a", "b"] }, "required": [] });
    assert_eq!(
        tool_changes("html_render", &was, &now),
        vec![
            "html_render: new input `path`".to_string(),
            "html_render: `action` takes [\"a\",\"b\"] (was [\"a\"])".to_string(),
            "html_render: input `title` is gone".to_string(),
            "html_render: requires [] (was [\"html\"])".to_string(),
        ]
    );
}

#[test]
fn the_shape_check_catches_a_missing_field_and_a_changed_kind_but_not_nulls_or_extras() {
    let fake = json!({ "threads": [{ "threadId": "t", "status": "running", "itemCount": 3, "runId": null }], "nextCursor": 5 });
    let live = json!({ "threads": [{ "threadId": "x", "status": 7, "extra": true }], "nextCursor": null });
    let mut out = BTreeSet::new();
    compare("t3_thread_list", &fake, &live, &mut out);
    assert_eq!(
        out.into_iter().collect::<Vec<_>>(),
        vec![
            "t3_thread_list.threads[].itemCount: the fake has it, T3 doesn't send it".to_string(),
            "t3_thread_list.threads[].status: the fake has a string, T3 sends a number".to_string(),
        ]
    );
}

#[test]
fn the_shape_check_matches_timeline_items_by_type() {
    let fake = json!({ "items": [{ "type": "user_message", "senderThreadId": "a" }] });
    let live = json!({ "items": [{ "type": "assistant_message" }, { "type": "user_message", "senderThreadId": "b" }] });
    let mut out = BTreeSet::new();
    compare("t3_thread_read", &fake, &live, &mut out);
    assert!(out.is_empty(), "{out:?}");
}
