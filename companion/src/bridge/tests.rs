use super::*;
use std::sync::Mutex;

struct FakeT3 {
    calls: Mutex<Vec<(String, Value)>>,
    /// When each call began, on the rig's clock.
    call_times: Mutex<Vec<i64>>,
    threads: Mutex<Vec<Value>>,
    /// false: what's pending is an approval, which T3's pending list leaves out.
    questions: Mutex<bool>,
    /// The server's own environment id; "local" also links a Mac.
    env_id: Mutex<String>,
    /// The machines the "local" server links.
    links: Mutex<Value>,
    /// Calls that take a while (ms, on the rig's clock) or fail, by tool name.
    delays: Mutex<HashMap<String, i64>>,
    failing: Mutex<HashMap<String, String>>,
    clock: Mutex<Option<Arc<Mutex<i64>>>>,
    /// Threads reads find but lists leave out.
    unlisted: Mutex<Vec<Value>>,
    /// Each thread's messages (two short ones when not set).
    messages: Mutex<HashMap<String, Vec<Value>>>,
}

impl FakeT3 {
    fn new() -> Arc<FakeT3> {
        Arc::new(FakeT3 {
            calls: Mutex::new(Vec::new()),
            call_times: Mutex::new(Vec::new()),
            threads: Mutex::new(vec![json!({
                "threadId": "t1", "projectId": "p1", "title": "Fix tests", "status": "running",
                "settled": false, "updatedAt": "2026-11-06T10:00:00Z", "itemCount": 3
            })]),
            questions: Mutex::new(true),
            env_id: Mutex::new("local".into()),
            links: Mutex::new(json!([{ "environmentId": "mac", "label": "Mac", "status": "reachable" }])),
            delays: Mutex::new(HashMap::new()),
            failing: Mutex::new(HashMap::new()),
            clock: Mutex::new(None),
            unlisted: Mutex::new(Vec::new()),
            messages: Mutex::new(HashMap::new()),
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
        let unlisted = self.unlisted.lock().unwrap();
        threads.iter().chain(unlisted.iter()).find(|t| t["threadId"] == *id).unwrap_or(&threads[0]).clone()
    }

    fn calls_to(&self, name: &str) -> Vec<Value> {
        self.calls.lock().unwrap().iter().filter(|(n, _)| n == name).map(|(_, a)| a.clone()).collect()
    }

    /// Calls to `name`, with when each began.
    fn calls_at(&self, name: &str) -> Vec<(Value, i64)> {
        let times = self.call_times.lock().unwrap();
        self.calls.lock().unwrap().iter().zip(times.iter()).filter(|((n, _), _)| n == name).map(|((_, a), t)| (a.clone(), *t)).collect()
    }
}

impl T3 for FakeT3 {
    fn call(&self, name: &str, args: Value) -> Result<Value, T3Error> {
        {
            let mut calls = self.calls.lock().unwrap();
            calls.push((name.to_string(), args.clone()));
            let at = self.clock.lock().unwrap().as_ref().map_or(0, |clock| *clock.lock().unwrap());
            self.call_times.lock().unwrap().push(at);
        }
        if let (Some(ms), Some(clock)) = (self.delays.lock().unwrap().get(name), self.clock.lock().unwrap().as_ref()) {
            *clock.lock().unwrap() += ms;
        }
        if let Some(message) = self.failing.lock().unwrap().get(name) {
            return Err(T3Error::Other(message.clone()));
        }
        Ok(match name {
            "t3_environment_links" => {
                let env_id = self.env_id.lock().unwrap().clone();
                let links = if env_id == "local" { self.links.lock().unwrap().clone() } else { json!([]) };
                json!({ "environmentId": env_id, "links": links })
            }
            // The "local" server leaves its name out, as an older T3 might.
            "t3_environment_read" if *self.env_id.lock().unwrap() != "local" => json!({ "label": "Server B" }),
            "t3_thread_launch" => json!({ "threadId": "new1" }),
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
            "t3_thread_read" if args["view"].is_string() => {
                // Messages after afterPosition, a page at a time.
                let id = str_of(&args["threadId"]).to_string();
                let all = self.messages.lock().unwrap().get(&id).cloned().unwrap_or_else(|| {
                    vec![
                        json!({ "position": 0, "type": "user_message", "text": "hi", "senderThreadId": self.thread(&args["threadId"])["sender"] }),
                        json!({ "position": 1, "type": "assistant_message", "text": "**done**" }),
                    ]
                });
                let after = args["afterPosition"].as_i64().unwrap_or(-1);
                let limit = args["limit"].as_u64().unwrap_or(100) as usize;
                let rest: Vec<Value> = all.into_iter().filter(|m| m["position"].as_i64().unwrap() > after).collect();
                let page: Vec<Value> = rest.iter().take(limit).cloned().collect();
                json!({
                    "thread": self.thread(&args["threadId"]),
                    "items": page,
                    "hasMore": rest.len() > limit,
                    "nextPosition": page.last().map(|m| m["position"].clone()),
                })
            }
            "t3_thread_read" => json!({ "thread": self.thread(&args["threadId"]), "items": [], "hasMore": false }),
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
    /// While set, every write fails with this error.
    failing: Option<String>,
}

struct FakeSlots(Arc<Mutex<Written>>);

impl SlotWriter for FakeSlots {
    fn write_slot(&mut self, n: u32, lua: &str) -> std::io::Result<()> {
        let mut written = self.0.lock().unwrap();
        if let Some(error) = &written.failing {
            return Err(std::io::Error::other(error.clone()));
        }
        written.slots.insert(n, lua.to_string());
        Ok(())
    }
    fn raise_signal(&mut self, n: u32) -> std::io::Result<()> {
        let mut written = self.0.lock().unwrap();
        if let Some(error) = &written.failing {
            return Err(std::io::Error::other(error.clone()));
        }
        written.raised.push(n);
        Ok(())
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
        *t3.clock.lock().unwrap() = Some(Arc::clone(&clock));
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
    assert_eq!(sends[0], json!({ "threadId": "t1", "message": "go", "mode": "auto", "clientRequestId": sends[0]["clientRequestId"], "environmentId": "mac" }));
    assert!(sends[0]["clientRequestId"].as_str().unwrap().starts_with("t3wow:c1:2:"));
    // The game watches file 3 (past anything raised before this UI session): raise it and its later neighbour.
    assert_eq!(rig.raised(), vec![3, 4]);
    let inbox = rig.slot(1);
    assert!(inbox.starts_with("T3WoW_Inbox = {v=1,k=\"c1\",u=\"ui1\",ack=2,"), "{inbox}");
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
    assert_eq!(iso_seconds("2026-11-06T21:22:41-0300"), 1_794_010_961);
    assert_eq!(iso_seconds("2026-11-07T05:52:41.5+05:30"), 1_794_010_961);
    assert_eq!(iso_seconds("2026-11-07T05:52:41+0530"), 1_794_010_961);
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

#[test]
fn pointed_at_another_server_the_old_servers_threads_and_open_thread_go() {
    let mut rig = Rig::new(9_000_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "e": "mac", "id": "t1" }] }));
    rig.bridge.tick();
    let snapshot = rig.bridge.snapshot();
    assert_eq!(snapshot["threads"].as_array().unwrap().len(), 2, "t1 on this server and on the Mac");
    assert!(snapshot["view"].is_object());

    // The same server again (T3 restarted on another port): the open thread stays.
    rig.bridge.set_client(rig.t3.clone());
    rig.advance(5000);
    rig.bridge.tick();
    assert!(rig.bridge.snapshot()["view"].is_object());

    // Another server, with one thread of its own and no linked machines.
    let other = FakeT3::new();
    *other.env_id.lock().unwrap() = "other".into();
    other.threads.lock().unwrap()[0]["threadId"] = json!("b1");
    rig.bridge.set_client(other.clone());
    rig.advance(5000);
    rig.bridge.tick();
    let snapshot = rig.bridge.snapshot();
    let ids: Vec<&str> = snapshot["threads"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["b1"]);
    // Its threads go under its own name, not this PC's.
    assert_eq!(snapshot["envs"], json!([{ "id": "other", "name": "Server B", "ok": true }]));
    assert!(snapshot["view"].is_null(), "{}", snapshot["view"]);
    assert!(other.calls_to("t3_thread_read").iter().all(|args| args["threadId"] != "t1"));
    assert!(rig.bridge.status().t3_ok);
}

#[test]
fn after_a_game_crash_rolls_command_numbers_back_new_commands_still_run() {
    let mut rig = Rig::new(10_000_000);
    let sends = |n: u64| (1..=n).map(|s| json!({ "s": s, "t": "send", "id": "t1", "m": format!("message {s}") })).collect::<Vec<_>>();
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": sends(5) }));
    rig.bridge.tick();
    assert_eq!(rig.t3.calls_to("t3_thread_send").len(), 5);
    assert_eq!(rig.bridge.link.acked["c1"], 5);

    // WoW crashed before saving: the next session numbers its commands from 1 again.
    rig.frame(json!({ "k": "c1", "u": "ui2", "n": 0, "l": 1, "q": [{ "s": 1, "t": "hello", "o": false }, { "s": 2, "t": "send", "id": "t1", "m": "after the crash" }] }));
    rig.advance(5000);
    rig.bridge.tick();
    let sent = rig.t3.calls_to("t3_thread_send");
    assert_eq!(sent.len(), 6);
    assert_eq!(sent[5]["message"], "after the crash");
    // Same number as an earlier message, but another request id: T3 doesn't drop it as a repeat.
    assert_ne!(sent[5]["clientRequestId"], sent[1]["clientRequestId"]);
    assert_eq!(rig.bridge.link.acked["c1"], 2);
    let inbox = rig.written.lock().unwrap().slots.values().last().cloned().unwrap();
    assert!(inbox.contains("u=\"ui2\",ack=2,"), "{inbox}");
}

#[test]
fn a_command_counts_as_done_only_once_it_ran() {
    let mut rig = Rig::new(11_000_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "send", "id": "t1", "m": "hi" }] }));
    // Seen, not yet run: the saved ack hasn't moved, and the same frame again doesn't queue it twice.
    assert_eq!(rig.bridge.link.acked.get("c1").copied().unwrap_or(0), 0);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "send", "id": "t1", "m": "hi" }] }));
    rig.bridge.tick();
    assert_eq!(rig.t3.calls_to("t3_thread_send").len(), 1);
    assert_eq!(rig.bridge.link.acked["c1"], 1);
}

