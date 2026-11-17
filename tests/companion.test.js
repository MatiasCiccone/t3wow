// The window and chat say to update the companion when it is older than the addon: the
// two are released together under one version, so CurseForge can update the addon and
// leave the companion behind. The companion names its version in each update (inbox.cv);
// one from before it did leaves it out.
import assert from "node:assert/strict";
import { test } from "node:test";
import { runGame } from "./game.js";

// The game loaded the addon as `loaded` (nil: the client doesn't say). `inbox(cv, u)` hands
// the window an update from the companion (`cv` left out when nil), written for this UI
// session unless `u` says otherwise. `extra` runs before the addon loads, too.
const run = (script, loaded = "0.9.2", extra = "") =>
  runGame(
    `
function inbox(cv, u)
  UI.OnInbox({ k = "c1", u = u or Link.Session(), ack = 0, envs = {}, threads = {}, projects = {}, notes = {}, events = {}, now = time(), cv = cv })
end
function companionLines()
  local n = 0
  for _, line in ipairs(printed) do if line:find("update it from its tray menu", 1, true) then n = n + 1 end end
  return n
end
function hover() return table.concat(UI.StatusLines(), "~") end
-- The pill says it in a few words; the tooltip's first line and chat say it in full.
function companionPill() return showing("^|cffff8000Update the T3 WoW companion|r$") end
Link.Init()
UI.Toggle()
${script}`,
    {
      ui: true,
      before:
        (loaded
          ? `C_AddOns.GetAddOnMetadata = function(name, field) if name == "T3WoW" and field == "Version" then return "${loaded}" end end`
          : "") + `\n${extra}`,
    },
  );

test("an older companion shows on the pill, in the tooltip, and once in chat", () => {
  assert.equal(
    run(`
inbox("0.9.1")
assert(companionPill(), "the pill doesn't say it")
assert(not showing("is older than this addon"), "the pill says it in full")
assert(UI.StatusLines()[1] == "|cffff8000T3 WoW companion 0.9.1 is older than this addon (0.9.2): update it from its tray menu|r", hover())
assert(printed[#printed]:find("T3 WoW companion 0.9.1 is older than this addon (0.9.2): update it from its tray menu", 1, true), printed[#printed])
assert(companionLines() == 1, "chat lines: " .. companionLines())
assert(printed[#printed]:find("Check for updates", 1, true), printed[#printed])
assert(hover():find("Addon 0.9.2, companion 0.9.1~Right-click T3 WoW in the system tray", 1, true), hover())
inbox("0.9.1")
inbox("0.9.1")
assert(companionPill(), "the note went away")
assert(companionLines() == 1, "chat lines: " .. companionLines())
-- Updated (it restarts on the new version): the pill is back to Connected.
inbox("0.9.2")
assert(showing("^|cff40c040Connected|r$"), "not connected after the update")
assert(not hover():find("Right-click", 1, true), hover())
return "ok"
`),
    "ok",
  );
});

// On a Mac the companion is the T3 WoW app in the menu bar, not a tray, and it updates
// itself from there as Windows' does from the tray.
test("on a Mac an older companion says to update it from its menu bar icon, not a tray", () => {
  assert.equal(
    run(
      `
inbox("0.9.1")
assert(companionPill(), "the pill doesn't say it")
assert(UI.StatusLines()[1] == "|cffff8000T3 WoW companion 0.9.1 is older than this addon (0.9.2): update it from its menu bar icon|r", hover())
assert(printed[#printed] == "|cff8f8fffT3 WoW|r |cffff8000T3 WoW companion 0.9.1 is older than this addon (0.9.2): update it from its menu bar icon (click T3 WoW's icon in the menu bar: Check for updates).|r", printed[#printed])
assert(hover():find("~Click T3 WoW's T3 icon in the menu bar: Check for updates, then Update.", 1, true), hover())
for _, said in ipairs({ hover(), printed[#printed] }) do
  assert(not said:find("tray", 1, true) and not said:find("t3wow", 1, true) and not said:find("Ctrl+C", 1, true), "a tray or a bare t3wow is mentioned on a Mac: " .. said)
end
inbox(nil)
assert(UI.StatusLines()[1] == "|cffff8000The T3 WoW companion is older than this addon (0.9.2): update it from its menu bar icon|r", hover())
return "ok"
`,
      "0.9.2",
      "function IsMacClient() return true end",
    ),
    "ok",
  );
});

