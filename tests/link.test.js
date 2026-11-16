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

test("with the key Key.lua holds, every frame is signed, and the key leaves the namespace", () => {
  const key = "5a".repeat(32);
  runGame(`
assert(ns.key == nil, "the key stays in the namespace")
local drawnWith = {}
local encode = ns.Codec.Encode
ns.Codec.Encode = function(seq, payload, k)
  local cells = encode(seq, payload, k)
  drawnWith[#drawnWith + 1] = { payload = payload, cells = cells }
  return cells
end
Link.Init()
assert(Link.Status().signed, "not signed")
local last = drawnWith[#drawnWith]
assert(last and last.payload:find('"t":"hello"', 1, true), "hello not drawn")
-- The drawn bytes: version 3, then (after seq and length) the signature, made with the key
-- over the version byte and the payload, then the payload.
local bytes, acc, bits = {}, 0, 0
for _, cell in ipairs(last.cells) do
  acc, bits = acc * 8 + cell, bits + 3
  if bits >= 8 then
    bits = bits - 8
    bytes[#bytes + 1] = math.floor(acc / 2 ^ bits)
    acc = acc % 2 ^ bits
  end
end
local drawn = string.char(table.unpack(bytes))
assert(drawn:sub(1, 3) == "T3" .. string.char(3), "not version 3")
local tag = ns.Codec.HmacSha256(ns.Codec.KeyFromHex("${key}"), string.char(3) .. last.payload):sub(1, 8)
assert(drawn:sub(8, 15) == tag, "wrong signature")
assert(drawn:sub(16, 15 + #last.payload) == last.payload, "payload")
return "ok"
`, { key });
  // Without Key.lua (a game started before the companion wrote it): drawn as before signing.
  runGame(`
Link.Init()
assert(not Link.Status().signed, "signed without a key")
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
-- Nor is its wait: this session's update comes on the timer.
local slot = string.format("T3WoW_S%03d", Link.Status().nextSlot)
inboxes[slot] = { k = "c1", u = Link.state.uiSession, ack = 1, ns = 52, nt = time(), sigoff = "out" }
local count = #loads
tick(30)
assert(#loads > count and loads[count + 1] == slot, tostring(loads[count + 1]) .. " not " .. slot)
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
assert(Link.Status().signals == "off", Link.Status().signals .. " | " .. table.concat(Link.Trace(), " | "))
assert(Link.Status().signalCheck:find("never loaded"), Link.Status().signalCheck)
return "ok"
`);
});

test("on a Mac a test font that never loads says to reinstall from the menu bar icon, not a tray", () => {
  runGame(`
Link.Init()
tick(120)
assert(Link.Status().signalCheck:find("choose Reinstall addon from T3 WoW's menu bar icon, then restart WoW", 1, true), Link.Status().signalCheck)
assert(not Link.Status().signalCheck:find("tray"), Link.Status().signalCheck)
return "ok"
`, { before: "function IsMacClient() return true end" });
  runGame(`
Link.Init()
tick(120)
assert(Link.Status().signalCheck:find("Reinstall addon from the T3 WoW tray menu, then restart WoW", 1, true), Link.Status().signalCheck)
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
assert(Link.Status().signals == "off", Link.Status().signals .. " | " .. table.concat(Link.Trace(), " | "))
assert(Link.Status().signalCheck:find("font checks"), Link.Status().signalCheck)
return "ok"
`);
});

test("the queue, client key and command count stay off the global table and keep across logout and /reload", () => {
  const saved = runGame(`
Link.Init()
assert(T3WoWDB.client == nil and T3WoWDB.seq == nil and T3WoWDB.queue == nil, "still on the global")
assert(T3WoWDB.settings.sound == false, "other saved settings stay")
-- Another addon adding to the global table reaches nothing.
T3WoWDB.queue = { { s = 99, t = "new", m = "not ours" } }
Link.Send({ t = "send", id = "t1", m = "ours" })
assert(Link.Pending() == 3, "queued " .. Link.Pending())
fire("PLAYER_LOGOUT")
assert(T3WoWDB.client == "c7" and T3WoWDB.seq == 6 and #T3WoWDB.queue == 3, "not written back")
local parts = {}
for _, c in ipairs(T3WoWDB.queue) do parts[#parts + 1] = string.format("{ s = %d, t = %q }", c.s, c.t) end
return string.format("{ client = %q, seq = %d, queue = { %s } }", T3WoWDB.client, T3WoWDB.seq, table.concat(parts, ", "))
`, { db: `{ client = "c7", seq = 4, queue = { { s = 4, t = "send", id = "t1", m = "before" } }, settings = { sound = false } }` });
  assert.match(saved, /s = 4, t = "send".*s = 5, t = "hello".*s = 6, t = "send"/);

  // The next UI session picks up where that one left off.
  runGame(`
Link.Init()
local q = Link.Queue()
assert(#q == 4 and q[4].t == "hello" and q[4].s == 7, "after reload: " .. #q .. " queued, last #" .. tostring(q[#q].s))
assert(T3WoWDB.queue == nil, "back on the global")
inboxes.T3WoW_S001 = { k = "c7", u = Link.state.uiSession, ack = 7, now = time() }
tick(1)
assert(Link.Pending() == 0, "the client key didn't carry over: " .. Link.Pending())
return "ok"
`, { db: saved });
});