#[test]
fn near_the_last_slot_the_budget_shows_and_past_it_nothing_is_written() {
    let mut rig = Rig::new(12_000_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 199, "p": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();
    assert_eq!(rig.written.lock().unwrap().slots.keys().copied().collect::<Vec<_>>(), vec![199, 200], "never past slot 200");
    assert_eq!(rig.bridge.status().slot, 199);

    // The game loaded both and reports it is out.
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 201, "p": 1, "q": [] }));
    rig.written.lock().unwrap().slots.clear();
    rig.t3.set("completed", "2026-11-06T10:05:00Z");
    rig.advance(15_000);
    rig.bridge.tick();
    assert!(rig.written.lock().unwrap().slots.is_empty());
    assert_eq!(rig.bridge.status().slot, 201);
}

#[test]
fn when_the_signal_files_run_out_updates_still_go_out_on_the_timer() {
    let mut rig = Rig::new(13_000_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": false }] }));
    rig.bridge.tick();
    // A long day in one game process: the game now watches file 3998.
    rig.bridge.link.signal = 3998;
    rig.bridge.link.signal_time = rig.now_s();
    rig.t3.set("completed", "2026-11-06T10:05:00Z");
    rig.advance(15_000);
    rig.bridge.tick();
    // The last files there are carry an update that tells the game to use the timer.
    assert_eq!(rig.raised().last(), Some(&3999));
    let inbox = rig.slot(2);
    assert!(inbox.contains("sigoff=\""), "{inbox}");
    assert!(rig.bridge.status().signals_out);
    assert!(!rig.bridge.status().signals);

    // The next change is written without raising anything, where the game's count says.
    let raised = rig.raised().len();
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 3, "p": 1, "q": [{ "s": 2, "t": "ui", "o": true }] }));
    rig.advance(15_000);
    rig.bridge.tick();
    assert_eq!(rig.raised().len(), raised);
    assert!(rig.slot(3).contains("ack=2"), "{}", rig.slot(3));

    // A new WoW process starts with fresh signal files.
    rig.bridge.on_capture(CaptureMessage::Wow(Some("456@1".into())));
    assert!(!rig.bridge.status().signals_out);
}

