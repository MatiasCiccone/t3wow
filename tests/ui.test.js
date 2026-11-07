// The addon's window (UI.lua) in the simulated game: what it sends, and what it does with
// the companion's answers.
import assert from "node:assert/strict";
import { test } from "node:test";
import { runGame } from "./game.js";

// Helpers on top of the game: the composer, the buttons, and an inbox as the companion writes it.
const UI_HELPERS = `
function find(kind, text)
  for _, f in ipairs(frames) do
    if f.kind == kind and (text == nil or f.text == text) then return f end
  end
end
function click(text) find("Button", text).scripts.OnClick(find("Button", text), "LeftButton") end
function typeText(text) find("EditBox").text = text end
function typed() return find("EditBox").text end
function queue() return Link.Queue() end
function sent() return queue()[#queue()] end
local MAC = { id = "mac", name = "Mac", ok = true }
local LOCAL = { id = "local", name = "PC", ok = true }
function inboxWith(envs, extra)
  local data = { k = "c1", ack = extra and extra.ack or 0, envs = envs, threads = {}, projects = {}, notes = {}, events = {}, now = time() }
  -- This UI session's, unless the test says otherwise ("legacy": a 0.4.1 companion's, with none).
  if not (extra and extra.legacy) then data.u = extra and extra.u or Link.Session() end
  for _, t in ipairs(extra and extra.threads or {}) do data.threads[#data.threads + 1] = t end
  for _, p in ipairs(extra and extra.projects or {}) do data.projects[#data.projects + 1] = p end
  data.view, data.out, data.trust = extra and extra.view, extra and extra.out, extra and extra.trust
  data.more = extra and extra.more
  -- The companion has taken every command sent so far.
  for i = #queue(), 1, -1 do queue()[i] = nil end
  UI.OnInbox(data)
end
-- What a dropdown lists when opened, whichever menu system built it: { text, checked, pick }
-- (pick is nil for a line that can't be picked), and the list's scroll height if it scrolls.
function menuEntries(name)
  local menu, out = _G[name], {}
  if rawget(menu, "generator") then
    local root = {}
    function root:SetScrollMode(height) out.scroll = height end
    function root:CreateTitle(text) out[#out + 1] = { text = text } end
    function root:CreateRadio(text, isSelected, setSelected, data)
      out[#out + 1] = { text = text, checked = isSelected(data), pick = function() return setSelected(data) end }
    end
    menu.generator(menu, root)
  else
    menuItems = {}
    menu.init(menu, 1)
    for _, info in ipairs(menuItems) do out[#out + 1] = { text = info.text, checked = info.checked, pick = info.func } end
    menuItems = nil
  end
  return out
end
function menuTexts(name)
  local texts = {}
  for _, e in ipairs(menuEntries(name)) do texts[#texts + 1] = e.text end
  return table.concat(texts, "|")
end
function pick(name, text)
  for _, e in ipairs(menuEntries(name)) do
    if e.text == text then
      assert(e.pick, text .. " can't be picked")
      return e.pick()
    end
  end
  error("no " .. text .. " in " .. menuTexts(name))
end
-- What a dropdown shows while closed.
function shown(name) return _G[name].text end
ENVS = { LOCAL, MAC }
Link.Init()
UI.Toggle()
`;

// Blizzard's menu system (WowStyle1DropdownTemplate), which the game here lacks: a
// dropdown keeps its list function and the text it shows.
const MODERN_MENU = `
C_XMLUtil = { GetTemplateInfo = function(name) if name == "WowStyle1DropdownTemplate" then return {} end end }
MenuUtil = {}
MenuResponse = { Close = 3 }
local createFrame = CreateFrame
function CreateFrame(kind, name, parent, template)
  local f = createFrame(kind, name, parent, template)
  if template == "WowStyle1DropdownTemplate" then
    function f:SetupMenu(generator) self.generator = generator end
    function f:OverrideText(text) self.text = text end
  end
  return f
end
`;

const run = (script, { modern = false } = {}) =>
  runGame(UI_HELPERS + script, { ui: true, before: modern ? MODERN_MENU : "" });
// The New Thread dropdowns under both menu systems.
const MENUS = [["Blizzard's menu", true], ["UIDropDownMenu", false]];

