// The window and chat say to restart WoW when the companion reports a newer addon on disk
// than the one the game loaded (inbox.av): WoW reads addon files only when it starts.
import assert from "node:assert/strict";
import { test } from "node:test";
import { runGame } from "./game.js";

// The game loaded the addon as `loaded` (nil: the client doesn't say). `inbox(av)` hands the
// window an update from the companion, with `av` left out when nil.
const run = (script, loaded = "0.5.0") =>
  runGame(
    `
function inbox(av)
  UI.OnInbox({ k = "c1", u = Link.Session(), ack = 0, envs = {}, threads = {}, projects = {}, notes = {}, events = {}, now = time(), av = av })
end
function restartLines()
  local n = 0
  for _, line in ipairs(printed) do if line:find("restart WoW", 1, true) then n = n + 1 end end
  return n
end
Link.Init()
UI.Toggle()
${script}`,
    {
      ui: true,
      before: loaded
        ? `C_AddOns.GetAddOnMetadata = function(name, field) if name == "T3WoW" and field == "Version" then return "${loaded}" end end`
        : "",
    },
  );

test("a newer addon on disk shows in the window and says so once in chat", () => {
  assert.equal(
    run(`
inbox("0.6.0")
assert(showing("T3 WoW 0%.6%.0 is installed: restart WoW to load it %(/reload isn't enough%)"), "the window doesn't say to restart")
assert(restartLines() == 1, "chat lines: " .. restartLines())
-- Every later update still shows it, but chat says it once per UI session.
inbox("0.6.0")
inbox("0.6.0")
assert(showing("restart WoW to load it"), "the note went away")
assert(restartLines() == 1, "chat lines: " .. restartLines())
return "ok"
`),
    "ok",
  );
});

test("versions compare as numbers, part by part", () => {
  assert.equal(
    run(
      `
inbox("0.10.0")
assert(showing("T3 WoW 0%.10%.0 is installed"), "0.10.0 isn't newer than 0.9.2")
return "ok"
`,
      "0.9.2",
    ),
    "ok",
  );
});

test("the same or an older addon on disk shows nothing", () => {
  for (const av of ["0.5.0", "0.5", "0.4.9", "0.0.1"]) {
    assert.equal(
      run(`
inbox("${av}")
assert(not showing("restart WoW"), "a restart note for ${av}")
assert(showing("Connected"), "not connected")
assert(restartLines() == 0, "chat lines: " .. restartLines())
return "ok"
`),
      "ok",
    );
  }
});

test("an update without the version (an older companion) shows nothing", () => {
  assert.equal(
    run(`
inbox(nil)
assert(not showing("restart WoW"), "a restart note")
assert(showing("Connected"), "not connected")
assert(restartLines() == 0, "chat lines: " .. restartLines())
-- A newer companion later in the session: now it says so.
inbox("0.6.0")
assert(showing("restart WoW to load it"), "no note")
assert(restartLines() == 1, "chat lines: " .. restartLines())
return "ok"
`),
    "ok",
  );
});

test("without its own version (or with an odd one on disk) the addon says nothing", () => {
  assert.equal(
    run(
      `
inbox("0.6.0")
assert(not showing("restart WoW"), "a restart note")
assert(restartLines() == 0, "chat lines: " .. restartLines())
return "ok"
`,
      null,
    ),
    "ok",
  );
  assert.equal(
    run(`
inbox("next")
inbox("v0.6.0")
assert(not showing("restart WoW"), "a restart note")
return "ok"
`),
    "ok",
  );
});

test("a test build counts as the release it was built from, and another build of it needs a restart", () => {
  // Loaded a test build: the companion now has another build, or the release, on disk.
  for (const av of ["0.9.1-dev+0123abc", "0.9.1", "0.9.2-dev+0123abc"]) {
    assert.equal(
      run(
        `
inbox("${av}")
assert(showing("T3 WoW ${av.replace(/[.+-]/g, "%$&")} is installed: restart WoW"), "no note for ${av}")
return "ok"
`,
        "0.9.1-dev+57ee2ec",
      ),
      "ok",
    );
  }
  // The same build, or an older release: nothing to load.
  for (const av of ["0.9.1-dev+57ee2ec", "0.9.0", "0.9.0-dev+0123abc"]) {
    assert.equal(
      run(
        `
inbox("${av}")
assert(not showing("restart WoW"), "a restart note for ${av}")
return "ok"
`,
        "0.9.1-dev+57ee2ec",
      ),
      "ok",
    );
  }
  // Loaded the release; the companion put a test build of it on disk.
  assert.equal(
    run(
      `
inbox("0.9.1-dev+57ee2ec")
assert(showing("restart WoW to load it"), "no note")
return "ok"
`,
      "0.9.1",
    ),
    "ok",
  );
});
