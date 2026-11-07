use super::*;
use std::sync::Mutex;

struct FakeT3 {
    calls: Mutex<Vec<(String, Value)>>,
    threads: Mutex<Vec<Value>>,
    /// false: what's pending is an approval, which T3's pending list leaves out.
    questions: Mutex<bool>,
}

impl FakeT3 {
    fn new() -> Arc<FakeT3> {
        Arc::new(FakeT3 {
            calls: Mutex::new(Vec::new()),
            threads: Mutex::new(vec![json!({
                "threadId": "t1", "projectId": "p1", "title": "Fix tests", "status": "running",
                "settled": false, "updatedAt": "2026-11-06T10:00:00Z", "itemCount": 3
            })]),
            questions: Mutex::new(true),
        })
    }

    fn set(&self, status: &str, updated_at: &str) {
        let mut threads = self.threads.lock().unwrap();
        threads[0]["status"] = json!(status);
        threads[0]["updatedAt"] = json!(updated_at);
    }

    /// The thread a read asks for (the first one if it names none).
    fn thread(&self, id: &Value) -> Value {
        let threads = self.threads.lock().unwrap();
        threads.iter().find(|t| t["threadId"] == *id).unwrap_or(&threads[0]).clone()
    }

    fn calls_to(&self, name: &str) -> Vec<Value> {
        self.calls.lock().unwrap().iter().filter(|(n, _)| n == name).map(|(_, a)| a.clone()).collect()
    }
}

impl T3 for FakeT3 {
    fn call(&self, name: &str, args: Value) -> Result<Value, T3Error> {
        self.calls.lock().unwrap().push((name.to_string(), args.clone()));
        Ok(match name {
            "t3_environment_links" => json!({ "environmentId": "local", "links": [{ "environmentId": "mac", "label": "Mac", "status": "reachable" }] }),
            "t3_project_list" => json!({ "projects": [{ "id": "p1", "title": "app" }] }),
            "t3_thread_list" => {
                if args["settled"] == true {
                    json!({ "threads": [] })
                } else {
                    // Like T3, the list has no pending-question count: only a thread read does.
                    let mut threads = self.threads.lock().unwrap().clone();
                    for t in &mut threads {
                        t.as_object_mut().unwrap().remove("pendingRequestCount");
                    }
                    json!({ "threads": threads })
                }
            }
            "t3_thread_read" => json!({
                "thread": self.thread(&args["threadId"]),
                "items": if args["view"].is_string() {
                    json!([{ "type": "user_message", "text": "hi", "senderThreadId": self.thread(&args["threadId"])["sender"] }, { "type": "assistant_message", "text": "**done**" }])
                } else {
                    json!([])
                },
                "hasMore": false,
            }),
            // One pending question, in the shape T3 returns (see the probe in the commit log).
            "t3_pending_request" => match args["action"].as_str() {
                Some("list") if !*self.questions.lock().unwrap() => json!({ "requestIds": [] }),
                Some("list") => json!({ "requestIds": ["runtime-request:a-very-long-id"] }),
                Some("read") => json!({
                    "requestId": "runtime-request:a-very-long-id",
                    "questions": [
                        { "id": "Which color?", "header": "Color", "question": "Which color?", "multiSelect": false,
                          "options": [{ "label": "Red", "description": "Warm" }, { "label": "Blue", "description": "Cool" }] },
                        { "id": "Which extras?", "header": "Extras", "question": "Which extras?", "multiSelect": true,
                          "options": [{ "label": "Sound", "description": "" }, { "label": "Chat line", "description": "" }] },
                    ],
                }),
                _ => json!({ "sequence": 1 }),
            },
            _ => json!({}),
        })
    }
}

#[derive(Default)]
struct Written {
    slots: BTreeMap<u32, String>,
    raised: Vec<u32>,
}

struct FakeSlots(Arc<Mutex<Written>>);

impl SlotWriter for FakeSlots {
    fn write_slot(&mut self, n: u32, lua: &str) {
        self.0.lock().unwrap().slots.insert(n, lua.to_string());
    }
    fn raise_signal(&mut self, n: u32) {
        self.0.lock().unwrap().raised.push(n);
    }
    fn clear_signals(&mut self) {}
    fn flip_probe(&mut self, _real: bool) -> Vec<String> {
        Vec::new()
    }
}

