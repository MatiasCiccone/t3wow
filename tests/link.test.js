// The addon's link (Link.lua) in the simulated game: acks, slot budgets and backoff.
import assert from "node:assert/strict";
import { test } from "node:test";
import { runGame } from "./game.js";

test("an update written for an earlier UI session doesn't drop this session's commands", () => {
  runGame(`
Link.Init()
local session = Link.state.uiSession
Link.Send({ t = "send", id = "t1", m = "after the crash" })
assert(Link.Pending() == 2, "queued " .. Link.Pending())
-- A slot still holds the previous session's update, whose ack is past our numbers.
inboxes.T3WoW_S001 = { k = "c1", u = "an earlier session", ack = 9, now = time() }
tick(1)
assert(#loads == 1, "loads " .. #loads)
assert(Link.Pending() == 2, "kept " .. Link.Pending())
-- This session's update acks them; an update without u (an older companion) does too.
inboxes.T3WoW_S002 = { k = "c1", u = session, ack = 1, now = time() }
inboxes.T3WoW_S003 = { k = "c1", ack = 2, now = time() }
tick(30)
assert(Link.Pending() == 0, "left " .. Link.Pending())
return "ok"
`);
});

test("the last slots are announced, and loading slot 200 marks the session out of updates", () => {
  runGame(`
Link.Init()
Link.state.nextSlot = 189
inboxes.T3WoW_S189 = { k = "c1", ack = 1, now = time() }
tick(1)
assert(not Link.SlotsLow(), "low after 189")
Link.state.nextSlot = 190
tick(70)
assert(Link.SlotsLow(), "not low after 190")
assert(printed[1] and printed[1]:find("190 of 200"), tostring(printed[1]))
Link.state.nextSlot = 200
Link.state.pollAt = 0
tick(1)
assert(Link.Exhausted(), "not exhausted after loading slot 200")
assert(loads[#loads] == "T3WoW_S200", loads[#loads])
assert(printed[2] and printed[2]:find("all 200"), tostring(printed[2]))
local count = #loads
tick(600)
assert(#loads == count, "loaded past slot 200")
return "ok"
`);
});

test("the companion running out of signal files turns signals off, as does watching past the last file", () => {
  runGame(`
real[PROBE_VALID] = true
Link.Init()
inboxes.T3WoW_S001 = { k = "c1", u = Link.state.uiSession, ack = 1, ns = 3990, nt = time() }
tick(6)
assert(Link.Status().signals == "on", Link.Status().signals)
-- Ten files left: about four minutes later the game is past the last one.
tick(300)
assert(Link.Status().signals == "off", "after 300 s: " .. Link.Status().signals)
assert(Link.Status().signalCheck:find("signal files"), tostring(Link.Status().signalCheck))
return "ok"
`);
  runGame(`
real[PROBE_VALID] = true
Link.Init()
inboxes.T3WoW_S001 = { k = "c1", u = "an earlier session", ack = 1, ns = 50, nt = time(), sigoff = "out" }
tick(6)
assert(Link.Status().signals == "on", "an earlier session's sigoff counts: " .. Link.Status().signals)
inboxes.T3WoW_S002 = { k = "c1", u = Link.state.uiSession, ack = 1, ns = 52, nt = time(), sigoff = "out" }
for n = 49, 51 do real[sigPath(n)] = true end
tick(5)
assert(loads[2] == "T3WoW_S002", tostring(loads[2]))
assert(Link.Status().signals == "off", Link.Status().signals)
return "ok"
`);
});

test("with no companion running, slot loads back off instead of using up the session", () => {
  runGame(`
Link.Init()
tick(600)
assert(#loads <= 16, "loads in 10 minutes: " .. #loads)
assert(Link.Stalled(), "not stalled")
-- The companion starts: the next load brings its answer, and loads for a command are quick again.
local function answer(ack)
  for n = #loads + 1, #loads + 3 do inboxes[string.format("T3WoW_S%03d", n)] = { k = "c1", u = Link.state.uiSession, ack = ack, now = time() } end
end
answer(1)
tick(61)
assert(Link.Pending() == 0 and not Link.Stalled(), "still waiting")
Link.Send({ t = "send", id = "t1", m = "hi" })
local count = #loads
tick(4)
assert(#loads - count == 1, "after an answer: " .. (#loads - count) .. " loads in 4 s")
return "ok"
`);
});

test("a waiting command's nudges back off and stop once the companion counts as not answering", () => {
  runGame(`
real[PROBE_VALID] = true
Link.Init()
inboxes.T3WoW_S001 = { k = "c1", u = Link.state.uiSession, ack = 1, ns = 100, nt = time() }
tick(6)
assert(Link.Status().signals == "on", Link.Status().signals)
local count = #loads
Link.Send({ t = "send", id = "t1", m = "anyone there?" })
tick(300)
assert(#loads - count <= 3, "nudges in 5 minutes: " .. (#loads - count))
assert(Link.Stalled(), "not stalled")
return "ok"
`);
});

test("a fresh install doesn't replay the companion's older events as alerts", () => {
  runGame(`
Link.Init()
inboxes.T3WoW_S001 = { k = "c1", ack = 1, now = time(), events = {
  { n = (time() - 3600) * 1000, e = "", id = "t1", t = "An hour ago", s = "completed" },
  { n = (time() - 5) * 1000, e = "", id = "t2", t = "Just now", s = "completed" },
} }
tick(1)
assert(#alerts == 1 and alerts[1].t == "Just now", "alerts: " .. #alerts)
return "ok"
`, { db: `{ client = "c1", seq = 0, queue = {} }` });
});

test("a test font that never loads turns signals off after a dozen tries", () => {
  runGame(`
Link.Init()
tick(120)
assert(Link.Status().signals == "off", Link.Status().signals)
assert(Link.Status().signalCheck:find("never loaded"), Link.Status().signalCheck)
return "ok"
`);
});

test("past the font check budget, signals give way to the timer", () => {
  runGame(`
real[PROBE_VALID] = true
Link.Init()
inboxes.T3WoW_S001 = { k = "c1", u = Link.state.uiSession, ack = 1, ns = 100, nt = time() }
tick(6)
assert(Link.Status().signals == "on", Link.Status().signals)
Link.state.fontStrings = 19999
tick(5)
assert(Link.Status().signals == "off", Link.Status().signals)
assert(Link.Status().signalCheck:find("font checks"), Link.Status().signalCheck)
return "ok"
`);
});