#[test]
fn the_game_watching_past_the_last_signal_file_gets_its_updates_by_timer() {
    let mut rig = Rig::new(14_000_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": false }] }));
    rig.bridge.tick();
    rig.bridge.link.signal = 4000;
    rig.bridge.link.signal_time = rig.now_s() - 100;
    rig.t3.set("completed", "2026-11-06T10:05:00Z");
    rig.advance(15_000);
    rig.bridge.tick();
    assert!(rig.raised().iter().all(|n| *n <= SIGNAL_COUNT), "{:?}", rig.raised());
    assert!(rig.slot(2).contains("sigoff=\"") && rig.slot(2).contains("s=\"completed\""), "{}", rig.slot(2));
}

#[test]
fn a_failed_write_is_retried_and_doesnt_use_up_a_slot() {
    let mut rig = Rig::new(15_000_000);
    rig.written.lock().unwrap().failing = Some("access denied".into());
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    for _ in 0..3 {
        rig.bridge.tick();
        rig.advance(2_000);
    }
    assert_eq!(rig.bridge.link.slot, 1, "no slot spent");
    assert!(rig.bridge.status().write_problem.unwrap().contains("access denied"));

    rig.written.lock().unwrap().failing = None;
    rig.bridge.tick();
    assert!(rig.slot(1).contains("ack=1"), "{}", rig.slot(1));
    assert_eq!(rig.bridge.link.slot, 2);
    assert!(rig.bridge.status().write_problem.is_none());

    // Nothing new: the delivered state isn't sent again.
    rig.advance(2_000);
    rig.bridge.tick();
    assert_eq!(rig.bridge.link.slot, 2);
}

#[test]
fn after_a_reinstall_the_state_goes_out_again() {
    let mut rig = Rig::new(16_000_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": false }] }));
    rig.bridge.tick();
    rig.advance(2_000);
    rig.bridge.tick();
    assert_eq!(rig.bridge.link.slot, 2);
    rig.bridge.republish();
    rig.bridge.tick();
    assert_eq!(rig.bridge.link.slot, 3);
}

#[test]
fn a_machine_no_longer_linked_leaves_the_list_and_a_reorder_keeps_ids_straight() {
    let mut rig = Rig::new(17_000_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "e": "mac", "id": "t1" }] }));
    rig.bridge.tick();
    let machines = |rig: &Rig| -> Vec<(i64, String)> {
        let snapshot = rig.bridge.snapshot();
        let names: Vec<String> = snapshot["envs"].as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap().to_string()).collect();
        snapshot["threads"].as_array().unwrap().iter().map(|t| {
            let e = t["e"].as_i64().unwrap();
            (e, names[e as usize - 1].clone())
        }).collect()
    };
    assert_eq!(machines(&rig), vec![(1, "local".into()), (2, "mac".into())]);

    // Another machine is linked ahead of the Mac: the Mac's threads follow it to index 3.
    *rig.t3.links.lock().unwrap() = json!([
        { "environmentId": "linux", "label": "Linux", "status": "unreachable" },
        { "environmentId": "mac", "label": "Mac", "status": "reachable" },
    ]);
    rig.advance(61_000);
    rig.bridge.tick();
    assert_eq!(machines(&rig), vec![(1, "local".into()), (3, "mac".into())]);
    assert_eq!(rig.bridge.snapshot()["view"]["e"], 3);

    // The Mac is unlinked: its threads, projects and open thread go, instead of showing under this PC.
    *rig.t3.links.lock().unwrap() = json!([{ "environmentId": "linux", "label": "Linux", "status": "unreachable" }]);
    rig.advance(61_000);
    rig.bridge.tick();
    let snapshot = rig.bridge.snapshot();
    assert_eq!(machines(&rig), vec![(1, "local".into())]);
    assert!(snapshot["projects"].as_array().unwrap().iter().all(|p| p["e"] == 1), "{}", snapshot["projects"]);
    assert!(snapshot["view"].is_null());
}

