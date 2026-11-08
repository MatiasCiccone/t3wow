import assert from "node:assert/strict";
import { test } from "node:test";
import { runGame } from "./game.js";

test("a slow-loading signal font after five quiet minutes still brings the update in", () => {
  const result = runGame(`
real["Interface\\\\AddOns\\\\T3WoW\\\\ctl\\\\probe\\\\valid.ttf"] = true
Link.Init()
-- Fonts check out, then the first polled slot names the wait: file 100, starting now.
inboxes.T3WoW_S001 = { k = "c1", ack = 1, ns = 100, nt = time() }
tick(6)
assert(Link.Status().signals == "on", "signals " .. tostring(Link.Status().signals))
assert(#loads == 1, "polled loads " .. #loads)

-- Five quiet minutes: no slot is loaded, the watched file moves along with the clock.
tick(300)
assert(#loads == 1, "loaded while idle: " .. #loads)
local watched = Link.Status().nextSignal
assert(watched == 100 + math.floor((time() - (EPOCH + 6)) / 25), "watching " .. watched)

-- The companion publishes: the file it computed and its neighbours become real.
for n = watched - 1, watched + 1 do real[string.format("Interface\\\\AddOns\\\\T3WoW\\\\sig\\\\%04d.ttf", n)] = true end
inboxes.T3WoW_S002 = { k = "c1", ack = 1, ns = watched + 2, nt = time(), events = { { n = time() * 1000, e = "", id = "t1", t = "Done", s = "completed" } } }
local before = clock
for _ = 1, 40 do
  tick(0.25)
  if #alerts > 0 then break end
end
assert(#alerts == 1, "no alert after " .. (clock - before) .. "s")
assert(loads[2] == "T3WoW_S002", tostring(loads[2]))
assert(Link.Status().recentAlerts[1].via == "a signal", Link.Status().recentAlerts[1].via)
local trace = table.concat(Link.Trace(), " | ")
assert(trace:find("signal %d+ noticed"), trace)
assert(trace:find("load T3WoW_S002 via a signal"), trace)
return string.format("%.2f", clock - before)
`);
  // Load delay 1.5 s plus at most one check interval.
  assert.ok(Number(result) <= 2.5, `took ${result}s`);
});

test("a slow first font load right after the game starts is retried, not taken as 'signals off'", () => {
  runGame(`
LOAD_DELAY = 6 -- longer than the addon's 4 s check: a cold start
real["Interface\\\\AddOns\\\\T3WoW\\\\ctl\\\\probe\\\\valid.ttf"] = true
Link.Init()
tick(5)
assert(Link.Status().signals == "checking", "after 5 s: " .. Link.Status().signals)
tick(10)
assert(Link.Status().signals == "on", "after 15 s: " .. Link.Status().signals .. " | " .. table.concat(Link.Trace(), " | "))
local trace = table.concat(Link.Trace(), " | ")
assert(trace:find("retrying"), trace)
return "ok"
`);
});

test("a slot left from another UI session doesn't set the wait; the next update for this session does", () => {
  runGame(`
real[PROBE_VALID] = true
Link.Init()
-- Left over from a game session hours ago: that companion process had counted far ahead.
inboxes.T3WoW_S001 = { k = "c1", u = "earlier", ack = 1, ns = 191, nt = time() - 3 * 3600 }
tick(6)
assert(loads[1] == "T3WoW_S001", tostring(loads[1]))
assert(Link.Status().signals == "on", "signals " .. tostring(Link.Status().signals))
assert(Link.Status().nextSignal == nil, "watching " .. tostring(Link.Status().nextSignal))
assert(table.concat(Link.Trace(), " | "):find("another UI session.s, ignored"), table.concat(Link.Trace(), " | "))

-- Still on the timer, so the companion's first update for this session comes in and sets it.
local slot = string.format("T3WoW_S%03d", Link.Status().nextSlot)
inboxes[slot] = { k = "c1", u = Link.Session(), ack = 0, ns = 7, nt = time() }
local before, count = clock, #loads
while #loads == count and clock - before < 120 do tick(1) end
assert(loads[#loads] == slot, tostring(loads[#loads]) .. " not " .. slot)
local watched = Link.Status().nextSignal
assert(watched ~= nil and watched >= 7 and watched < 12, "watching " .. tostring(watched))
return "ok"
`);
});

test("with its signal files gone, a check-in after a quiet spell brings the update and moves to the timer", () => {
  runGame(`
real[PROBE_VALID] = true
Link.Init()
inboxes.T3WoW_S001 = { k = "c1", u = Link.Session(), ack = 1, ns = 100, nt = time() }
tick(6)
assert(Link.Status().signals == "on", "signals " .. tostring(Link.Status().signals))
assert(#loads == 1, "polled loads " .. #loads)

-- The addon folder was replaced: the companion can't raise a signal, so it writes the
-- update with sigoff and no signal file ever becomes real.
inboxes.T3WoW_S002 = { k = "c1", u = Link.Session(), ack = 1, ns = 100, nt = time() - 6,
  sigoff = "the addon folder lost its signal files",
  events = { { n = time() * 1000, e = "", id = "t1", t = "Done", s = "completed" } } }
tick(590)
assert(#loads == 1, "loaded before the quiet spell was up: " .. #loads)
tick(15)
assert(loads[2] == "T3WoW_S002", tostring(loads[2]))
assert(#alerts == 1, "alerts " .. #alerts)
assert(Link.Status().recentAlerts[1].via == "a check-in after a quiet spell", Link.Status().recentAlerts[1].via)
assert(Link.Status().signals == "off", "signals " .. tostring(Link.Status().signals))

-- From now on slots load on the timer.
local count = #loads
tick(130)
assert(#loads > count, "no timer loads after signals went off")
return "ok"
`);
});

test("check-ins come further apart while no signal arrives", () => {
  runGame(`
real[PROBE_VALID] = true
Link.Init()
inboxes.T3WoW_S001 = { k = "c1", u = Link.Session(), ack = 1, ns = 100, nt = time() }
tick(6)
local function loadsWithin(seconds) local before = #loads; tick(seconds); return #loads - before end
assert(loadsWithin(605) == 1, "first check-in after 10 minutes")
assert(loadsWithin(1180) == 0, "the second waits 20 minutes")
assert(loadsWithin(20) == 1, "second check-in")
assert(loadsWithin(1780) == 0, "the third waits 30 minutes")
assert(loadsWithin(20) == 1, "third check-in")
return "ok"
`);
});
