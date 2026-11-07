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