#[test]
fn a_slow_t3_doesnt_hold_up_the_games_commands_between_poll_steps() {
    let mut rig = Rig::new(18_000_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();
    // Every thread list now takes 5 s: a poll's list step is slower than a tick's budget.
    rig.t3.delays.lock().unwrap().insert("t3_thread_list".into(), 5_000);
    rig.advance(5_000);
    rig.bridge.tick();
    assert!(rig.bridge.poll_run.is_some(), "the poll gave way after its slow step");

    // Stop, pressed meanwhile, runs on the next tick, before the poll goes on.
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": 2, "t": "stop", "id": "t1" }] }));
    let first = rig.bridge.poll_run.as_ref().map(|run| run.started);
    rig.bridge.tick();
    assert_eq!(rig.t3.calls_to("t3_thread_interrupt").len(), 1);
    // The poll under way ends, and another follows straight away for the command.
    for _ in 0..20 {
        rig.bridge.tick();
        match rig.bridge.poll_run.as_ref() {
            Some(run) if Some(run.started) == first => continue,
            Some(_) => return,
            None => {
                assert!(rig.bridge.poll_at <= *rig.clock.lock().unwrap());
                return;
            }
        }
    }
    panic!("the poll never ended");
}

#[test]
fn a_command_waits_for_one_request_at_most_while_a_slow_poll_reads() {
    let mut rig = Rig::new(18_500_000);
    *rig.t3.links.lock().unwrap() = json!([]);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();

    // Every read now takes 3 s. The next poll has plenty of them: three lists, a question
    // check and what it waits on, a launcher, and the thread opened below, which has 120
    // messages over 2000 items (three passes that find none, then two pages), and its question.
    const REQUEST: i64 = 3_000;
    for tool in ["t3_environment_links", "t3_environment_read", "t3_project_list", "t3_thread_list", "t3_thread_read", "t3_pending_request"] {
        rig.t3.delays.lock().unwrap().insert(tool.into(), REQUEST);
    }
    {
        let mut threads = rig.t3.threads.lock().unwrap();
        threads[0]["updatedAt"] = json!("2026-11-06T10:01:00Z");
        threads[0]["pendingRequestCount"] = json!(1);
        threads[0]["itemCount"] = json!(2000);
        threads.push(json!({ "threadId": "launched", "projectId": "p1", "title": "Test it", "status": "completed", "createdBy": "agent", "sender": "t1", "updatedAt": "2026-11-06T10:00:00Z" }));
    }
    let messages: Vec<Value> = (0..120).map(|i| json!({ "position": i, "type": "assistant_message", "text": format!("m{i}") })).collect();
    rig.t3.messages.lock().unwrap().insert("t1".into(), messages);

    // The worker's loop, as in app.rs: frames come in between ticks, with no nap mid-poll.
    // Stop is pressed every 1.7 s, so presses land all through the poll's reads.
    let start = *rig.clock.lock().unwrap();
    let mut pressed: Vec<(u64, i64)> = Vec::new();
    while *rig.clock.lock().unwrap() < start + 60_000 {
        let now = *rig.clock.lock().unwrap();
        while pressed.last().map_or(start + 1_000, |(_, at)| at + 1_700) <= now {
            let at = pressed.last().map_or(start + 1_000, |(_, at)| at + 1_700);
            pressed.push((pressed.len() as u64 + 3, at));
        }
        let mut q = vec![json!({ "s": 2, "t": "open", "id": "t1" })];
        q.extend(pressed.iter().map(|(s, _)| json!({ "s": s, "t": "stop", "id": "t1" })));
        rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": q }));
        rig.bridge.tick();
        if !rig.bridge.polling() {
            rig.advance(250);
        }
    }

    // The poll read all of it meanwhile.
    let pages = rig.t3.calls_to("t3_thread_read").iter().filter(|a| a["threadId"] == "t1" && a["view"] == "messages" && a["limit"] == 100).count();
    assert_eq!(pages, 5);
    assert!(rig.t3.calls_to("t3_thread_read").iter().any(|a| a["threadId"] == "launched"));
    assert!(rig.t3.calls_to("t3_pending_request").iter().any(|a| a["action"] == "read"));
    assert_eq!(rig.bridge.snapshot()["view"]["items"].as_array().unwrap().len(), VIEW_MESSAGES);

    // Each Stop reached T3 within one request of being pressed.
    let stops = rig.t3.calls_at("t3_thread_interrupt");
    assert!(pressed.len() >= 30, "{} presses", pressed.len());
    for (s, at) in &pressed {
        let prefix = format!("t3wow:c1:{s}:");
        let Some((_, ran)) = stops.iter().find(|(a, _)| str_of(&a["clientRequestId"]).starts_with(&prefix)) else { continue };
        assert!(ran - at <= REQUEST, "Stop {s}, pressed at +{} ms, ran at +{} ms", at - start, ran - start);
    }
    assert!(stops.len() + 1 >= pressed.len(), "{} of {} presses ran", stops.len(), pressed.len());
}

#[test]
fn opening_another_thread_while_the_open_one_is_read_drops_that_read() {
    let mut rig = Rig::new(18_700_000);
    *rig.t3.links.lock().unwrap() = json!([]);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "t1" }] }));
    rig.bridge.tick();
    let mut t2 = rig.t3.threads.lock().unwrap()[0].clone();
    t2["threadId"] = json!("t2");
    t2["title"] = json!("Other");
    rig.t3.threads.lock().unwrap().push(t2);

    // t1 changes and reads are slow: the next poll reads it again, a request at a time.
    rig.t3.delays.lock().unwrap().insert("t3_thread_read".into(), 3_000);
    rig.t3.set("running", "2026-11-06T10:01:00Z");
    rig.advance(4_000);
    while rig.bridge.poll_run.as_ref().is_none_or(|run| run.reading.is_none()) {
        rig.bridge.tick();
    }
    let t1_pages = |rig: &Rig| rig.t3.calls_to("t3_thread_read").iter().filter(|a| a["threadId"] == "t1" && a["view"] == "messages").count();
    let pages = t1_pages(&rig);

    // Opening t2 meanwhile drops t1's read: it doesn't come back as the open thread, and
    // its cache stays for a later read.
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 3, "t": "open", "id": "t2" }] }));
    rig.bridge.tick();
    assert_eq!(t1_pages(&rig), pages);
    assert!(rig.bridge.view_key.is_none());
    assert_eq!(rig.bridge.view_cache.as_ref().map(|c| c.id.as_str()), Some("t1"));
    for _ in 0..20 {
        if rig.bridge.snapshot()["view"]["id"] == "t2" {
            break;
        }
        rig.bridge.tick();
    }
    assert_eq!(rig.bridge.snapshot()["view"]["t"], "Other");
    assert_eq!(t1_pages(&rig), pages);
}

