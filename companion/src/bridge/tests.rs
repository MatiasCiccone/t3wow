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
    /// What the settled list holds, newest first.
    settled: Mutex<Vec<Value>>,
    /// Each thread's activity (its messages when not set), and the full text of items read
    /// by id, a piece at a time.
    activity: Mutex<HashMap<String, Vec<Value>>>,
    item_texts: Mutex<HashMap<String, String>>,
    /// Each machine's projects ("local" for the server's own; one app project when not set).
    projects: Mutex<HashMap<String, Value>>,
    /// Each machine's "New thread mode" preference (left out when not set).
    thread_modes: Mutex<HashMap<String, &'static str>>,
    /// The server version each machine reports (left out when not set).
    versions: Mutex<HashMap<String, &'static str>>,
    /// Whether the threads' agents watch their pull requests.
    watching: Mutex<bool>,
    /// Images uploaded: the address, its content type and how many bytes.
    uploads: Mutex<Vec<(String, String, usize)>>,
    /// What each thread runs on (`t3_thread_configure`), and what the machines offer.
    configs: Mutex<HashMap<String, Value>>,
    capabilities: Mutex<Value>,
    /// A thread's latest runs, as a read of it lists them.
    runs: Mutex<Value>,
}

/// A read of the open thread's activity for its pictures and cards, not a working thread's
/// pending check (which reads only its latest few items).
fn is_scan(args: &Value) -> bool {
    args["view"] == "activity" && args["limit"] != CARD_LOOKBACK
}

/// An item's text cut to `max` chars from `offset`, as T3 cuts it.
fn cut_text(item: &mut Value, max: usize, offset: usize) {
    let text: Vec<char> = str_of(&item["text"]).chars().collect();
    let end = (offset + max).min(text.len());
    item["text"] = json!(text[offset.min(end)..end].iter().collect::<String>());
    item["textTruncated"] = json!(end < text.len());
    item["nextTextOffset"] = if end < text.len() { json!(end) } else { Value::Null };
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
            settled: Mutex::new(Vec::new()),
            activity: Mutex::new(HashMap::new()),
            projects: Mutex::new(HashMap::new()),
            thread_modes: Mutex::new(HashMap::new()),
            versions: Mutex::new(HashMap::new()),
            item_texts: Mutex::new(HashMap::new()),
            watching: Mutex::new(false),
            uploads: Mutex::new(Vec::new()),
            configs: Mutex::new(HashMap::new()),
            capabilities: Mutex::new(json!({})),
            runs: Mutex::new(json!([])),
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
            "t3_environment_read" if !self.thread_modes.lock().unwrap().is_empty() || !self.versions.lock().unwrap().is_empty() => {
                let env = args["environmentId"].as_str().unwrap_or("local");
                json!({
                    "label": env,
                    "serverVersion": self.versions.lock().unwrap().get(env),
                    "preferences": { "defaultThreadEnvMode": self.thread_modes.lock().unwrap().get(env) },
                })
            }
            // The "local" server leaves its name out, as an older T3 might.
            "t3_environment_read" if *self.env_id.lock().unwrap() != "local" => json!({ "label": "Server B" }),
            "t3_thread_launch" => json!({ "threadId": "new1" }),
            "orchestrator_capabilities" => self.capabilities.lock().unwrap().clone(),
            // As T3 answers: a thread's settings, or a sequence number for a change, which
            // replaces the whole selection, options and all.
            "t3_thread_configure" => {
                let id = str_of(&args["threadId"]).to_string();
                let mut configs = self.configs.lock().unwrap();
                if args["modelSelection"].is_object() {
                    configs.entry(id).or_insert_with(|| json!({ "runtimeMode": "auto" }))["modelSelection"] = args["modelSelection"].clone();
                    json!({ "sequence": 1 })
                } else {
                    configs.get(&id).cloned().unwrap_or_else(|| json!({}))
                }
            }
            "list_thread_pull_requests" => {
                let prs = self.thread(&args["threadId"])["pullRequests"].clone();
                let watching = *self.watching.lock().unwrap();
                let prs: Vec<Value> = prs
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|mut pr| {
                        pr["watching"] = json!(watching);
                        pr
                    })
                    .collect();
                json!({ "pullRequests": prs })
            }
            // As T3 answers: an id and an address that carries its own permission.
            "t3_attachment_prepare_upload" => {
                let n = self.calls_to("t3_attachment_prepare_upload").len() - 1;
                json!({ "attachmentId": format!("pending-{n}"), "relativeUrl": format!("/api/attachments/upload/token{n}"), "expiresAt": 0 })
            }
            // As T3 answers: the interrupt is requested for a run under way; with none, it
            // says so (or names how the last run ended).
            "t3_thread_interrupt" => {
                let status = str_of(&self.thread(&args["threadId"])["status"]).to_string();
                let answer = match status.as_str() {
                    s if working(s) => "interrupt_requested",
                    s @ ("completed" | "failed" | "cancelled" | "interrupted" | "rolled_back") => s,
                    _ => "no_active_run",
                };
                json!({ "threadId": args["threadId"], "runId": null, "status": answer })
            }
            "t3_project_list" => {
                let env = args["environmentId"].as_str().unwrap_or("local");
                let projects = self.projects.lock().unwrap().get(env).cloned();
                json!({ "projects": projects.unwrap_or_else(|| json!([{ "id": "p1", "title": "app" }])) })
            }
            "t3_thread_list" => {
                let mut threads = if args["settled"] == true { self.settled.lock().unwrap().clone() } else { self.threads.lock().unwrap().clone() };
                if let Some(statuses) = args["statuses"].as_array() {
                    threads.retain(|t| statuses.contains(&t["status"]));
                }
                // Like T3, the list has no pending-question count or branch: only a thread read does.
                for t in &mut threads {
                    t.as_object_mut().unwrap().remove("pendingRequestCount");
                    t.as_object_mut().unwrap().remove("branch");
                }
                // A page at a time, as T3 pages: at most 100 from the cursor on.
                let limit = args["limit"].as_u64().unwrap_or(50) as usize;
                assert!(limit <= 100, "T3 refuses a limit over 100");
                let cursor = args["cursor"].as_u64().unwrap_or(0) as usize;
                let total = threads.len();
                let page: Vec<Value> = threads.into_iter().skip(cursor).take(limit).collect();
                let next = (cursor + page.len() < total).then_some(cursor + page.len());
                json!({ "threads": page, "nextCursor": next, "total": total })
            }
            "t3_thread_read" if args["itemId"].is_string() => {
                let id = str_of(&args["itemId"]).to_string();
                let text = self.item_texts.lock().unwrap().get(&id).cloned().unwrap_or_default();
                let mut item = json!({ "position": 1, "itemId": id, "type": "dynamic_tool", "status": "completed", "text": text });
                cut_text(&mut item, args["maxCharsPerItem"].as_u64().unwrap_or(50_000) as usize, args["textOffset"].as_u64().unwrap_or(0) as usize);
                json!({ "thread": self.thread(&args["threadId"]), "items": [item], "hasMore": false })
            }
            "t3_thread_read" if args["view"].is_string() => {
                // Messages (or activity) after afterPosition, a page at a time.
                let id = str_of(&args["threadId"]).to_string();
                let activity = if args["view"] == "activity" { self.activity.lock().unwrap().get(&id).cloned() } else { None };
                let max = args["maxCharsPerItem"].as_u64().unwrap_or(50_000) as usize;
                let cut = |items: Vec<Value>| -> Vec<Value> {
                    items
                        .into_iter()
                        .map(|mut it| {
                            cut_text(&mut it, max, 0);
                            it
                        })
                        .collect()
                };
                let all = activity.map(cut).or_else(|| self.messages.lock().unwrap().get(&id).cloned()).unwrap_or_else(|| {
                    vec![
                        json!({ "position": 0, "type": "user_message", "text": self.thread(&args["threadId"])["opening"].as_str().unwrap_or("hi"), "senderThreadId": self.thread(&args["threadId"])["sender"] }),
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
            "t3_thread_read" => json!({ "thread": self.thread(&args["threadId"]), "recentRuns": *self.runs.lock().unwrap(), "items": [], "hasMore": false }),
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

    fn upload(&self, relative_url: &str, content_type: &str, bytes: &[u8]) -> Result<(), T3Error> {
        if let Some(message) = self.failing.lock().unwrap().get("upload") {
            return Err(T3Error::Other(message.clone()));
        }
        self.uploads.lock().unwrap().push((relative_url.to_string(), content_type.to_string(), bytes.len()));
        Ok(())
    }
}

#[derive(Default)]
struct Written {
    slots: BTreeMap<u32, String>,
    raised: Vec<u32>,
    /// How many times every slot was blanked.
    cleared: u32,
    /// While set, every write fails with this error.
    failing: Option<String>,
    /// While set, raising a signal fails with this kind of error (slots still write).
    failing_signals: Option<std::io::ErrorKind>,
    /// The addon version "on disk".
    addon_version: Option<String>,
    /// What each image slot holds, and the slots blanked.
    images: BTreeMap<u32, Vec<u8>>,
    blanked: Vec<u32>,
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
        if let Some(kind) = written.failing_signals {
            return Err(std::io::Error::new(kind, "The system cannot find the path specified. (os error 3)"));
        }
        written.raised.push(n);
        Ok(())
    }
    fn clear_signals(&mut self) {}
    fn clear_slots(&mut self) {
        let mut written = self.0.lock().unwrap();
        written.slots.clear();
        written.cleared += 1;
    }
    fn flip_probe(&mut self, _real: bool) -> Vec<String> {
        Vec::new()
    }
    fn write_image(&mut self, n: u32, tga: &[u8]) -> std::io::Result<()> {
        let mut written = self.0.lock().unwrap();
        if let Some(error) = &written.failing {
            return Err(std::io::Error::other(error.clone()));
        }
        written.images.insert(n, tga.to_vec());
        Ok(())
    }
    fn blank_image(&mut self, n: u32) {
        let mut written = self.0.lock().unwrap();
        written.images.remove(&n);
        written.blanked.push(n);
    }
    fn has_image(&self, n: u32) -> bool {
        self.0.lock().unwrap().images.contains_key(&n)
    }
    fn addon_version(&self) -> Option<String> {
        self.0.lock().unwrap().addon_version.clone()
    }
}

struct Rig {
    bridge: Bridge,
    t3: Arc<FakeT3>,
    written: Arc<Mutex<Written>>,
    clock: Arc<Mutex<i64>>,
    opened: Arc<Mutex<Vec<String>>>,
}

impl Rig {
    fn new(start_ms: i64) -> Rig {
        let t3 = FakeT3::new();
        let written = Arc::new(Mutex::new(Written::default()));
        let clock = Arc::new(Mutex::new(start_ms));
        *t3.clock.lock().unwrap() = Some(Arc::clone(&clock));
        let now = Arc::clone(&clock);
        let mut bridge = Bridge::new(t3.clone(), Box::new(FakeSlots(Arc::clone(&written))), None, Box::new(|_| {}), Box::new(move || *now.lock().unwrap()));
        // Links are recorded, never opened: a test must not bring the real T3 to the front.
        let opened = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&opened);
        bridge.opener = Box::new(move |link| sink.lock().unwrap().push(link.to_string()));
        bridge.on_capture(CaptureMessage::Wow(Some("123@1".into())));
        Rig { bridge, t3, written, clock, opened }
    }

    fn advance(&self, ms: i64) {
        *self.clock.lock().unwrap() += ms;
    }

    fn now_s(&self) -> i64 {
        *self.clock.lock().unwrap() / 1000
    }

    fn frame(&mut self, value: Value) {
        let cells = crate::codec::encode_frame(1, &value.to_string());
        let at = *self.clock.lock().unwrap();
        self.bridge.on_capture(CaptureMessage::Strip(cells, at));
    }

    /// The links the bridge would have opened, in order.
    fn opened(&self) -> Vec<String> {
        self.opened.lock().unwrap().clone()
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
fn a_row_names_the_threads_pull_request_and_how_many_more() {
    let mut rig = Rig::new(3_000_000);
    let thread = |id: &str, prs: Value| {
        json!({ "threadId": id, "projectId": "p1", "title": id, "status": "completed", "pullRequests": prs, "updatedAt": "2026-11-06T10:00:00Z" })
    };
    let github = |number: u64, state: &str| json!({ "number": number, "state": state, "url": format!("https://github.com/o/r/pull/{number}") });
    let gitlab = json!({ "number": 12, "state": "open", "url": "https://code.example.com/o/r/-/merge_requests/12" });
    rig.t3.threads.lock().unwrap().extend([
        thread("one", json!([github(31, "merged")])),
        thread("still-open", json!([github(41, "merged"), github(46, "open"), github(47, "open")])),
        thread("all-merged", json!([github(41, "merged"), github(46, "merged")])),
        thread("gitlab", json!([gitlab])),
        thread("none", json!([])),
    ]);
    rig.bridge.tick();
    let snapshot = rig.bridge.snapshot();
    let label = |id: &str| snapshot["threads"].as_array().unwrap().iter().find(|t| t["e"] == 1 && t["id"] == id).unwrap()["pr"].clone();
    assert_eq!(label("one"), "#31");
    assert_eq!(label("still-open"), "#46 +2");
    assert_eq!(label("all-merged"), "#46 +1");
    assert_eq!(label("gitlab"), "!12");
    assert_eq!(label("none"), Value::Null);
    let state = |id: &str| snapshot["threads"].as_array().unwrap().iter().find(|t| t["e"] == 1 && t["id"] == id).unwrap()["ps"].clone();
    assert_eq!(state("still-open"), "open", "the state of the one named");
    assert_eq!(state("all-merged"), "merged");
    assert_eq!(state("none"), Value::Null);
    let links = |id: &str| snapshot["threads"].as_array().unwrap().iter().find(|t| t["e"] == 1 && t["id"] == id).unwrap()["prs"].clone();
    assert_eq!(
        links("still-open"),
        json!([
            { "n": "#41", "s": "merged", "u": "https://github.com/o/r/pull/41" },
            { "n": "#46", "s": "open", "u": "https://github.com/o/r/pull/46" },
            { "n": "#47", "s": "open", "u": "https://github.com/o/r/pull/47" },
        ]),
        "every one, in T3's order"
    );
    assert_eq!(links("none"), Value::Null);
}

#[test]
fn the_open_thread_lists_its_pull_requests_and_the_game_can_open_only_those() {
    let mut rig = Rig::new(3_100_000);
    *rig.t3.links.lock().unwrap() = json!([]);
    let merged = "https://github.com/o/r/pull/41";
    let open = "https://code.example.com/o/r/-/merge_requests/46";
    rig.t3.threads.lock().unwrap()[0]["pullRequests"] = json!([
        { "number": 41, "state": "merged", "url": merged },
        { "number": 46, "state": "open", "url": open },
        { "number": 47, "state": "open", "url": "file:///C:/Windows/System32/calc.exe" },
    ]);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "t1" }] }));
    rig.bridge.tick();
    assert_eq!(
        rig.bridge.snapshot()["view"]["prs"],
        json!([{ "n": "#41", "s": "merged", "u": merged }, { "n": "!46", "s": "open", "u": open }, { "n": "#47", "s": "open" }]),
        "every one, in T3's order, with a link only when it's a web page"
    );

    // A merge shows without the thread itself changing.
    rig.t3.threads.lock().unwrap()[0]["pullRequests"][1]["state"] = json!("merged");
    rig.advance(5000);
    rig.bridge.tick();
    assert_eq!(rig.bridge.snapshot()["view"]["prs"][1]["s"], "merged");

    let open_command = |s: u32, url: &str| json!({ "s": s, "t": "pr", "id": "t1", "u": url });
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [
        open_command(3, open),
        open_command(4, "https://evil.example.com/"),
        open_command(5, "file:///C:/Windows/System32/calc.exe"),
    ] }));
    rig.advance(2_000);
    rig.bridge.tick();
    assert_eq!(rig.opened(), vec![open.to_string()], "only what T3 linked, and only a web page");
}

#[test]
fn the_list_follows_t3s_sidebar() {
    let mut rig = Rig::new(3_000_000);
    rig.t3.threads.lock().unwrap().extend([
        json!({ "threadId": "child", "projectId": "p1", "title": "Review", "status": "completed", "parentThreadId": "t1", "updatedAt": "2026-11-06T10:00:00Z" }),
        json!({ "threadId": "launched", "projectId": "p1", "title": "Test it", "status": "completed", "createdBy": "agent", "sender": "t1", "updatedAt": "2026-11-06T10:00:00Z" }),
        // A task from a linked machine: its first message names no sender, and T3's sidebar leaves it out.
        json!({ "threadId": "delegated", "projectId": "p1", "title": "Review it", "status": "completed", "createdBy": "agent", "opening": "Act as the research sub-agent for this task.\n\nLook.", "updatedAt": "2026-11-06T10:00:00Z" }),
        // Launched over T3's tools with no thread behind it: T3 records an agent, and lists it.
        json!({ "threadId": "tool", "projectId": "p1", "title": "Research it", "status": "completed", "createdBy": "agent", "opening": "Research only: figure it out", "updatedAt": "2026-11-06T10:00:00Z" }),
        json!({ "threadId": "nap", "projectId": "p1", "title": "Later", "status": "completed", "snoozed": true, "updatedAt": "2026-11-06T10:00:00Z" }),
    ]);
    rig.bridge.tick();
    let snapshot = rig.bridge.snapshot();
    let ids: Vec<(String, bool, String, bool)> = snapshot["threads"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["e"] == 1)
        .map(|t| (t["id"].as_str().unwrap().to_string(), t["sn"] == true, str_of(&t["par"]).to_string(), t["ag"] == true))
        .collect();
    // Subagents stay out; a thread an agent launched comes under the thread that launched it,
    // and a delegated task says so, so the game gives it no row of its own while it's idle.
    let row = |id: &str, sn: bool, par: &str, ag: bool| (id.to_string(), sn, par.to_string(), ag);
    assert_eq!(
        ids,
        vec![row("t1", false, "", false), row("launched", false, "t1", true), row("delegated", false, "", true), row("tool", false, "", false), row("nap", true, "", false)]
    );

    // The launcher is read once, not on every poll.
    let reads = rig.t3.calls_to("t3_thread_read").len();
    rig.advance(15_000);
    rig.bridge.tick();
    assert_eq!(rig.t3.calls_to("t3_thread_read").len(), reads);
    for args in rig.t3.calls_to("t3_thread_list").into_iter().filter(|a| a["statuses"].is_null()) {
        assert_eq!(args["includeSubagents"], false);
    }
}

/// Whether the update to the game says an agent made this thread.
fn marked_agent_made(rig: &Rig, id: &str) -> bool {
    let snapshot = rig.bridge.snapshot();
    let thread = snapshot["threads"].as_array().unwrap().iter().find(|t| t["id"] == id).cloned();
    thread.unwrap_or_else(|| panic!("{id} isn't listed"))["ag"] == true
}

#[test]
fn a_thread_started_from_the_game_stays_yours_after_a_restart() {
    let dir = std::env::temp_dir().join(format!("t3wow-started-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let state = dir.join("state.json");
    // T3 records what its tools launch as an agent's, whoever asked.
    let listed = [
        json!({ "threadId": "new1", "projectId": "p1", "title": "Mine", "status": "completed", "createdBy": "agent", "opening": "Act as the research sub-agent for this task.", "updatedAt": "2026-11-06T10:00:00Z" }),
        json!({ "threadId": "delegated", "projectId": "p1", "title": "Review it", "status": "completed", "createdBy": "agent", "opening": "Act as the research sub-agent for this task.", "updatedAt": "2026-11-06T10:00:00Z" }),
    ];

    let mut rig = Rig::new(3_000_000);
    rig.bridge.state_file = Some(state.clone());
    rig.t3.threads.lock().unwrap().extend(listed.clone());
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "new", "p": "p1", "m": "Mine" }] }));
    rig.bridge.tick();
    assert_eq!(rig.t3.calls_to("t3_thread_launch").len(), 1);
    assert!(!marked_agent_made(&rig, "new1"), "a thread started from the game counts as an agent's");
    assert!(marked_agent_made(&rig, "delegated"), "a delegated thread stopped counting as an agent's");

    // After a restart, from the state file.
    let saved: LinkState = serde_json::from_str(&std::fs::read_to_string(&state).unwrap()).unwrap();
    let mut restarted = Rig::new(3_000_000);
    restarted.bridge.link.started_threads = saved.started_threads;
    restarted.t3.threads.lock().unwrap().extend(listed);
    restarted.bridge.tick();
    assert!(!marked_agent_made(&restarted, "new1"), "forgotten after a restart");
    assert!(marked_agent_made(&restarted, "delegated"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn only_the_latest_started_threads_are_remembered() {
    let mut rig = Rig::new(3_000_000);
    rig.bridge.link.started_threads = (0..STARTED_KEPT).map(|n| format!("old{n}")).collect();
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "new", "p": "p1", "m": "Mine" }] }));
    rig.bridge.tick();
    let started = &rig.bridge.link.started_threads;
    assert_eq!(started.len(), STARTED_KEPT);
    assert_eq!(started[0], "old1");
    assert_eq!(started[STARTED_KEPT - 1], "new1");
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

    // A change you can see but that answers nothing, then the 10 s gap passes.
    rig.t3.set("queued", "2026-11-06T10:00:09Z");
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
    assert!(inbox.contains("e=\"local\",id=\"t1\",t=\"Fix tests\",s=\"waiting\"}"), "an alert as soon as it shows: {inbox}");

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
fn a_thread_watching_its_pull_request_shows_as_watching_and_notifies_when_the_watch_ends() {
    let mut rig = Rig::new(6_500_000);
    rig.t3.threads.lock().unwrap()[0]["pullRequests"] = json!([{ "number": 31, "state": "open" }]);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();
    assert!(rig.slot(1).contains("s=\"running\""), "{}", rig.slot(1));
    // A running thread isn't asked: it shows as working anyway.
    assert!(rig.t3.calls_to("list_thread_pull_requests").is_empty());

    // The run ends with the agent watching the PR's checks: T3 says "completed".
    *rig.t3.watching.lock().unwrap() = true;
    rig.t3.set("completed", "2026-11-06T10:01:00Z");
    rig.advance(15_000);
    rig.bridge.tick();
    let inbox = rig.written.lock().unwrap().slots.values().last().cloned().unwrap();
    assert!(inbox.contains("id=\"t1\",t=\"Fix tests\",s=\"watching\""), "{inbox}");
    assert!(!inbox.contains("t=\"Fix tests\",s=\"watching\"}"), "notified while it still watches: {inbox}");
    assert!(!inbox.contains("t=\"Fix tests\",s=\"completed\"}"), "notified while it still watches: {inbox}");

    // Nothing changed: the next poll trusts the last answer.
    let asked = rig.t3.calls_to("list_thread_pull_requests").len();
    rig.advance(5_000);
    rig.bridge.tick();
    assert_eq!(rig.t3.calls_to("list_thread_pull_requests").len(), asked);

    // The watch ends without the thread changing: asked again once the answer is old, it
    // finished.
    *rig.t3.watching.lock().unwrap() = false;
    rig.advance(WATCH_RECHECK_MS);
    rig.bridge.tick();
    let latest = rig.written.lock().unwrap().slots.values().last().cloned().unwrap();
    assert!(latest.contains("id=\"t1\",t=\"Fix tests\",s=\"completed\""), "{latest}");
    assert!(latest.contains("e=\"local\",id=\"t1\",t=\"Fix tests\",s=\"completed\"}"), "an alert: {latest}");

    // Merged: nothing left to watch, so it isn't asked.
    rig.t3.threads.lock().unwrap()[0]["pullRequests"] = json!([{ "number": 31, "state": "merged" }]);
    rig.t3.set("completed", "2026-11-06T10:05:00Z");
    let asked = rig.t3.calls_to("list_thread_pull_requests").len();
    rig.advance(WATCH_RECHECK_MS);
    rig.bridge.tick();
    assert_eq!(rig.t3.calls_to("list_thread_pull_requests").len(), asked);
}