test("a thread stays selected by its machine's id when the machine list changes", () => {
  run(`
inboxWith(ENVS, { threads = { { e = 2, id = "t1", t = "Mac thread", s = "completed", set = false, at = time(), p = "" } } })
UI.Select({ e = 2, id = "t1" })
assert(sent().t == "open" and sent().e == "mac", sent().e)
-- A machine is linked ahead of the Mac: the Mac is now third.
inboxWith({ ENVS[1], { id = "linux", name = "Linux", ok = false }, ENVS[2] },
  { threads = { { e = 3, id = "t1", t = "Mac thread", s = "completed", set = false, at = time(), p = "" } } })
typeText("hello")
click("Send")
assert(sent().t == "send" and sent().e == "mac", "sent to " .. tostring(sent().e))
-- The Mac is unlinked: nothing is sent and the text stays.
inboxWith({ ENVS[1] })
typeText("still there?")
local before = #queue()
click("Send")
assert(#queue() == before, "sent to a machine that's gone")
assert(typed() == "still there?", typed())
return "ok"
`);
});

for (const [menus, modern] of MENUS) test(`New Thread keeps the machine and project it was given, and refuses once they're gone (${menus})`, () => {
  run(`
local projects = { { e = 1, id = "pa", t = "alpha" }, { e = 2, id = "pm", t = "mac app" } }
inboxWith(ENVS, { projects = projects })
click("New Thread")
assert(shown("T3WoWMachineMenu") == "PC", shown("T3WoWMachineMenu"))
pick("T3WoWMachineMenu", "Mac")
assert(shown("T3WoWMachineMenu") == "Mac", "machine didn't change")
pick("T3WoWProjectMenu", "mac app")
assert(shown("T3WoWProjectMenu") == "mac app", shown("T3WoWProjectMenu"))
-- The local machine drops out of the list: the Mac's project is still the one picked.
inboxWith({ ENVS[2] }, { projects = { { e = 1, id = "pm", t = "mac app" } } })
typeText("build it")
click("Start")
assert(sent().t == "new" and sent().e == "mac" and sent().p == "pm", "started " .. tostring(sent().e) .. "/" .. tostring(sent().p))
-- The project is removed: Start refuses and keeps the text.
inboxWith({ ENVS[2] })
typeText("again")
local before = #queue()
click("Start")
assert(#queue() == before, "started with a project that's gone")
assert(typed() == "again", typed())
assert(shown("T3WoWProjectMenu"):find("gone: pick another"), shown("T3WoWProjectMenu"))
-- The Mac is unlinked too: Start refuses, keeps the text, and the dropdown says so.
inboxWith({ ENVS[1] })
pick("T3WoWProjectMenu", "No project (scratch)")
click("Start")
assert(#queue() == before, "started on a machine that's gone")
assert(typed() == "again", typed())
assert(shown("T3WoWMachineMenu"):find("no longer linked"), shown("T3WoWMachineMenu"))
return "ok"
`, { modern });
});

for (const [menus, modern] of MENUS) test(`the project dropdown lists the picked machine's projects (${menus})`, () => {
  run(`
local projects = { { e = 1, id = "pb", t = "beta" }, { e = 1, id = "pa", t = "Alpha" }, { e = 2, id = "pm", t = "mac app" } }
inboxWith(ENVS, { projects = projects })
click("New Thread")
assert(menuTexts("T3WoWMachineMenu") == "PC|Mac", menuTexts("T3WoWMachineMenu"))
assert(menuTexts("T3WoWProjectMenu") == "No project (scratch)|Alpha|beta", menuTexts("T3WoWProjectMenu"))
assert(menuEntries("T3WoWProjectMenu")[1].checked, "scratch isn't checked")
pick("T3WoWProjectMenu", "beta")
assert(menuEntries("T3WoWProjectMenu")[3].checked and not menuEntries("T3WoWProjectMenu")[1].checked, "beta isn't checked")
-- Another machine: its projects, and the project picked goes back to none.
pick("T3WoWMachineMenu", "Mac")
assert(menuEntries("T3WoWMachineMenu")[2].checked, "Mac isn't checked")
assert(menuTexts("T3WoWProjectMenu") == "No project (scratch)|mac app", menuTexts("T3WoWProjectMenu"))
assert(shown("T3WoWProjectMenu") == "No project (scratch)", shown("T3WoWProjectMenu"))
-- Picking the machine already picked keeps the project.
pick("T3WoWProjectMenu", "mac app")
pick("T3WoWMachineMenu", "Mac")
assert(shown("T3WoWProjectMenu") == "mac app", shown("T3WoWProjectMenu"))
typeText("build it")
click("Start")
assert(sent().t == "new" and sent().e == "mac" and sent().p == "pm", tostring(sent().e) .. "/" .. tostring(sent().p))
return "ok"
`, { modern });
});