#[test]
fn the_next_poll_counts_from_the_end_of_the_last_and_failures_space_them_out() {
    let mut rig = Rig::new(19_000_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": false }] }));
    rig.bridge.tick();
    // A poll that takes 30 s: the next one is 12 s after it ends, not straight away.
    rig.t3.delays.lock().unwrap().insert("t3_thread_list".into(), 5_000);
    rig.advance(12_000);
    while {
        rig.bridge.tick();
        rig.bridge.poll_run.is_some()
    } {}
    let ended = *rig.clock.lock().unwrap();
    assert_eq!(rig.bridge.poll_at, ended + 12_000);
    rig.t3.delays.lock().unwrap().clear();

    // T3 fails: 24 s, 48 s, then at most a minute between tries.
    rig.t3.failing.lock().unwrap().insert("t3_thread_list".into(), "connection refused".into());
    rig.t3.failing.lock().unwrap().insert("t3_environment_links".into(), "connection refused".into());
    let mut gaps = Vec::new();
    for _ in 0..4 {
        let wait = rig.bridge.poll_at - *rig.clock.lock().unwrap();
        rig.advance(wait.max(0));
        rig.bridge.tick();
        gaps.push(rig.bridge.poll_at - *rig.clock.lock().unwrap());
    }
    assert_eq!(gaps, vec![24_000, 48_000, 60_000, 60_000]);
    assert!(!rig.bridge.status().t3_ok);

    rig.t3.failing.lock().unwrap().clear();
    rig.advance(60_000);
    rig.bridge.tick();
    assert!(rig.bridge.status().t3_ok);
    assert_eq!(rig.bridge.poll_at - *rig.clock.lock().unwrap(), 12_000);
}

#[test]
fn past_the_polls_deadline_question_checks_wait_for_the_next_poll() {
    let mut rig = Rig::new(20_000_000);
    {
        let mut threads = rig.t3.threads.lock().unwrap();
        for n in 2..=6 {
            let mut t = threads[0].clone();
            t["threadId"] = json!(format!("t{n}"));
            threads.push(t);
        }
    }
    // Each check reads the thread, and each read takes 8 s.
    rig.t3.delays.lock().unwrap().insert("t3_thread_read".into(), 8_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": false }] }));
    while {
        rig.bridge.tick();
        rig.bridge.poll_run.is_some()
    } {}
    // 12 working threads on two machines; the deadline (20 s) allows three reads, plus the one under way.
    let checks = rig.t3.calls_to("t3_thread_read").len();
    assert!(checks <= 4, "{checks} reads");
    assert_eq!(rig.bridge.snapshot()["threads"].as_array().unwrap().len(), 12, "the lists are all there");
}

#[test]
fn a_thread_answered_and_finished_between_two_looks_still_notifies() {
    let mut rig = Rig::new(21_000_000);
    {
        let mut threads = rig.t3.threads.lock().unwrap();
        threads[0]["pendingRequestCount"] = json!(1);
    }
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": false }] }));
    rig.bridge.tick();
    rig.t3.set("running", "2026-11-06T10:01:00Z");
    rig.advance(15_000);
    rig.bridge.tick();
    assert_eq!(rig.bridge.last_status["local:t1"], "waiting");

    // Answered in T3, and the agent finished before the next poll.
    rig.t3.threads.lock().unwrap()[0]["pendingRequestCount"] = json!(0);
    rig.t3.set("completed", "2026-11-06T10:02:00Z");
    rig.advance(15_000);
    rig.bridge.tick();
    let latest = rig.written.lock().unwrap().slots.values().last().cloned().unwrap();
    assert!(latest.contains("e=\"local\",id=\"t1\",t=\"Fix tests\",s=\"completed\"}"), "{latest}");
}

#[test]
fn a_working_thread_that_drops_off_the_list_still_notifies_when_it_ends() {
    let mut rig = Rig::new(22_000_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": false }] }));
    rig.bridge.tick();
    // t1 leaves the lists (settled past their limits) but still runs; reads still find it.
    let t1 = rig.t3.threads.lock().unwrap()[0].clone();
    rig.t3.threads.lock().unwrap()[0]["threadId"] = json!("t2");
    rig.t3.unlisted.lock().unwrap().push(t1);
    rig.advance(15_000);
    rig.bridge.tick();
    assert_eq!(rig.bridge.last_status.get("local:t1").map(String::as_str), Some("running"), "still watched while it runs");

    rig.t3.unlisted.lock().unwrap()[0]["status"] = json!("completed");
    rig.advance(15_000);
    rig.bridge.tick();
    let latest = rig.written.lock().unwrap().slots.values().last().cloned().unwrap();
    assert!(latest.contains("e=\"local\",id=\"t1\",t=\"Fix tests\",s=\"completed\"}"), "{latest}");
    assert!(!rig.bridge.last_status.contains_key("local:t1"), "forgotten once it ended");
}

#[test]
fn a_working_thread_off_the_list_survives_failed_reads_and_still_notifies() {
    let mut rig = Rig::new(22_500_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": false }] }));
    rig.bridge.tick();
    let t1 = rig.t3.threads.lock().unwrap()[0].clone();
    rig.t3.threads.lock().unwrap()[0]["threadId"] = json!("t2");
    rig.t3.unlisted.lock().unwrap().push(t1);
    // T3 can't read threads for a while: t1 is still watched, not forgotten.
    rig.t3.failing.lock().unwrap().insert("t3_thread_read".into(), "busy".into());
    for _ in 0..2 {
        rig.advance(15_000);
        rig.bridge.tick();
    }
    assert_eq!(rig.bridge.last_status.get("local:t1").map(String::as_str), Some("running"), "kept through failed reads");

    rig.t3.failing.lock().unwrap().clear();
    rig.t3.unlisted.lock().unwrap()[0]["status"] = json!("completed");
    rig.advance(15_000);
    rig.bridge.tick();
    let latest = rig.written.lock().unwrap().slots.values().last().cloned().unwrap();
    assert!(latest.contains("e=\"local\",id=\"t1\",t=\"Fix tests\",s=\"completed\"}"), "{latest}");

    // A thread that stays unreadable (deleted) is dropped after a few polls.
    rig.t3.unlisted.lock().unwrap()[0]["status"] = json!("running");
    rig.bridge.last_status.insert("local:t1".into(), "running".into());
    rig.t3.failing.lock().unwrap().insert("t3_thread_read".into(), "not found".into());
    for _ in 0..UNLISTED_MISSES {
        rig.advance(15_000);
        rig.bridge.tick();
    }
    assert!(!rig.bridge.last_status.contains_key("local:t1"), "dropped after {UNLISTED_MISSES} failed reads");
}

/// A thread's messages, as T3 numbers them (every other position, as tool calls sit between).
fn conversation(texts: &[&str]) -> Vec<Value> {
    texts
        .iter()
        .enumerate()
        .map(|(i, text)| json!({ "position": i * 2, "type": if i % 2 == 0 { "user_message" } else { "assistant_message" }, "text": text }))
        .collect()
}

#[test]
fn the_open_thread_reads_only_whats_new_and_still_sees_its_last_message_grow() {
    let mut rig = Rig::new(23_000_000);
    rig.t3.messages.lock().unwrap().insert("t1".into(), conversation(&["one", "two", "three"]));
    rig.t3.threads.lock().unwrap()[0]["itemCount"] = json!(5);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "t1" }] }));
    rig.bridge.tick();
    let message_reads = |rig: &Rig| rig.t3.calls_to("t3_thread_read").into_iter().filter(|a| a["view"] == "messages" && a["threadId"] == "t1" && a["limit"] == 100).collect::<Vec<_>>();
    assert_eq!(message_reads(&rig).len(), 1);

    // The agent's last message grows, and a new one comes.
    rig.t3.messages.lock().unwrap().insert("t1".into(), conversation(&["one", "two", "three, and more", "four"]));
    rig.t3.threads.lock().unwrap()[0]["itemCount"] = json!(7);
    rig.t3.set("running", "2026-11-06T10:00:05Z");
    rig.advance(5_000);
    rig.bridge.tick();
    let reads = message_reads(&rig);
    assert_eq!(reads.len(), 2);
    assert_eq!(reads[1]["afterPosition"], 3, "read on from the last message seen (position 4), itself included");
    let view = rig.bridge.snapshot()["view"].clone();
    let texts: Vec<&str> = view["items"].as_array().unwrap().iter().map(|i| i["x"].as_str().unwrap()).collect();
    assert_eq!(texts, vec!["one", "two", "three, and more", "four"]);

    // A rollback leaves fewer items: everything is read again.
    rig.t3.messages.lock().unwrap().insert("t1".into(), conversation(&["one", "two"]));
    rig.t3.threads.lock().unwrap()[0]["itemCount"] = json!(3);
    rig.t3.set("running", "2026-11-06T10:00:10Z");
    rig.advance(5_000);
    rig.bridge.tick();
    let reads = message_reads(&rig);
    assert!(reads[2].get("afterPosition").is_none(), "{}", reads[2]);
    let view = rig.bridge.snapshot()["view"].clone();
    assert_eq!(view["items"].as_array().unwrap().len(), 2);
}

