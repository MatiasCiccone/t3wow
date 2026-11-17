// What a /reload does for a thread that already waits on you: one silent notification,
// unless one for this wait has already been on screen to its end or been clicked.
import { test } from "node:test";
import { runGame } from "./game.js";

const HELPERS = `
SOUNDKIT = { TELL_MESSAGE = 1 }
played = 0
function PlaySound() played = played + 1 end
MAC = { id = "mac", name = "Mac", ok = true }
BOX = { id = "box", name = "Box", ok = true }
function thread(id, status, extra)
  local t = { e = 1, id = id, t = "Thread " .. id, s = status, set = false, at = time(), p = "" }
  for k, v in pairs(extra or {}) do t[k] = v end
  return t
end
function update(threads, fields)
  local inbox = { k = "c1", u = Link.Session(), lid = "aaaa", ack = 0, now = time(), envs = { MAC, BOX }, threads = threads,
    projects = {}, notes = {}, events = {} }
  for k, v in pairs(fields or {}) do inbox[k] = v end
  Link.state.pollAt = 0
  inboxes[string.format("T3WoW_S%03d", Link.state.nextSlot)] = inbox
  tick(0.5)
end
function shownToasts()
  local out = {}
  for _, f in ipairs(frames) do
    if rawget(f, "head") and f.shown then out[#out + 1] = f end
  end
  return out
end
function count() return #shownToasts() end
function top() return shownToasts()[1] end
function said(pattern)
  local n = 0
  for _, line in ipairs(printed) do if line:find(pattern) then n = n + 1 end end
  return n
end
function seen(key) return T3WoWDB.seenWaiting ~= nil and T3WoWDB.seenWaiting[key] ~= nil end
Link.Init()
`;

const SAVED = (entries) => `{ client = "c1", seq = 0, queue = {}, login = "aaaa", seenWaiting = { ${entries} } }`;

test("a thread that already waits gets one notification after a reload, without a sound or another chat line", () => {
  runGame(`${HELPERS}
update({ thread("t1", "waiting") })
assert(count() == 1, "no notification: " .. count())
assert(top().title.text == "Thread t1", top().title.text)
assert(played == 0, "it made a sound")
assert(said("Awaiting input") == 0, "it wrote a chat line")
assert(said("connected to T3") == 1)
-- The next update of the same session doesn't show it again.
update({ thread("t1", "waiting") })
assert(count() == 1, "shown twice")
return "ok"
`, { ui: true });
});

test("the same goes for a thread waiting for an approval", () => {
  runGame(`${HELPERS}
update({ thread("t1", "approval") })
assert(count() == 1, "no notification")
return "ok"
`, { ui: true });
});

test("a thread waiting on a card only T3 can answer (a secret) notifies like an approval", () => {
  runGame(`${HELPERS}
update({ thread("t1", "card") })
assert(count() == 1, "no notification")
assert(top().head.text:find("Answer in T3", 1, true), top().head.text)
assert(top().sub.text:find("click to open$"), top().sub.text)
return "ok"
`, { ui: true });
  // With "Clicking an approval opens T3" on, a click opens it in T3.
  runGame(`${HELPERS}
T3WoWDB.settings = T3WoWDB.settings or {}
T3WoWDB.settings.approvalOpensT3 = true
update({ thread("t2", "card") })
local toast = top()
assert(toast.sub.text:find("click to open in T3", 1, true), toast.sub.text)
toast.scripts.OnClick(toast, "LeftButton")
local last = Link.Queue()[#Link.Queue()]
assert(last and last.t == "t3" and last.id == "t2", "not opened in T3: " .. tostring(last and last.t))
return "ok"
`, { ui: true });
});

test("a notification that ran its time isn't shown again by the next reload", () => {
  runGame(`${HELPERS}
update({ thread("t1", "waiting") })
assert(not seen("mac:t1"), "counted as seen while still on screen")
tick(10)
assert(count() == 0, "still on screen")
assert(seen("mac:t1"), "not remembered once it ran its time")
return "ok"
`, { ui: true });
  runGame(`${HELPERS}
update({ thread("t1", "waiting") })
assert(count() == 0, "shown again after it ran its time")
return "ok"
`, { ui: true, db: SAVED(`["mac:t1"] = "mac"`) });
});

test("clicking a notification counts as seen, left or right", () => {
  for (const mouse of ["LeftButton", "RightButton"]) {
    runGame(`${HELPERS}
update({ thread("t1", "waiting") })
local toast = top()
toast.scripts.OnClick(toast, "${mouse}")
assert(seen("mac:t1"), "not remembered after a click")
return "ok"
`, { ui: true });
  }
});

test("a reload during a notification brings it back", () => {
  runGame(`${HELPERS}
-- The alert for the wait, with its sound, then a reload before it ran its time.
update({ thread("t1", "waiting") }, { events = { { n = time() * 1000, e = "mac", id = "t1", t = "Thread t1", s = "waiting" } } })
assert(count() == 1, "shown " .. count() .. " times: the alert and the catch-up should be one")
assert(played == 1, "the alert's sound")
assert(not seen("mac:t1"), "counted as seen before it ran its time")
return "ok"
`, { ui: true });
  runGame(`${HELPERS}
update({ thread("t1", "waiting") })
assert(count() == 1, "not brought back")
return "ok"
`, { ui: true, db: `{ client = "c1", seq = 0, queue = {}, login = "aaaa", lastEvent = 1794000005000 }` });
});