struct Rig {
    bridge: Bridge,
    t3: Arc<FakeT3>,
    written: Arc<Mutex<Written>>,
    clock: Arc<Mutex<i64>>,
}

impl Rig {
    fn new(start_ms: i64) -> Rig {
        let t3 = FakeT3::new();
        let written = Arc::new(Mutex::new(Written::default()));
        let clock = Arc::new(Mutex::new(start_ms));
        let now = Arc::clone(&clock);
        let mut bridge = Bridge::new(t3.clone(), Box::new(FakeSlots(Arc::clone(&written))), None, Box::new(|_| {}), Box::new(move || *now.lock().unwrap()));
        bridge.on_capture(CaptureMessage::Wow(Some("123@1".into())));
        Rig { bridge, t3, written, clock }
    }

    fn advance(&self, ms: i64) {
        *self.clock.lock().unwrap() += ms;
    }

    fn now_s(&self) -> i64 {
        *self.clock.lock().unwrap() / 1000
    }

    fn frame(&mut self, value: Value) {
        let cells = crate::codec::encode_frame(1, &value.to_string());
        self.bridge.on_capture(CaptureMessage::Strip(cells));
    }

    fn raised(&self) -> Vec<u32> {
        self.written.lock().unwrap().raised.clone()
    }

    fn slot(&self, n: u32) -> String {
        self.written.lock().unwrap().slots.get(&n).cloned().unwrap_or_default()
    }

    /// (ns, nt) the update in slot `n` names.
    fn wait_in(&self, n: u32) -> (u32, i64) {
        let lua = self.slot(n);
        let field = |name: &str| -> i64 {
            let at = lua.find(&format!(",{name}=")).unwrap() + name.len() + 2;
            lua[at..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap()
        };
        (field("ns") as u32, field("nt"))
    }

    fn publishes(&self) -> usize {
        let written = self.written.lock().unwrap();
        let mut times: Vec<String> = written.slots.values().map(|lua| {
            let at = lua.find(",now=").unwrap() + 5;
            lua[at..].chars().take_while(|c| c.is_ascii_digit()).collect()
        }).collect();
        times.sort();
        times.dedup();
        times.len()
    }
}

#[test]
fn commands_from_a_frame_run_once_reach_the_linked_machine_and_are_acked() {
    let mut rig = Rig::new(1_000_000);
    let q = json!([{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "send", "e": "mac", "id": "t1", "m": "go" }]);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 5, "l": 1, "q": q }));
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 5, "l": 1, "q": q }));
    rig.bridge.tick();

    let sends = rig.t3.calls_to("t3_thread_send");
    assert_eq!(sends.len(), 1);
    assert_eq!(sends[0], json!({ "threadId": "t1", "message": "go", "mode": "auto", "clientRequestId": "t3wow:c1:2", "environmentId": "mac" }));
    // The game watches file 3 (past anything raised before this UI session): raise it and its later neighbour.
    assert_eq!(rig.raised(), vec![3, 4]);
    let inbox = rig.slot(1);
    assert!(inbox.starts_with("T3WoW_Inbox = {v=1,k=\"c1\",ack=2,"), "{inbox}");
    assert!(inbox.contains("{id=\"mac\",name=\"Mac\",ok=true}"));
    assert!(inbox.contains("{r=\"a\",x=\"done\"}"));
    assert!(inbox.contains(",slot=1,ns=5,"));

    // Nothing changed: no slot is spent.
    rig.advance(5000);
    rig.bridge.tick();
    assert_eq!(rig.raised(), vec![3, 4]);

    // The thread finishes: the next slot carries the change and an alert.
    rig.t3.set("completed", "2026-11-06T10:05:00Z");
    rig.advance(5000);
    rig.bridge.tick();
    assert_eq!(rig.raised(), vec![3, 4, 5, 6]);
    assert!(rig.slot(2).contains("e=\"mac\",id=\"t1\",t=\"Fix tests\",s=\"completed\"}"), "{}", rig.slot(2));
}