for (const [menus, modern] of MENUS) test(`New Thread with no machines or no projects says so (${menus})`, () => {
  run(`
inboxWith({})
click("New Thread")
inboxWith({})
assert(shown("T3WoWMachineMenu"):find("none linked"), shown("T3WoWMachineMenu"))
local machines = menuEntries("T3WoWMachineMenu")
assert(#machines == 1 and machines[1].text:find("No machines linked") and not machines[1].pick, menuTexts("T3WoWMachineMenu"))
typeText("build it")
local before = #queue()
click("Start")
assert(#queue() == before and typed() == "build it", "started with no machine")
assert(showing("No machine to start it on"), "no reason given")
-- A machine with no projects: scratch, and a line saying there are none.
inboxWith(ENVS)
click("New Thread")
local list = menuEntries("T3WoWProjectMenu")
assert(#list == 2 and list[1].pick and list[2].text == "No projects on this machine" and not list[2].pick, menuTexts("T3WoWProjectMenu"))
return "ok"
`, { modern });
});

test("a long project list scrolls in Blizzard's menu and is capped in UIDropDownMenu", () => {
  const many = `
local projects = {}
for i = 1, 60 do projects[i] = { e = 1, id = "p" .. i, t = string.format("project %02d", i) } end
inboxWith(ENVS, { projects = projects })
click("New Thread")
`;
  run(`${many}
local list = menuEntries("T3WoWProjectMenu")
assert(#list == 61 and list.scroll and list.scroll > 0, #list .. " entries, scroll " .. tostring(list.scroll))
assert(not menuEntries("T3WoWMachineMenu").scroll, "a short list scrolls")
pick("T3WoWProjectMenu", "project 60")
assert(shown("T3WoWProjectMenu") == "project 60", shown("T3WoWProjectMenu"))
return "ok"
`, { modern: true });
  run(`${many}
local list = menuEntries("T3WoWProjectMenu")
assert(#list == 41 and list[41].text == "…and 21 more" and not list[41].pick, #list .. " " .. tostring(list[#list].text))
pick("T3WoWProjectMenu", "project 39")
assert(shown("T3WoWProjectMenu") == "project 39", shown("T3WoWProjectMenu"))
return "ok"
`);
});

test("a message T3 refused comes back to the box and its '(sending)' line goes", () => {
  run(`
local thread = { e = 1, id = "t1", t = "Thread", s = "completed", set = false, at = time(), p = "" }
local view = { e = 1, id = "t1", t = "Thread", s = "completed", items = { { r = "a", x = "Hi" } } }
inboxWith(ENVS, { threads = { thread }, view = view })
UI.Select(thread)
typeText("please fix it")
click("Send")
local s = sent().s
assert(typed() == "", "box not cleared")
assert(showing("%(sending%)"), "no echo")
inboxWith(ENVS, { threads = { thread }, view = view, out = { { s = s, ok = false, x = "T3 refused the message" } } })
assert(typed() == "please fix it", "text not back: " .. typed())
assert(not showing("%(sending%)"), "echo still there")
assert(showing("Not sent: T3 refused the message"), "no notice")
-- The same outcome again (it stays in updates for a minute) changes nothing.
typeText("")
inboxWith(ENVS, { threads = { thread }, view = view, out = { { s = s, ok = false, x = "T3 refused the message" } } })
assert(typed() == "", "text put back twice")
return "ok"
`);
});