test("an alert skipped because you were reading the thread doesn't count as seen", () => {
  runGame(`${HELPERS}
local waiting = thread("t1", "running")
update({ waiting })
tick(10)
UI.Toggle()
UI.Select(waiting)
waiting.s = "waiting"
update({ waiting }, { events = { { n = time() * 1000 + 1, e = "mac", id = "t1", t = "Thread t1", s = "waiting" } } })
assert(count() == 0, "shown while reading it")
assert(not seen("mac:t1"), "counted as seen")
return "ok"
`, { ui: true });
});

test("no notification for the thread you're reading when the update arrives", () => {
  runGame(`${HELPERS}
-- Box's list comes first, so the game knows the machines and Mac still has to catch up.
local waiting = thread("t1", "waiting")
update({ thread("b1", "completed", { e = 2 }) })
UI.Toggle()
UI.Select(waiting)
update({ waiting })
assert(count() == 0, "shown while reading it")
assert(not seen("mac:t1"), "counted as seen")
return "ok"
`, { ui: true });
});

test("a machine that first lists its threads later in the session catches up then", () => {
  runGame(`${HELPERS}
update({ thread("t1", "completed") })
assert(count() == 0)
-- Mac was caught up by the update above; this one's a new wait, which gets its own alert.
update({ thread("t1", "waiting"), thread("b1", "waiting", { e = 2 }) })
assert(count() == 1, "expected only the new machine's: " .. count())
assert(top().title.text == "Thread b1", top().title.text)
return "ok"
`, { ui: true });
});

test("an update left in a slot by an earlier UI session doesn't count", () => {
  runGame(`${HELPERS}
update({ thread("t1", "waiting") }, { u = "an earlier session" })
assert(count() == 0, "a stale update notified")
update({ thread("t1", "completed") })
assert(count() == 0, "the thread has been answered since")
return "ok"
`, { ui: true });
});

test("settled and snoozed threads, and the settings that turn these off, stay quiet", () => {
  runGame(`${HELPERS}
update({ thread("t1", "waiting", { set = true }), thread("t2", "waiting", { sn = true }), thread("t3", "running") })
assert(count() == 0, "shown " .. count())
return "ok"
`, { ui: true });
  for (const off of ["toasts", "onWaiting"]) {
    runGame(`${HELPERS}
update({ thread("t1", "waiting") })
assert(count() == 0, "shown with ${off} off")
assert(not seen("mac:t1"), "counted as seen")
return "ok"
`, { ui: true, db: `{ client = "c1", seq = 0, queue = {}, login = "aaaa", settings = { ${off} = false } }` });
  }
});

test("more threads than the notifications allowed: the newest show, the others wait for the next reload", () => {
  runGame(`${HELPERS}
local list = {}
for i = 1, 6 do list[i] = thread("t" .. i, "waiting") end
update(list)
assert(count() == 4, "shown " .. count())
tick(10)
assert(seen("mac:t1") and seen("mac:t4"), "the newest aren't remembered")
assert(not seen("mac:t5") and not seen("mac:t6"), "ones pushed off count as seen")
return "ok"
`, { ui: true });
});

test("a thread that stopped waiting is forgotten, so its next wait notifies", () => {
  runGame(`${HELPERS}
update({ thread("t1", "running"), thread("t2", "waiting") })
assert(not seen("mac:t1"), "kept after it stopped waiting")
assert(seen("mac:t2"), "dropped while it still waits")
assert(seen("box:b1"), "dropped for a machine that hasn't listed its threads")
return "ok"
`, { ui: true, db: SAVED(`["mac:t1"] = "mac", ["mac:t2"] = "mac", ["box:b1"] = "box"`) });
});

test("a notification still up when its wait ends doesn't count for the next wait", () => {
  runGame(`${HELPERS}
update({ thread("t1", "waiting") })
assert(count() == 1)
update({ thread("t1", "running") })
assert(count() == 1, "gone before it ran its time")
tick(10)
assert(count() == 0)
assert(not seen("mac:t1"), "the ended wait's notification counted")
return "ok"
`, { ui: true });
});

test("moving to another T3 drops what was seen", () => {
  runGame(`${HELPERS}
update({ thread("t1", "running") })
assert(seen("box:b1"), "dropped too early")
update({ thread("t1", "waiting") }, { lid = "bbbb" })
assert(T3WoWDB.login == "bbbb")
assert(not seen("box:b1"), "the previous T3's kept")
assert(count() == 1, "the new T3 doesn't catch up")
return "ok"
`, { ui: true, db: SAVED(`["box:b1"] = "box"`) });
});

test("opening a thread another way dismisses its notifications, as clicking one does", () => {
  // From the list, and from chat or a link (UI.OpenThread). Only that thread on that machine:
  // the same thread id on another machine and other threads keep theirs.
  for (const open of ["UI.Toggle() UI.Select(t1)", `UI.OpenThread("t1", "mac")`]) {
    runGame(`${HELPERS}
t1 = thread("t1", "waiting")
update({ t1, thread("t2", "waiting"), thread("t1", "waiting", { e = 2 }) })
assert(count() == 3, "notifications: " .. count())
${open}
assert(count() == 2, "still on screen: " .. count())
for _, toast in ipairs(shownToasts()) do
  assert(not (toast.threadId == "t1" and toast.envId == "mac"), "the opened thread's notification stayed")
end
assert(seen("mac:t1"), "not counted as seen")
assert(not seen("box:t1") and not seen("mac:t2"), "another thread counted as seen")
return "ok"
`, { ui: true });
  }
});
