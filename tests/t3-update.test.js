// The window and chat suggest updating T3 Code when the companion reports that the T3 it
// talks to runs an older one than T3 WoW is made for (inbox.t3u, and inbox.t3s for the pill).
import assert from "node:assert/strict";
import { test } from "node:test";
import { runGame } from "./game.js";

const LINE = "Update T3 Code to 0.1.9 (it runs 0.1.8): in T3 Code, Settings > About > Check for Updates";
const SHORT = "Update T3 Code to 0.1.9";

// `inbox(t3u, extra)` hands the window an update from the companion, with `t3u` left out
// when nil, and `t3s` with it unless `extra` says otherwise.
const run = (script) =>
  runGame(
    `
function inbox(t3u, extra)
  local data = { k = "c1", u = Link.Session(), ack = 0, envs = {}, threads = {}, projects = {}, notes = {}, events = {}, now = time(), t3u = t3u, t3s = t3u and SHORT }
  for key, value in pairs(extra or {}) do data[key] = value end
  UI.OnInbox(data)
end
function updateLines()
  local n = 0
  for _, line in ipairs(printed) do if line:find("Update T3 Code", 1, true) then n = n + 1 end end
  return n
end
function hover() return table.concat(UI.StatusLines(), "~") end
LINE = "${LINE}"
SHORT = "${SHORT}"
Link.Init()
UI.Toggle()
${script}`,
    { ui: true },
  );

test("an older T3 Code shows briefly on the pill, in full in its tooltip, and once in chat", () => {
  assert.equal(
    run(`
inbox(LINE)
assert(showing("^|cffffd100Update T3 Code to 0%.1%.9|r$"), "the pill doesn't say it in a few words")
assert(not showing("it runs 0%.1%.8"), "the pill says it in full")
assert(hover():find("^|cffffd100" .. LINE:gsub("%p", "%%%0") .. "|r~"), "the tooltip doesn't start with it in full: " .. hover())
assert(select(2, hover():gsub("Update T3 Code", "")) == 1, "the tooltip says it twice: " .. hover())
assert(printed[#printed]:find(LINE, 1, true), "chat doesn't say it in full: " .. printed[#printed])
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

test("an older companion's update (only t3u) shows it in full on the pill", () => {
  assert.equal(
    run(`
inbox(LINE, { t3s = false })
assert(showing("it runs 0%.1%.8"), "the pill doesn't say it")
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
assert(hover():find(LINE, 1, true), "the tooltip doesn't have the update: " .. hover())
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