test("Send answer comes back after a failed answer, or after a while with no word", () => {
  run(`
local thread = { e = 1, id = "t1", t = "Thread", s = "waiting", set = false, at = time(), p = "" }
local ask = { k = "abcd1234", qs = { { h = "Color", q = "Which?", m = false, o = { { l = "Red", d = "" }, { l = "Blue", d = "" } } } } }
local view = { e = 1, id = "t1", t = "Thread", s = "waiting", items = {}, ask = ask }
inboxWith(ENVS, { threads = { thread }, view = view })
UI.Select(thread)
click("Red")
click("Send answer")
local s = sent().s
assert(sent().t == "answer", sent().t)
assert(find("Button", "Sending…"), "not sending")
inboxWith(ENVS, { threads = { thread }, view = view, out = { { s = s, ok = false, x = "timed out" } } })
assert(find("Button", "Send answer") and find("Button", "Send answer"):IsShown(), "still stuck")
assert(showing("Answer not sent: timed out"), "no notice")

-- An older companion says nothing: the button comes back after the timeout.
click("Send answer")
assert(find("Button", "Sending…"), "not sending again")
inboxWith(ENVS, { threads = { thread }, view = view })
tick(50)
assert(find("Button", "Send answer"), "stuck without word")
return "ok"
`);
});

test("a started thread opens in place of the New Thread composer", () => {
  run(`
inboxWith(ENVS, { projects = { { e = 2, id = "pm", t = "mac app" } } })
click("New Thread")
pick("T3WoWMachineMenu", "Mac")
typeText("Build it")
click("Start")
local s = sent().s
assert(sent().t == "new" and sent().e == "mac", tostring(sent().e))
local started = { e = 2, id = "new1", t = "Build it", s = "running", set = false, at = time(), p = "" }
inboxWith(ENVS, { threads = { started }, view = { e = 2, id = "new1", t = "Build it", s = "running", items = {} },
  out = { { s = s, ok = true, env = "mac", id = "new1" } } })
assert(find("Button", "Send") and not find("Button", "Start"), "still in the composer")
typeText("and test it")
click("Send")
assert(sent().t == "send" and sent().id == "new1" and sent().e == "mac", tostring(sent().t) .. " " .. tostring(sent().id))
return "ok"
`);
});

test("an outcome from an earlier UI session doesn't pick a thread", () => {
  run(`
inboxWith(ENVS, { projects = { { e = 2, id = "pm", t = "mac app" } } })
click("New Thread")
typeText("Build it")
click("Start")
local s = sent().s
-- A slot written for the session before a crash, whose command s was another Start.
inboxWith(ENVS, { u = "previous-session", view = { e = 1, id = "old-thread", t = "Old", s = "running", items = {} },
  out = { { s = s, ok = true, env = "local", id = "old-thread" } } })
assert(find("Button", "Start"), "left the composer for an earlier session's thread")
-- This session's outcome still opens the new thread.
local started = { e = 1, id = "new1", t = "Build it", s = "running", set = false, at = time(), p = "" }
inboxWith(ENVS, { threads = { started }, view = { e = 1, id = "new1", t = "Build it", s = "running", items = {} },
  out = { { s = s, ok = true, env = "local", id = "new1" } } })
assert(find("Button", "Send") and not find("Button", "Start"), "still in the composer")
return "ok"
`);
});

test("with a 0.4.1 companion, a started thread still opens once the Start is acked", () => {
  run(`
inboxWith(ENVS, { legacy = true })
click("New Thread")
typeText("Build it")
click("Start")
local s = sent().s
-- Before the companion took the Start: an older view means nothing.
inboxWith(ENVS, { legacy = true, ack = s - 1, view = { e = 1, id = "old-thread", t = "Old", s = "running", items = {} } })
assert(find("Button", "Start"), "followed a view from before the Start")
local started = { e = 1, id = "new1", t = "Build it", s = "running", set = false, at = time(), p = "" }
inboxWith(ENVS, { legacy = true, ack = s, threads = { started }, view = { e = 1, id = "new1", t = "Build it", s = "running", items = {} } })
assert(find("Button", "Send") and not find("Button", "Start"), "still in the composer")
typeText("and test it")
click("Send")
assert(sent().t == "send" and sent().id == "new1", tostring(sent().id))
return "ok"
`);
});