#[test]
fn without_signals_the_games_own_slot_count_drives_where_updates_go() {
    let mut rig = Rig::new(2_000_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 1, "l": 1, "p": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();
    assert!(rig.raised().is_empty());
    assert_eq!(rig.written.lock().unwrap().slots.keys().copied().collect::<Vec<_>>(), vec![1, 2, 3]);

    // The game polled ahead on its own (slots 1..6) and reports slot 7 in a sync frame.
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 1, "l": 7, "p": 1, "q": [] }));
    rig.t3.set("completed", "2026-11-06T10:05:00Z");
    rig.advance(5000);
    rig.bridge.tick();
    assert!(rig.raised().is_empty());
    assert!(rig.slot(7).contains("s=\"completed\""));
    assert_eq!(rig.bridge.link.slot, 7);
}

#[test]
fn the_list_follows_t3s_sidebar() {
    let mut rig = Rig::new(3_000_000);
    rig.t3.threads.lock().unwrap().extend([
        json!({ "threadId": "child", "projectId": "p1", "title": "Review", "status": "completed", "parentThreadId": "t1", "updatedAt": "2026-11-06T10:00:00Z" }),
        json!({ "threadId": "launched", "projectId": "p1", "title": "Test it", "status": "completed", "createdBy": "agent", "sender": "t1", "updatedAt": "2026-11-06T10:00:00Z" }),
        json!({ "threadId": "nap", "projectId": "p1", "title": "Later", "status": "completed", "snoozed": true, "updatedAt": "2026-11-06T10:00:00Z" }),
    ]);
    rig.bridge.tick();
    let snapshot = rig.bridge.snapshot();
    let ids: Vec<(String, bool, String)> = snapshot["threads"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["e"] == 1)
        .map(|t| (t["id"].as_str().unwrap().to_string(), t["sn"] == true, str_of(&t["par"]).to_string()))
        .collect();
    // Subagents stay out; a thread an agent launched comes under the thread that launched it.
    let row = |id: &str, sn: bool, par: &str| (id.to_string(), sn, par.to_string());
    assert_eq!(ids, vec![row("t1", false, ""), row("launched", false, "t1"), row("nap", true, "")]);

    // The launcher is read once, not on every poll.
    let reads = rig.t3.calls_to("t3_thread_read").len();
    rig.advance(15_000);
    rig.bridge.tick();
    assert_eq!(rig.t3.calls_to("t3_thread_read").len(), reads);
    for args in rig.t3.calls_to("t3_thread_list") {
        assert_eq!(args["includeSubagents"], false);
    }
}

#[test]
fn signals_follow_the_clock_between_updates_and_carry_on_after_reload() {
    let mut rig = Rig::new(3_000_000);
    // Link.lua's arithmetic: the file the game watches at time t for a wait (ns, nt).
    let game_watches = |(ns, nt): (u32, i64), t: i64| ns + ((t - nt).max(0) / 25) as u32;

    // Fresh UI, still polling: nothing raised, the update names the wait.
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "p": 1, "q": [{ "s": 1, "t": "hello", "o": false }] }));
    rig.bridge.tick();
    assert!(rig.raised().is_empty());
    let first = rig.wait_in(1);

    // The game took that wait and watches signals. 15 s later a thread finishes.
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [] }));
    rig.t3.set("completed", "2026-11-06T10:05:00Z");
    rig.advance(15_000);
    rig.bridge.tick();
    assert!(rig.raised().contains(&game_watches(first, rig.now_s())));

    // Five quiet minutes: the game has moved on 12 files; the next update raises that one.
    let second = rig.wait_in(2);
    rig.written.lock().unwrap().raised.clear();
    rig.t3.set("running", "2026-11-06T10:06:00Z");
    rig.advance(15_000);
    rig.bridge.tick();
    assert!(rig.raised().is_empty(), "the window is closed: a running thread isn't worth a slot");
    rig.t3.set("completed", "2026-11-06T10:10:00Z");
    rig.advance(285_000);
    rig.bridge.tick();
    let watched = game_watches(second, rig.now_s());
    assert_eq!(watched, second.0 + 12);
    assert_eq!(rig.raised(), vec![watched - 1, watched, watched + 1]);

    // /reload: the new wait starts past every file raised in this game process.
    rig.frame(json!({ "k": "c1", "u": "ui2", "n": 0, "l": 1, "p": 1, "q": [{ "s": 2, "t": "hello", "o": false }] }));
    rig.advance(5000);
    rig.bridge.tick();
    assert!(rig.wait_in(1).0 > watched + 1);
}

