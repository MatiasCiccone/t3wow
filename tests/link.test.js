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