#[test]
fn a_thread_waiting_on_a_subagent_shows_as_watching_and_notifies_when_it_ends() {
    let mut rig = Rig::new(6_500_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();
    assert!(rig.slot(1).contains("s=\"running\""), "{}", rig.slot(1));

    // The agent delegated a review and its run ended: T3 says "completed".
    rig.t3.threads.lock().unwrap().push(json!({ "threadId": "kid", "projectId": "p1", "title": "Review", "status": "running", "parentThreadId": "t1", "relationshipToParent": "subagent", "updatedAt": "2026-11-06T10:01:00Z" }));
    rig.t3.set("completed", "2026-11-06T10:01:00Z");
    rig.advance(15_000);
    rig.bridge.tick();
    let inbox = rig.written.lock().unwrap().slots.values().last().cloned().unwrap();
    assert!(inbox.contains("id=\"t1\",t=\"Fix tests\",s=\"watching\""), "{inbox}");
    assert!(!inbox.contains("t=\"Fix tests\",s=\"completed\"}"), "notified while it still waits: {inbox}");

    // The subagent ended and nothing woke the thread: it finished.
    rig.t3.threads.lock().unwrap()[1]["status"] = json!("completed");
    rig.advance(15_000);
    rig.bridge.tick();
    let latest = rig.written.lock().unwrap().slots.values().last().cloned().unwrap();
    assert!(latest.contains("id=\"t1\",t=\"Fix tests\",s=\"completed\""), "{latest}");
    assert!(latest.contains("e=\"local\",id=\"t1\",t=\"Fix tests\",s=\"completed\"}"), "an alert: {latest}");
}

#[test]
fn a_running_fork_doesnt_make_the_thread_it_came_from_wait() {
    let list = json!({ "threads": [
        { "threadId": "fork", "status": "running", "parentThreadId": "t1", "relationshipToParent": "fork" },
        { "threadId": "kid", "status": "running", "parentThreadId": "t2", "relationshipToParent": "subagent" },
    ] });
    assert_eq!(waiting_parents(&list), HashSet::from(["t2".to_string()]));
}

#[test]
fn a_run_t3_lists_as_waiting_while_it_ends_stays_working_and_only_notifies_finished() {
    let mut rig = Rig::new(6_500_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "t1" }] }));
    rig.bridge.tick();

    // T3 lists every run whose turn just ended as "waiting" before "completed", with
    // nothing pending: neither the list nor the open thread may say it needs you.
    rig.t3.set("waiting", "2026-11-06T10:01:00Z");
    rig.advance(15_000);
    rig.bridge.tick();
    assert_eq!(rig.bridge.last_status["local:t1"], "running");
    let snapshot = rig.bridge.snapshot();
    assert_eq!(snapshot["threads"][0]["s"], "running", "the row: {snapshot}");
    assert_eq!(snapshot["view"]["s"], "running", "the open thread: {snapshot}");
    assert!(snapshot["view"]["approval"].is_null(), "not an approval either: {snapshot}");

    rig.t3.set("completed", "2026-11-06T10:01:01Z");
    rig.advance(15_000);
    rig.bridge.tick();
    let slots: Vec<String> = rig.written.lock().unwrap().slots.values().cloned().collect();
    assert!(slots.iter().all(|s| !s.contains("t=\"Fix tests\",s=\"waiting\"")), "never Needs you: {slots:?}");
    let latest = slots.last().unwrap();
    assert!(latest.contains("e=\"local\",id=\"t1\",t=\"Fix tests\",s=\"completed\"}"), "a Finished alert: {latest}");
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
    assert_eq!(rig.opened(), vec!["t3code://threads/local/mcp%3At1".to_string()]);
}

/// A working thread's latest activity while its agent waits on a secret card, in the shapes
/// T3 0.18.0 returns (see the probe in the commit log): T3's list says "running" and its
/// pending count is 0; only the card's item says it waits.
fn secret_card_activity(card_status: &str) -> Vec<Value> {
    vec![
        json!({ "position": 0, "visibility": "local", "type": "user_message", "status": "completed", "title": null, "text": "Deploy it", "updatedAt": "2026-11-06T10:00:00Z" }),
        json!({ "position": 1, "visibility": "local", "type": "command_execution", "status": "completed", "title": "Workspace ready", "text": "$ Preparing workspace", "updatedAt": "2026-11-06T10:00:01Z" }),
        json!({ "position": 2, "visibility": "local", "type": "reasoning", "status": "completed", "title": null, "text": "Needs a token", "updatedAt": "2026-11-06T10:00:02Z" }),
        json!({ "position": 3, "visibility": "local", "type": "dynamic_tool", "status": "running", "title": null,
                "text": "{
  \"toolName\": \"mcp__t3-code__request_secret\",
  \"input\": { \"label\": \"GitHub token\" } }", "updatedAt": "2026-11-06T10:01:00Z" }),
        json!({ "position": 4, "visibility": "local", "itemId": "turn-item:secret-request:mcp%3At1:13357e6f", "type": "secret_request", "status": card_status,
                "title": "GitHub token", "text": format!("Asked the user for GitHub token: {}.", if card_status == "waiting" { "pending" } else { "saved" }), "updatedAt": "2026-11-06T10:01:00Z" }),
    ]
}

#[test]
fn a_secret_request_shows_as_answer_in_t3_and_the_game_can_open_t3_on_it() {
    let mut rig = Rig::new(8_500_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();

    // The agent asks for a secret: T3 says "running" with nothing pending and nothing in its
    // pending list; only the card's item, last in the activity, says it waits.
    *rig.t3.questions.lock().unwrap() = false;
    rig.t3.threads.lock().unwrap()[0]["threadId"] = json!("mcp:t1");
    rig.t3.threads.lock().unwrap()[0]["itemCount"] = json!(5);
    rig.t3.threads.lock().unwrap()[0]["pendingRequestCount"] = json!(0);
    rig.t3.activity.lock().unwrap().insert("mcp:t1".into(), secret_card_activity("waiting"));
    rig.t3.set("running", "2026-11-06T10:01:00Z");
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": 2, "t": "open", "id": "mcp:t1" }] }));
    rig.advance(15_000);
    rig.bridge.tick();
    let inbox = rig.written.lock().unwrap().slots.values().last().cloned().unwrap();
    assert!(inbox.contains("e=\"local\",id=\"mcp:t1\",t=\"Fix tests\",s=\"card\"}"), "an alert: {inbox}");
    let snapshot = rig.bridge.snapshot();
    assert_eq!(snapshot["threads"][0]["s"], "card", "Needs you, not Working");
    assert_eq!(snapshot["view"]["s"], "card");
    assert_eq!(snapshot["view"]["card"], json!({ "k": "secret", "l": "GitHub token" }));
    assert!(snapshot["view"]["ask"].is_null() && snapshot["view"]["approval"].is_null(), "{}", snapshot["view"]);

    // The check is the read it always made, now with the latest few items' titles: no
    // other request, and nothing that could answer the card.
    let read = rig.t3.calls_to("t3_thread_read").into_iter().rfind(|a| a["view"] == "activity" && a["limit"] == 4).expect("the pending read");
    assert_eq!((read["afterPosition"].clone(), read["maxCharsPerItem"].clone()), (json!(0), json!(1)));
    assert!(rig.t3.calls_to("t3_pending_request").iter().all(|a| a["action"] == "list"), "the game answered something");

    // While the card waits the thread holds still: no read for it until its count is old.
    let reads = || rig.t3.calls_to("t3_thread_read").iter().filter(|a| a["view"] == "activity" && a["limit"] == 4).count();
    let before = reads();
    rig.advance(5_000);
    rig.bridge.tick();
    assert_eq!(reads(), before, "read again with nothing changed");
    assert_eq!(rig.bridge.snapshot()["threads"][0]["s"], "card");

    // Open in T3, as for an approval.
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 3, "q": [{ "s": 3, "t": "t3", "e": "local", "id": "mcp:t1" }] }));
    rig.advance(2_000);
    rig.bridge.tick();
    assert_eq!(rig.opened(), vec!["t3code://threads/local/mcp%3At1".to_string()]);

    // Answered in T3: the card ends, the thread changes, and it is back to Working.
    rig.t3.activity.lock().unwrap().insert("mcp:t1".into(), secret_card_activity("completed"));
    rig.t3.set("running", "2026-11-06T10:02:00Z");
    rig.advance(15_000);
    rig.bridge.tick();
    let snapshot = rig.bridge.snapshot();
    assert_eq!(snapshot["threads"][0]["s"], "running");
    assert_eq!(snapshot["view"]["s"], "running");
    assert!(snapshot["view"]["card"].is_null(), "{}", snapshot["view"]);
}

#[test]
fn only_a_card_still_waiting_here_needs_you() {
    let item = |kind: &str, status: &str, visibility: &str| json!({ "type": kind, "status": status, "visibility": visibility, "title": "Mac mini" });
    assert_eq!(waiting_card(&item("link_request", "waiting", "local")), Some(Card { kind: "link", label: "Mac mini".into() }));
    assert_eq!(waiting_card(&item("secret_request", "waiting", "local")).map(|c| c.kind), Some("secret"));
    // Answered, declined or timed out.
    for status in ["completed", "cancelled", "failed"] {
        assert_eq!(waiting_card(&item("secret_request", status, "local")), None, "{status}");
    }
    // Inherited from the thread it was forked from: only answerable there.
    assert_eq!(waiting_card(&item("secret_request", "waiting", "inherited")), None);
    assert_eq!(waiting_card(&item("dynamic_tool", "running", "local")), None);
    // The latest items: from the end of a long thread, from the start of a short one.
    assert_eq!(pending_read_args("t", 50)["afterPosition"], json!(45));
    assert!(pending_read_args("t", 4)["afterPosition"].is_null());
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
fn when_the_signal_files_are_gone_the_game_is_told_to_use_the_timer() {
    let mut rig = Rig::new(15_500_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": false }] }));
    rig.bridge.tick();
    let raised = rig.raised().len();
    assert!(raised > 0, "signals work at first");

    // Something replaced the addon folder: sig\ is gone, the slots are still there.
    rig.written.lock().unwrap().failing_signals = Some(std::io::ErrorKind::NotFound);
    rig.t3.set("completed", "2026-11-06T10:05:00Z");
    rig.advance(15_000);
    rig.bridge.tick();
    // The update still goes out, and it tells the game to load slots on a timer.
    let inbox = rig.slot(2);
    assert!(inbox.contains("sigoff=\"the addon folder lost its signal files"), "{inbox}");
    assert!(inbox.contains("s=\"completed\"}"), "{inbox}");
    let status = rig.bridge.status();
    assert!(status.signals_out && status.signals_lost && !status.signals);
    assert!(status.write_problem.is_none(), "the update files were written");
    assert_eq!(rig.raised().len(), raised);

    // A new WoW process (after Reinstall addon) watches signals again.
    rig.written.lock().unwrap().failing_signals = None;
    rig.bridge.on_capture(CaptureMessage::Wow(Some("456@1".into())));
    assert!(!rig.bridge.status().signals_lost);
}

#[test]
fn a_signal_that_keeps_failing_to_write_turns_signals_off_after_a_few_tries() {
    let mut rig = Rig::new(15_700_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": false }] }));
    rig.bridge.tick();
    rig.written.lock().unwrap().failing_signals = Some(std::io::ErrorKind::PermissionDenied);
    rig.t3.set("completed", "2026-11-06T10:05:00Z");
    rig.advance(15_000);
    rig.bridge.tick();
    assert!(!rig.bridge.status().signals_out, "one failure is retried");
    for _ in 0..2 {
        rig.advance(2_000);
        rig.bridge.tick();
    }
    let status = rig.bridge.status();
    assert!(status.signals_out && !status.signals_lost);
    assert!(rig.slot(2).contains("sigoff=\"couldn't write the signal files"), "{}", rig.slot(2));
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
fn the_open_thread_says_which_branch_it_is_on() {
    let mut rig = Rig::new(18_600_000);
    *rig.t3.links.lock().unwrap() = json!([]);
    rig.t3.threads.lock().unwrap()[0]["branch"] = json!("feature/show|branch");
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "t1" }] }));
    rig.bridge.tick();
    let snapshot = rig.bridge.snapshot();
    // Pipes escaped for the game, as other text is.
    assert_eq!(snapshot["view"]["br"], "feature/show||branch");
    // T3's list has no branch, so the listed threads don't either.
    assert!(snapshot["threads"][0]["br"].is_null());

    // A thread with no checkout of its own (a scratch thread) has none.
    rig.t3.threads.lock().unwrap()[0].as_object_mut().unwrap().remove("branch");
    rig.t3.set("completed", "2026-11-06T10:05:00Z");
    rig.advance(5000);
    rig.bridge.tick();
    assert!(rig.bridge.snapshot()["view"]["br"].is_null());
}

#[test]
fn a_saved_script_is_offered_by_name_and_its_command_goes_to_the_threads_agent() {
    let mut rig = Rig::new(18_650_000);
    *rig.t3.links.lock().unwrap() = json!([]);
    // As on a real install: T3's settings hold the list, and the project's own is left over.
    rig.t3.projects.lock().unwrap().insert("local".into(), json!([{ "id": "p1", "title": "app", "scripts": [{ "id": "old", "name": "leftover", "command": "npm run old" }] }]));
    let settings = std::env::temp_dir().join(format!("t3wow-scripts-{}.json", std::process::id()));
    let saved = json!({ "projectSettingsFolded": true, "projectSettingsOverrides": { "p1": { "defaultProjectScripts": [
        { "id": "dev", "name": "serve | app", "command": "npm run dev", "icon": "play", "runOnWorktreeCreate": false },
        { "id": "lint", "name": "lint", "command": "npm run lint", "icon": "lint", "runOnWorktreeCreate": false },
    ] } } });
    std::fs::write(&settings, saved.to_string()).unwrap();
    rig.bridge.set_t3_settings(Some(settings.clone()));
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "t1" }] }));
    rig.bridge.tick();
    let snapshot = rig.bridge.snapshot();
    assert_eq!(snapshot["view"]["scr"], json!([{ "id": "dev", "n": "serve || app" }, { "id": "lint", "n": "lint" }]));
    assert!(snapshot["view"]["pid"].is_null());
    assert!(!snapshot.to_string().contains("npm run"), "commands stay out of the game");

    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": 3, "t": "script", "id": "t1", "sc": "dev" }] }));
    rig.bridge.tick();
    let sends = rig.t3.calls_to("t3_thread_send");
    assert_eq!(sends.len(), 1);
    assert_eq!(sends[0]["threadId"], "t1");
    assert_eq!(sends[0]["mode"], "auto");
    assert!(str_of(&sends[0]["message"]).contains("saved script \"serve | app\""), "{}", sends[0]);
    assert!(str_of(&sends[0]["message"]).contains("```\nnpm run dev\n```"), "{}", sends[0]);

    // Only a script T3 has saved for the open thread's project: another addon can draw this.
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 3, "q": [{ "s": 4, "t": "script", "id": "t1", "sc": "rm -rf" }, { "s": 5, "t": "script", "id": "t2", "sc": "dev" }] }));
    rig.bridge.tick();
    assert_eq!(rig.t3.calls_to("t3_thread_send").len(), 1);
    let snapshot = rig.bridge.snapshot();
    for s in [4, 5] {
        let refused = snapshot["out"].as_array().unwrap().iter().find(|o| o["s"] == s).unwrap_or_else(|| panic!("no outcome for {s}: {snapshot}"));
        assert_eq!(refused["ok"], false);
    }

    rig.bridge.set_trust(Trust::NoNewThreads);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 4, "q": [{ "s": 6, "t": "script", "id": "t1", "sc": "lint" }] }));
    rig.bridge.tick();
    assert_eq!(rig.t3.calls_to("t3_thread_send").len(), 2, "like any message, under No new threads");

    rig.bridge.set_trust(Trust::ReadOnly);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 5, "q": [{ "s": 7, "t": "script", "id": "t1", "sc": "lint" }] }));
    rig.bridge.tick();
    assert_eq!(rig.t3.calls_to("t3_thread_send").len(), 2, "refused under Read-only");
    let _ = std::fs::remove_file(&settings);
}

#[test]
fn a_projects_scripts_are_worked_out_as_t3s_own_dropdown_does() {
    let own = [json!({ "id": "own" })];
    let ids = |settings: Value, own: &[Value]| -> Vec<String> { resolved_scripts(&settings, "p1", own).iter().map(|s| str_of(&s["id"]).to_string()).collect() };
    let one = |id: &str| json!([{ "id": id }]);

    assert_eq!(ids(json!({ "projectSettingsFolded": true, "defaultProjectScripts": one("all"), "projectSettingsOverrides": { "p1": { "defaultProjectScripts": one("mine") } } }), &own), ["mine"]);
    assert_eq!(ids(json!({ "projectSettingsFolded": true, "projectSettingsOverrides": { "p1": { "defaultProjectScripts": [] } }, "defaultProjectScripts": one("all") }), &own), Vec::<String>::new());
    assert_eq!(ids(json!({ "projectSettingsFolded": true, "defaultProjectScripts": one("all"), "projectScriptOverrides": { "p1": one("old") } }), &own), ["all"], "folded: the older settings no longer count");
    assert_eq!(ids(json!({ "projectScriptOverrides": { "p1": one("old") }, "defaultProjectScripts": one("all") }), &own), ["old"]);
    assert_eq!(ids(json!({ "projectScriptOverrides": { "p1": null }, "defaultProjectScripts": one("all") }), &own), ["all"]);
    assert_eq!(ids(json!({ "defaultProjectScripts": one("all") }), &own), ["own"]);
    assert_eq!(ids(json!({ "defaultProjectScripts": one("all") }), &[]), ["all"]);
    assert_eq!(ids(Value::Null, &own), ["own"]);
}

#[test]
fn only_t3_on_this_pc_can_say_which_scripts_a_project_has() {
    let script = json!([{ "id": "p1", "title": "app", "scripts": [{ "id": "old", "name": "leftover", "command": "npm run old" }] }]);
    let mut rig = Rig::new(18_660_000);
    rig.t3.projects.lock().unwrap().insert("mac".into(), script.clone());
    rig.bridge.set_t3_settings(Some(std::env::temp_dir().join("t3wow-no-such-settings.json")));
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "e": "mac", "id": "t1" }] }));
    rig.bridge.tick();
    let snapshot = rig.bridge.snapshot();
    assert_eq!(snapshot["view"]["sna"], true, "a linked machine's list can't be known");
    assert!(snapshot["view"]["scr"].is_null(), "and its leftover list isn't shown");
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": 3, "t": "script", "e": "mac", "id": "t1", "sc": "old" }] }));
    rig.bridge.tick();
    assert!(rig.t3.calls_to("t3_thread_send").is_empty(), "nothing to run");

    // Connected to a T3 elsewhere: this PC's disk says nothing about its projects.
    let mut rig = Rig::new(18_670_000);
    *rig.t3.links.lock().unwrap() = json!([]);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "t1" }] }));
    rig.bridge.tick();
    assert_eq!(rig.bridge.snapshot()["view"]["sna"], true);
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
fn a_message_with_code_also_comes_in_segments_and_its_text_stays_as_before() {
    let mut rig = Rig::new(23_000_000);
    rig.t3.messages.lock().unwrap().insert("t1".into(), conversation(&["plain", "Run `npm test`:\n```\nok | fail\n```"]));
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "t1" }] }));
    rig.bridge.tick();
    let view = rig.bridge.snapshot()["view"].clone();
    let items = view["items"].as_array().unwrap();
    assert_eq!(items[0], json!({ "r": "u", "x": "plain" }), "no segments without code");
    assert_eq!(items[1]["x"], "Run |cffc8c8a0npm test|r:\n|cffc8c8a0ok || fail|r");
    assert_eq!(items[1]["g"], json!(["Run \u{1}npm test\u{2}:", { "c": "ok | fail" }]));
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
    assert_eq!(rig.bridge.last_status["local:t1"], "waiting", "the row still needs you");
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