#[test]
fn with_the_window_open_routine_changes_wait_but_answers_go_out_at_once() {
    let mut rig = Rig::new(4_000_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();
    assert_eq!(rig.publishes(), 1);

    // A running agent touches the thread: routine, held back.
    rig.t3.set("running", "2026-11-06T10:00:01Z");
    rig.advance(4_000);
    rig.bridge.tick();
    assert_eq!(rig.publishes(), 1);

    // You send a message: its ack goes out right away.
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": 2, "t": "send", "id": "t1", "m": "hi" }] }));
    rig.advance(2_000);
    rig.bridge.tick();
    assert_eq!(rig.publishes(), 2);

    // Another routine change, then the 10 s gap passes.
    rig.t3.set("running", "2026-11-06T10:00:09Z");
    rig.advance(4_000);
    rig.bridge.tick();
    assert_eq!(rig.publishes(), 2);
    rig.advance(7_000);
    rig.bridge.tick();
    assert_eq!(rig.publishes(), 3);
}

#[test]
fn iso_dates_become_epoch_seconds() {
    assert_eq!(iso_seconds("1970-01-01T00:00:00Z"), 0);
    assert_eq!(iso_seconds("2026-11-07T00:22:41.923Z"), 1_794_010_961);
    assert_eq!(iso_seconds("2026-11-06T21:22:41-03:00"), 1_794_010_961);
    assert_eq!(iso_seconds(""), 0);
    assert_eq!(iso_seconds("nonsense"), 0);
}

#[test]
fn a_pending_question_reaches_the_game_and_its_answer_goes_back_by_label() {
    let mut rig = Rig::new(5_000_000);
    {
        let mut threads = rig.t3.threads.lock().unwrap();
        threads[0]["status"] = json!("waiting");
        threads[0]["pendingRequestCount"] = json!(1);
    }
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "t1" }] }));
    rig.bridge.tick();
    let inbox = rig.slot(1);
    assert!(inbox.contains("ask={k=\""), "{inbox}");
    assert!(inbox.contains("{h=\"Color\",q=\"Which color?\",m=false,o={{l=\"Red\",d=\"Warm\"},{l=\"Blue\",d=\"Cool\"}}}"), "{inbox}");
    assert!(!inbox.contains("a-very-long-id"), "the long request id stays in the companion");
    let token = {
        let at = inbox.find("ask={k=\"").unwrap() + 8;
        inbox[at..at + 8].to_string()
    };

    // The game answers by index: Blue, and both extras.
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": 3, "t": "answer", "id": "t1", "k": token, "a": [2, [1, 2]] }] }));
    rig.advance(2_000);
    rig.bridge.tick();
    let responds: Vec<Value> = rig.t3.calls_to("t3_pending_request").into_iter().filter(|a| a["action"] == "respond").collect();
    assert_eq!(responds.len(), 1);
    assert_eq!(responds[0]["requestId"], "runtime-request:a-very-long-id");
    assert_eq!(responds[0]["answers"], json!({ "Which color?": "Blue", "Which extras?": ["Sound", "Chat line"] }));

    // A stale token (the question changed) is refused, not sent.
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 3, "q": [{ "s": 4, "t": "answer", "id": "t1", "k": "00000000", "a": [1, [1]] }] }));
    rig.advance(2_000);
    rig.bridge.tick();
    assert_eq!(rig.t3.calls_to("t3_pending_request").into_iter().filter(|a| a["action"] == "respond").count(), 1);
}