// Loads `inbox` as the next slot right away; the frames the strip draws are kept in `drawn`.
const KEEPING = `
drawn = {}
local json = ns.Codec.Json
ns.Codec.Json = function(value)
  local out = json(value)
  drawn[#drawn + 1] = out
  return out
end
function load(inbox)
  Link.state.pollAt = 0
  inboxes[string.format("T3WoW_S%03d", Link.state.nextSlot)] = inbox
  tick(0.5)
end
function lastFrame() return drawn[#drawn] or "" end
`;

test("an update that leaves out parts keeps ours when they're the ones named, and the strip says which update we're in step with", () => {
  runGame(`${KEEPING}
Link.Init()
local u = Link.Session()
load({ k = "c1", u = u, ack = 1, now = time(), g = 7, hx = { view = "v1", threads = "l1" },
  view = { id = "t1", items = { { x = "hi" } } }, threads = { { id = "t1", s = "running" } }, events = {} })
assert(Link.state.inStep == 7, tostring(Link.state.inStep))
assert(lastFrame():find('"g":7', 1, true), lastFrame())

-- Left out, and ours is the one named: kept.
load({ k = "c1", u = u, ack = 1, now = time(), g = 8, hx = { view = "v1", threads = "l1" }, same = { "view", "threads" } })
local inbox = Link.state.inbox
assert(inbox.view and inbox.view.items[1].x == "hi", "the open thread wasn't kept")
assert(inbox.threads[1].s == "running", "the list wasn't kept")
assert(Link.state.inStep == 8, tostring(Link.state.inStep))

-- One sent, one left out.
load({ k = "c1", u = u, ack = 1, now = time(), g = 9, hx = { view = "v1", threads = "l2" },
  threads = { { id = "t1", s = "completed" } }, same = { "view" } })
assert(Link.state.inbox.view.items[1].x == "hi" and Link.state.inbox.threads[1].s == "completed")
assert(Link.state.inStep == 9 and lastFrame():find('"g":9', 1, true), lastFrame())

-- Left out, but ours isn't the one named: ours stays on screen, and the strip says 0 so
-- the companion sends them all again.
load({ k = "c1", u = u, ack = 1, now = time(), g = 10, hx = { view = "v2", threads = "l2" }, same = { "view", "threads" } })
assert(Link.state.inbox.view.items[1].x == "hi", "the open thread went")
assert(Link.state.inbox.hx.view == "v1", "claims " .. tostring(Link.state.inbox.hx.view))
assert(Link.state.inStep == 0, tostring(Link.state.inStep))
assert(lastFrame():find('"g":0', 1, true), lastFrame())
load({ k = "c1", u = u, ack = 1, now = time(), g = 11, hx = { view = "v2", threads = "l2" },
  view = { id = "t1", items = { { x = "new" } } }, threads = { { id = "t1", s = "completed" } } })
assert(Link.state.inbox.view.items[1].x == "new" and Link.state.inStep == 11)

-- An earlier session's slot: shown (its parts kept if they're ours), but not in step.
load({ k = "c1", u = "an earlier session", ack = 1, now = time(), g = 12, hx = { view = "v2", threads = "l2" }, same = { "view", "threads" } })
assert(Link.state.inbox.view.items[1].x == "new")
assert(Link.state.inStep == 0, tostring(Link.state.inStep))
return "ok"
`);
});

test("after a Reload UI the first slot has everything, and a slot left out of it is filled from it", () => {
  runGame(`${KEEPING}
Link.Init()
-- Slot 1 always carries every part; slot 4 left them out for the session before.
inboxes.T3WoW_S001 = { k = "c1", u = "the session before", ack = 1, now = time(), g = 20, hx = { view = "v1", threads = "l1" },
  view = { id = "t1", items = { { x = "hi" } } }, threads = { { id = "t1" } } }
tick(1)
assert(Link.state.inbox.view.items[1].x == "hi", "slot 1 didn't bring the open thread")
assert(Link.state.inStep == 0, tostring(Link.state.inStep))
Link.state.nextSlot = 4
load({ k = "c1", u = "the session before", ack = 1, now = time(), g = 21, hx = { view = "v1", threads = "l1" }, same = { "view", "threads" } })
assert(Link.state.inbox.view.items[1].x == "hi" and #Link.state.inbox.threads == 1)
return "ok"
`);
});