#[test]
fn new_thread_starts_in_a_new_worktree_or_the_checkout_and_defaults_as_t3_does() {
    let mut rig = Rig::new(27_000_000);
    rig.t3.thread_modes.lock().unwrap().extend([("local".to_string(), "worktree"), ("mac".to_string(), "local")]);
    let projects = json!([
        { "id": "p1", "title": "app" },
        { "id": "p2", "title": "lib", "defaultThreadEnvMode": "local" },
        { "id": "p3", "title": "tool", "defaultThreadEnvMode": "worktree" },
    ]);
    rig.t3.projects.lock().unwrap().extend([("local".to_string(), projects.clone()), ("mac".to_string(), projects)]);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();
    rig.advance(61_000);
    rig.bridge.tick();

    // A project's own mode wins; otherwise its machine's preference sets the default.
    let defaults: Vec<(i64, String, bool)> = rig.bridge.snapshot()["projects"].as_array().unwrap().iter()
        .map(|p| (p["e"].as_i64().unwrap(), str_of(&p["id"]).to_string(), p["w"] == true))
        .collect();
    assert_eq!(defaults, vec![
        (1, "p1".into(), true), (1, "p2".into(), false), (1, "p3".into(), true),
        (2, "p1".into(), false), (2, "p2".into(), false), (2, "p3".into(), true),
    ]);
    // A preference changed in T3 sets the default once the machines are read again.
    rig.t3.thread_modes.lock().unwrap().insert("mac".to_string(), "worktree");
    rig.advance(61_000);
    rig.bridge.tick();
    let mac_app = rig.bridge.snapshot()["projects"].as_array().unwrap().iter().find(|p| p["e"] == 2 && p["id"] == "p1").cloned().unwrap();
    assert_eq!(mac_app["w"], true, "{mac_app}");

    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [
        { "s": 2, "t": "new", "e": "mac", "p": "p1", "w": true, "m": "In a worktree" },
        { "s": 3, "t": "new", "e": "mac", "p": "p3", "m": "In the checkout" },
        { "s": 4, "t": "new", "e": "mac", "w": true, "m": "Scratch" },
    ] }));
    rig.bridge.tick();
    let launches = rig.t3.calls_to("t3_thread_launch");
    assert_eq!(launches.len(), 3);
    assert_eq!(launches[0]["workspaceStrategy"], json!({ "type": "worktree", "baseRef": "HEAD" }));
    assert_eq!(launches[0]["projectId"], "p1");
    assert_eq!(launches[0]["environmentId"], "mac");
    // Without "w", T3's own default: the project's checkout.
    assert!(launches[1]["workspaceStrategy"].is_null(), "{}", launches[1]);
    // A scratch thread has no checkout to branch from.
    assert!(launches[2]["workspaceStrategy"].is_null() && launches[2]["scratch"] == true, "{}", launches[2]);
}

#[test]
fn fresh_sessions_need_t3_code_0_18_on_every_server_a_send_goes_through() {
    for (version, takes) in [
        ("0.1.8", true), ("0.1.9", true), ("0.2.0", true), ("1.0", true), ("v0.1.8", true), ("0.1.8-nightly.3", true),
        ("0.1.7", false), ("0.0.99", false), ("", false), ("dev", false), ("0.1.x", false),
    ] {
        assert_eq!(takes_fresh_context(version), takes, "{version:?}");
    }
}

#[test]
fn a_send_from_the_games_clear_starts_a_fresh_session_where_t3_can() {
    let mut rig = Rig::new(27_500_000);
    rig.t3.versions.lock().unwrap().extend([("local".to_string(), "0.1.8"), ("mac".to_string(), "0.1.7")]);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();
    rig.advance(61_000);
    rig.bridge.tick();
    // The game offers /clear for this server's threads, not for a machine on an older T3.
    let envs = rig.bridge.snapshot()["envs"].clone();
    assert_eq!(envs[0]["fc"], true, "{envs}");
    assert!(envs[1]["fc"].is_null(), "{envs}");

    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [
        { "s": 2, "t": "send", "id": "t1", "m": "start over", "fc": true },
        { "s": 3, "t": "send", "id": "t1", "m": "and go on" },
        { "s": 4, "t": "send", "e": "mac", "id": "t1", "m": "start over there", "fc": true },
    ] }));
    rig.bridge.tick();
    let sends = rig.t3.calls_to("t3_thread_send");
    assert_eq!(sends.len(), 2, "the Mac's fresh send reached T3: {sends:?}");
    assert_eq!((&sends[0]["message"], &sends[0]["freshContext"], &sends[0]["mode"]), (&json!("start over"), &json!(true), &json!("auto")));
    assert!(sends[1]["freshContext"].is_null(), "only the armed message starts fresh: {}", sends[1]);
    let out = rig.bridge.snapshot()["out"].clone();
    assert_eq!(out, json!([{ "s": 4, "ok": false, "x": "this T3 can't start a fresh session: update it to use /clear" }]));

    // This server going back to an older T3 takes it away from every machine it reaches.
    rig.t3.versions.lock().unwrap().extend([("local".to_string(), "0.1.7"), ("mac".to_string(), "0.1.8")]);
    rig.advance(61_000);
    rig.bridge.tick();
    let envs = rig.bridge.snapshot()["envs"].clone();
    assert!(envs[0]["fc"].is_null() && envs[1]["fc"].is_null(), "{envs}");
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
        ("list", json!({ "g": "settled" })),
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
fn t3s_scratch_project_is_told_by_its_folder_or_icon_and_not_by_its_title() {
    let scratch = |root: &str| json!({ "title": "No project", "workspaceRoot": root, "repositoryIdentity": null });
    assert!(is_scratch_project(&scratch("C:\\Users\\me\\.t3\\scratch")));
    assert!(is_scratch_project(&scratch("/home/me/.t3/scratch/")));
    assert!(is_scratch_project(&json!({ "workspaceRoot": "D:\\elsewhere", "projectIcon": { "name": "message-square-dashed" } })), "a T3 home moved elsewhere");
    assert!(!is_scratch_project(&scratch("C:\\dev\\no-project")), "your own project may have that title");
    assert!(!is_scratch_project(&json!({ "workspaceRoot": "/repo/.t3/scratch", "repositoryIdentity": { "name": "x" } })), "a repository isn't T3's scratch");
}

#[test]
fn new_thread_isnt_offered_t3s_scratch_project_but_its_threads_keep_the_name() {
    let mut rig = Rig::new(32_000_000);
    rig.t3.projects.lock().unwrap().insert("local".to_string(), json!([
        { "id": "p1", "title": "app", "repositoryIdentity": { "name": "app" } },
        { "id": "s1", "title": "No project", "workspaceRoot": "C:\\Users\\me\\.t3\\scratch", "repositoryIdentity": null },
    ]));
    *rig.t3.threads.lock().unwrap() = vec![json!({ "threadId": "t9", "projectId": "s1", "title": "Quick question", "status": "completed", "updatedAt": "2026-11-06T10:00:00Z" })];
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();
    rig.advance(61_000);
    rig.bridge.tick();
    let snapshot = rig.bridge.snapshot();
    let ids: Vec<&str> = snapshot["projects"].as_array().unwrap().iter().map(|p| str_of(&p["id"])).collect();
    assert!(!ids.is_empty() && !ids.contains(&"s1"), "{}", snapshot["projects"]);
    let row = snapshot["threads"].as_array().unwrap().iter().find(|t| t["id"] == "t9").expect("the thread is listed");
    assert_eq!(row["p"], "No project", "{row}");
}

#[test]
fn a_working_thread_says_when_its_run_started_from_one_read_per_run() {
    let mut rig = Rig::new(33_000_000);
    *rig.t3.threads.lock().unwrap() = vec![json!({ "threadId": "t9", "projectId": "p1", "title": "Busy", "status": "running", "latestRunId": "r1", "updatedAt": "2026-11-06T10:00:00Z" })];
    *rig.t3.runs.lock().unwrap() = json!([{ "runId": "r1", "startedAt": "2026-11-06T09:58:00.000Z" }]);
    let started = |rig: &Rig| rig.bridge.snapshot()["threads"].as_array().unwrap().iter().find(|t| t["id"] == "t9").unwrap()["st"].clone();
    let reads = |rig: &Rig| rig.t3.calls_to("t3_thread_read").iter().filter(|a| a["runLimit"] == 1).count();
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();
    rig.advance(61_000);
    rig.bridge.tick();
    assert_eq!(started(&rig), iso_seconds("2026-11-06T09:58:00.000Z"));
    let once = reads(&rig);
    assert!(once >= 1, "read for the run");
    rig.advance(61_000);
    rig.bridge.tick();
    assert_eq!(reads(&rig), once, "the same run isn't read again");

    // Another run of the thread is another start.
    rig.t3.threads.lock().unwrap()[0]["latestRunId"] = json!("r2");
    *rig.t3.runs.lock().unwrap() = json!([{ "runId": "r2", "startedAt": "2026-11-06T10:30:00.000Z" }]);
    rig.advance(61_000);
    rig.bridge.tick();
    assert_eq!(started(&rig), iso_seconds("2026-11-06T10:30:00.000Z"));
}

#[test]
fn a_row_ages_from_its_last_run_or_when_it_was_settled_as_in_t3() {
    let mut rig = Rig::new(34_000_000);
    // Touched long after its last run ended (a sweep, a pull request linked): T3 0.18 ages
    // the row from the run, not from updatedAt.
    *rig.t3.threads.lock().unwrap() = vec![
        json!({ "threadId": "t1", "projectId": "p1", "title": "Replied", "status": "completed", "latestRunId": "r1", "updatedAt": "2026-11-06T10:57:00.000Z" }),
        json!({ "threadId": "t2", "projectId": "p1", "title": "Settled", "status": "completed", "latestRunId": "r1", "settled": true, "settledAt": "2026-11-06T08:00:00.000Z", "updatedAt": "2026-11-06T10:58:00.000Z" }),
        json!({ "threadId": "t3", "projectId": "p1", "title": "No run", "status": "idle", "updatedAt": "2026-11-06T09:00:00.000Z" }),
    ];
    *rig.t3.runs.lock().unwrap() = json!([{ "runId": "r1", "requestedAt": "2026-11-06T04:06:16.064Z", "startedAt": "2026-11-06T04:06:16.089Z", "completedAt": "2026-11-06T04:07:43.686Z" }]);
    let age = |rig: &Rig, id: &str| {
        let snapshot = rig.bridge.snapshot();
        let row = snapshot["threads"].as_array().unwrap().iter().find(|t| t["id"] == id).unwrap().clone();
        row["la"].as_i64().unwrap_or_else(|| row["at"].as_i64().unwrap())
    };
    let reads = |rig: &Rig| rig.t3.calls_to("t3_thread_read").iter().filter(|a| a["threadId"] == "t1" && a["runLimit"] == 1 && a["environmentId"].is_null()).count();
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();
    rig.advance(61_000);
    rig.bridge.tick();
    assert_eq!(age(&rig, "t1"), iso_seconds("2026-11-06T04:07:43.686Z"), "the newest time of its latest run");
    assert_eq!(age(&rig, "t2"), iso_seconds("2026-11-06T08:00:00.000Z"), "when it was settled");
    assert_eq!(age(&rig, "t3"), iso_seconds("2026-11-06T09:00:00.000Z"), "no run: updatedAt");
    let once = reads(&rig);
    assert_eq!(once, 1, "read once");
    rig.advance(61_000);
    rig.bridge.tick();
    assert_eq!(reads(&rig), once, "not read again while the thread is unchanged");

    // Another run ended: the thread moved, so it's read again.
    rig.t3.threads.lock().unwrap()[0]["updatedAt"] = json!("2026-11-06T11:30:05.000Z");
    *rig.t3.runs.lock().unwrap() = json!([{ "runId": "r2", "requestedAt": "2026-11-06T11:20:00.000Z", "startedAt": "2026-11-06T11:20:01.000Z", "completedAt": "2026-11-06T11:30:00.000Z" }]);
    rig.advance(61_000);
    rig.bridge.tick();
    assert_eq!(age(&rig, "t1"), iso_seconds("2026-11-06T11:30:00.000Z"));
}

#[test]
fn a_time_is_written_as_t3_writes_them() {
    assert_eq!(iso_from_seconds(0), "1970-01-01T00:00:00.000Z");
    assert_eq!(iso_from_seconds(1_709_210_096), "2024-02-29T12:34:56.000Z", "a leap day");
    for seconds in [951_782_399, 1_735_689_599, 1_735_689_600, 1_794_700_000] {
        assert_eq!(iso_seconds(&iso_from_seconds(seconds)), seconds);
    }
}

#[test]
fn snooze_asks_t3_for_a_time_and_wake_undoes_it() {
    let mut rig = Rig::new(30_000_000);
    let q = json!([
        { "s": 1, "t": "snooze", "e": "local", "id": "t1", "mn": 240 },
        { "s": 2, "t": "unsnooze", "e": "local", "id": "t1" },
        { "s": 3, "t": "snooze", "e": "local", "id": "t1" },
        { "s": 4, "t": "snooze", "e": "local", "id": "t1", "mn": 0 },
    ]);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": q }));
    let now = rig.bridge.now_seconds();
    rig.bridge.tick();
    let calls = rig.t3.calls_to("t3_thread_organize");
    assert_eq!(calls.len(), 2, "a snooze with no length doesn't reach T3: {calls:?}");
    assert_eq!(calls[0]["action"], "snooze");
    assert_eq!(calls[0]["threadId"], "t1");
    let until = iso_seconds(str_of(&calls[0]["snoozedUntil"]));
    assert!((now + 240 * 60..=now + 240 * 60 + 5).contains(&until), "{calls:?}");
    assert_eq!(calls[1]["action"], "unsnooze");
    assert!(calls[1]["snoozedUntil"].is_null());

    // The tray's Read-only stops both.
    let mut rig = Rig::new(31_000_000);
    rig.bridge.set_trust(Trust::ReadOnly);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [
        { "s": 1, "t": "snooze", "e": "local", "id": "t1", "mn": 60 },
        { "s": 2, "t": "unsnooze", "e": "local", "id": "t1" },
    ] }));
    rig.bridge.tick();
    assert!(rig.t3.calls_to("t3_thread_organize").is_empty());
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
    assert_eq!(rig.opened(), vec!["t3code://threads/local/t1"], "t3 brings T3 to the front");
    assert!(rig.bridge.snapshot()["trust"].is_null(), "Full isn't named");

    // No new threads: only the start is refused; it comes back as a failure, and is acked.
    let mut rig = Rig::new(28_000_000);
    rig.bridge.set_trust(Trust::NoNewThreads);
    let sent = every_kind(&mut rig, "ui1");
    assert_eq!(blocked(&rig, &sent), vec!["new"]);
    assert!(rig.t3.calls_to("t3_thread_launch").is_empty());
    assert_eq!(count(&rig), 4);
    assert_eq!(rig.opened(), vec!["t3code://threads/local/t1"], "t3 brings T3 to the front");
    assert_eq!(answers(&rig), 1);
    assert_eq!(rig.bridge.snapshot()["ack"], 14);
    assert_eq!(rig.bridge.snapshot()["trust"], "nonew");
    assert!(rig.slot(1).contains("trust=\"nonew\""), "{}", rig.slot(1));

    // Read-only: nothing that changes T3 gets through; looking still works.
    let mut rig = Rig::new(29_000_000);
    rig.bridge.set_trust(Trust::ReadOnly);
    let sent = every_kind(&mut rig, "ui1");
    assert_eq!(blocked(&rig, &sent), vec!["send", "new", "settle", "unsettle", "stop", "answer"]);
    assert_eq!(count(&rig), 0);
    assert_eq!(answers(&rig), 0);
    assert_eq!(rig.opened(), vec!["t3code://threads/local/t1"], "t3 still brings T3 to the front");
    assert_eq!(rig.bridge.snapshot()["view"]["id"], "t1", "open still opens");
    assert!(rig.bridge.window_open, "ui still says the window is open");
    assert_eq!(rig.bridge.snapshot()["ack"], 14);
    assert_eq!(rig.bridge.snapshot()["trust"], "readonly");

    // Back to Full, at once: the next command runs.
    rig.bridge.set_trust(Trust::Full);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": 15, "t": "send", "id": "t1", "m": "now" }] }));
    rig.bridge.tick();
    assert_eq!(rig.t3.calls_to("t3_thread_send").len(), 1);
    assert!(rig.bridge.snapshot()["trust"].is_null());
}

#[test]
fn the_game_learns_which_addon_version_is_on_disk() {
    // Unknown (no toc to read): left out, as an older companion does.
    let mut rig = Rig::new(30_000_000);
    assert!(rig.bridge.snapshot().get("av").is_none());

    // Known: in every update, so the game can tell it loaded older files.
    rig.written.lock().unwrap().addon_version = Some("0.6.0".into());
    assert_eq!(rig.bridge.snapshot()["av"], "0.6.0");
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": false }] }));
    rig.bridge.tick();
    assert!(rig.slot(1).contains("av=\"0.6.0\""), "{}", rig.slot(1));

    // A change on disk (a reinstall, a CurseForge update) is urgent: it goes out even with
    // the window closed and nothing else new.
    rig.written.lock().unwrap().addon_version = Some("0.7.0".into());
    rig.advance(2_000);
    rig.bridge.tick();
    assert!(rig.slot(2).contains("av=\"0.7.0\""), "{}", rig.slot(2));
}

/// `n` settled threads, newest first.
fn settled_threads(n: usize) -> Vec<Value> {
    (0..n)
        .map(|i| json!({ "threadId": format!("s{i}"), "projectId": "p1", "title": format!("Done {i}"), "status": "completed", "settled": true, "updatedAt": "2026-11-05T10:00:00Z" }))
        .collect()
}

fn settled_shown(rig: &Rig) -> usize {
    rig.bridge.snapshot()["threads"].as_array().unwrap().iter().filter(|t| t["set"] == true).count()
}

/// The settled lists asked for since `from`: (cursor, limit) of each.
fn settled_pages(rig: &Rig, from: usize) -> Vec<(Option<u64>, u64)> {
    rig.t3.calls_to("t3_thread_list")[from..].iter().filter(|a| a["settled"] == true).map(|a| (a["cursor"].as_u64(), a["limit"].as_u64().unwrap())).collect()
}

#[test]
fn show_more_lists_more_of_a_group_up_to_its_cap_for_the_ui_session() {
    let mut rig = Rig::new(30_000_000);
    *rig.t3.links.lock().unwrap() = json!([]);
    *rig.t3.settled.lock().unwrap() = settled_threads(150);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();
    assert_eq!(settled_shown(&rig), 25);
    assert_eq!(rig.bridge.snapshot()["settledTotal"], 150);
    assert_eq!(rig.bridge.snapshot()["more"], json!({ "settled": true }), "only the settled list has more");

    // Each Show more doubles what the next poll lists.
    let mut s = 1;
    let mut show_more = |rig: &mut Rig| -> Vec<(Option<u64>, u64)> {
        s += 1;
        let from = rig.t3.calls_to("t3_thread_list").len();
        rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": s, "t": "list", "g": "settled" }] }));
        rig.bridge.tick();
        settled_pages(rig, from)
    };
    assert_eq!(show_more(&mut rig), vec![(None, 50)]);
    assert_eq!(settled_shown(&rig), 50);
    assert_eq!(show_more(&mut rig), vec![(None, 100)]);
    assert_eq!(settled_shown(&rig), 100);
    // Past T3's page of 100, the list takes a request per page, one step each.
    assert_eq!(show_more(&mut rig), vec![(None, 100), (Some(100), 100)]);
    assert_eq!(settled_shown(&rig), 150);
    assert!(rig.bridge.snapshot()["more"].is_null(), "every settled thread is listed");

    // At the cap, more settled threads don't offer Show more: it couldn't list them.
    rig.t3.settled.lock().unwrap().extend(settled_threads(250).into_iter().skip(150));
    rig.advance(5000);
    rig.bridge.tick();
    assert_eq!(settled_shown(&rig), 200);
    assert_eq!(rig.bridge.snapshot()["settledTotal"], 250);
    assert!(rig.bridge.snapshot()["more"].is_null());
    assert_eq!(show_more(&mut rig), vec![(None, 100), (Some(100), 100)], "the cap holds");

    // A new UI session (a /reload) starts from the first page again.
    rig.advance(2000);
    rig.frame(json!({ "k": "c1", "u": "ui2", "n": 0, "l": 1, "q": [{ "s": 10, "t": "ui", "o": true }] }));
    rig.bridge.tick();
    assert_eq!(settled_shown(&rig), 25);
    assert_eq!(rig.bridge.snapshot()["more"], json!({ "settled": true }));
    assert!(rig.slot(rig.bridge.link.slot - 1).contains("more={settled=true}"), "{}", rig.slot(rig.bridge.link.slot - 1));
}