#[test]
fn an_open_thread_past_the_lists_limits_is_still_read_again_now_and_then() {
    let mut rig = Rig::new(24_000_000);
    let old = json!({ "threadId": "old", "projectId": "p1", "title": "Old", "status": "completed", "settled": true, "updatedAt": "2026-01-01T00:00:00Z", "itemCount": 3 });
    rig.t3.unlisted.lock().unwrap().push(old);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "old" }] }));
    rig.bridge.tick();
    assert_eq!(rig.bridge.snapshot()["view"]["t"], "Old");

    rig.t3.unlisted.lock().unwrap()[0]["title"] = json!("Old, renamed");
    rig.advance(25_000);
    rig.bridge.tick();
    assert_eq!(rig.bridge.snapshot()["view"]["t"], "Old, renamed");
}

#[test]
fn a_question_that_cant_be_read_once_stays_a_question() {
    let mut rig = Rig::new(25_000_000);
    {
        let mut threads = rig.t3.threads.lock().unwrap();
        threads[0]["status"] = json!("waiting");
        threads[0]["pendingRequestCount"] = json!(1);
    }
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "t1" }] }));
    rig.bridge.tick();
    assert!(rig.bridge.snapshot()["view"]["ask"].is_object());

    rig.t3.failing.lock().unwrap().insert("t3_pending_request".into(), "timed out".into());
    rig.t3.set("waiting", "2026-11-06T10:00:05Z");
    rig.advance(5_000);
    rig.bridge.tick();
    let view = rig.bridge.snapshot()["view"].clone();
    assert!(view["ask"].is_object(), "{view}");
    assert!(view["approval"].is_null(), "not shown as an approval: {view}");
}