test("with the companion note on the pill, the tooltip still shows the T3 Code update", () => {
  assert.equal(
    run(`
UI.OnInbox({ k = "c1", u = Link.Session(), ack = 0, envs = {}, threads = {}, projects = {}, notes = {}, events = {}, now = time(), cv = "0.9.1",
  t3u = "Update T3 Code: this T3 WoW is made for 0.1.9" })
assert(companionPill(), "the pill doesn't show the companion note")
assert(hover():find("~|cffffd100Update T3 Code: this T3 WoW is made for 0.1.9|r~", 1, true), hover())
-- Once the companion is updated the T3 Code note has the pill, and the tooltip doesn't repeat it.
UI.OnInbox({ k = "c1", u = Link.Session(), ack = 0, envs = {}, threads = {}, projects = {}, notes = {}, events = {}, now = time(), cv = "0.9.2",
  t3u = "Update T3 Code: this T3 WoW is made for 0.1.9" })
local _, n = hover():gsub("Update T3 Code", "")
assert(n == 1, hover())
return "ok"
`),
    "ok",
  );
});

test("a companion from before it named its version is older too", () => {
  assert.equal(
    run(`
inbox(nil)
assert(companionPill(), "the pill doesn't say it")
assert(UI.StatusLines()[1] == "|cffff8000The T3 WoW companion is older than this addon (0.9.2): update it from its tray menu|r", hover())
assert(printed[#printed]:find("The T3 WoW companion is older than this addon (0.9.2): update it from its tray menu", 1, true), printed[#printed])
assert(companionLines() == 1, "chat lines: " .. companionLines())
assert(hover():find("Addon 0.9.2, companion older (it doesn't say)", 1, true), hover())
return "ok"
`),
    "ok",
  );
});

test("a companion from before it named the UI session (0.4.1) is older too", () => {
  assert.equal(
    run(`
UI.OnInbox({ k = "c1", ack = 0, envs = {}, threads = {}, projects = {}, notes = {}, events = {}, now = time() })
assert(companionPill(), "the pill doesn't say it")
assert(UI.StatusLines()[1]:find("The T3 WoW companion is older than this addon", 1, true), hover())
assert(companionLines() == 1, "chat lines: " .. companionLines())
return "ok"
`),
    "ok",
  );
});

test("the same or a newer companion, or one with a build label, shows nothing", () => {
  for (const cv of ["0.9.2", "0.9.2.0", "0.9.3", "0.10.0", "1.0", "0.9.2-dev", "0.9.2+abc1234"]) {
    assert.equal(
      run(`
inbox("${cv}")
assert(showing("^|cff40c040Connected|r$"), "not connected with ${cv}")
assert(companionLines() == 0, "chat lines: " .. companionLines())
return "ok"
`),
      "ok",
      cv,
    );
  }
});

test("versions compare as numbers, part by part, and a build label doesn't hide an older one", () => {
  assert.equal(run(`inbox("0.9.10") return tostring(companionPill())`, "0.10.0"), "true");
  assert.equal(run(`inbox("0.9.1-dev") return tostring(companionPill())`), "true");
});

test("an update left from an earlier UI session doesn't count, nor an odd version", () => {
  assert.equal(
    run(`
inbox(nil, "an earlier session")
inbox("next")
assert(not companionPill(), "a note")
assert(companionLines() == 0, "chat lines: " .. companionLines())
return "ok"
`),
    "ok",
  );
});

test("without its own version the addon says nothing", () => {
  assert.equal(
    run(
      `
inbox("0.0.1")
inbox(nil)
assert(not companionPill(), "a note")
assert(companionLines() == 0, "chat lines: " .. companionLines())
return "ok"
`,
      null,
    ),
    "ok",
  );
});

test("a newer addon on disk comes first: restart WoW, then update the companion", () => {
  assert.equal(
    run(`
UI.OnInbox({ k = "c1", u = Link.Session(), ack = 0, envs = {}, threads = {}, projects = {}, notes = {}, events = {}, now = time(), av = "0.9.3", cv = "0.9.1" })
assert(showing("restart WoW to load it"), "no restart note")
assert(not companionPill(), "the companion note hides the restart one")
return "ok"
`),
    "ok",
  );
});