test("an overlong typed answer stays in the box instead of being dropped", () => {
  run(`
local thread = { e = 1, id = "t1", t = "Thread", s = "waiting", set = false, at = time(), p = "" }
local ask = { k = "abcd1234", qs = { { h = "Color", q = "Which?", m = false, o = { { l = "Red", d = "" } } } } }
local view = { e = 1, id = "t1", t = "Thread", s = "waiting", items = {}, ask = ask }
inboxWith(ENVS, { threads = { thread }, view = view })
UI.Select(thread)
local long = string.rep('"', 2400)
typeText(long)
local before = #queue()
click("Send answer")
assert(#queue() == before, "sent")
assert(typed() == long, "box cleared")
assert(find("Button", "Send answer"), "stuck on Sending")
return "ok"
`);
});

test("an overlong message stays in the box instead of being dropped", () => {
  run(`
local thread = { e = 1, id = "t1", t = "Thread", s = "completed", set = false, at = time(), p = "" }
inboxWith(ENVS, { threads = { thread } })
UI.Select(thread)
-- 2400 bytes (the box's limit) of quotes: each one doubles once escaped.
local long = string.rep('"', 2400)
typeText(long)
local before = #queue()
click("Send")
assert(#queue() == before, "sent")
assert(typed() == long, "box cleared")
return "ok"
`);
});

test("what the tray setting blocks is greyed out, says why, and keeps the text", () => {
  run(`
local thread = { e = 1, id = "t1", t = "Thread", s = "running", set = false, at = time(), p = "" }
local view = { e = 1, id = "t1", t = "Thread", s = "running", items = { { r = "a", x = "Hi" } } }
inboxWith(ENVS, { threads = { thread }, view = view, trust = "readonly" })
UI.Select(thread)
assert(not find("Button", "Send").enabled, "Send enabled under Read-only")
assert(not find("Button", "Stop").enabled, "Stop enabled under Read-only")
assert(not find("Button", "Settle").enabled, "Settle enabled under Read-only")
assert(showing("Read%-only: see Commands from the game"), "the window doesn't say it's read-only")
typeText("please fix it")
local before = #queue()
click("Send")
click("Stop")
click("Settle")
assert(#queue() == before, "sent under Read-only")
assert(typed() == "please fix it", typed())
assert(showing("Sending messages from the game is off"), "no reason given")
-- A question: Send answer is off, and the hint says to answer in T3.
local ask = { k = "abcd1234", qs = { { h = "Color", q = "Which?", m = false, o = { { l = "Red", d = "" } } } } }
view = { e = 1, id = "t1", t = "Thread", s = "waiting", items = {}, ask = ask }
inboxWith(ENVS, { threads = { thread }, view = view, trust = "readonly" })
assert(not find("Button", "Send answer").enabled, "Send answer enabled under Read-only")
assert(not find("Button", "Red").enabled, "an option enabled under Read-only")
assert(showing("Answering from the game is off"), "no reason on the question")

-- No new threads: messages go, Start doesn't.
inboxWith(ENVS, { threads = { thread }, view = view, trust = "nonew" })
assert(find("Button", "Send answer").enabled, "Send answer off under No new threads")
click("New Thread")
assert(not find("Button", "Start").enabled, "Start enabled under No new threads")
assert(showing("Starting threads from the game is off"), "no reason in the composer")
typeText("build it")
before = #queue()
click("Start")
assert(#queue() == before and typed() == "build it", "started under No new threads")
-- Full again (the companion leaves the field out): Start works.
inboxWith(ENVS, {})
assert(find("Button", "Start").enabled, "Start still off")
click("Start")
assert(sent().t == "new", tostring(sent().t))
return "ok"
`);
});