#[test]
fn every_stop_tells_the_game_how_it_went_and_stopping_ends_once_the_run_has() {
    let mut rig = Rig::new(31_000_000);
    *rig.t3.links.lock().unwrap() = json!([]);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();
    let last_note = |rig: &Rig| str_of(&rig.bridge.snapshot()["notes"].as_array().unwrap().last().unwrap()["x"]).to_string();
    let outcome = |rig: &Rig, s: u64| rig.bridge.snapshot()["out"].as_array().into_iter().flatten().find(|o| o["s"] == s).cloned();
    let poll = |rig: &mut Rig| {
        rig.advance(5_000);
        for _ in 0..10 {
            rig.bridge.tick();
        }
    };

    // Running: T3 takes the interrupt, and the game hears so.
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 2, "t": "stop", "id": "t1" }] }));
    rig.bridge.tick();
    assert_eq!(outcome(&rig, 2), Some(json!({ "s": 2, "ok": true, "st": "stopping" })));
    assert_eq!(last_note(&rig), "Stopping.");
    // Still winding down: still stopping.
    poll(&mut rig);
    assert_eq!(last_note(&rig), "Stopping.");
    // The run ended: the note says so, instead of "Stopping." until it ages out.
    rig.t3.set("interrupted", "2026-11-06T10:05:00Z");
    poll(&mut rig);
    assert_eq!(last_note(&rig), "Stopped.");
    // Later lists don't say it again.
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 3, "t": "refresh" }] }));
    poll(&mut rig);
    assert_eq!(rig.bridge.snapshot()["notes"].as_array().unwrap().iter().filter(|n| n["x"] == "Stopped.").count(), 1);

    // Already stopped: nothing to interrupt, and the game hears that rather than "Stopping.".
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 4, "t": "stop", "id": "t1" }] }));
    rig.bridge.tick();
    assert_eq!(outcome(&rig, 4), Some(json!({ "s": 4, "ok": true, "st": "idle" })));
    assert_eq!(last_note(&rig), "Nothing to stop: the run had already ended.");
    rig.t3.set("idle", "2026-11-06T10:06:00Z");
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 5, "t": "stop", "id": "t1" }] }));
    rig.bridge.tick();
    assert_eq!(outcome(&rig, 5), Some(json!({ "s": 5, "ok": true, "st": "idle" })));

    // Refused (or failed, or timed out): a failure with T3's reason, like any command.
    rig.t3.set("running", "2026-11-06T10:07:00Z");
    rig.t3.failing.lock().unwrap().insert("t3_thread_interrupt".into(), "thread_not_interruptible: it runs elsewhere".into());
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 6, "t": "stop", "id": "t1" }] }));
    rig.bridge.tick();
    assert_eq!(outcome(&rig, 6), Some(json!({ "s": 6, "ok": false, "x": "thread_not_interruptible: it runs elsewhere" })));
    assert_eq!(last_note(&rig), "stop failed: thread_not_interruptible: it runs elsewhere");
    // A refused stop isn't waited on: the run ending later isn't "Stopped." by the game.
    rig.t3.set("completed", "2026-11-06T10:08:00Z");
    poll(&mut rig);
    assert_eq!(last_note(&rig), "stop failed: thread_not_interruptible: it runs elsewhere");
}

/// The `lid` an update in a slot carries.
fn lid_in(lua: &str) -> String {
    let at = lua.find(",lid=\"").unwrap_or_else(|| panic!("no lid in {lua}")) + 6;
    lua[at..].chars().take_while(|c| *c != '"').collect()
}

#[test]
fn moved_to_another_t3_everything_from_the_previous_one_goes_even_before_the_new_one_answers() {
    let mut rig = Rig::new(9_500_000);
    rig.bridge.set_login("local");
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "e": "mac", "id": "t1" }] }));
    rig.bridge.tick();
    rig.t3.set("completed", "2026-11-06T10:05:00Z");
    rig.advance(5000);
    rig.bridge.tick();
    let before = rig.bridge.snapshot();
    assert_eq!(before["threads"].as_array().unwrap().len(), 2);
    assert!(before["view"].is_object() && !before["events"].as_array().unwrap().is_empty());
    let old_lid = before["lid"].as_str().unwrap().to_string();
    assert_eq!(lid_in(&rig.slot(2)), old_lid);

    // `login --origin` points the companion at a T3 that doesn't answer yet.
    let other = FakeT3::new();
    other.failing.lock().unwrap().insert("t3_environment_links".into(), "connection refused".into());
    other.failing.lock().unwrap().insert("t3_thread_list".into(), "connection refused".into());
    rig.bridge.set_client(other.clone());
    rig.bridge.set_login("http://box:3773");
    // The slots holding the previous T3's threads were blanked, then the new state written.
    assert_eq!(rig.written.lock().unwrap().cleared, 2, "once for the first sign-in, once for this one");
    rig.advance(5000);
    rig.bridge.tick();
    let after = rig.bridge.snapshot();
    let new_lid = after["lid"].as_str().unwrap().to_string();
    assert_ne!(new_lid, old_lid);
    assert_eq!(after["threads"], json!([]));
    assert_eq!(after["projects"], json!([]));
    assert_eq!(after["events"], json!([]));
    assert!(after["view"].is_null());
    assert_eq!(after["envs"].as_array().unwrap().len(), 1, "only the placeholder for the server itself: {}", after["envs"]);
    assert_eq!(after["envs"][0]["id"], "");
    assert!(!rig.bridge.status().t3_ok);
    // Published with the new stamp, and nothing in the slots still has the old one.
    let slots: Vec<String> = rig.written.lock().unwrap().slots.values().cloned().collect();
    assert!(!slots.is_empty());
    assert!(slots.iter().all(|lua| lid_in(lua) == new_lid && !lua.contains("Fix tests")), "{slots:?}");
    assert!(other.calls_to("t3_thread_read").is_empty(), "the old open thread isn't read on the new T3");
}

#[test]
fn the_same_t3_on_another_port_keeps_its_sign_in_and_what_it_showed() {
    let mut rig = Rig::new(9_600_000);
    rig.bridge.set_login("local");
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "e": "mac", "id": "t1" }] }));
    rig.bridge.tick();
    let lid = rig.bridge.snapshot()["lid"].clone();
    rig.bridge.set_client(rig.t3.clone());
    rig.bridge.set_login("local");
    assert_eq!(rig.written.lock().unwrap().cleared, 1, "blanked once, when the first sign-in was set");
    rig.advance(5000);
    rig.bridge.tick();
    let snapshot = rig.bridge.snapshot();
    assert_eq!(snapshot["lid"], lid);
    assert!(snapshot["view"].is_object());
}

#[test]
fn a_restart_after_moving_to_another_t3_starts_the_game_afresh_and_one_without_keeps_the_stamp() {
    let dir = std::env::temp_dir().join(format!("t3wow-login-state-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let state = dir.join("state.json");
    let start = |key: &str| {
        let written = Arc::new(Mutex::new(Written::default()));
        let mut bridge = Bridge::new(FakeT3::new(), Box::new(FakeSlots(Arc::clone(&written))), Some(state.clone()), Box::new(|_| {}), Box::new(|| 1_000_000));
        bridge.set_login(key);
        let cleared = written.lock().unwrap().cleared;
        (bridge.link.login.clone().unwrap(), cleared)
    };
    let (first, cleared) = start("local");
    assert_eq!(cleared, 1, "no record of which T3 the slots were written for: blank them");
    let (again, cleared) = start("local");
    assert_eq!((again.as_str(), cleared), (first.as_str(), 0), "the same T3 after a restart");
    let (moved, cleared) = start("http://box:3773");
    assert_ne!(moved, first);
    assert_eq!(cleared, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn signing_out_drops_what_was_read_with_that_sign_in() {
    let mut rig = Rig::new(9_700_000);
    rig.bridge.set_login("local");
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();
    let lid = rig.bridge.snapshot()["lid"].clone();
    assert!(!rig.bridge.snapshot()["threads"].as_array().unwrap().is_empty());
    rig.bridge.sign_out();
    let snapshot = rig.bridge.snapshot();
    assert_ne!(snapshot["lid"], lid);
    assert_eq!(snapshot["threads"], json!([]));
    assert_eq!(rig.bridge.link.login_key.as_deref(), Some("local"), "still the same T3");
}

#[test]
fn a_command_drawn_under_the_previous_sign_ins_stamp_comes_back_refused() {
    let mut rig = Rig::new(9_800_000);
    rig.bridge.set_login("local");
    let old = rig.bridge.link.login.clone().unwrap();
    rig.bridge.set_login("http://box:3773");
    let lid = rig.bridge.link.login.clone().unwrap();
    // The game still shows the previous T3: a message to one of its threads, a start, and
    // harmless ones.
    let q = json!([
        { "s": 1, "t": "hello", "o": true },
        { "s": 2, "t": "send", "e": "mac", "id": "t1", "m": "meant for the other T3" },
        { "s": 3, "t": "new", "m": "a task for the other T3" },
        { "s": 4, "t": "refresh" },
    ]);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "lid": old, "q": q }));
    rig.bridge.tick();
    assert!(rig.t3.calls_to("t3_thread_send").is_empty());
    assert!(rig.t3.calls_to("t3_thread_launch").is_empty());
    assert_eq!(rig.bridge.link.acked["c1"], 4, "refused commands count as done");
    let out = rig.bridge.snapshot()["out"].clone();
    let refused: Vec<u64> = out.as_array().unwrap().iter().filter(|o| o["ok"] == false).map(|o| o["s"].as_u64().unwrap()).collect();
    assert_eq!(refused, vec![2, 3], "{out}");
    assert!(out[0]["x"].as_str().unwrap().contains("another T3"));

    // Once the game has the new stamp, its commands run; so do an older addon's (no stamp).
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "lid": lid, "q": [{ "s": 5, "t": "send", "id": "t1", "m": "now" }] }));
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": 6, "t": "send", "id": "t1", "m": "old addon" }] }));
    rig.bridge.tick();
    assert_eq!(rig.t3.calls_to("t3_thread_send").len(), 2);
}

/// Draws nothing: records what it was asked, and answers a 7x3 picture for a page (a page
/// whose HTML says BROKEN fails) and a 5x2 one for a file.
#[derive(Clone, Default)]
struct FakePainter(Arc<Mutex<Vec<String>>>);

impl crate::render::Painter for FakePainter {
    fn page(&self, html: &str, height: u32) -> Result<crate::render::Bitmap, String> {
        self.0.lock().unwrap().push(format!("page {height} {html}"));
        if html.contains("BROKEN") {
            return Err("the browser drew nothing".into());
        }
        Ok(crate::render::Bitmap { width: 7, height: 3, rgba: vec![9; 7 * 3 * 4] })
    }
    fn picture(&self, path: &std::path::Path) -> Result<crate::render::Bitmap, String> {
        self.0.lock().unwrap().push(format!("picture {}", path.display()));
        Ok(crate::render::Bitmap { width: 5, height: 2, rgba: vec![9; 5 * 2 * 4] })
    }
}

impl FakePainter {
    fn calls(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

/// An html_render call as T3's activity shows it.
fn page_call(title: &str, height: u64, html: &str) -> String {
    serde_json::to_string_pretty(&json!({
        "toolName": "mcp__t3-code__html_render",
        "input": { "title": title, "height": height, "html": html },
        "output": { "message": "Shown to the reader above your reply." },
    }))
    .unwrap()
}

/// A rig whose game can show pictures, with thread t1 open: a user message (0), a page the
/// agent showed (1) and the agent's reply (2).
fn picture_rig(start_ms: i64, html: &str) -> (Rig, FakePainter) {
    let mut rig = Rig::new(start_ms);
    let painter = FakePainter::default();
    rig.bridge.set_painter(Some(Box::new(painter.clone())));
    rig.t3.messages.lock().unwrap().insert("t1".into(), conversation(&["chart it", "Here it is."]));
    let page = json!({ "position": 1, "itemId": "item-page", "type": "dynamic_tool", "status": "completed", "text": page_call("Sales", 880, html) });
    let activity = vec![
        json!({ "position": 0, "itemId": "m0", "type": "user_message", "status": "completed", "text": "chart it" }),
        page,
        json!({ "position": 2, "itemId": "m2", "type": "assistant_message", "status": "completed", "text": "Here it is." }),
    ];
    rig.t3.activity.lock().unwrap().insert("t1".into(), activity);
    rig.t3.item_texts.lock().unwrap().insert("item-page".into(), page_call("Sales", 880, html));
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "im": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "t1" }] }));
    (rig, painter)
}

fn view_items(rig: &Rig) -> Vec<Value> {
    rig.bridge.snapshot()["view"]["items"].as_array().cloned().unwrap_or_default()
}

#[test]
fn a_page_an_agent_showed_reaches_the_game_as_a_picture_where_t3_shows_it() {
    let (mut rig, painter) = picture_rig(30_000_000, "<p>chart</p>");
    rig.bridge.tick();
    assert_eq!(painter.calls(), vec!["page 880 <p>chart</p>".to_string()]);
    let items = view_items(&rig);
    assert_eq!(items.len(), 3, "{items:?}");
    assert_eq!(items[0]["x"], "chart it");
    assert_eq!(items[1], json!({ "r": "a", "x": "[Page: Sales]", "im": { "f": 1, "w": 7, "h": 3 } }));
    assert_eq!(items[2]["x"], "Here it is.");
    let tga = rig.written.lock().unwrap().images.get(&1).cloned().unwrap();
    assert_eq!(tga.len(), 18 + 7 * 3 * 4, "a 32-bit TGA");
    assert_eq!(rig.bridge.link.images_shown, vec![1], "named to the game: kept until WoW quits");
    // It went out in the update.
    rig.advance(2_000);
    rig.bridge.tick();
    let sent = rig.written.lock().unwrap().slots.values().any(|lua| lua.contains("im={f=1,w=7,h=3}"));
    assert!(sent, "the update names the slot");

    // Nothing new: the page isn't drawn or read again, and the activity is read on from
    // where the last read stopped.
    let reads_before = rig.t3.calls_to("t3_thread_read").len();
    rig.t3.set("running", "2026-11-06T10:00:05Z");
    rig.advance(5_000);
    rig.bridge.tick();
    assert_eq!(painter.calls().len(), 1);
    let reads: Vec<Value> = rig.t3.calls_to("t3_thread_read").into_iter().skip(reads_before).collect();
    assert!(reads.iter().all(|a| !a["itemId"].is_string()), "{reads:?}");
    let scan = reads.iter().find(|a| is_scan(a)).unwrap();
    assert_eq!(scan["afterPosition"], 2);
    assert_eq!(view_items(&rig)[1]["im"]["f"], 1);
}

#[test]
fn without_an_addon_that_shows_pictures_nothing_more_is_read_or_drawn() {
    let mut rig = Rig::new(31_000_000);
    let painter = FakePainter::default();
    rig.bridge.set_painter(Some(Box::new(painter.clone())));
    rig.t3.messages.lock().unwrap().insert("t1".into(), conversation(&["chart it", "Here it is."]));
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "t1" }] }));
    rig.bridge.tick();
    assert!(!rig.t3.calls_to("t3_thread_read").iter().any(is_scan));
    assert!(painter.calls().is_empty());
    assert_eq!(view_items(&rig).len(), 2);
}

#[test]
fn a_long_page_is_read_a_piece_at_a_time() {
    let html = format!("<p>{}</p>", "x".repeat(120_000));
    let (mut rig, painter) = picture_rig(32_000_000, &html);
    rig.bridge.tick();
    let offsets: Vec<Value> = rig.t3.calls_to("t3_thread_read").iter().filter(|a| a["itemId"] == "item-page").map(|a| a["textOffset"].clone()).collect();
    assert_eq!(offsets, vec![Value::Null, json!(50_000), json!(100_000)]);
    assert_eq!(painter.calls(), vec![format!("page 880 {html}")]);
    assert_eq!(view_items(&rig)[1]["im"]["f"], 1);
}

#[test]
fn a_page_that_cant_be_drawn_says_why_and_is_tried_again_later() {
    let (mut rig, painter) = picture_rig(33_000_000, "<p>BROKEN</p>");
    rig.bridge.tick();
    let item = view_items(&rig)[1].clone();
    assert!(item.get("im").is_none());
    assert_eq!(item["x"], "|cff808080[Page: Sales] (couldn't show it in the game: the browser drew nothing)|r");
    rig.t3.set("running", "2026-11-06T10:00:05Z");
    rig.advance(5_000);
    rig.bridge.tick();
    assert_eq!(painter.calls().len(), 1, "not tried again at once");
    rig.t3.set("running", "2026-11-06T10:10:05Z");
    rig.advance(PICTURE_RETRY_MS);
    rig.bridge.tick();
    assert_eq!(painter.calls().len(), 2, "tried again later");
}

#[test]
fn image_slots_the_game_has_seen_keep_their_picture_until_wow_restarts() {
    let (mut rig, painter) = picture_rig(34_000_000, "<p>chart</p>");
    // Every slot holds a picture this game process was told about.
    for n in 1..=IMAGE_COUNT {
        rig.bridge.link.images.insert(n, ImageSlot { key: format!("old{n}"), w: 1, h: 1, used: n as i64 });
        rig.bridge.link.images_shown.push(n);
    }
    rig.bridge.tick();
    assert!(rig.written.lock().unwrap().images.is_empty(), "none written over");
    assert!(painter.calls().len() == 1);
    assert_eq!(view_items(&rig)[1]["x"], format!("|cff808080[Page: Sales] (couldn't show it in the game: {IMAGES_OUT})|r"));

    // WoW restarts: every slot can be written again, and the oldest give their space back.
    rig.bridge.on_capture(CaptureMessage::Wow(Some("456@2".into())));
    assert!(rig.bridge.link.images_shown.is_empty());
    assert_eq!(rig.written.lock().unwrap().blanked.len(), IMAGE_COUNT as usize - IMAGES_KEPT);
    assert_eq!(rig.bridge.link.images.len(), IMAGES_KEPT);
    rig.bridge.picture_errors.clear();
    rig.frame(json!({ "k": "c1", "u": "ui2", "n": 0, "l": 1, "im": 1, "q": [{ "s": 3, "t": "open", "id": "t1" }] }));
    rig.bridge.tick();
    assert_eq!(painter.calls().len(), 2);
    let slot = view_items(&rig)[1]["im"]["f"].as_u64().unwrap() as u32;
    assert!(slot <= IMAGE_COUNT - IMAGES_KEPT as u32, "an emptied slot: {slot}");
}

#[test]
fn an_image_a_message_embeds_from_a_file_follows_that_message() {
    let dir = std::env::temp_dir().join(format!("t3wow-embed-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("chart one.png");
    std::fs::write(&file, b"png bytes").unwrap();
    let mut rig = Rig::new(35_000_000);
    let painter = FakePainter::default();
    rig.bridge.set_painter(Some(Box::new(painter.clone())));
    let text = format!("Saved it:\n\n![the chart]({})\n\n![gone](C:\\nowhere\\x.png)", file.display());
    rig.t3.messages.lock().unwrap().insert("t1".into(), conversation(&["chart it", &text, "thanks"]));
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "im": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "t1" }] }));
    rig.bridge.tick();
    assert_eq!(painter.calls(), vec![format!("picture {}", file.display())]);
    let items = view_items(&rig);
    let texts: Vec<String> = items.iter().map(|i| i["x"].as_str().unwrap_or("").to_string()).collect();
    assert_eq!(texts.len(), 4, "{texts:?}");
    assert!(texts[1].starts_with("Saved it:"));
    assert_eq!(items[2], json!({ "r": "a", "x": "[Image: the chart]", "im": { "f": 1, "w": 5, "h": 2 } }));
    assert_eq!(texts[3], "thanks");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An html_render call that passed a file, as T3 0.18's activity shows it: its result names
/// the snapshot T3 saved.
fn path_call(title: &str, path: &std::path::Path, attachment: &str) -> String {
    let render = json!({ "htmlRender": { "attachmentId": attachment, "title": title, "height": 120 }, "message": "Shown to the reader above your reply." });
    serde_json::to_string_pretty(&json!({
        "toolName": "mcp__t3-code__html_render",
        "input": { "title": title, "height": 120, "path": path.display().to_string() },
        "output": { "content": render.to_string(), "structuredContent": render },
    }))
    .unwrap()
}

