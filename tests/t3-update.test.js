// The window and chat suggest updating T3 Code when the companion reports that the T3 it
// talks to runs an older one than T3 WoW is made for (inbox.t3u).
import assert from "node:assert/strict";
import { test } from "node:test";
import { runGame } from "./game.js";

const LINE = "Update T3 Code to 0.19.0 (it runs 0.18.0): Help > Check for Updates... in T3 Code";

// `inbox(t3u)` hands the window an update from the companion, with `t3u` left out when nil.
const run = (script) =>
  runGame(
    `
function inbox(t3u, extra)
  local data = { k = "c1", u = Link.Session(), ack = 0, envs = {}, threads = {}, projects = {}, notes = {}, events = {}, now = time(), t3u = t3u }
  for key, value in pairs(extra or {}) do data[key] = value end
  UI.OnInbox(data)
end
function updateLines()
  local n = 0
  for _, line in ipairs(printed) do if line:find("Update T3 Code", 1, true) then n = n + 1 end end
  return n
end
LINE = "${LINE}"
Link.Init()
UI.Toggle()
${script}`,
    { ui: true },
  );

test("an older T3 Code shows in the window and says so once in chat", () => {
  assert.equal(
    run(`
inbox(LINE)
assert(showing("Update T3 Code to 0%.19%.0 %(it runs 0%.18%.0%)"), "the window doesn't suggest the update")
assert(updateLines() == 1, "chat lines: " .. updateLines())
inbox(LINE)
inbox(LINE)
assert(showing("Update T3 Code"), "the suggestion went away")
assert(updateLines() == 1, "chat lines: " .. updateLines())
-- Updated: the window is back to Connected.
inbox(nil)
assert(not showing("Update T3 Code"), "still suggesting it")
assert(showing("Connected"), "not connected")
return "ok"
`),
    "ok",
  );
});

test("a refused sign-in says that first", () => {
  assert.equal(
    run(`
inbox(LINE, { auth = "T3 refused the sign-in: sign in again from the tray menu" })
assert(showing("T3 refused the sign%-in"), "the sign-in problem doesn't show")
assert(not showing("Update T3 Code to"), "the update hides the sign-in problem")
return "ok"
`),
    "ok",
  );
});

test("an update without it (a current T3, or an older companion) shows nothing", () => {
  assert.equal(
    run(`
inbox(nil)
assert(not showing("Update T3 Code"), "a suggestion")
assert(showing("Connected"), "not connected")
assert(updateLines() == 0, "chat lines: " .. updateLines())
return "ok"
`),
    "ok",
  );
});