test("Show more shows only under a group with more threads, and asks the companion for them", () => {
  run(`
local function moreRow(group)
  for _, f in ipairs(frames) do
    if f.kind == "Button" and f.text == "Show more" and f.group == group and f.shown then return f end
  end
end
local function header(label)
  for _, f in ipairs(frames) do
    if type(f.text) == "table" and type(f.text.text) == "string" and f.text.text:find(label, 1, true) then return f end
  end
end
local thread = { e = 1, id = "t1", t = "Thread", s = "completed", set = false, at = time(), p = "" }
-- A 0.4.1 companion sends no "more": no row.
inboxWith(ENVS, { threads = { thread } })
assert(not moreRow("open") and not moreRow("settled"), "a row with nothing more to list")
inboxWith(ENVS, { threads = { thread }, more = { open = true, settled = true } })
local open = moreRow("open")
assert(open, "no Show more under Open")
assert(not moreRow("snoozed"), "Show more under Snoozed")
assert(not moreRow("settled"), "Show more under a collapsed Settled")
open.scripts.OnClick(open, "LeftButton")
assert(sent().t == "list" and sent().g == "open", "sent " .. tostring(sent().t) .. "/" .. tostring(sent().g))
local settledHeader = header("Settled")
settledHeader.scripts.OnClick(settledHeader)
local settled = moreRow("settled")
assert(settled, "no Show more under Settled")
settled.scripts.OnClick(settled, "LeftButton")
assert(sent().t == "list" and sent().g == "settled", "sent " .. tostring(sent().g))
-- Everything listed: the rows go.
inboxWith(ENVS, { threads = { thread } })
assert(not moreRow("open") and not moreRow("settled"), "rows stayed")
return "ok"
`);
});

test("Stop shows Stopping… until the run has stopped, and clears on every other outcome too", () => {
  run(`
local running = { e = 1, id = "t1", t = "Thread", s = "running", set = false, at = time(), p = "" }
local function stopButton() return find("Button", "Stop") or find("Button", "Stopping…") end
inboxWith(ENVS, { threads = { running } })
UI.Select(running)

-- T3 takes the interrupt: Stopping… while the run winds down, then Stop again once it stopped.
click("Stop")
local s = sent().s
assert(sent().t == "stop" and sent().id == "t1" and sent().e == "local", tostring(sent().t))
assert(stopButton().text == "Stopping…" and not stopButton().enabled, "pressed: " .. stopButton().text)
inboxWith(ENVS, { threads = { running }, out = { { s = s, ok = true, st = "stopping" } } })
assert(stopButton().text == "Stopping…", "taken: " .. stopButton().text)
local stopped = { e = 1, id = "t1", t = "Thread", s = "interrupted", set = false, at = time(), p = "" }
inboxWith(ENVS, { threads = { stopped } })
assert(stopButton().text == "Stop" and not stopButton().enabled, "stopped: " .. stopButton().text)

-- Already stopped (the list hadn't caught up): Stop comes back, and the window says so.
inboxWith(ENVS, { threads = { running } })
click("Stop")
s = sent().s
inboxWith(ENVS, { threads = { running }, out = { { s = s, ok = true, st = "idle" } } })
assert(stopButton().text == "Stop" and stopButton().enabled, "already stopped: " .. stopButton().text)
assert(showing("Nothing to stop"), "no word that it had already stopped")

-- Refused: Stop comes back with T3's reason.
click("Stop")
s = sent().s
inboxWith(ENVS, { threads = { running }, out = { { s = s, ok = false, x = "thread_not_interruptible" } } })
assert(stopButton().text == "Stop" and stopButton().enabled, "refused: " .. stopButton().text)
assert(showing("Couldn't stop: thread_not_interruptible"), "no reason given")

-- Taken, but the run never ends: Stop comes back after a while.
click("Stop")
s = sent().s
inboxWith(ENVS, { threads = { running }, out = { { s = s, ok = true, st = "stopping" } } })
tick(65)
assert(stopButton().text == "Stop" and stopButton().enabled, "stuck while still running: " .. stopButton().text)
assert(showing("Still running"), "no word that it's still running")

-- No word back at all (an older companion, or one not answering): the same.
click("Stop")
inboxWith(ENVS, { threads = { running } })
tick(65)
assert(stopButton().text == "Stop" and stopButton().enabled, "stuck without word: " .. stopButton().text)
assert(showing("No word back on Stop"), "no word about the timeout")

-- Stopping is one thread's: another thread shows its own Stop.
local other = { e = 2, id = "t2", t = "Other", s = "running", set = false, at = time(), p = "" }
inboxWith(ENVS, { threads = { running, other } })
click("Stop")
UI.Select(other)
assert(stopButton().text == "Stop" and stopButton().enabled, "another thread: " .. stopButton().text)
UI.Select(running)
assert(stopButton().text == "Stopping…", "back on the first: " .. stopButton().text)
return "ok"
`);
});