#[test]
fn a_page_passed_as_a_file_is_drawn_from_the_snapshot_t3_saved() {
    let dir = std::env::temp_dir().join(format!("t3wow-page-file-{}", std::process::id()));
    let attachments = dir.join("userdata").join("attachments");
    std::fs::create_dir_all(&attachments).unwrap();
    let file = dir.join("chart.html");
    std::fs::write(&file, "<p>edited since</p>").unwrap();
    let snapshot = "<!doctype html><html><head><meta name=\"viewport\" content=\"width=device-width\"><style id=\"t3-theme\">:root{color-scheme:dark}@media (prefers-color-scheme: light){:root{color-scheme:light}}</style><script>(function(){parent.postMessage(1,\"*\")})();</script><title>Chart</title></head><body><p>as shown</p></body></html>";
    std::fs::write(attachments.join("t1-abc-html.html"), snapshot).unwrap();

    let (mut rig, painter) = picture_rig(36_000_000, "<p>x</p>");
    rig.t3.item_texts.lock().unwrap().insert("item-page".into(), path_call("Sales", &file, "t1-abc-html"));
    rig.bridge.set_t3_settings(Some(dir.join("userdata").join("settings.json")));
    rig.bridge.tick();
    let drawn = "page 120 <!doctype html><html><head><meta name=\"viewport\" content=\"width=device-width\"><title>Chart</title></head><body><p>as shown</p></body></html>";
    assert_eq!(painter.calls(), vec![drawn.to_string()], "the snapshot, without T3's theme and script");
    assert_eq!(view_items(&rig)[1]["im"]["f"], 1);

    // Without the snapshot, the file as it is now; with neither, a line says why.
    let text = path_call("Sales", &file, "t1-gone-html");
    assert_eq!(page_html(&text, Some(&attachments)).as_deref(), Ok("<p>edited since</p>"));
    let gone = path_call("Sales", &dir.join("deleted.html"), "t1-gone-html");
    assert_eq!(page_html(&gone, Some(&attachments)), Err("its file is gone".to_string()));
    let txt = dir.join("notes.txt");
    std::fs::write(&txt, "<p>").unwrap();
    assert_eq!(page_html(&path_call("Sales", &txt, ""), Some(&attachments)), Err("its file isn't an .html file".to_string()));
    let escape = path_call("Sales", &file, "..\\..\\chart");
    assert_eq!(page_html(&escape, Some(&attachments)).as_deref(), Ok("<p>edited since</p>"), "an odd id isn't a path");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_page_passed_as_a_file_on_another_machine_says_why_it_isnt_shown() {
    let (mut rig, painter) = picture_rig(37_000_000, "<p>x</p>");
    // No T3 settings: the T3 in use isn't the one on this PC.
    rig.t3.item_texts.lock().unwrap().insert("item-page".into(), path_call("Sales", std::path::Path::new("/box/chart.html"), "t1-abc-html"));
    rig.bridge.tick();
    assert!(painter.calls().is_empty());
    assert_eq!(view_items(&rig)[1]["x"], "|cff808080[Page: Sales] (couldn't show it in the game: the page is a file on another machine)|r");
}

#[test]
fn pages_and_embedded_files_are_found_in_t3s_text() {
    assert!(is_page_item(&page_call("x", 100, "<p>")));
    assert!(is_page_item(&path_call("x", std::path::Path::new("/a.html"), "id")));
    assert_eq!(page_title(&path_call("Q4", std::path::Path::new("/a.html"), "id")), "Q4");
    assert!(is_page_item(r#"{"toolName":"t3-code.html_render","input":{"#));
    assert!(!is_page_item(r#"{"toolName":"mcp__t3-code__html_preview","input":{"#));
    assert!(!is_page_item("I used html_render"));
    assert_eq!(page_title(&page_call("Q3 \"sales\"", 100, "<p>")), "Q3 \"sales\"");
    assert_eq!(page_title(r#"{"toolName":"html_render","input":{"height":3"#), "");
    let found = embedded_files(r#"a ![one](C:\a b\one.png) b ![](file:///D:/two%20x.jpg "t") ![web](https://x/y.png) ![rel](pics/z.png) `![code](C:/c.png)`"#);
    assert_eq!(
        found,
        vec![
            ("one".to_string(), PathBuf::from(r"C:\a b\one.png")),
            (String::new(), PathBuf::from("D:/two x.jpg")),
            ("code".to_string(), PathBuf::from("C:/c.png")),
        ]
    );
}

#[test]
fn a_picture_whose_file_is_gone_is_forgotten_at_start() {
    let dir = std::env::temp_dir().join(format!("t3wow-state-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let state = dir.join("state.json");
    let mut link = LinkState::default();
    for n in [1, 2] {
        link.images.insert(n, ImageSlot { key: format!("page{n}"), w: 7, h: 3, used: 1 });
    }
    // An image you sent stays listed even so: nothing can draw it again, and its message
    // says it couldn't be loaded.
    link.images.insert(3, ImageSlot { key: "sent:mac:t1:m1:0".into(), w: 7, h: 3, used: 1 });
    std::fs::write(&state, serde_json::to_string(&link).unwrap()).unwrap();
    // Only slot 2's file still holds its picture (a reinstall blanked the rest).
    let written = Arc::new(Mutex::new(Written::default()));
    written.lock().unwrap().images.insert(2, b"tga".to_vec());
    let bridge = Bridge::new(FakeT3::new(), Box::new(FakeSlots(written)), Some(state), Box::new(|_| {}), Box::new(|| 0));
    assert_eq!(bridge.link.images.keys().copied().collect::<Vec<_>>(), vec![2, 3]);
    let _ = std::fs::remove_dir_all(&dir);
}

const TASK_LINK: &str = "<t3-thread://v2/local/thread:delegated-task:command%3Amcp%3Ax%3Adelegate-task%3Ar1>";

/// A rig with thread t1 open: you asked (0), it delegated a review (1, `status`, with
/// `result`), and said where it runs (2), linking the task and the thread itself.
fn delegate_rig(start_ms: i64, cards: bool, status: &str, result: &str) -> Rig {
    let mut rig = Rig::new(start_ms);
    let said = format!("Sol is reviewing in {TASK_LINK}. Back in <t3-thread://v2/local/t1> after.");
    rig.t3.messages.lock().unwrap().insert("t1".into(), conversation(&["review it", &said]));
    let task = json!({
        "position": 1, "itemId": "turn-item:delegated-task:command%3Amcp%3Ax%3Adelegate-task%3Ar1", "type": "subagent",
        "status": status, "title": "Sol review (round 1)", "text": result, "updatedAt": "2026-11-06T10:00:30Z",
    });
    let activity = vec![
        json!({ "position": 0, "itemId": "m0", "type": "user_message", "status": "completed", "text": "review it" }),
        task,
        json!({ "position": 2, "itemId": "m2", "type": "assistant_message", "status": "completed", "text": said }),
    ];
    rig.t3.activity.lock().unwrap().insert("t1".into(), activity);
    let mut frame = json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "t1" }] });
    if cards {
        frame["dc"] = json!(1);
    }
    rig.frame(frame);
    rig
}

fn card_reads(rig: &Rig) -> Vec<Value> {
    rig.t3.calls_to("t3_thread_read").into_iter().filter(|a| a["view"] == "activity" && a["limit"] == 1).collect()
}

#[test]
fn a_delegated_task_shows_as_a_card_where_t3_shows_it_and_links_to_it_say_its_title() {
    let result = format!("VERDICT: APPROVE\n\nS1 [nit] {}", "a".repeat(500));
    let mut rig = delegate_rig(40_000_000, true, "completed", &result);
    rig.bridge.tick();
    let items = view_items(&rig);
    assert_eq!(items.len(), 3, "{items:?}");
    let card = &items[1]["d"];
    assert_eq!(card["t"], "Sol review (round 1)");
    assert_eq!(card["s"], "completed");
    assert_eq!(card["at"], iso_seconds("2026-11-06T10:00:30Z"));
    // The scan reads 200 chars of it; the card reads more once (it ended), and shows 400.
    let body = card["b"].as_str().unwrap();
    assert!(body.starts_with("VERDICT: APPROVE\n\nS1 [nit] aaa"), "{body}");
    assert!(body.chars().count() > 300 && body.ends_with('…'), "{body}");
    assert!(str_of(&items[1]["x"]).starts_with("Sol review (round 1) (finished)\nVERDICT"), "{}", items[1]["x"]);
    let reads = card_reads(&rig);
    assert_eq!(reads.len(), 1);
    assert_eq!((reads[0]["afterPosition"].clone(), reads[0]["maxCharsPerItem"].clone()), (json!(0), json!(CARD_CHARS)));
    assert_eq!(items[2]["x"], "Sol is reviewing in “Sol review (round 1)”. Back in “Fix tests” after.");
    // Nothing new: it isn't read again.
    rig.t3.set("running", "2026-11-06T10:00:05Z");
    rig.advance(5_000);
    rig.bridge.tick();
    assert_eq!(card_reads(&rig).len(), 1);
    assert_eq!(view_items(&rig)[1]["d"]["b"], json!(body));
}

#[test]
fn a_running_delegated_task_says_so_and_its_card_fills_in_once_it_ends() {
    let mut rig = delegate_rig(41_000_000, true, "running", "");
    rig.bridge.tick();
    let card = view_items(&rig)[1].clone();
    assert_eq!((card["d"]["s"].clone(), card["d"]["b"].clone()), (json!("running"), json!("")));
    assert_eq!(card["x"], "Sol review (round 1) (working)");
    // It ends: T3 tells the thread, which changes, and the card is read again.
    rig.t3.activity.lock().unwrap().get_mut("t1").unwrap()[1]["status"] = json!("completed");
    rig.t3.activity.lock().unwrap().get_mut("t1").unwrap()[1]["text"] = json!("VERDICT: APPROVE");
    rig.t3.set("running", "2026-11-06T10:01:00Z");
    rig.advance(5_000);
    rig.bridge.tick();
    let card = view_items(&rig)[1].clone();
    assert_eq!((card["d"]["s"].clone(), card["d"]["b"].clone()), (json!("completed"), json!("VERDICT: APPROVE")));
}

#[test]
fn a_compaction_shows_as_a_line_where_t3_shows_it_once_it_is_done() {
    let mut rig = Rig::new(43_000_000);
    rig.t3.messages.lock().unwrap().insert("t1".into(), conversation(&["/compact", "after it"]));
    let activity = vec![
        json!({ "position": 0, "itemId": "m0", "type": "user_message", "status": "completed", "text": "/compact" }),
        json!({ "position": 1, "itemId": "c1", "type": "compaction", "status": "running", "title": "Context compacted", "text": null, "updatedAt": "2026-11-06T10:00:30Z" }),
        json!({ "position": 2, "itemId": "m2", "type": "assistant_message", "status": "completed", "text": "after it" }),
    ];
    rig.t3.activity.lock().unwrap().insert("t1".into(), activity);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "dc": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "t1" }] }));
    rig.bridge.tick();
    assert_eq!(view_items(&rig).len(), 2, "still under way");
    rig.t3.activity.lock().unwrap().get_mut("t1").unwrap()[1]["status"] = json!("completed");
    rig.t3.set("running", "2026-11-06T10:01:00Z");
    rig.advance(5_000);
    rig.bridge.tick();
    let items = view_items(&rig);
    assert_eq!(items.len(), 3, "{items:?}");
    assert_eq!(items[0]["x"], "/compact");
    assert_eq!(items[1], json!({ "r": "a", "x": "Context compacted", "rule": 1 }));
    assert_eq!(items[2]["x"], "after it");
}

#[test]
fn without_an_addon_that_draws_cards_links_still_read_as_words_and_no_activity_is_read() {
    let mut rig = delegate_rig(42_000_000, false, "completed", "VERDICT: APPROVE");
    rig.bridge.tick();
    assert!(!rig.t3.calls_to("t3_thread_read").iter().any(is_scan));
    let items = view_items(&rig);
    assert_eq!(items.len(), 2);
    assert_eq!(items[1]["x"], "Sol is reviewing in a delegated task. Back in “Fix tests” after.");
}

// ---- pasted images ----

/// A rig whose game can show pictures, with an image on the clipboard (`w` x `h`, its PNG
/// `png_bytes` long) and Ctrl+V presses set by the test.
fn paste_rig(start_ms: i64, w: u32, h: u32, png_bytes: usize) -> (Rig, Arc<Mutex<(u64, i64)>>) {
    let mut rig = Rig::new(start_ms);
    let presses = Arc::new(Mutex::new((0u64, 0i64)));
    let read = Arc::clone(&presses);
    rig.bridge.presses = Box::new(move || *read.lock().unwrap());
    rig.bridge.clipboard = Box::new(move || {
        Ok(Some(Clipped { png: vec![7; png_bytes], bitmap: Bitmap { width: w, height: h, rgba: vec![200; (w * h * 4) as usize] } }))
    });
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "im": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();
    (rig, presses)
}

/// You press Ctrl+V in WoW now.
fn press(rig: &Rig, presses: &Mutex<(u64, i64)>) {
    let mut p = presses.lock().unwrap();
    *p = (p.0 + 1, *rig.clock.lock().unwrap());
}

/// The outcome the game hears for command `s`.
fn outcome(rig: &Rig, s: u64) -> Value {
    rig.bridge.snapshot()["out"].as_array().into_iter().flatten().find(|o| o["s"] == s).cloned().unwrap_or(Value::Null)
}

#[test]
fn a_paste_right_after_ctrl_v_in_wow_shows_above_the_box_and_goes_with_the_message() {
    let (mut rig, presses) = paste_rig(50_000_000, 1920, 1080, 5000);
    rig.advance(2000);
    press(&rig, &presses);
    rig.advance(300);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "im": 1, "q": [{ "s": 2, "t": "paste", "d": "mac:t1" }] }));
    rig.bridge.tick();
    assert_eq!(outcome(&rig, 2), json!({ "s": 2, "ok": true }));
    // Its small picture is in an image slot, named to the game process.
    let att = rig.bridge.snapshot()["att"].clone();
    assert_eq!(att, json!([{ "d": "mac:t1", "i": 1, "w": 1920, "h": 1080, "f": 1, "tw": 142, "th": 80, "bf": 2, "bw": 1024, "bh": 576 }]));
    assert!(rig.written.lock().unwrap().images.contains_key(&1));
    assert!(rig.bridge.link.images_shown.contains(&1));
    assert!(rig.slot(2).contains("att={{d=\"mac:t1\",i=1,"), "{}", rig.slot(2));

    // The message carries it: uploaded, then sent with T3's send for attachments.
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 3, "q": [{ "s": 3, "t": "send", "e": "mac", "id": "t1", "m": "what's this?" }] }));
    rig.bridge.tick();
    let prepared = rig.t3.calls_to("t3_attachment_prepare_upload");
    assert_eq!(prepared, vec![json!({ "upload": { "type": "image", "name": "pasted-image-1.png", "mimeType": "image/png", "sizeBytes": 5000 } })]);
    assert_eq!(rig.t3.uploads.lock().unwrap().clone(), vec![("/api/attachments/upload/token0".to_string(), "image/png".to_string(), 5000)]);
    let sends = rig.t3.calls_to("t3_thread_send_attachments");
    assert_eq!(sends, vec![json!({
        "threadId": "t1",
        "attachments": [{ "type": "image", "id": "pending-0", "name": "pasted-image-1.png", "mimeType": "image/png", "sizeBytes": 5000 }],
        "message": "what's this?",
        "environmentId": "mac",
    })]);
    assert!(rig.t3.calls_to("t3_thread_send").is_empty());
    assert!(rig.bridge.snapshot()["att"].is_null(), "sent: no longer above the box");

    // The next message has none: an ordinary send.
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 4, "q": [{ "s": 4, "t": "send", "e": "mac", "id": "t1", "m": "thanks" }] }));
    rig.bridge.tick();
    assert_eq!(rig.t3.calls_to("t3_thread_send").len(), 1);
    assert_eq!(rig.t3.calls_to("t3_thread_send_attachments").len(), 1);
}

#[test]
fn a_sent_image_shows_under_its_own_message_and_not_under_older_words_like_it() {
    let (mut rig, presses) = paste_rig(58_000_000, 1920, 1080, 5000);
    rig.bridge.set_painter(Some(Box::new(FakePainter::default())));
    // The same words said long before, with no image.
    let old = json!({ "position": 0, "messageId": "old", "type": "user_message", "text": "look", "updatedAt": "1970-01-01T10:00:00Z" });
    rig.t3.messages.lock().unwrap().insert("t1".into(), vec![old.clone()]);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "im": 1, "q": [{ "s": 2, "t": "open", "e": "mac", "id": "t1" }] }));
    rig.bridge.tick();
    press(&rig, &presses);
    rig.advance(200);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 3, "im": 1, "q": [{ "s": 3, "t": "paste", "d": "mac:t1" }] }));
    rig.bridge.tick();
    assert_eq!(rig.bridge.snapshot()["att"][0]["bf"], 2);

    // Sent, but T3 doesn't show it yet: the older message doesn't take the picture.
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 4, "im": 1, "q": [{ "s": 4, "t": "send", "e": "mac", "id": "t1", "m": "look" }] }));
    rig.bridge.tick();
    assert!(rig.bridge.snapshot()["att"].is_null());
    assert!(view_items(&rig).iter().all(|item| item["im"].is_null()), "{:?}", view_items(&rig));

    // T3 shows it: the large picture follows that message, with no second copy written.
    let written = rig.written.lock().unwrap().images.len();
    let new = json!({ "position": 2, "messageId": "mcp:new", "type": "user_message", "text": "look", "updatedAt": "1970-01-01T16:06:50Z" });
    rig.t3.messages.lock().unwrap().insert("t1".into(), vec![old.clone(), new.clone()]);
    rig.t3.set("completed", "1970-01-01T16:06:51Z");
    rig.advance(5_000);
    rig.bridge.tick();
    let items = view_items(&rig);
    assert_eq!(items.len(), 3, "{items:?}");
    assert_eq!(items[2], json!({ "r": "u", "x": "[Pasted image]", "im": { "f": 2, "w": 1024, "h": 576, "sm": 1 } }));
    assert_eq!(rig.written.lock().unwrap().images.len(), written);
    assert_eq!(rig.bridge.link.images[&2].key, "sent:mac:t1:mcp:new:0");

    // A new game process still finds it: it's filed under the message now.
    rig.bridge.on_capture(CaptureMessage::Wow(Some("456@2".into())));
    rig.t3.set("completed", "1970-01-01T16:06:59Z");
    rig.advance(5_000);
    rig.bridge.tick();
    assert_eq!(view_items(&rig)[2]["im"]["f"], 2);
    assert!(rig.bridge.link.images_shown.contains(&2), "named to the new game process");
}

#[test]
fn a_sent_image_whose_file_was_emptied_says_so_and_its_slot_is_never_named() {
    let (mut rig, presses) = paste_rig(60_000_000, 4, 4, 100);
    rig.bridge.set_painter(Some(Box::new(FakePainter::default())));
    rig.t3.messages.lock().unwrap().insert("t1".into(), Vec::new());
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "im": 1, "q": [{ "s": 2, "t": "open", "e": "mac", "id": "t1" }] }));
    rig.bridge.tick();
    press(&rig, &presses);
    rig.advance(200);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 3, "im": 1, "q": [{ "s": 3, "t": "paste", "d": "mac:t1" }] }));
    rig.bridge.tick();
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 4, "im": 1, "q": [{ "s": 4, "t": "send", "e": "mac", "id": "t1", "m": "" }] }));
    rig.bridge.tick();
    let new = json!({ "position": 0, "messageId": "mcp:new", "type": "user_message", "text": "", "updatedAt": "1970-01-01T16:40:10Z" });
    rig.t3.messages.lock().unwrap().insert("t1".into(), vec![new]);
    rig.t3.set("completed", "1970-01-01T16:40:11Z");
    rig.advance(5_000);
    rig.bridge.tick();
    assert_eq!(view_items(&rig)[0]["im"]["f"], 2);

    // A reinstall empties the file while the companion runs, with the thread unchanged in
    // T3; then a new game process.
    rig.written.lock().unwrap().images.remove(&2);
    rig.bridge.republish();
    rig.bridge.on_capture(CaptureMessage::Wow(Some("456@2".into())));
    rig.advance(5_000);
    rig.bridge.tick();
    let items = view_items(&rig);
    assert_eq!(items, vec![json!({ "r": "u", "x": "|cff808080[Pasted image] (couldn't be loaded)|r", "lost": 1 })], "no empty bubble, and no slot named");
    assert!(!rig.bridge.link.images_shown.contains(&2), "the game must never ask for the blank file");
    // The new game's window connects, and the update reaches it.
    rig.frame(json!({ "k": "c1", "u": "ui2", "n": 0, "l": 1, "im": 1, "q": [] }));
    rig.advance(2_000);
    rig.bridge.tick();
    assert!(rig.written.lock().unwrap().slots.values().any(|lua| lua.contains("lost=1")), "the update says so");
}

#[test]
fn a_page_whose_file_was_emptied_is_drawn_again_in_another_slot() {
    let (mut rig, painter) = picture_rig(61_000_000, "<p>chart</p>");
    rig.bridge.tick();
    assert_eq!(view_items(&rig)[1]["im"]["f"], 1);

    // Emptied while this game process may already have asked for slot 1: it isn't used again.
    rig.written.lock().unwrap().images.remove(&1);
    rig.t3.set("completed", "2026-11-06T10:00:09Z");
    rig.advance(5_000);
    rig.bridge.tick();
    assert_eq!(painter.calls().len(), 2);
    assert_eq!(view_items(&rig)[1]["im"]["f"], 2);
}

#[test]
fn an_image_sent_with_no_words_is_only_its_picture_and_other_blank_messages_stay() {
    let (mut rig, presses) = paste_rig(59_000_000, 4, 4, 100);
    rig.bridge.set_painter(Some(Box::new(FakePainter::default())));
    let blank = json!({ "position": 0, "messageId": "blank", "type": "user_message", "text": "", "updatedAt": "1970-01-01T10:00:00Z" });
    rig.t3.messages.lock().unwrap().insert("t1".into(), vec![blank.clone()]);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "im": 1, "q": [{ "s": 2, "t": "open", "e": "mac", "id": "t1" }] }));
    rig.bridge.tick();
    press(&rig, &presses);
    rig.advance(200);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 3, "im": 1, "q": [{ "s": 3, "t": "paste", "d": "mac:t1" }] }));
    rig.bridge.tick();
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 4, "im": 1, "q": [{ "s": 4, "t": "send", "e": "mac", "id": "t1", "m": "" }] }));
    rig.bridge.tick();
    let new = json!({ "position": 2, "messageId": "mcp:new", "type": "user_message", "text": "", "updatedAt": "1970-01-01T16:23:30Z" });
    rig.t3.messages.lock().unwrap().insert("t1".into(), vec![blank, new]);
    rig.t3.set("completed", "1970-01-01T16:23:31Z");
    rig.advance(5_000);
    rig.bridge.tick();
    let items = view_items(&rig);
    assert_eq!(items.len(), 2, "{items:?}");
    assert_eq!(items[0]["x"], "", "a blank message with no picture stays");
    assert_eq!(items[1], json!({ "r": "u", "x": "[Pasted image]", "im": { "f": 2, "w": 4, "h": 4, "sm": 1 } }));
}

#[test]
fn ctrl_v_is_looked_for_only_while_you_type_in_the_message_box() {
    let mut rig = Rig::new(60_000_000);
    let armed = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&armed);
    rig.bridge.arm_keys = Box::new(move |on| log.lock().unwrap().push(on));
    let states = [
        json!({ "o": true, "f": false }),
        json!({ "o": true, "f": true }),
        json!({ "o": true, "f": false }),
        json!({ "o": false, "f": true }),
        // An addon older than this doesn't say whether you're typing.
        json!({ "o": true }),
        json!({ "o": false }),
    ];
    for (i, state) in states.iter().enumerate() {
        let s = i as u64 + 1;
        rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": s, "q": [{ "s": s, "t": "ui", "o": state["o"], "f": state["f"] }] }));
        rig.bridge.tick();
    }
    assert_eq!(*armed.lock().unwrap(), [false, true, false, false, true, false]);
}