#[test]
fn the_game_learns_which_commands_failed_and_which_thread_a_start_opened() {
    let mut rig = Rig::new(26_000_000);
    rig.t3.failing.lock().unwrap().insert("t3_thread_send".into(), "T3 refused the message".into());
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [
        { "s": 1, "t": "ui", "o": true },
        { "s": 2, "t": "send", "e": "mac", "id": "t1", "m": "hello" },
        { "s": 3, "t": "new", "e": "mac", "p": "p1", "m": "Build it" },
        { "s": 4, "t": "answer", "id": "t1", "k": "00000000", "a": [1] },
    ] }));
    rig.bridge.tick();
    let snapshot = rig.bridge.snapshot();
    assert_eq!(snapshot["out"], json!([
        { "s": 2, "ok": false, "x": "T3 refused the message" },
        { "s": 3, "ok": true, "env": "mac", "id": "new1" },
        { "s": 4, "ok": false, "x": "that question was already answered or has changed" },
    ]));
    assert_eq!(snapshot["view"]["id"], "new1", "the companion opens the new thread");
    assert!(rig.slot(1).contains("out={{s=2,ok=false,"), "{}", rig.slot(1));

    // They stay a minute, then go; a new UI session starts without them.
    rig.advance(61_000);
    rig.bridge.tick();
    assert!(rig.bridge.snapshot()["out"].is_null());
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": 5, "t": "send", "id": "t1", "m": "again" }] }));
    rig.bridge.tick();
    assert_eq!(rig.bridge.snapshot()["out"].as_array().unwrap().len(), 1);
    rig.frame(json!({ "k": "c1", "u": "ui2", "n": 0, "l": 1, "q": [{ "s": 6, "t": "hello", "o": false }] }));
    assert!(rig.bridge.snapshot()["out"].is_null());
}