#[test]
fn a_running_thread_with_a_question_needs_you_and_notifies() {
    let mut rig = Rig::new(6_000_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();
    assert!(rig.slot(1).contains("s=\"running\""), "{}", rig.slot(1));

    // T3 keeps saying "running" while the agent waits on a question.
    rig.t3.threads.lock().unwrap()[0]["pendingRequestCount"] = json!(1);
    rig.t3.set("running", "2026-11-06T10:01:00Z");
    rig.advance(15_000);
    rig.bridge.tick();
    let inbox = rig.written.lock().unwrap().slots.values().last().cloned().unwrap();
    assert!(inbox.contains("id=\"t1\",t=\"Fix tests\",s=\"waiting\""), "{inbox}");
    assert!(inbox.contains("e=\"local\",id=\"t1\",t=\"Fix tests\",s=\"waiting\"}"), "an alert: {inbox}");

    // Nothing changed: the next poll reuses the count instead of reading every thread again.
    let reads = rig.t3.calls_to("t3_thread_read").len();
    rig.advance(5_000);
    rig.bridge.tick();
    assert_eq!(rig.t3.calls_to("t3_thread_read").len(), reads);

    // Answered in T3: the agent runs again.
    rig.t3.threads.lock().unwrap()[0]["pendingRequestCount"] = json!(0);
    rig.t3.set("running", "2026-11-06T10:02:00Z");
    rig.advance(15_000);
    rig.bridge.tick();
    let latest = rig.written.lock().unwrap().slots.values().last().cloned().unwrap();
    assert!(latest.contains("id=\"t1\",t=\"Fix tests\",s=\"running\""), "{latest}");
}

#[test]
fn a_new_thread_that_already_asks_notifies_but_old_ones_dont_at_start() {
    let mut rig = Rig::new(7_000_000);
    rig.t3.set("waiting", "2026-11-06T10:00:00Z");
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": false }] }));
    rig.bridge.tick();
    assert!(rig.slot(1).contains("events={}"), "already waiting at start: {}", rig.slot(1));

    // A thread started since then, whose agent asked before the next list.
    rig.t3.threads.lock().unwrap().push(json!({
        "threadId": "t2", "projectId": "p1", "title": "Quick question", "status": "running",
        "settled": false, "updatedAt": "2026-11-06T10:03:00Z", "itemCount": 2, "pendingRequestCount": 1
    }));
    rig.advance(15_000);
    rig.bridge.tick();
    let latest = rig.written.lock().unwrap().slots.values().last().cloned().unwrap();
    assert!(latest.contains("e=\"local\",id=\"t2\",t=\"Quick question\",s=\"waiting\"}"), "{latest}");
    assert!(!latest.contains("id=\"t1\",t=\"Fix tests\",s=\"waiting\"}"), "{latest}");
}

#[test]
fn an_approval_shows_as_approve_in_t3_and_the_game_can_open_t3_on_it() {
    let mut rig = Rig::new(8_000_000);
    let opened = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = Arc::clone(&opened);
    rig.bridge.opener = Box::new(move |link| sink.lock().unwrap().push(link.to_string()));
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();

    // T3 says "running" with one pending request that its pending list doesn't show.
    *rig.t3.questions.lock().unwrap() = false;
    rig.t3.threads.lock().unwrap()[0]["threadId"] = json!("mcp:t1");
    rig.t3.threads.lock().unwrap()[0]["pendingRequestCount"] = json!(1);
    rig.t3.set("running", "2026-11-06T10:01:00Z");
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": 2, "t": "open", "id": "mcp:t1" }] }));
    rig.advance(15_000);
    rig.bridge.tick();
    let inbox = rig.written.lock().unwrap().slots.values().last().cloned().unwrap();
    assert!(inbox.contains("e=\"local\",id=\"mcp:t1\",t=\"Fix tests\",s=\"approval\"}"), "an alert: {inbox}");
    assert!(inbox.contains("approval=true"), "{inbox}");
    assert!(!inbox.contains("ask="), "{inbox}");

    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 3, "q": [{ "s": 3, "t": "t3", "e": "local", "id": "mcp:t1" }] }));
    rig.advance(2_000);
    rig.bridge.tick();
    assert_eq!(*opened.lock().unwrap(), vec!["t3code://threads/local/mcp%3At1".to_string()]);
}

#[test]
fn typed_text_answers_a_question_in_your_own_words() {
    let ask = PendingAsk {
        env: String::new(),
        thread: "t1".into(),
        token: "x".into(),
        request_id: "r".into(),
        questions: vec![PendingQuestion { key: "Name?".into(), labels: vec!["A".into()], multi: false }],
    };
    assert_eq!(Bridge::answers_for(&ask, &json!(["  my own words "])), json!({ "Name?": "my own words" }).as_object().unwrap().clone());
    assert!(Bridge::answers_for(&ask, &json!([9])).is_empty(), "an index past the options is ignored");
}