#[test]
fn no_addon_can_paste_without_your_ctrl_v_and_each_press_pastes_once() {
    let (mut rig, presses) = paste_rig(51_000_000, 4, 4, 100);
    let paste = |rig: &mut Rig, s: u64| {
        rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": s, "t": "paste", "d": "mac:t1" }] }));
        rig.bridge.tick();
        outcome(rig, s)
    };
    // No Ctrl+V at all: another addon drew it.
    let refused = paste(&mut rig, 2);
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["x"], UNPROVEN_PASTE);
    // A Ctrl+V long before: refused too.
    press(&rig, &presses);
    rig.advance(PASTE_PRESS_MS + 1000);
    assert_eq!(paste(&mut rig, 3)["ok"], false);
    // Right after one: taken, once.
    press(&rig, &presses);
    rig.advance(200);
    assert_eq!(paste(&mut rig, 4)["ok"], true);
    rig.advance(200);
    assert_eq!(paste(&mut rig, 5)["x"], UNPROVEN_PASTE, "that press was used");
    // A paste drawn before the press it would ride on (the frame was read first).
    let count = presses.lock().unwrap().0;
    *presses.lock().unwrap() = (count + 1, *rig.clock.lock().unwrap() + 1);
    assert_eq!(paste(&mut rig, 6)["ok"], false);
    assert_eq!(rig.bridge.snapshot()["att"].as_array().map(Vec::len), Some(1));
    // Removing one needs no press: it only takes back what you pasted.
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 3, "q": [{ "s": 7, "t": "unpaste", "d": "mac:t1", "i": 1 }] }));
    rig.bridge.tick();
    assert!(rig.bridge.snapshot()["att"].is_null());
}

#[test]
fn a_paste_reads_the_frame_time_not_when_a_slow_t3_let_it_run() {
    let (mut rig, presses) = paste_rig(52_000_000, 4, 4, 100);
    press(&rig, &presses);
    rig.advance(300);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": 2, "t": "paste", "d": "new" }] }));
    // The worker was busy with a slow request: the command runs much later.
    rig.advance(PASTE_PRESS_MS * 3);
    rig.bridge.tick();
    assert_eq!(outcome(&rig, 2)["ok"], true);
}

#[test]
fn a_new_thread_starts_with_its_pasted_images_and_a_failed_send_keeps_them() {
    let (mut rig, presses) = paste_rig(53_000_000, 4, 4, 100);
    for s in 2..4 {
        press(&rig, &presses);
        rig.advance(100);
        rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": s, "t": "paste", "d": "new" }] }));
        rig.bridge.tick();
    }
    assert_eq!(rig.bridge.snapshot()["att"].as_array().map(Vec::len), Some(2));
    // The launch fails: what was uploaded is given back, and the images stay for another try.
    rig.t3.failing.lock().unwrap().insert("t3_thread_launch".into(), "T3 is busy".into());
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 3, "q": [{ "s": 4, "t": "new", "e": "mac", "m": "" }] }));
    rig.bridge.tick();
    assert_eq!(outcome(&rig, 4)["ok"], false);
    let discarded: Vec<Value> = rig.t3.calls_to("t3_attachment_discard").iter().map(|a| a["attachmentId"].clone()).collect();
    assert_eq!(discarded, vec![json!("pending-0"), json!("pending-1")]);
    assert_eq!(rig.bridge.snapshot()["att"].as_array().map(Vec::len), Some(2));
    // Again, and it starts: images only, so it gets a title of its own.
    rig.t3.failing.lock().unwrap().clear();
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 4, "q": [{ "s": 5, "t": "new", "e": "mac", "m": "" }] }));
    rig.bridge.tick();
    let launched = rig.t3.calls_to("t3_thread_launch").last().cloned().unwrap();
    assert_eq!(launched["title"], "Pasted image");
    assert!(launched["message"].is_null());
    let ids: Vec<Value> = launched["attachments"].as_array().unwrap().iter().map(|a| a["id"].clone()).collect();
    assert_eq!(ids, vec![json!("pending-2"), json!("pending-3")]);
    assert_eq!(outcome(&rig, 5)["id"], "new1");
    assert!(rig.bridge.snapshot()["att"].is_null());
}

#[test]
fn a_failed_upload_sends_nothing_and_says_why() {
    let (mut rig, presses) = paste_rig(54_000_000, 4, 4, 100);
    press(&rig, &presses);
    rig.advance(100);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": 2, "t": "paste", "d": ":t1" }] }));
    rig.bridge.tick();
    rig.t3.failing.lock().unwrap().insert("upload".into(), "T3 refused the image (413): too large".into());
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 3, "q": [{ "s": 3, "t": "send", "id": "t1", "m": "look" }] }));
    rig.bridge.tick();
    assert_eq!(outcome(&rig, 3)["x"], "couldn't upload the image: T3 refused the image (413): too large");
    assert!(rig.t3.calls_to("t3_thread_send_attachments").is_empty());
    assert!(rig.t3.calls_to("t3_thread_send").is_empty(), "not sent without its image");
    assert_eq!(rig.t3.calls_to("t3_attachment_discard").len(), 1);
    assert_eq!(rig.bridge.snapshot()["att"].as_array().map(Vec::len), Some(1));
}

#[test]
fn pastes_stop_at_eight_need_an_image_and_follow_the_tray_setting() {
    let (mut rig, presses) = paste_rig(55_000_000, 4, 4, 100);
    for s in 2..=10 {
        press(&rig, &presses);
        rig.advance(100);
        rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": s, "t": "paste", "d": "new" }] }));
        rig.bridge.tick();
    }
    assert_eq!(rig.bridge.snapshot()["att"].as_array().map(Vec::len), Some(MAX_PASTED));
    assert_eq!(outcome(&rig, 10)["x"], "a message takes 8 images at most");

    rig.bridge.clipboard = Box::new(|| Ok(None));
    press(&rig, &presses);
    rig.advance(100);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 3, "q": [{ "s": 11, "t": "paste", "d": "mac:t1" }] }));
    rig.bridge.tick();
    assert_eq!(outcome(&rig, 11)["x"], NO_IMAGE);

    // Read-only: no paste (it would only be sent), but taking one back still works.
    rig.bridge.set_trust(Trust::ReadOnly);
    press(&rig, &presses);
    rig.advance(100);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 4, "q": [{ "s": 12, "t": "paste", "d": "mac:t1" }, { "s": 13, "t": "unpaste", "d": "new", "i": 1 }] }));
    rig.bridge.tick();
    assert!(str_of(&outcome(&rig, 12)["x"]).starts_with("blocked by the T3 WoW tray setting"));
    assert_eq!(rig.bridge.snapshot()["att"].as_array().map(Vec::len), Some(MAX_PASTED - 1));
}

#[test]
fn pasted_images_go_with_the_sign_in_and_the_game_process_they_came_from() {
    let (mut rig, presses) = paste_rig(56_000_000, 4, 4, 100);
    press(&rig, &presses);
    rig.advance(100);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": 2, "t": "paste", "d": "new" }] }));
    rig.bridge.tick();
    assert!(rig.bridge.snapshot()["att"].is_array());
    // A /reload keeps them: the game shows them again from the next update.
    rig.frame(json!({ "k": "c1", "u": "ui2", "n": 0, "l": 1, "im": 1, "q": [{ "s": 3, "t": "hello", "o": false }] }));
    rig.bridge.tick();
    assert!(rig.bridge.snapshot()["att"].is_array());
    // WoW restarted: gone.
    rig.bridge.on_capture(CaptureMessage::Wow(Some("456@2".into())));
    assert!(rig.bridge.snapshot()["att"].is_null());
    // And so with another sign-in.
    press(&rig, &presses);
    rig.frame(json!({ "k": "c1", "u": "ui3", "n": 0, "l": 1, "im": 1, "q": [{ "s": 4, "t": "paste", "d": "new" }] }));
    rig.bridge.tick();
    assert!(rig.bridge.snapshot()["att"].is_array());
    rig.bridge.set_login("another-t3");
    assert!(rig.bridge.snapshot()["att"].is_null());
}

#[test]
fn without_pictures_in_the_game_a_paste_still_works_with_no_small_picture() {
    let mut rig = Rig::new(57_000_000);
    rig.bridge.presses = Box::new(|| (1, 57_000_050));
    rig.bridge.clipboard = Box::new(|| Ok(Some(Clipped { png: vec![1; 10], bitmap: Bitmap { width: 2, height: 2, rgba: vec![0; 16] } })));
    rig.advance(100);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "paste", "d": "new" }] }));
    rig.bridge.tick();
    assert_eq!(rig.bridge.snapshot()["att"], json!([{ "d": "new", "i": 1, "w": 2, "h": 2 }]));
    assert!(rig.written.lock().unwrap().images.is_empty());
    assert_eq!(rig.bridge.snapshot()["pi"], true);
}

/// What T3 says a machine offers, in the shape it answers (see the probe in the PR).
fn capabilities() -> Value {
    json!({
        "runtimeMode": "full-access",
        "providers": [
            { "providerInstanceId": "codex", "displayName": "Codex", "constraints": [], "models": [
                { "id": "gpt-luna", "options": [
                    { "id": "reasoningEffort", "values": ["low", "medium"], "default": "medium" },
                    { "id": "serviceTier", "values": ["default", "priority"], "default": "default" },
                ] },
            ] },
            { "providerInstanceId": "claudeAgent", "displayName": "Claude", "constraints": [], "models": [
                { "id": "claude-opus", "label": "Claude Opus", "options": [
                    { "id": "effort", "values": ["low", "high"], "default": "medium" },
                    { "id": "fastMode", "values": [false, true] },
                ] },
                { "id": "claude-haiku", "label": "Claude Haiku", "options": [{ "id": "effort", "values": ["low"], "default": "low" }] },
            ] },
            { "providerInstanceId": "picky", "displayName": "Picky", "constraints": [], "runtimeModes": ["approval-required"], "models": [{ "id": "only", "label": "Only" }] },
            // Disabled ones still list models.
            { "providerInstanceId": "grok", "displayName": "Grok", "constraints": ["Provider instance is disabled."], "models": [{ "id": "grok-build" }] },
            { "providerInstanceId": "empty", "displayName": "Empty", "constraints": [], "models": [] },
        ],
    })
}

/// A rig whose machines have been read once, with the window open.
fn rig_with_catalog(start_ms: i64) -> Rig {
    let mut rig = Rig::new(start_ms);
    *rig.t3.capabilities.lock().unwrap() = capabilities();
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();
    rig.advance(61_000);
    rig.bridge.tick();
    rig
}

#[test]
fn known_models_go_by_one_style_of_name_and_new_ones_by_what_t3_calls_them() {
    let listed = |model: Value| {
        let capabilities = json!({ "providers": [{ "providerInstanceId": "p", "constraints": [], "models": [model] }] });
        str_of(&compact_providers(&capabilities)[0]["m"][0]["t"]).to_string()
    };
    assert_eq!(listed(json!({ "id": "gpt-6.1-sol" })), "GPT-6.1 Sol", "T3 gives it no label");
    assert_eq!(listed(json!({ "id": "claude-opus-5-5", "label": "something else" })), "Claude Opus 5.5");
    assert_eq!(listed(json!({ "id": "brand-new", "label": "Brand New 1" })), "Brand New 1", "not ours: T3's label");
    assert_eq!(listed(json!({ "id": "brand-new" })), "brand-new", "no label either: its id");

    // A thread on a model the short list leaves out, and the handoff line's name for one.
    let thread = |model: &str| json!({ "model": model, "providerInstanceId": "codex", "runtimeMode": "auto" });
    assert_eq!(thread_settings(&thread("gpt-6-sol"), None).unwrap()["mt"], "GPT-6 Sol");
    assert!(thread_settings(&thread("brand-new"), None).unwrap()["mt"].is_null());
    let rig = Rig::new(40_000_000);
    assert_eq!(rig.bridge.model_name("e", &json!("codex"), &json!("gpt-6-sol")), "GPT-6 Sol");
    assert_eq!(rig.bridge.model_name("e", &json!("codex"), &json!("brand-new")), "brand-new");
}

#[test]
fn the_game_hears_what_each_machine_offers_and_a_new_threads_default_permissions() {
    let rig = rig_with_catalog(40_000_000);
    let snapshot = rig.bridge.snapshot();
    assert_eq!(snapshot["sel"], true);
    let models = snapshot["models"].as_array().unwrap();
    let on_local: Vec<&str> = models.iter().filter(|p| p["e"] == 1).map(|p| str_of(&p["id"])).collect();
    assert_eq!(on_local, ["codex", "claudeAgent", "picky"], "disabled and empty providers stay out");
    let on_mac: Vec<&str> = models.iter().filter(|p| p["e"] == 2).map(|p| str_of(&p["id"])).collect();
    assert_eq!(on_mac, ["codex", "claudeAgent", "picky"]);
    assert_eq!(snapshot["envs"][0]["dm"], "full-access");

    let claude = models.iter().find(|p| p["id"] == "claudeAgent" && p["e"] == 1).unwrap();
    assert_eq!(claude["t"], "Claude");
    assert_eq!(claude["m"][0], json!({ "id": "claude-opus", "t": "Claude Opus", "o": "effort", "ef": ["low", "high"] }));
    assert!(claude["rm"].is_null(), "a provider with no list takes every mode");
    let codex = models.iter().find(|p| p["id"] == "codex" && p["e"] == 1).unwrap();
    assert_eq!(codex["m"][0]["t"], "gpt-luna", "a model with no label goes by its id");
    assert_eq!(codex["m"][0]["o"], "reasoningEffort");
    let picky = models.iter().find(|p| p["id"] == "picky" && p["e"] == 1).unwrap();
    assert_eq!(picky["rm"], json!(["approval-required"]));
    assert!(picky["m"][0]["ef"].is_null(), "no efforts to list");
}

#[test]
fn what_a_machine_offers_is_read_every_few_minutes_and_a_failed_read_keeps_the_last() {
    let mut rig = rig_with_catalog(41_000_000);
    // One read a machine, though the machines were read twice.
    assert_eq!(rig.t3.calls_to("orchestrator_capabilities").len(), 2);
    assert!(rig.t3.calls_to("orchestrator_capabilities").iter().any(|a| a["environmentId"] == "mac"));
    rig.advance(CATALOG_TTL_MS);
    rig.bridge.tick();
    assert_eq!(rig.t3.calls_to("orchestrator_capabilities").len(), 4);

    rig.t3.failing.lock().unwrap().insert("orchestrator_capabilities".into(), "no such tool".into());
    rig.advance(CATALOG_TTL_MS);
    rig.bridge.tick();
    assert_eq!(rig.t3.calls_to("orchestrator_capabilities").len(), 6);
    assert!(!rig.bridge.snapshot()["models"].as_array().unwrap().is_empty(), "lost what it knew");
    // Not asked again every minute.
    rig.advance(61_000);
    rig.bridge.tick();
    assert_eq!(rig.t3.calls_to("orchestrator_capabilities").len(), 6);
}

#[test]
fn what_a_machine_offers_goes_when_it_is_unlinked_or_the_t3_changes() {
    let mut rig = rig_with_catalog(42_000_000);
    *rig.t3.links.lock().unwrap() = json!([]);
    rig.advance(61_000);
    rig.bridge.tick();
    let models = rig.bridge.snapshot()["models"].clone();
    assert!(models.as_array().unwrap().iter().all(|p| p["e"] == 1), "{models}");
    rig.bridge.set_login("another");
    assert!(rig.bridge.snapshot()["models"].is_null());
}

#[test]
fn a_model_named_in_a_message_reads_as_its_name_when_the_machine_has_it() {
    let mut rig = Rig::new(43_500_000);
    *rig.t3.capabilities.lock().unwrap() = capabilities();
    let said = "Asking model:claudeAgent/claude-opus, not model:claudeAgent/gone or model:nobody/x.\n\n\
                | Who | Why |\n|---|---|\n| model:codex/gpt-luna | `model:codex/gpt-luna` |";
    rig.t3.messages.lock().unwrap().insert("t1".into(), conversation(&["delegate to model:codex/gpt-luna", said]));
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "t1" }] }));
    rig.bridge.tick();
    rig.advance(61_000);
    rig.bridge.tick();
    let items = view_items(&rig);
    assert_eq!(items[0]["x"], "delegate to gpt-luna");
    assert_eq!(items[0]["cx"], "delegate to model:codex/gpt-luna", "copying keeps T3's text");
    let x = str_of(&items[1]["x"]);
    assert!(x.starts_with("Asking Claude Opus, not model:claudeAgent/gone or model:nobody/x."), "{x}");
    assert!(str_of(&items[1]["cx"]).starts_with("Asking model:claudeAgent/claude-opus, not"), "{}", items[1]["cx"]);
    // A table's cells are named too; code keeps the reference.
    let table = items[1]["g"].as_array().unwrap().iter().find(|s| s["t"].is_array()).unwrap();
    assert_eq!(table["t"][1], json!(["gpt-luna", "model:codex/gpt-luna"]));
    assert!(!rig.bridge.snapshot().to_string().contains('\u{5}'), "a mark left in");
}

#[test]
fn a_provider_says_its_driver_when_that_isnt_its_id() {
    let capabilities = json!({ "providers": [
        { "providerInstanceId": "claudeAgent", "driverKind": "claudeAgent", "displayName": "Claude", "constraints": [], "models": [{ "id": "a" }] },
        { "providerInstanceId": "work", "driverKind": "claudeAgent", "displayName": "Work", "constraints": [], "models": [{ "id": "a" }] },
    ] });
    let providers = compact_providers(&capabilities);
    assert!(providers[0]["dk"].is_null(), "the same as its id");
    assert_eq!(providers[1]["dk"], "claudeAgent");
}

#[test]
fn the_open_thread_says_what_it_runs_on_even_if_t3_wont_say_its_effort() {
    let mut rig = rig_with_catalog(43_000_000);
    {
        let mut threads = rig.t3.threads.lock().unwrap();
        threads[0]["providerInstanceId"] = json!("claudeAgent");
        threads[0]["model"] = json!("claude-opus");
        threads[0]["runtimeMode"] = json!("auto");
    }
    rig.t3.configs.lock().unwrap().insert("t1".into(), json!({
        "modelSelection": { "instanceId": "claudeAgent", "model": "claude-opus", "options": [{ "id": "effort", "value": "high" }, { "id": "fastMode", "value": false }] },
        "runtimeMode": "approval-required",
    }));
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": 2, "t": "open", "e": "local", "id": "t1" }] }));
    rig.advance(5_000);
    rig.bridge.tick();
    assert_eq!(rig.bridge.snapshot()["view"]["cfg"], json!({ "pv": "claudeAgent", "m": "claude-opus", "ef": "high", "rm": "approval-required" }));

    // Without T3's read of its settings, the thread's own read still names the model.
    rig.t3.failing.lock().unwrap().insert("t3_thread_configure".into(), "no such tool".into());
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 3, "q": [{ "s": 3, "t": "refresh" }] }));
    rig.advance(5_000);
    rig.bridge.tick();
    let view = rig.bridge.snapshot()["view"].clone();
    assert_eq!(view["cfg"], json!({ "pv": "claudeAgent", "m": "claude-opus", "rm": "auto" }));
    assert_eq!(view["id"], "t1", "the conversation is still shown");
}

#[test]
fn a_new_thread_starts_on_the_picks_and_on_t3s_defaults_for_the_rest() {
    let mut rig = rig_with_catalog(44_000_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [
        { "s": 2, "t": "new", "e": "mac", "m": "All picked", "pv": "claudeAgent", "md": "claude-opus", "ef": "high", "rm": "approval-required" },
        { "s": 3, "t": "new", "e": "mac", "m": "Only a model", "pv": "codex", "md": "gpt-luna" },
        { "s": 4, "t": "new", "e": "mac", "m": "Nothing picked" },
    ] }));
    rig.bridge.tick();
    let launches = rig.t3.calls_to("t3_thread_launch");
    assert_eq!(launches[0]["modelSelection"], json!({ "instanceId": "claudeAgent", "model": "claude-opus", "options": [{ "id": "effort", "value": "high" }] }));
    assert_eq!(launches[0]["runtimeMode"], "approval-required");
    assert_eq!(launches[0]["message"], "All picked");
    assert_eq!(launches[1]["modelSelection"], json!({ "instanceId": "codex", "model": "gpt-luna" }));
    assert!(launches[1]["runtimeMode"].is_null(), "{}", launches[1]);
    assert!(launches[2]["modelSelection"].is_null() && launches[2]["runtimeMode"].is_null(), "{}", launches[2]);
}

#[test]
fn a_permission_mode_or_effort_it_cant_name_starts_nothing() {
    let mut rig = rig_with_catalog(45_000_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [
        { "s": 2, "t": "new", "e": "mac", "m": "Oops", "rm": "root" },
        { "s": 3, "t": "new", "e": "mac", "m": "Oops", "pv": "claudeAgent", "md": "claude-opus", "ef": "nonsense" },
    ] }));
    rig.bridge.tick();
    assert!(rig.t3.calls_to("t3_thread_launch").is_empty(), "started at T3's default instead");
    let out = rig.bridge.snapshot()["out"].clone();
    assert_eq!(out[0]["ok"], false);
    assert_eq!(out[0]["x"], "unknown permission mode \"root\"");
    assert_eq!(out[1]["x"], "claude-opus has no effort \"nonsense\"");
}

#[test]
fn changing_a_threads_effort_keeps_its_other_options_and_a_new_model_starts_clean() {
    let mut rig = rig_with_catalog(46_000_000);
    rig.t3.configs.lock().unwrap().insert("t1".into(), json!({
        "modelSelection": { "instanceId": "claudeAgent", "model": "claude-opus", "options": [{ "id": "effort", "value": "high" }, { "id": "fastMode", "value": true }] },
        "runtimeMode": "auto",
    }));
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": 2, "t": "model", "e": "mac", "id": "t1", "pv": "claudeAgent", "md": "claude-opus", "ef": "low" }] }));
    rig.bridge.tick();
    let sets: Vec<Value> = rig.t3.calls_to("t3_thread_configure").into_iter().filter(|a| a["modelSelection"].is_object()).collect();
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0]["environmentId"], "mac");
    assert_eq!(sets[0]["threadId"], "t1");
    let mut options = sets[0]["modelSelection"]["options"].as_array().unwrap().clone();
    options.sort_by_key(|o| str_of(&o["id"]).to_string());
    assert_eq!(options, vec![json!({ "id": "effort", "value": "low" }), json!({ "id": "fastMode", "value": true })]);

    // Another model: its options are its own (fast mode may not exist there).
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 3, "q": [{ "s": 3, "t": "model", "e": "mac", "id": "t1", "pv": "claudeAgent", "md": "claude-haiku", "ef": "low" }] }));
    rig.bridge.tick();
    let sets: Vec<Value> = rig.t3.calls_to("t3_thread_configure").into_iter().filter(|a| a["modelSelection"].is_object()).collect();
    assert_eq!(sets[1]["modelSelection"], json!({ "instanceId": "claudeAgent", "model": "claude-haiku", "options": [{ "id": "effort", "value": "low" }] }));
}