// The shown font string whose text matches the Lua pattern.
const SHOWN = `
function shownString(pattern)
  for _, fs in ipairs(fontStrings) do
    if not fs.hidden and type(fs.text) == "string" and fs.text:find(pattern) then return fs end
  end
end
function below(fs, above) return fs.point and fs.point[2] == above and fs.point[3] == "BOTTOMLEFT" end
`;

test("each message and the Working… line sit below the one before, however tall its text is", () => {
  run(SHOWN + `
local thread = { e = 1, id = "t1", t = "Thread", s = "running", set = false, at = time(), p = "" }
local items = { { r = "a", x = "Hi" } }
local view = { e = 1, id = "t1", t = "Thread", s = "running", items = items }
inboxWith(ENVS, { threads = { thread }, view = view })
UI.Select(thread)
typeText("No tailscale. Just lan and t3")
click("Send")
assert(below(shownString("%(sending%)"), shownString("Hi")), "the echo isn't below the last message")
assert(below(shownString("Working…"), shownString("%(sending%)")), "Working… isn't below the echo")
-- Your message and a long reply arrive together: the reply takes the string Working… had.
local reply = string.rep("The SSH tunnel I suggested earlier won't help here. ", 40)
items = { items[1], { r = "u", x = "No tailscale. Just lan and t3" }, { r = "a", x = reply } }
view = { e = 1, id = "t1", t = "Thread", s = "running", items = items }
inboxWith(ENVS, { threads = { thread }, view = view })
local you, agent, working = shownString("No tailscale"), shownString("SSH tunnel"), shownString("Working…")
assert(not shownString("%(sending%)"), "echo still there")
assert(below(you, shownString("Hi")), "your message isn't below the one before")
assert(below(agent, you), "the reply isn't below your message")
assert(below(working, agent), "Working… isn't below the reply")
return "ok"
`);
});

test("clicking a message opens it highlighted in a read-only copy box, without game text codes", () => {
  run(SHOWN + `
local thread = { e = 1, id = "t1", t = "Thread", s = "completed", set = false, at = time(), p = "" }
local text = "Run |cffc8c8a0npm test|r in a || pipe"
local view = { e = 1, id = "t1", t = "Thread", s = "completed", items = { { r = "a", x = text } } }
inboxWith(ENVS, { threads = { thread }, view = view })
UI.Select(thread)
local hit = shownString("npm test").hit
assert(hit and hit.shown, "the message can't be clicked")
hit.scripts.OnClick(hit, "LeftButton")
local edit
for _, f in ipairs(frames) do
  if f.kind == "EditBox" and f.text == "Run npm test in a | pipe" then edit = f end
end
assert(edit, "no copy box with the message's plain text")
local box = edit.parent.parent
assert(box.shown, "copy box hidden")
-- Read-only: typing puts the message back.
edit.text = "changed"
edit.scripts.OnTextChanged(edit, true)
assert(edit.text == "Run npm test in a | pipe", edit.text)
edit.scripts.OnEscapePressed(edit)
assert(not box.shown, "Escape didn't close it")
-- Ctrl+C closes it too, once the game has copied.
hit.scripts.OnClick(hit, "LeftButton")
assert(box.shown, "didn't open again")
controlDown = true
edit.scripts.OnKeyDown(edit, "C")
controlDown = false
assert(not box.shown, "Ctrl+C didn't close it")
-- Opening another thread closes it.
hit.scripts.OnClick(hit, "LeftButton")
UI.Select({ e = 1, id = "t2" })
assert(not box.shown, "stayed open on another thread")
-- Status lines can't be copied.
inboxWith(ENVS, { threads = { { e = 1, id = "t2", t = "Other", s = "running", set = false, at = time(), p = "" } },
  view = { e = 1, id = "t2", t = "Other", s = "running", items = {} } })
assert(not shownString("Working…").hit.shown, "Working… is clickable")
return "ok"
`);
});