/// Every command kind the window sends, one frame's worth (the answer names the question
/// the fake T3 asks, so it reaches T3 when allowed).
fn every_kind(rig: &mut Rig, u: &str) -> Vec<(u64, &'static str)> {
    {
        let mut threads = rig.t3.threads.lock().unwrap();
        threads[0]["status"] = json!("waiting");
        threads[0]["pendingRequestCount"] = json!(1);
    }
    rig.bridge.view = Some(View { env: "local".into(), id: "t1".into(), depth: 1 });
    rig.bridge.tick();
    let token = rig.bridge.snapshot()["view"]["ask"]["k"].clone();
    let kinds = [
        ("hello", json!({})),
        ("ui", json!({ "o": true })),
        ("open", json!({ "id": "t1" })),
        ("more", json!({})),
        ("refresh", json!({})),
        ("probe", json!({})),
        ("t3", json!({ "id": "t1" })),
        ("send", json!({ "id": "t1", "m": "go" })),
        ("new", json!({ "p": "p1", "m": "Build it" })),
        ("settle", json!({ "id": "t1" })),
        ("unsettle", json!({ "id": "t1" })),
        ("stop", json!({ "id": "t1" })),
        ("answer", json!({ "id": "t1", "k": token, "a": [1, [2]] })),
    ];
    let q: Vec<Value> = kinds
        .iter()
        .enumerate()
        .map(|(i, (t, extra))| with_args(json!({ "s": i as u64 + 1, "t": t }), extra.as_object().unwrap()))
        .collect();
    rig.frame(json!({ "k": "c1", "u": u, "n": 0, "l": 1, "q": q }));
    rig.bridge.tick();
    kinds.iter().enumerate().map(|(i, (t, _))| (i as u64 + 1, *t)).collect()
}

/// The kinds whose outcome says the tray setting blocked them.
fn blocked(rig: &Rig, sent: &[(u64, &'static str)]) -> Vec<&'static str> {
    let out = rig.bridge.snapshot()["out"].as_array().cloned().unwrap_or_default();
    sent.iter()
        .filter(|(s, _)| out.iter().any(|o| o["s"] == *s && o["ok"] == false && str_of(&o["x"]).contains("Commands from the game")))
        .map(|(_, t)| *t)
        .collect()
}

#[test]
fn the_tray_setting_decides_which_commands_from_the_game_reach_t3() {
    let changes = ["t3_thread_send", "t3_thread_launch", "t3_thread_organize", "t3_thread_interrupt"];
    let count = |rig: &Rig| -> usize { changes.iter().map(|tool| rig.t3.calls_to(tool).len()).sum() };
    let answers = |rig: &Rig| rig.t3.calls_to("t3_pending_request").iter().filter(|a| a["action"] == "respond").count();

    // Full: nothing is blocked, and every change reaches T3.
    let mut rig = Rig::new(27_000_000);
    let sent = every_kind(&mut rig, "ui1");
    assert!(blocked(&rig, &sent).is_empty());
    assert_eq!(count(&rig), 5, "send, new, settle, unsettle and stop");
    assert_eq!(answers(&rig), 1);
    assert!(rig.bridge.snapshot()["trust"].is_null(), "Full isn't named");

    // No new threads: only the start is refused; it comes back as a failure, and is acked.
    let mut rig = Rig::new(28_000_000);
    rig.bridge.set_trust(Trust::NoNewThreads);
    let sent = every_kind(&mut rig, "ui1");
    assert_eq!(blocked(&rig, &sent), vec!["new"]);
    assert!(rig.t3.calls_to("t3_thread_launch").is_empty());
    assert_eq!(count(&rig), 4);
    assert_eq!(answers(&rig), 1);
    assert_eq!(rig.bridge.snapshot()["ack"], 13);
    assert_eq!(rig.bridge.snapshot()["trust"], "nonew");
    assert!(rig.slot(1).contains("trust=\"nonew\""), "{}", rig.slot(1));

    // Read-only: nothing that changes T3 gets through; looking still works.
    let mut rig = Rig::new(29_000_000);
    rig.bridge.set_trust(Trust::ReadOnly);
    let opened = std::sync::Arc::new(Mutex::new(Vec::new()));
    let log = std::sync::Arc::clone(&opened);
    rig.bridge.opener = Box::new(move |link| log.lock().unwrap().push(link.to_string()));
    let sent = every_kind(&mut rig, "ui1");
    assert_eq!(blocked(&rig, &sent), vec!["send", "new", "settle", "unsettle", "stop", "answer"]);
    assert_eq!(count(&rig), 0);
    assert_eq!(answers(&rig), 0);
    assert_eq!(opened.lock().unwrap().len(), 1, "t3 still brings T3 to the front");
    assert_eq!(rig.bridge.snapshot()["view"]["id"], "t1", "open still opens");
    assert!(rig.bridge.window_open, "ui still says the window is open");
    assert_eq!(rig.bridge.snapshot()["ack"], 13);
    assert_eq!(rig.bridge.snapshot()["trust"], "readonly");

    // Back to Full, at once: the next command runs.
    rig.bridge.set_trust(Trust::Full);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": 14, "t": "send", "id": "t1", "m": "now" }] }));
    rig.bridge.tick();
    assert_eq!(rig.t3.calls_to("t3_thread_send").len(), 1);
    assert!(rig.bridge.snapshot()["trust"].is_null());
}