#[test]
fn a_thread_moved_to_another_provider_takes_that_providers_effort_option_and_none_of_the_old_ones() {
    let mut rig = rig_with_catalog(46_500_000);
    rig.t3.configs.lock().unwrap().insert("t1".into(), json!({
        "modelSelection": { "instanceId": "claudeAgent", "model": "claude-opus", "options": [{ "id": "effort", "value": "low" }, { "id": "fastMode", "value": true }] },
        "runtimeMode": "auto",
    }));
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": 2, "t": "model", "e": "mac", "id": "t1", "pv": "codex", "md": "gpt-luna", "ef": "low" }] }));
    rig.bridge.tick();
    let sets: Vec<Value> = rig.t3.calls_to("t3_thread_configure").into_iter().filter(|a| a["modelSelection"].is_object()).collect();
    assert_eq!(sets[0]["modelSelection"], json!({ "instanceId": "codex", "model": "gpt-luna", "options": [{ "id": "reasoningEffort", "value": "low" }] }));
}

#[test]
fn the_first_message_on_a_new_provider_has_the_handoff_line_and_a_new_model_on_the_same_one_doesnt() {
    let mut rig = rig_with_catalog(46_700_000);
    let message = |position: u64, kind: &str, run: &str| json!({ "position": position, "type": kind, "text": format!("m{position}"), "runId": run });
    let run = |id: &str, provider: &str, model: &str| json!({ "runId": id, "providerInstanceId": provider, "model": model });
    rig.t3.messages.lock().unwrap().insert("t1".into(), vec![
        message(0, "user_message", "r1"), message(1, "assistant_message", "r1"),
        // Another model on the same provider: no line.
        message(2, "user_message", "r2"), message(3, "assistant_message", "r2"),
        // Another provider, with a model the machine doesn't list.
        message(4, "user_message", "r3"), message(5, "assistant_message", "r3"),
        // And back: named as the machine names it.
        message(6, "user_message", "r4"),
        // A run T3 didn't list: passed over, no line.
        message(7, "assistant_message", "gone"),
    ]);
    *rig.t3.runs.lock().unwrap() = json!([
        run("r4", "claudeAgent", "claude-opus"), run("r3", "codex", "unlisted-model"),
        run("r2", "claudeAgent", "claude-haiku"), run("r1", "claudeAgent", "claude-opus"),
    ]);
    rig.t3.threads.lock().unwrap()[0]["itemCount"] = json!(8);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": 2, "t": "open", "e": "local", "id": "t1" }] }));
    rig.advance(5_000);
    rig.bridge.tick();
    let view = rig.bridge.snapshot()["view"].clone();
    let lines: Vec<(&str, &Value)> = view["items"].as_array().unwrap().iter().map(|i| (str_of(&i["x"]), &i["ho"])).filter(|(_, ho)| !ho.is_null()).collect();
    assert_eq!(lines, vec![
        ("m4", &json!("Claude Haiku > unlisted-model")),
        ("m6", &json!("unlisted-model > Claude Opus")),
    ], "{view}");
    assert!(rig.t3.calls_to("t3_thread_read").iter().any(|a| a["runLimit"] == 50), "a long thread's runs wouldn't be listed");
}

#[test]
fn a_refused_model_change_comes_back_with_its_reason_and_an_unknown_effort_never_reaches_t3() {
    let mut rig = rig_with_catalog(47_000_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": 2, "t": "model", "e": "mac", "id": "t1", "pv": "claudeAgent", "md": "claude-opus", "ef": "nonsense" }] }));
    rig.bridge.tick();
    assert!(rig.t3.calls_to("t3_thread_configure").iter().all(|a| !a["modelSelection"].is_object()), "T3 stores whatever effort it's given");
    assert_eq!(rig.bridge.snapshot()["out"][0]["ok"], false);

    rig.t3.failing.lock().unwrap().insert("t3_thread_configure".into(), "Not allowed".into());
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 3, "q": [{ "s": 3, "t": "model", "e": "mac", "id": "t1", "pv": "claudeAgent", "md": "claude-opus" }] }));
    rig.bridge.tick();
    let out = rig.bridge.snapshot()["out"].clone();
    assert_eq!(out[1], json!({ "s": 3, "ok": false, "x": "Not allowed" }));
}

#[test]
fn the_trays_read_only_refuses_a_model_change_and_no_new_threads_refuses_only_starting() {
    let mut rig = rig_with_catalog(48_000_000);
    rig.bridge.set_trust(crate::config::Trust::ReadOnly);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 2, "q": [{ "s": 2, "t": "model", "e": "mac", "id": "t1", "pv": "claudeAgent", "md": "claude-opus" }] }));
    rig.bridge.tick();
    assert!(rig.t3.calls_to("t3_thread_configure").iter().all(|a| !a["modelSelection"].is_object()));
    assert_eq!(rig.bridge.snapshot()["out"][0]["ok"], false);

    rig.bridge.set_trust(crate::config::Trust::NoNewThreads);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 3, "q": [
        { "s": 3, "t": "model", "e": "mac", "id": "t1", "pv": "claudeAgent", "md": "claude-opus" },
        { "s": 4, "t": "new", "e": "mac", "m": "Nope", "rm": "full-access" },
    ] }));
    rig.bridge.tick();
    assert_eq!(rig.t3.calls_to("t3_thread_configure").iter().filter(|a| a["modelSelection"].is_object()).count(), 1);
    assert!(rig.t3.calls_to("t3_thread_launch").is_empty());
}

#[test]
#[ignore = "starts one small thread on the T3 the companion is signed in to, sends it a message in a fresh session, then archives it"]
fn clear_starts_a_fresh_session_on_the_signed_in_t3() {
    let target = crate::t3::target().expect("no T3 to talk to");
    let client = Arc::new(crate::t3::McpClient::new(&target.origin, target.token.as_deref().expect("not signed in")));
    let key = format!("probe-{}", crate::config::now_ms());
    let mut rig = Rig::new(49_500_000);
    rig.bridge.set_client(client.clone());
    rig.frame(json!({ "k": key, "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    let settle = |rig: &mut Rig, until: &dyn Fn(&Value) -> bool| -> Value {
        for _ in 0..60 {
            rig.advance(61_000);
            for _ in 0..40 {
                rig.bridge.tick();
                let snapshot = rig.bridge.snapshot();
                if until(&snapshot) {
                    return snapshot;
                }
            }
        }
        panic!("never settled: {}", rig.bridge.snapshot());
    };
    // This T3 says it can: the game offers /clear for its threads.
    settle(&mut rig, &|s| s["envs"][0]["fc"] == true);

    rig.frame(json!({ "k": key, "u": "ui1", "n": 0, "l": 2, "q": [
        { "s": 2, "t": "new", "m": "Reply with just OK, and keep this word for later in this conversation: kumquat.", "pv": "claudeAgent", "md": "claude-haiku-5-5", "ef": "low", "rm": "approval-required" },
    ] }));
    let snapshot = settle(&mut rig, &|s| s["out"].is_array());
    let id = snapshot["out"][0]["id"].as_str().unwrap_or_else(|| panic!("{}", snapshot["out"])).to_string();
    let finished = |least: u64| -> Value {
        for _ in 0..60 {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let head = client.call("t3_thread_read", json!({ "threadId": id, "limit": 1, "maxCharsPerItem": 1 })).unwrap();
            if head["thread"]["runCount"].as_u64().unwrap_or(0) >= least && !working(str_of(&head["thread"]["status"])) {
                return head;
            }
        }
        panic!("the run never ended");
    };
    // The word stays out of the title (60 chars), and so out of the scratch folder's name.
    finished(1);
    rig.frame(json!({ "k": key, "u": "ui1", "n": 0, "l": 3, "q": [
        { "s": 3, "t": "send", "id": id, "m": "What word did I ask you to remember? If you don't know, reply with just NONE.", "fc": true },
    ] }));
    rig.bridge.tick();
    finished(2);
    let read = client.call("t3_thread_read", json!({ "threadId": id, "view": "activity", "limit": 20, "maxCharsPerItem": 400 })).unwrap();
    client.call("t3_thread_organize", json!({ "action": "archive", "threadId": id })).unwrap();
    let items = read["items"].as_array().unwrap();
    for item in items {
        println!("{} {}: {}", item["position"], item["type"], str_of(&item["text"]));
    }
    // Whether T3's read marks the fresh message, for the game's New session line.
    println!("marked: {}", items.iter().chain(read["recentRuns"].as_array().unwrap()).any(|i| i.get("freshContext").is_some()));
    let reply = items.iter().rev().find(|i| i["type"] == "assistant_message").expect("no reply");
    assert!(!str_of(&reply["text"]).to_lowercase().contains("kumquat"), "the fresh session saw the earlier message: {reply}");
}

#[test]
#[ignore = "starts one scratch thread on the T3 the companion is signed in to whose agent opens a secret card, stops it unanswered, then archives it"]
fn a_secret_card_on_the_signed_in_t3_shows_as_answer_in_t3_until_it_ends() {
    let target = crate::t3::target().expect("no T3 to talk to");
    let client = Arc::new(crate::t3::McpClient::new(&target.origin, target.token.as_deref().expect("not signed in")));
    let key = format!("probe-{}", crate::config::now_ms());
    let mut rig = Rig::new(51_000_000);
    rig.bridge.set_client(client.clone());
    rig.frame(json!({ "k": key, "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    let settle = |rig: &mut Rig, what: &str, until: &dyn Fn(&Value) -> bool| -> Value {
        for _ in 0..90 {
            std::thread::sleep(std::time::Duration::from_secs(2));
            rig.advance(21_000);
            for _ in 0..40 {
                rig.bridge.tick();
                let snapshot = rig.bridge.snapshot();
                if until(&snapshot) {
                    return snapshot;
                }
            }
        }
        panic!("never {what}: {}", rig.bridge.snapshot());
    };
    let label = "T3 WoW probe value (test, don't fill)";
    let message = format!(
        "This is a test of how T3 WoW shows a pending secret request. Call the request_secret tool once, right away, with label \"{label}\", reason \"Testing T3 WoW; this card will be cancelled, don't fill it\" and timeoutMs 900000. Do nothing else; when it returns, reply with one word."
    );
    rig.frame(json!({ "k": key, "u": "ui1", "n": 0, "l": 2, "q": [{ "s": 2, "t": "new", "m": message, "rm": "full-access" }] }));
    let started = settle(&mut rig, "started", &|s| s["out"].is_array());
    let id = started["out"][0]["id"].as_str().unwrap_or_else(|| panic!("{}", started["out"])).to_string();
    println!("probe thread: {id}");
    let row = |s: &Value| s["threads"].as_array().and_then(|ts| ts.iter().find(|t| t["id"] == id.as_str()).cloned()).unwrap_or(Value::Null);

    let waiting = settle(&mut rig, "waiting on the card", &|s| row(s)["s"] == "card" && s["view"]["card"].is_object());
    println!("row: {}", row(&waiting));
    println!("view: s={} card={} approval={} ask={}", waiting["view"]["s"], waiting["view"]["card"], waiting["view"]["approval"], waiting["view"]["ask"]);
    let event = waiting["events"].as_array().and_then(|es| es.iter().rfind(|e| e["id"] == id.as_str()).cloned());
    println!("notification: {event:?}");
    let head = client.call("t3_thread_read", json!({ "threadId": id, "limit": 1, "maxCharsPerItem": 1 })).unwrap();
    println!("T3 says: status={} pendingRequestCount={}", head["thread"]["status"], head["thread"]["pendingRequestCount"]);

    // Stopped from the game, unanswered: T3 cancels the card and the status clears.
    rig.frame(json!({ "k": key, "u": "ui1", "n": 0, "l": 3, "q": [{ "s": 3, "t": "stop", "id": id }] }));
    let ended = settle(&mut rig, "cleared", &|s| !needs_you(str_of(&row(s)["s"])) && !working(str_of(&row(s)["s"])) && s["view"]["card"].is_null());
    println!("after stop: row s={} view s={} card={}", row(&ended)["s"], ended["view"]["s"], ended["view"]["card"]);
    client.call("t3_thread_organize", json!({ "action": "archive", "threadId": id })).unwrap();

    assert_eq!(waiting["view"]["card"], json!({ "k": "secret", "l": label }));
    assert_eq!(waiting["view"]["s"], "card");
    assert!(waiting["view"]["ask"].is_null() && waiting["view"]["approval"].is_null());
    assert_eq!(event.map(|e| e["s"].clone()), Some(json!("card")), "no notification");
}

#[test]
#[ignore = "starts one small thread on the T3 the companion is signed in to, changes its effort, then archives it"]
fn the_selectors_work_on_the_signed_in_t3() {
    let target = crate::t3::target().expect("no T3 to talk to");
    let client = Arc::new(crate::t3::McpClient::new(&target.origin, target.token.as_deref().expect("not signed in")));
    // T3 treats the same command from the same client as a retry: each run is a client of its own.
    let key = format!("probe-{}", crate::config::now_ms());
    let mut rig = Rig::new(49_000_000);
    rig.bridge.set_client(client.clone());
    rig.frame(json!({ "k": key, "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    let settle = |rig: &mut Rig, until: &dyn Fn(&Value) -> bool| -> Value {
        for _ in 0..60 {
            rig.advance(61_000);
            for _ in 0..40 {
                rig.bridge.tick();
                let snapshot = rig.bridge.snapshot();
                if until(&snapshot) {
                    return snapshot;
                }
            }
        }
        panic!("never settled: {}", rig.bridge.snapshot());
    };
    let snapshot = settle(&mut rig, &|s| s["models"].is_array());
    let haiku = "claude-haiku-5-5";
    assert!(snapshot["models"].as_array().unwrap().iter().any(|p| p["id"] == "claudeAgent"), "{}", snapshot["models"]);
    println!("default permissions: {}", snapshot["envs"][0]["dm"]);

    rig.frame(json!({ "k": key, "u": "ui1", "n": 0, "l": 2, "q": [
        { "s": 2, "t": "new", "m": "Reply with just OK.", "pv": "claudeAgent", "md": haiku, "ef": "low", "rm": "approval-required" },
    ] }));
    let snapshot = settle(&mut rig, &|s| s["out"].is_array());
    let id = snapshot["out"][0]["id"].as_str().unwrap_or_else(|| panic!("{}", snapshot["out"])).to_string();
    let read = |client: &crate::t3::McpClient| client.call("t3_thread_configure", json!({ "threadId": id })).unwrap();
    let started = read(&client);
    println!("started: {started}");

    let opened = settle(&mut rig, &|s| s["view"]["cfg"]["ef"] == "low");
    println!("view: {}", opened["view"]["cfg"]);

    rig.frame(json!({ "k": key, "u": "ui1", "n": 0, "l": 3, "q": [
        { "s": 3, "t": "model", "id": id, "pv": "claudeAgent", "md": haiku, "ef": "medium" },
    ] }));
    let changed = settle(&mut rig, &|s| s["view"]["cfg"]["ef"] == "medium");
    println!("changed: {}", changed["view"]["cfg"]);
    let after = read(&client);

    // Another provider, once its run has ended; the next turn runs there.
    let finished = |least: u64| -> Value {
        for _ in 0..60 {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let head = client.call("t3_thread_read", json!({ "threadId": id, "limit": 1, "maxCharsPerItem": 1 })).unwrap();
            if head["thread"]["runCount"].as_u64().unwrap_or(0) >= least && !working(str_of(&head["thread"]["status"])) {
                return head;
            }
        }
        panic!("the run never ended");
    };
    finished(1);
    let codex = changed["models"].as_array().unwrap().iter().find(|p| p["id"] == "codex").expect("codex isn't enabled").clone();
    let other = str_of(&codex["m"][0]["id"]).to_string();
    rig.frame(json!({ "k": key, "u": "ui1", "n": 0, "l": 4, "q": [
        { "s": 4, "t": "model", "id": id, "pv": "codex", "md": other },
    ] }));
    let moved = settle(&mut rig, &|s| s["view"]["cfg"]["pv"] == "codex");
    println!("moved: {}", moved["view"]["cfg"]);
    rig.frame(json!({ "k": key, "u": "ui1", "n": 0, "l": 5, "q": [{ "s": 5, "t": "send", "id": id, "m": "Reply with just OK." }] }));
    rig.bridge.tick();
    let second = finished(2);
    println!("runs: {}", second["recentRuns"]);
    println!("status: {}", second["thread"]["status"]);
    let drawn = settle(&mut rig, &|s| s["view"]["items"].as_array().is_some_and(|items| items.iter().any(|i| i["ho"].is_string())));
    let line = drawn["view"]["items"].as_array().unwrap().iter().find(|i| i["ho"].is_string()).unwrap().clone();
    println!("handoff over {}: {}", line["r"], line["ho"]);
    client.call("t3_thread_organize", json!({ "action": "archive", "threadId": id })).unwrap();

    let last = second["recentRuns"].as_array().unwrap().iter().max_by_key(|r| r["ordinal"].as_u64()).unwrap().clone();
    assert_eq!(last["providerInstanceId"], "codex", "{last}");
    assert_eq!(last["model"], other.as_str(), "{last}");
    assert_eq!(last["status"], "completed", "{last}");
    assert_eq!(line["r"], "u", "the first message on the new provider is yours");
    assert_eq!(line["ho"], format!("Claude Haiku 5.5 > {}", str_of(&codex["m"][0]["t"])));

    assert_eq!(started["modelSelection"]["model"], haiku);
    assert_eq!(started["modelSelection"]["options"], json!([{ "id": "effort", "value": "low" }]));
    assert_eq!(started["runtimeMode"], "approval-required");
    assert_eq!(after["modelSelection"]["options"], json!([{ "id": "effort", "value": "medium" }]));
    assert_eq!(after["runtimeMode"], "approval-required", "a change of model leaves the permissions alone");
}

#[test]
fn a_composer_list_keeps_what_can_be_typed_and_says_how_each_runs() {
    let cache = json!({
        "slashCommands": [
            { "name": "compact", "description": "Summarize  it | all" },
            { "name": "review", "description": "Review" },
            { "name": "has space" },
        ],
        "skills": [
            { "name": "review", "description": "Skill text", "enabled": true },
            { "name": "imagegen", "description": "Make one" },
            { "name": "off", "enabled": false },
        ],
    });
    assert_eq!(compact_picks(&cache), vec![
        json!({ "n": "compact", "d": "Summarize it || all", "c": true }),
        json!({ "n": "review", "d": "Review", "c": true, "s": true }),
        json!({ "n": "imagegen", "d": "Make one", "s": true }),
    ]);
}

#[test]
fn only_this_pcs_t3_lists_its_composers_and_a_bad_read_keeps_the_last() {
    let dir = std::env::temp_dir().join(format!("t3wow-picks-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("claudeAgent.json"), json!({ "slashCommands": [{ "name": "compact", "description": "Short" }] }).to_string()).unwrap();
    // Without the folder (T3 on another machine) nothing is listed.
    let remote = rig_with_catalog(42_000_000);
    assert!(remote.bridge.snapshot()["picks"].is_null());

    let mut rig = Rig::new(43_000_000);
    *rig.t3.capabilities.lock().unwrap() = capabilities();
    rig.bridge.set_provider_caches(Some(dir.clone()));
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();
    rig.advance(61_000);
    rig.bridge.tick();
    let expected = json!([{ "pv": "claudeAgent", "e": 1, "c": [{ "n": "compact", "d": "Short", "c": true }] }]);
    assert_eq!(rig.bridge.snapshot()["picks"], expected, "this PC only, though the Mac offers the same provider");

    std::fs::write(dir.join("claudeAgent.json"), "{ half written").unwrap();
    rig.advance(CATALOG_TTL_MS);
    rig.bridge.tick();
    assert_eq!(rig.bridge.snapshot()["picks"], expected);
    let _ = std::fs::remove_dir_all(&dir);
}

fn tool_item(position: u64, name: &str, title: Option<&str>) -> Value {
    let text = json!({ "toolName": name, "input": { "file_path": "a.lua" }, "output": { "file": { "content": format!("body of {name}") } } });
    json!({
        "position": position, "itemId": format!("i{position}"), "type": "dynamic_tool", "status": "completed", "title": title,
        "text": serde_json::to_string_pretty(&text).unwrap(), "updatedAt": "2026-11-06T10:00:30Z",
    })
}

fn said(position: u64, kind: &str, text: &str) -> Value {
    json!({ "position": position, "itemId": format!("i{position}"), "type": kind, "status": "completed", "text": text, "updatedAt": "2026-11-06T10:00:30Z" })
}

/// Thread t1 open in a game that draws rows: you asked, it thought, said something, used
/// three tools, said something else and used one more.
fn work_rig(start_ms: i64, status: &str) -> Rig {
    let mut rig = Rig::new(start_ms);
    let activity = vec![
        said(0, "user_message", "go"),
        said(1, "reasoning", "**Looking around**\nmore of it"),
        said(2, "assistant_message", "first"),
        tool_item(3, "Read", Some("Read a.lua")),
        said(4, "command_execution", "$ npm test\nok"),
        tool_item(5, "mcp__t3__unwatch", None),
        said(6, "assistant_message", "second"),
        tool_item(7, "Grep", Some("Searched a in b")),
    ];
    let messages: Vec<Value> = activity.iter().filter(|i| str_of(&i["type"]).ends_with("_message")).cloned().collect();
    rig.t3.messages.lock().unwrap().insert("t1".into(), messages);
    rig.t3.activity.lock().unwrap().insert("t1".into(), activity);
    rig.t3.set(status, "2026-11-06T10:00:40Z");
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "wk": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "t1" }] }));
    rig.bridge.tick();
    rig
}

fn texts(rig: &Rig) -> Vec<String> {
    view_items(rig).iter().map(|i| str_of(&i["x"]).to_string()).collect()
}

#[test]
fn what_an_agent_did_shows_as_rows_where_t3_shows_them_with_tools_in_a_row_folded() {
    let rig = work_rig(50_000_000, "completed");
    assert_eq!(texts(&rig), ["go", "Looking around more of it", "first", "Read 1 file, ran 1 command, and used 1 tool", "second", "Searched a in b"]);
    let items = view_items(&rig);
    assert_eq!(items[3]["w"], json!({ "p": 3, "l": 5, "k": "t" }));
    assert_eq!(items[1]["w"], json!({ "p": 1, "l": 1, "k": "b" }));
    assert!(items[2]["w"].is_null());
}

#[test]
fn a_game_that_draws_no_rows_gets_none() {
    let mut rig = Rig::new(51_000_000);
    rig.t3.messages.lock().unwrap().insert("t1".into(), conversation(&["go", "done"]));
    rig.t3.activity.lock().unwrap().insert("t1".into(), vec![said(0, "user_message", "go"), tool_item(1, "Read", None), said(2, "assistant_message", "done")]);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "dc": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "t1" }] }));
    rig.bridge.tick();
    assert_eq!(texts(&rig), ["go", "done"]);
}

#[test]
fn a_stretch_of_work_shows_only_once_it_is_finished() {
    let mut rig = work_rig(52_000_000, "running");
    assert_eq!(texts(&rig), ["go", "Looking around more of it", "first", "Read 1 file, ran 1 command, and used 1 tool", "second"]);
    // The turn ends: the last stretch has nothing after it, and shows.
    rig.t3.set("completed", "2026-11-06T10:01:00Z");
    rig.advance(5_000);
    rig.bridge.tick();
    assert_eq!(texts(&rig).last().unwrap(), "Searched a in b");
}

#[test]
fn a_tool_read_while_running_gets_its_title_once_it_ends() {
    let mut rig = Rig::new(53_000_000);
    let mut running = tool_item(1, "Read", None);
    running["status"] = json!("running");
    rig.t3.messages.lock().unwrap().insert("t1".into(), vec![said(0, "user_message", "go")]);
    rig.t3.activity.lock().unwrap().insert("t1".into(), vec![said(0, "user_message", "go"), running]);
    rig.t3.set("completed", "2026-11-06T10:00:40Z");
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "wk": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "t1" }] }));
    rig.bridge.tick();
    assert_eq!(texts(&rig), ["go", "Read 1 file"]);
    rig.t3.activity.lock().unwrap().get_mut("t1").unwrap()[1] = tool_item(1, "Read", Some("Read a.lua"));
    rig.t3.set("completed", "2026-11-06T10:01:00Z");
    rig.advance(5_000);
    rig.bridge.tick();
    assert_eq!(texts(&rig), ["go", "Read a.lua"]);
}

#[test]
fn opening_a_row_reads_all_of_it_in_one_request_for_the_update_that_answers_the_click() {
    let mut rig = work_rig(54_000_000, "completed");
    let before = rig.t3.calls_to("t3_thread_read").len();
    rig.advance(5_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 2, "l": 1, "wk": 1, "q": [{ "s": 3, "t": "detail", "id": "t1", "p": 3, "l": 5 }] }));
    rig.bridge.tick();
    let snapshot = rig.bridge.snapshot();
    assert_eq!(snapshot["ack"], 3);
    let detail = &snapshot["view"]["det"][0];
    assert_eq!(detail["l"], 5);
    let body = detail["b"].as_array().unwrap();
    assert_eq!(body[0], "Read a.lua");
    assert_eq!(body[1]["c"], "a.lua");
    assert_eq!(body[2]["c"], "body of Read");
    assert_eq!(body[3]["c"], "$ npm test\nok");
    assert_eq!(body[4], "mcp__t3__unwatch");
    let reads = rig.t3.calls_to("t3_thread_read");
    let opened: Vec<&Value> = reads[before..].iter().filter(|a| a["limit"] == 3).collect();
    assert_eq!(opened.len(), 1, "{:?}", &reads[before..]);
    assert_eq!((opened[0]["afterPosition"].clone(), opened[0]["maxCharsPerItem"].clone()), (json!(2), json!(DETAIL_CHARS)));
}

#[test]
fn an_opened_row_names_models_as_the_conversation_does() {
    let mut rig = Rig::new(54_500_000);
    *rig.t3.capabilities.lock().unwrap() = capabilities();
    let activity = vec![
        said(0, "user_message", "go"),
        said(1, "reasoning", "Hand it to model:claudeAgent/claude-opus, not model:claudeAgent/gone"),
        said(2, "assistant_message", "done"),
    ];
    let messages: Vec<Value> = activity.iter().filter(|i| str_of(&i["type"]).ends_with("_message")).cloned().collect();
    rig.t3.messages.lock().unwrap().insert("t1".into(), messages);
    rig.t3.activity.lock().unwrap().insert("t1".into(), activity);
    rig.t3.set("completed", "2026-11-06T10:00:40Z");
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "wk": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "t1" }] }));
    rig.bridge.tick();
    rig.advance(61_000);
    rig.bridge.tick();
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 2, "l": 1, "wk": 1, "q": [{ "s": 3, "t": "detail", "id": "t1", "p": 1, "l": 1 }] }));
    rig.bridge.tick();
    let snapshot = rig.bridge.snapshot();
    assert_eq!(snapshot["view"]["det"][0]["b"], json!(["Hand it to Claude Opus, not model:claudeAgent/gone"]));
    assert!(!snapshot.to_string().contains('\u{5}'), "a mark left in");
}

#[test]
fn a_question_row_shows_what_was_asked_and_what_you_answered() {
    let questions = json!([{ "question": "Merge it now?", "options": [{ "label": "Yes" }, { "label": "No" }] }]);
    let call = json!({ "toolName": "AskUserQuestion", "input": {}, "output": { "questions": questions, "answers": { "Merge it now?": "not yet" } } });
    let items = vec![
        json!({ "type": "dynamic_tool", "text": serde_json::to_string_pretty(&call).unwrap() }),
        json!({ "type": "user_input_request", "text": serde_json::to_string_pretty(&questions).unwrap() }),
    ];
    assert_eq!(detail_segments(&items), vec![json!("Merge it now?\n• Yes\n• No\n\nYou answered: not yet")]);
    assert_eq!(question_start("[\n  {\n    \"id\": \"Is this \\\"ok\\\" with you?\","), "Is this \"ok\" with you?");
    assert_eq!(question_start("[\n  {\n    \"id\": \"q1\","), "Asked a question");
}

fn in_run(mut item: Value, run: &str) -> Value {
    item["runId"] = json!(run);
    item
}

/// Thread t1 open in a game that draws rows, with `activity`, the given runs, and `status`.
fn fold_rig(start_ms: i64, status: &str, activity: Vec<Value>, runs: Value) -> Rig {
    let mut rig = Rig::new(start_ms);
    let messages: Vec<Value> = activity.iter().filter(|i| str_of(&i["type"]).ends_with("_message")).cloned().collect();
    rig.t3.messages.lock().unwrap().insert("t1".into(), messages);
    rig.t3.activity.lock().unwrap().insert("t1".into(), activity);
    *rig.t3.runs.lock().unwrap() = runs;
    rig.t3.set(status, "2026-11-06T10:03:00Z");
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "wk": 1, "q": [{ "s": 1, "t": "ui", "o": true }, { "s": 2, "t": "open", "id": "t1" }] }));
    rig.bridge.tick();
    rig
}

fn a_turn() -> Vec<Value> {
    vec![
        said(0, "user_message", "go"),
        in_run(said(1, "reasoning", "thinking it over"), "r1"),
        in_run(said(2, "assistant_message", "working on it"), "r1"),
        in_run(tool_item(3, "Read", None), "r1"),
        in_run(said(4, "assistant_message", "all done"), "r1"),
    ]
}

fn run_of(id: &str, status: &str) -> Value {
    json!({ "runId": id, "status": status, "startedAt": "2026-11-06T10:00:00Z", "completedAt": "2026-11-06T10:02:47Z" })
}

#[test]
fn a_finished_turn_folds_under_worked_for_with_its_last_message_outside() {
    let rig = fold_rig(60_000_000, "completed", a_turn(), json!([run_of("r1", "completed")]));
    let items = view_items(&rig);
    assert_eq!(texts(&rig), ["go", "Worked for 2m 47s", "thinking it over", "working on it", "Read 1 file", "all done"]);
    let name = items[1]["w"]["l"].clone();
    assert_eq!(items[1]["w"]["fold"], 1);
    assert_eq!([items[2]["f"].clone(), items[3]["f"].clone(), items[4]["f"].clone()], [name.clone(), name.clone(), name]);
    assert!(items[0]["f"].is_null() && items[5]["f"].is_null(), "{items:?}");
}

#[test]
fn a_turn_that_is_running_or_ended_badly_stays_open() {
    for status in ["running", "interrupted", "failed"] {
        let rig = fold_rig(61_000_000, "completed", a_turn(), json!([run_of("r1", status)]));
        assert!(view_items(&rig).iter().all(|i| i["f"].is_null() && i["w"]["fold"].is_null()), "{status}");
    }
}

#[test]
fn a_question_stays_out_of_the_fold_with_what_you_answered_and_when() {
    let questions = json!([{ "question": "Merge it now?", "options": [{ "label": "Yes" }] }]);
    let call = json!({ "toolName": "AskUserQuestion", "input": {}, "output": { "questions": questions, "answers": { "Merge it now?": "not yet" } } });
    let mut asked = tool_item(3, "AskUserQuestion", None);
    asked["text"] = json!(serde_json::to_string_pretty(&call).unwrap());
    let mut request = in_run(said(4, "user_input_request", ""), "r1");
    request["text"] = json!(serde_json::to_string_pretty(&questions).unwrap());
    let mut activity = a_turn();
    activity.pop();
    activity.push(in_run(asked, "r1"));
    activity.push(request);
    activity.push(in_run(said(5, "assistant_message", "all done"), "r1"));
    let mut rig = fold_rig(62_000_000, "completed", activity, json!([run_of("r1", "completed")]));
    // The answer comes from reading its tool call, once.
    rig.advance(5_000);
    rig.bridge.tick();
    let items = view_items(&rig);
    let question = items.iter().find(|i| i["x"] == "Merge it now?").unwrap();
    assert!(question["f"].is_null(), "{question}");
    assert_eq!(question["w"]["q"], json!({ "a": "not yet", "at": iso_seconds("2026-11-06T10:00:30Z") }));
    let answer_reads = rig.t3.calls_to("t3_thread_read").into_iter().filter(|a| a["limit"] == 2).count();
    assert_eq!(answer_reads, 1);
}

#[test]
fn a_question_still_waiting_for_you_is_never_taken_for_over() {
    let waiting = json!({ "type": "user_input_request", "status": "running", "text": "", "updatedAt": "2026-11-06T10:00:30Z" });
    let long_after = (iso_seconds("2026-11-06T10:00:30Z") + 3_600) * 1000;
    let found = FoundWork::from_item(&waiting, 4, long_after).unwrap();
    assert!(!found.ended);
    assert_eq!(found.label, "Input requested");
    let row = &work_rows(&[found], 0, None, &[])[0].1;
    assert_eq!((row["x"].clone(), row["w"]["k"].clone(), row["w"]["still"].clone()), (json!("Input requested"), json!("q"), json!(1)));
    assert!(row["w"]["q"].is_null(), "{row}");
    let mut tool = tool_item(5, "Read", None);
    tool["status"] = json!("running");
    assert!(FoundWork::from_item(&tool, 5, long_after).unwrap().ended, "a tool left running does count as over");
}

#[test]
fn a_turn_whose_run_is_not_known_to_have_finished_stays_open() {
    let rig = fold_rig(63_000_000, "completed", a_turn(), json!([]));
    assert!(view_items(&rig).iter().all(|i| i["f"].is_null() && i["w"]["fold"].is_null()));
}

fn at(mut item: Value, time: &str) -> Value {
    item["updatedAt"] = json!(time);
    item
}

#[test]
fn a_message_sent_into_a_turn_splits_it_at_that_message_with_each_part_folded_on_its_own() {
    let activity = vec![
        said(0, "user_message", "go"),
        in_run(said(1, "reasoning", "thinking it over"), "r1"),
        in_run(said(2, "assistant_message", "working on it"), "r1"),
        at(in_run(said(3, "user_message", "first rebase"), "r1"), "2026-11-06T10:01:00Z"),
        in_run(tool_item(4, "Read", None), "r1"),
        in_run(said(5, "assistant_message", "rebasing"), "r1"),
        at(in_run(said(6, "assistant_message", "all done"), "r1"), "2026-11-06T10:02:47Z"),
    ];
    let rig = fold_rig(67_000_000, "completed", activity, json!([run_of("r1", "completed")]));
    let items = view_items(&rig);
    assert_eq!(texts(&rig), ["go", "Worked for 2m 47s", "thinking it over", "working on it", "first rebase", "Continued for 1m 47s", "Read 1 file", "rebasing", "all done"]);
    // Each opens only its own part, and the message between them is in neither.
    assert_eq!(items[1]["w"]["l"], "f2");
    assert_eq!((items[2]["f"].clone(), items[3]["f"].clone()), (json!("f2"), json!("f2")));
    assert!(items[4]["f"].is_null(), "{}", items[4]);
    assert_eq!(items[5]["w"]["l"], "f8");
    assert_eq!((items[6]["f"].clone(), items[7]["f"].clone()), (json!("f8"), json!("f8")));
    assert!(items[8]["f"].is_null(), "{}", items[8]);
}

#[test]
fn a_continued_part_lasts_until_the_last_tool_of_its_rows() {
    let activity = vec![
        said(0, "user_message", "go"),
        in_run(said(1, "assistant_message", "working on it"), "r1"),
        at(in_run(said(2, "user_message", "one"), "r1"), "2026-11-06T10:01:00Z"),
        at(in_run(tool_item(3, "Read", None), "r1"), "2026-11-06T10:01:10Z"),
        at(in_run(tool_item(4, "Read", None), "r1"), "2026-11-06T10:01:50Z"),
        at(in_run(said(5, "user_message", "two"), "r1"), "2026-11-06T10:02:00Z"),
        at(in_run(tool_item(6, "Read", None), "r1"), "2026-11-06T10:02:10Z"),
        at(in_run(said(7, "assistant_message", "all done"), "r1"), "2026-11-06T10:02:30Z"),
    ];
    let rig = fold_rig(69_000_000, "completed", activity, json!([run_of("r1", "completed")]));
    assert_eq!(texts(&rig), ["go", "Worked for 2m 47s", "working on it", "one", "Continued for 50s", "Read 2 files", "two", "Continued for 30s", "Read 1 file", "all done"]);
}

#[test]
fn back_to_back_messages_sent_into_a_turn_split_it_once_after_the_last() {
    let activity = vec![
        said(0, "user_message", "go"),
        in_run(said(1, "assistant_message", "working on it"), "r1"),
        at(in_run(said(2, "user_message", "one"), "r1"), "2026-11-06T10:00:40Z"),
        at(in_run(said(3, "user_message", "two"), "r1"), "2026-11-06T10:01:00Z"),
        in_run(tool_item(4, "Read", None), "r1"),
        at(in_run(said(5, "assistant_message", "all done"), "r1"), "2026-11-06T10:02:00Z"),
    ];
    let rig = fold_rig(68_000_000, "completed", activity, json!([run_of("r1", "completed")]));
    assert_eq!(texts(&rig), ["go", "Worked for 2m 47s", "working on it", "one", "two", "Continued for 1m", "Read 1 file", "all done"]);
}

#[test]
fn durations_read_as_t3_says_them() {
    assert_eq!(format_duration(22), "22s");
    assert_eq!(format_duration(167), "2m 47s");
    assert_eq!(format_duration(120), "2m");
    assert_eq!(format_duration(3605), "1h 5s");
}

#[test]
fn a_row_covers_no_more_than_the_one_read_that_opens_it_can_return() {
    let work: Vec<FoundWork> = (1..=130).map(|position| FoundWork { position, kind: WorkKind::Read, label: String::new(), ended: true, answer: None, answered: false, at: 0 }).collect();
    let spans: Vec<Value> = work_rows(&work, 0, None, &[]).into_iter().map(|(_, row)| row["w"].clone()).collect();
    assert_eq!(spans, [json!({ "p": 1, "l": 100, "k": "r" }), json!({ "p": 101, "l": 130, "k": "r" })]);
}

#[test]
fn a_row_of_one_kind_of_work_has_its_icon_and_a_mix_has_the_hammer() {
    let row_of = |kinds: &[WorkKind]| {
        let work: Vec<FoundWork> = kinds.iter().enumerate().map(|(i, kind)| FoundWork { position: i as u64 + 1, kind: *kind, label: String::new(), ended: true, answer: None, answered: false, at: 0 }).collect();
        work_rows(&work, 0, None, &[])[0].1.clone()
    };
    assert_eq!(row_of(&[WorkKind::Command, WorkKind::Command])["w"]["k"], "c");
    assert_eq!(row_of(&[WorkKind::Read])["w"]["k"], "r");
    assert_eq!(row_of(&[WorkKind::Edit])["w"]["k"], "e");
    assert_eq!(row_of(&[WorkKind::Search])["w"]["k"], "s");
    assert_eq!(row_of(&[WorkKind::Web])["w"]["k"], "w");
    assert_eq!(row_of(&[WorkKind::Read, WorkKind::Command])["w"]["k"], "t");
    assert_eq!(row_of(&[WorkKind::Search, WorkKind::Web])["x"], "Ran 2 searches");
}

#[test]
fn the_turn_still_going_is_headed_by_when_it_started() {
    let rig = fold_rig(64_000_000, "running", a_turn(), json!([run_of("r1", "running"), run_of("r0", "completed")]));
    let items = view_items(&rig);
    assert_eq!(items[0]["x"], "go");
    assert_eq!(items[1]["w"]["live"], iso_seconds("2026-11-06T10:00:00Z"));
    assert_eq!(items[1]["w"]["still"], 1);
    // Not once it has ended.
    let done = fold_rig(65_000_000, "completed", a_turn(), json!([run_of("r1", "completed")]));
    assert!(view_items(&done).iter().all(|i| i["w"]["live"].is_null()));
}

#[test]
fn a_turn_with_nothing_to_show_yet_still_says_it_is_working() {
    let activity = vec![said(0, "user_message", "go")];
    let rig = fold_rig(66_000_000, "running", activity, json!([run_of("r1", "running")]));
    let items = view_items(&rig);
    assert_eq!(items.len(), 2, "{items:?}");
    assert!(items[1]["w"]["live"].is_number());
}

#[test]
fn a_long_row_is_cut_into_code_boxes_and_says_when_it_stops() {
    let text = "line of output\n".repeat(3_000);
    let body = detail_segments(&[json!({ "type": "command_execution", "text": text })]);
    assert!(body.len() > 5, "{}", body.len());
    let shown: usize = body.iter().filter_map(|s| s["c"].as_str()).map(|c| c.chars().count()).sum();
    assert_eq!(shown, DETAIL_CHARS);
    assert_eq!(body.last().unwrap(), "… the rest is in T3");
    assert!(body.iter().filter_map(|s| s["c"].as_str()).all(|c| c.chars().count() <= DETAIL_PIECE + 500));
}

#[test]
fn a_thread_only_getting_newer_spends_no_update_until_there_is_something_to_show() {
    let mut rig = work_rig(55_000_000, "running");
    rig.advance(20_000);
    rig.bridge.tick();
    let settled = rig.publishes();
    rig.t3.set("running", "2026-11-06T10:05:00Z");
    rig.advance(20_000);
    rig.bridge.tick();
    assert_eq!(rig.publishes(), settled, "a new time alone spent an update");
    rig.t3.set("completed", "2026-11-06T10:06:00Z");
    rig.advance(20_000);
    rig.bridge.tick();
    assert!(rig.publishes() > settled, "a status change didn't reach the game");
}

#[test]
fn a_read_only_sign_in_shows_on_the_game_s_top_line_until_one_can_change_threads() {
    let mut rig = Rig::new(30_000_000);
    rig.frame(json!({ "k": "c1", "u": "ui1", "n": 0, "l": 1, "q": [{ "s": 1, "t": "ui", "o": true }] }));
    rig.bridge.tick();
    assert!(rig.bridge.snapshot()["auth"].is_null());
    assert!(!rig.bridge.status().sign_in_read_only);
    let latest = |rig: &Rig| {
        let written = rig.written.lock().unwrap();
        written.slots.iter().max_by_key(|(n, _)| **n).map(|(_, lua)| lua.clone()).unwrap_or_default()
    };

    // T3 granted Read only: the game says so where it says a sign-in was refused, right away.
    rig.bridge.set_sign_in_read_only(true);
    rig.advance(2000);
    rig.bridge.tick();
    assert_eq!(rig.bridge.snapshot()["auth"], READ_ONLY_LINE);
    assert!(rig.bridge.status().sign_in_read_only, "the tray's line says it too");
    assert!(latest(&rig).contains("Read-only T3 sign-in"), "{}", latest(&rig));

    // A refused sign-in is the bigger problem, and says what to do about it.
    rig.bridge.auth_error = Some("T3 refused the sign-in (401). Sign in again from the tray menu.".into());
    assert_eq!(rig.bridge.snapshot()["auth"], "T3 refused the sign-in (401). Sign in again from the tray menu.");
    rig.bridge.auth_error = None;

    // Signed in again with Auto: the line goes.
    rig.bridge.set_sign_in_read_only(false);
    rig.advance(2000);
    rig.bridge.tick();
    assert!(rig.bridge.snapshot()["auth"].is_null());
    assert!(!latest(&rig).contains("Read-only T3 sign-in"), "{}", latest(&rig));
}

mod live;
