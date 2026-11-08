// The addon's window (UI.lua) in the simulated game: what it sends, and what it does with
// the companion's answers.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runGame } from "./game.js";

// A JSON value as a Lua literal (control characters as decimal escapes).
const luaValue = (v) =>
  typeof v === "string"
    ? `"${v.replace(/[\\"]/g, "\\$&").replace(/[\x00-\x1f]/g, (c) => `\\${String(c.charCodeAt(0)).padStart(3, "0")}`)}"`
    : Array.isArray(v)
      ? `{${v.map(luaValue).join(",")}}`
      : `{${Object.entries(v).map(([k, x]) => `${k}=${luaValue(x)}`).join(",")}}`;

// Helpers on top of the game: the composer, the buttons, and an inbox as the companion writes it.
const UI_HELPERS = `
function find(kind, text)
  for _, f in ipairs(frames) do
    if f.kind == kind and (text == nil or f.text == text) then return f end
  end
end
function click(text) find("Button", text).scripts.OnClick(find("Button", text), "LeftButton") end
-- The message box: the first unnamed edit box (the search box over the list has a name).
function composer()
  for _, f in ipairs(frames) do
    if f.kind == "EditBox" and rawget(f, "name") == nil then return f end
  end
end
function typeText(text) composer().text = text end
function typed() return composer().text end
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
  data.att, data.pi = extra and extra.att, extra and extra.pi
  data.sel, data.models = extra and extra.sel, extra and extra.models
  -- The companion has taken every command sent so far.
  for i = #queue(), 1, -1 do queue()[i] = nil end
  UI.OnInbox(data)
end
-- What a dropdown lists when clicked open: { text, checked, pick } (pick is nil for a line
-- that can't be picked), and the list's scroll height if it scrolls.
function menuEntries(name)
  local dd, list, out = _G[name], T3WoWDropdownList, {}
  if list.shown then list:Hide() end
  dd.scripts.OnClick(dd, "LeftButton")
  assert(list.shown, name .. " didn't open")
  for _, r in ipairs(list.rows) do
    if r.shown then
      out[#out + 1] = { text = r.text, checked = r.checked,
        pick = r.enabled and function() return r.scripts.OnClick(r, "LeftButton") end or nil }
    end
  end
  out.scroll = list.scrollHeight
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

const run = (script, options) => runGame(UI_HELPERS + script, { ui: true, ...options });

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

test(`New Thread keeps the machine and project it was given, and refuses once they're gone`, () => {
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
`);
});

test(`the project dropdown lists the picked machine's projects`, () => {
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
`);
});

test(`New Thread asks for a new worktree or the current checkout, starting at the project's default`, () => {
  run(`
local projects = { { e = 1, id = "pa", t = "alpha", w = true }, { e = 1, id = "pb", t = "beta" } }
inboxWith(ENVS, { projects = projects })
click("New Thread")
-- A scratch thread has no checkout: no workspace to pick.
assert(not T3WoWWorkspaceMenu:IsShown(), "workspace shown for scratch")
-- A project whose default is a new worktree starts there.
pick("T3WoWProjectMenu", "alpha")
assert(T3WoWWorkspaceMenu:IsShown(), "workspace hidden for a project")
assert(shown("T3WoWWorkspaceMenu") == "New worktree", shown("T3WoWWorkspaceMenu"))
assert(menuTexts("T3WoWWorkspaceMenu") == "Current checkout|New worktree", menuTexts("T3WoWWorkspaceMenu"))
assert(menuEntries("T3WoWWorkspaceMenu")[2].checked, "New worktree isn't checked")
typeText("in a worktree")
click("Start")
assert(sent().t == "new" and sent().p == "pa" and sent().w == true, "w=" .. tostring(sent().w))
-- One that runs in its checkout by default: nothing asks for a worktree.
click("New Thread")
pick("T3WoWProjectMenu", "beta")
assert(shown("T3WoWWorkspaceMenu") == "Current checkout", shown("T3WoWWorkspaceMenu"))
typeText("in the checkout")
click("Start")
assert(sent().t == "new" and sent().p == "pb" and sent().w == nil, "w=" .. tostring(sent().w))
-- Picked by hand, and kept while the same project is picked again.
click("New Thread")
pick("T3WoWProjectMenu", "beta")
pick("T3WoWWorkspaceMenu", "New worktree")
pick("T3WoWProjectMenu", "beta")
assert(shown("T3WoWWorkspaceMenu") == "New worktree", shown("T3WoWWorkspaceMenu"))
-- Another project goes back to its own default.
pick("T3WoWProjectMenu", "alpha")
pick("T3WoWWorkspaceMenu", "Current checkout")
typeText("alpha, in its checkout")
click("Start")
assert(sent().p == "pa" and sent().w == nil, "w=" .. tostring(sent().w))
-- Back to scratch: the workspace goes, and a worktree picked before isn't asked for.
click("New Thread")
pick("T3WoWProjectMenu", "alpha")
pick("T3WoWProjectMenu", "No project (scratch)")
assert(not T3WoWWorkspaceMenu:IsShown(), "workspace shown for scratch")
typeText("scratch")
click("Start")
assert(sent().t == "new" and sent().p == nil and sent().w == nil, "w=" .. tostring(sent().w))
return "ok"
`);
});

test(`New Thread with no machines or no projects says so`, () => {
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
`);
});

test("a long project list scrolls, and picking closes the list", () => {
  run(`
local projects = {}
for i = 1, 60 do projects[i] = { e = 1, id = "p" .. i, t = string.format("project %02d", i) } end
inboxWith(ENVS, { projects = projects })
click("New Thread")
local list = menuEntries("T3WoWProjectMenu")
assert(#list == 61 and list.scroll and list.scroll > 0, #list .. " entries, scroll " .. tostring(list.scroll))
assert(not menuEntries("T3WoWMachineMenu").scroll, "a short list scrolls")
pick("T3WoWProjectMenu", "project 60")
assert(shown("T3WoWProjectMenu") == "project 60", shown("T3WoWProjectMenu"))
assert(not T3WoWDropdownList.shown, "the list stayed open after a pick")
-- Clicking the dropdown again closes an open list; so does hiding the dropdown.
local dd = T3WoWProjectMenu
dd.scripts.OnClick(dd, "LeftButton")
assert(T3WoWDropdownList.shown, "didn't open")
dd.scripts.OnClick(dd, "LeftButton")
assert(not T3WoWDropdownList.shown, "a second click didn't close it")
dd.scripts.OnClick(dd, "LeftButton")
dd.scripts.OnHide(dd)
assert(not T3WoWDropdownList.shown, "it outlived its dropdown")
return "ok"
`);
});

test("the thread panel's header names the branch the open thread is on", () => {
  run(`
local thread = { e = 1, id = "t1", t = "Thread", s = "completed", set = false, at = time(), p = "" }
inboxWith(ENVS, { threads = { thread } })
UI.Select(thread)
-- Until the companion reads the thread there's no branch to show.
assert(not showing("feature/"), "a branch before the read")
local view = { e = 1, id = "t1", t = "Thread", s = "completed", br = "feature/show-branch", items = { { r = "a", x = "Hi" } } }
inboxWith(ENVS, { threads = { thread }, view = view })
assert(showing("%· feature/show%-branch$"), "no branch after the machine and age")
-- A scratch thread, or an older companion, sends none.
view.br = nil
inboxWith(ENVS, { threads = { thread }, view = view })
assert(not showing("feature/"), "the branch stayed")
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

test("the box keeps the keyboard after Enter sends a message or an answer", () => {
  run(`
local lost = 0
composer().ClearFocus = function() lost = lost + 1 end
local function enter() composer().scripts.OnEnterPressed(composer()) end
local thread = { e = 1, id = "t1", t = "Thread", s = "running", set = false, at = time(), p = "" }
inboxWith(ENVS, { threads = { thread }, view = { e = 1, id = "t1", t = "Thread", s = "running", items = {} } })
UI.Select(thread)
typeText("first")
enter()
assert(sent().m == "first", "not sent")
assert(typed() == "", "box not cleared")
assert(lost == 0, "the message took the keyboard away")
local ask = { k = "abcd1234", qs = { { h = "Color", q = "Which?", m = false, o = { { l = "Red", d = "" } } } } }
thread.s = "waiting"
inboxWith(ENVS, { threads = { thread }, view = { e = 1, id = "t1", t = "Thread", s = "waiting", items = {}, ask = ask } })
typeText("Green")
enter()
assert(typed() == "", "answer not sent")
assert(lost == 0, "the answer took the keyboard away")
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

test("An agent-launched thread with no known launcher keeps out of Open, as in T3's sidebar", () => {
  run(`
local function count(label)
  for _, f in ipairs(frames) do
    if type(f.text) == "table" and f.text.text == label and f.shown then return tonumber(f.count.text) end
  end
end
local function listed(title)
  for _, f in ipairs(frames) do
    if type(f.thread) == "table" and f.thread.t == title and f.shown then return true end
  end
  return false
end
local now = time()
inboxWith(ENVS, { threads = {
  { e = 1, id = "u1", t = "Mine", s = "completed", set = false, at = now, p = "" },
  -- delegate_task leaves no sender on the first message: the companion knows only that an agent made it.
  { e = 1, id = "a1", t = "Delegated, done", s = "completed", set = false, ag = true, at = now, p = "" },
  { e = 1, id = "a2", t = "Delegated, running", s = "running", set = false, ag = true, at = now, p = "" },
  { e = 1, id = "a3", t = "Delegated, asking", s = "waiting", set = false, ag = true, at = now, p = "" },
  { e = 1, id = "a4", t = "Delegated, settled", s = "completed", set = true, ag = true, at = now, p = "" },
  -- Its launcher is known and listed: it sits under it.
  { e = 1, id = "a5", t = "Launched by Mine", s = "completed", set = false, ag = true, par = "u1", at = now, p = "" },
} })
-- Open: Mine (its launched thread under it doesn't count) and the one that needs you.
assert(count("Open") == 2, "Open counts " .. tostring(count("Open")))
assert(listed("Mine") and listed("Launched by Mine"), "Mine or its launched thread missing")
assert(not listed("Delegated, done"), "a finished delegated thread shows in Open")
assert(listed("Delegated, running") and count("Working") == 1, "a working delegated thread missing")
assert(listed("Delegated, asking"), "a delegated thread that needs you is missing")
assert(count("Settled") == 1, "Settled counts " .. tostring(count("Settled")))
return "ok"
`);
});

test("a thread watching a pull request shows as Waiting under Working, with nothing to stop", () => {
  run(`
local function count(label)
  for _, f in ipairs(frames) do
    if type(f.text) == "table" and f.text.text == label and f.shown then return tonumber(f.count.text) end
  end
end
local thread = { e = 1, id = "t1", t = "Watching CI", s = "watching", set = false, at = time(), p = "" }
inboxWith(ENVS, { threads = { thread }, view = { e = 1, id = "t1", t = "Watching CI", s = "completed", items = { { r = "a", x = "Watching CI." } } } })
UI.Select(thread)
assert(count("Working") == 1 and (count("Open") or 0) == 0, "Working " .. tostring(count("Working")) .. ", Open " .. tostring(count("Open")))
assert(showing("^Waiting$"), "no Waiting pill")
assert(not showing("Working…"), "says Working… with no run")
assert(not find("Button", "Stop").enabled, "Stop is enabled with no run")
return "ok"
`);
});

test("Search narrows the list to threads matching every word, opens groups on their matches, and Escape clears it", () => {
  run(`
local function count(label)
  for _, f in ipairs(frames) do
    if type(f.text) == "table" and f.text.text == label and f.shown then return tonumber(f.count.text) end
  end
end
local function listed(title)
  for _, f in ipairs(frames) do
    if type(f.thread) == "table" and f.thread.t == title and f.shown then return true end
  end
  return false
end
local function searchFor(text)
  T3WoWSearch.text = text
  T3WoWSearch.scripts.OnTextChanged(T3WoWSearch, true)
end
local now = time()
inboxWith(ENVS, { threads = {
  { e = 2, id = "t1", t = "Fix login bug", s = "completed", set = false, at = now, p = "web" },
  { e = 1, id = "t2", t = "Write docs", s = "completed", set = false, at = now, p = "t3wow" },
  { e = 1, id = "t3", t = "Login subtask", s = "completed", set = false, par = "t2", at = now, p = "t3wow" },
  { e = 1, id = "t4", t = "Old LOGIN cleanup", s = "completed", set = true, at = now, p = "" },
  { e = 1, id = "t5", t = "Refactor", s = "running", set = false, at = now, p = "" },
  -- Finished, launched by a thread that isn't listed (or isn't known): kept out of Open.
  { e = 1, id = "t6", t = "Delegated login", s = "completed", set = false, ag = true, at = now, p = "" },
  { e = 1, id = "t7", t = "Orphan login", s = "completed", set = false, par = "gone", at = now, p = "" },
}, more = { settled = true } })
assert(not listed("Delegated login") and not listed("Orphan login"), "a hidden launched thread shows")
assert(listed("Write docs") and listed("Login subtask") and not listed("Old LOGIN cleanup"), "before searching")
assert(showing("^Search$"), "no placeholder")

-- Case aside; a launched thread whose launcher doesn't match stands on its own; collapsed
-- Settled opens on its match and counts only its matches; Working, with none, hides.
searchFor("login")
assert(listed("Fix login bug") and listed("Login subtask") and listed("Old LOGIN cleanup"), "a match missing")
assert(listed("Delegated login") and listed("Orphan login"), "a launched thread with no listed launcher missing")
assert(not listed("Write docs") and not listed("Refactor"), "a thread that doesn't match")
assert(count("Open") == 4 and count("Settled") == 1, "counts " .. tostring(count("Open")) .. "/" .. tostring(count("Settled")))
assert(count("Working") == nil, "Working shows with no match")
assert(not showing("^Search$"), "placeholder over the text")

-- Every word must match, in the title, the project or the machine.
searchFor("mac LOGIN")
assert(listed("Fix login bug") and not listed("Login subtask") and not listed("Old LOGIN cleanup"), "machine")
searchFor("t3wow")
assert(listed("Write docs") and listed("Login subtask"), "project")

-- Nothing matches: the list says so, and Settled stays for its Show more.
searchFor("zzz")
assert(showing("No threads match") == false and count("Settled") == 0, "Settled with more to list hid")
inboxWith(ENVS, { threads = { { e = 1, id = "t2", t = "Write docs", s = "completed", set = false, at = now, p = "t3wow" } } })
assert(showing("No threads match") and count("Open") == nil, "no word that nothing matches")

-- Enter opens the first thread shown.
searchFor("docs")
T3WoWSearch.scripts.OnEnterPressed(T3WoWSearch)
assert(sent().t == "open" and sent().id == "t2", "Enter sent " .. tostring(sent().t))

-- Escape clears it: the whole list, with Settled collapsed again.
inboxWith(ENVS, { threads = {
  { e = 1, id = "t2", t = "Write docs", s = "completed", set = false, at = now, p = "t3wow" },
  { e = 1, id = "t4", t = "Old LOGIN cleanup", s = "completed", set = true, at = now, p = "" },
} })
searchFor("login")
assert(listed("Old LOGIN cleanup"), "settled match missing")
T3WoWSearch.scripts.OnEscapePressed(T3WoWSearch)
T3WoWSearch.scripts.OnTextChanged(T3WoWSearch, false) -- the game fires it for SetText
assert(T3WoWSearch.text == "" and listed("Write docs") and not listed("Old LOGIN cleanup"), "Escape left the search")
assert(showing("^Search$") and not showing("No threads match"), "after Escape")

-- The magnifying glass shows only if the game found its file (it started after the update).
local glyph = T3WoWSearch.glyph
T3WoWSearch.scripts.OnShow(T3WoWSearch)
assert(glyph.shown, "glyph hidden although its file is there")
missingTextures[glyph.texturePath] = true
T3WoWSearch.scripts.OnShow(T3WoWSearch)
assert(not glyph.shown, "glyph shown with no file behind it")
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

test("a reply that arrives while Working… shows leaves one Working…, not a second one under the reply", () => {
  run(SHOWN + `
local function shownCount(pattern)
  local n = 0
  for _, fs in ipairs(fontStrings) do
    if not fs.hidden and type(fs.text) == "string" and fs.text:find(pattern) then n = n + 1 end
  end
  return n
end
-- An answer picked from the options sends no echo, so Working… is drawn with a gap before it.
local thread = { e = 1, id = "t1", t = "Thread", s = "running", set = false, at = time(), p = "" }
local items = { { r = "u", x = "Add a tray option" } }
inboxWith(ENVS, { threads = { thread }, view = { e = 1, id = "t1", t = "Thread", s = "running", items = items } })
UI.Select(thread)
assert(shownCount("Working…") == 1, "Working… missing")
items = { items[1], { r = "a", x = "Building both on the same branch." } }
inboxWith(ENVS, { threads = { thread }, view = { e = 1, id = "t1", t = "Thread", s = "running", items = items } })
assert(shownCount("Working…") == 1, shownCount("Working…") .. " Working… lines")
assert(below(shownString("Working…"), shownString("Building both")), "Working… isn't below the reply")
-- The run finishes with no new message: Working… goes.
thread.s = "completed"
inboxWith(ENVS, { threads = { thread }, view = { e = 1, id = "t1", t = "Thread", s = "completed", items = items } })
assert(shownCount("Working…") == 0, shownCount("Working…") .. " Working… lines after the run finished")
return "ok"
`);
});

// Scroll frames that keep their offset and child height, with a 300 px view; the
// conversation's one is the frame the copy box hides with. A child's top is at TOP (0
// unless a test sets it).
const SCROLLING = `
local createFrame = CreateFrame
function CreateFrame(kind, ...)
  local f = createFrame(kind, ...)
  if kind == "ScrollFrame" then
    f.offset = 0
    function f:SetScrollChild(child)
      self.child = child
      function child:SetHeight(h) self.h = h end
      function child:GetTop() return TOP or 0 end
    end
    function f:GetHeight() return 300 end
    function f:GetVerticalScrollRange() return math.max(0, (rawget(self.child, "h") or 0) - 300) end
    function f:SetVerticalScroll(v) self.offset = v end
    function f:GetVerticalScroll() return self.offset end
    function f:HookScript(event) if event == "OnHide" then messageScroll = self end end
  end
  return f
end
`;

test("a thread opens at the top of its own messages, not where the last long thread was scrolled", () => {
  run(`
local long = { e = 1, id = "t1", t = "Long", s = "completed", set = false, at = time(), p = "" }
local fresh = { e = 1, id = "t2", t = "Fresh", s = "running", set = false, at = time(), p = "" }
inboxWith(ENVS, { threads = { long, fresh }, view = { e = 1, id = "t1", t = "Long", s = "completed", items = { { r = "a", x = "Hi" } } } })
UI.Select(long)
-- As the game lays out a long conversation, read at its end.
messageScroll.child.h = 5000
messageScroll:SetVerticalScroll(4700)
-- Its messages aren't in yet: Loading… shows at the top.
UI.Select(fresh)
assert(messageScroll.offset == 0, "loading scrolled to " .. messageScroll.offset)
assert(messageScroll.child.h < 300, "loading left the old height " .. messageScroll.child.h)
-- In, but nothing to draw.
fresh.s = "completed"
inboxWith(ENVS, { threads = { long, fresh }, view = { e = 1, id = "t2", t = "Fresh", s = "completed", items = {} } })
messageScroll.child.h, messageScroll.offset = 5000, 4700
UI.Refresh()
assert(messageScroll.child.h < 300 and messageScroll.offset == 0, "empty thread kept " .. messageScroll.child.h .. "/" .. messageScroll.offset)
return "ok"
`, { before: SCROLLING });
});

test("a short thread measured at a long message's height settles back to its messages, not empty space", () => {
  run(`
local thread = { e = 1, id = "t1", t = "Short", s = "completed", set = false, at = time(), p = "" }
inboxWith(ENVS, { threads = { thread }, view = { e = 1, id = "t1", t = "Short", s = "completed",
  items = { { r = "u", x = "Fix the label" }, { r = "a", x = "Done." } } } })
UI.Select(thread)
-- Where the game says the last message ends.
TOP, BOTTOM = 1000, 1000
for _, fs in ipairs(fontStrings) do fs.GetBottom = function() return BOTTOM end end
-- Measured as it is drawn, a string still has the long message it held before.
BOTTOM = -4000
UI.Refresh()
assert(messageScroll.offset > 4000, "not at the end it measured: " .. messageScroll.offset)
-- A frame later the text is laid out: two short messages, which fit the view.
BOTTOM = 880
tick(0.25)
assert(messageScroll.child.h < 300, "kept the stale height " .. messageScroll.child.h)
assert(messageScroll.offset == 0, "scrolled past the messages to " .. messageScroll.offset)
-- Read from the middle of a long conversation, a shorter one keeps the view within it...
BOTTOM = -4000
tick(0.25)
messageScroll:SetVerticalScroll(1000)
BOTTOM = 500
tick(0.25)
assert(messageScroll.offset == messageScroll.child.h - 300, "past the end at " .. messageScroll.offset)
-- ...and a longer one leaves your place alone.
messageScroll:SetVerticalScroll(100)
BOTTOM = -2000
tick(0.25)
assert(messageScroll.offset == 100, "moved you to " .. messageScroll.offset)
return "ok"
`, { before: SCROLLING });
});

test("a picture draws from its image slot among the messages, and a click enlarges it instead of copying", () => {
  run(SHOWN + `
local thread = { e = 1, id = "t1", t = "Thread", s = "completed", set = false, at = time(), p = "" }
local items = { { r = "u", x = "chart it" }, { r = "a", x = "[Page: Sales]", im = { f = 7, w = 728, h = 300 } }, { r = "a", x = "Done." } }
inboxWith(ENVS, { threads = { thread }, view = { e = 1, id = "t1", t = "Thread", s = "completed", items = items } })
UI.Select(thread)
local SLOT = ROOT .. "img\\\\0007.tga"
local function picture()
  for _, f in ipairs(frames) do
    local zoomed = T3WoWPictureZoom and T3WoWPictureZoom.picture
    if f.kind == "Texture" and f.shown and f.texturePath == SLOT and f.point and f ~= zoomed then return f end
  end
end
local tex = picture()
assert(tex, "no picture from image slot 7")
assert(not shownString("Page: Sales"), "its text shows too")
assert(below(tex, shownString("chart it")), "not below the message before it")
assert(below(shownString("Done%."), tex), "the next message isn't below it")
assert(tex.width ~= 0 and math.abs(tex.height / tex.width - 300 / 728) < 0.001, "out of shape")
-- Pixel for pixel: on a 1080-pixel screen at UI scale 1 a UI unit is 1080/768 pixels, so a
-- 728-pixel picture is 728 * 768/1080 units wide in a window with room for it; enlarged,
-- twice that.
for _, f in ipairs(frames) do rawset(f, "GetEffectiveScale", function() return 1 end) end
rawset(T3WoWFrame, "GetWidth", function() return 1600 end)
rawset(UIParent, "GetWidth", function() return 1365 end)
rawset(UIParent, "GetHeight", function() return 768 end)
UI.Refresh()
tex = picture()
assert(math.abs(tex.width - 728 * 768 / 1080) < 0.01, "not pixel for pixel: " .. tostring(tex.width))
-- A click enlarges it; a click closes it again.
local hit
for _, f in ipairs(frames) do if f.kind == "Button" and f.shown and rawget(f, "picture") then hit = f end end
assert(hit and hit.picture.f == 7, "the picture can't be clicked")
hit.scripts.OnClick(hit, "LeftButton")
assert(T3WoWPictureZoom.shown, "not enlarged")
assert(T3WoWPictureZoom.picture.texturePath == SLOT, "another picture enlarged")
rawset(T3WoWPictureZoom, "GetEffectiveScale", function() return 1 end)
hit.scripts.OnClick(hit, "LeftButton")
assert(math.abs(T3WoWPictureZoom.picture.width - 2 * 728 * 768 / 1080) < 0.01, "not twice its size: " .. tostring(T3WoWPictureZoom.picture.width))
for _, f in ipairs(frames) do assert(not (f.kind == "EditBox" and f.shown and f.text == "[Page: Sales]"), "copied instead") end
T3WoWPictureZoom.scripts.OnClick(T3WoWPictureZoom, "LeftButton")
assert(not T3WoWPictureZoom.shown, "didn't close")
-- The same string as text again (another thread, a picture not drawn yet): it copies.
items = { { r = "u", x = "chart it" }, { r = "a", x = "[Page: Sales] (drawing it for the game…)" } }
inboxWith(ENVS, { threads = { thread }, view = { e = 1, id = "t1", t = "Thread", s = "completed", items = items } })
assert(not picture(), "the picture stayed")
assert(shownString("drawing it"), "the text doesn't show")
for _, f in ipairs(frames) do assert(not (f.kind == "Button" and f.shown and rawget(f, "picture")), "still enlarges") end
return "ok"
`);
});

test("a delegated task draws as a card among the messages: title, status and the start of its result", () => {
  run(SHOWN + `
local thread = { e = 1, id = "t1", t = "Thread", s = "completed", set = false, at = time(), p = "" }
local card = { r = "a", x = "Sol review (round 1) (finished)\\nVERDICT: APPROVE", d = { t = "Sol review (round 1)", s = "completed", at = time(), b = "VERDICT: APPROVE" } }
local items = { { r = "u", x = "review it" }, card, { r = "a", x = "Back after." } }
inboxWith(ENVS, { threads = { thread }, view = { e = 1, id = "t1", t = "Thread", s = "completed", items = items } })
UI.Select(thread)
local head, state, body = shownString("^Sol review %(round 1%)$"), shownString("^|cff40c040Finished|r  "), shownString("^VERDICT: APPROVE$")
assert(head and state and body, "the card's title, status or result doesn't show")
assert(not shownString("%(finished%)"), "the text for older addons shows too")
assert(head.fontObject == ns.Skin.fonts.strong, "the title isn't in the strong font")
assert(body.fontObject == ns.Skin.fonts.muted, "the result isn't muted")
assert(state.text:find(tostring(time()), 1, true), "no time it ended: " .. state.text)
assert(below(head, shownString("review it")), "not below the message before it")
assert(below(body, head), "the result isn't under the title")
assert(below(shownString("Back after"), body), "the next message isn't below the card")
-- Clicking it copies the text.
local hit
for _, f in ipairs(frames) do if f.kind == "Button" and f.shown and rawget(f, "copyText") == card.x then hit = f end end
assert(hit, "the card can't be clicked to copy")
-- Still running: no result yet, and the next message hangs from the title.
card.d = { t = "Sol review (round 1)", s = "running", b = "" }
inboxWith(ENVS, { threads = { thread }, view = { e = 1, id = "t1", t = "Thread", s = "completed", items = items } })
head = shownString("^Sol review %(round 1%)$")
assert(shownString("^|cffffd100Working|r$"), "a running task doesn't say Working")
assert(below(shownString("Back after"), head), "the next message isn't below the card")
-- Gone: the strings it used are plain messages again.
items = { { r = "u", x = "review it" }, { r = "a", x = "Plain again" }, { r = "a", x = "Back after." } }
inboxWith(ENVS, { threads = { thread }, view = { e = 1, id = "t1", t = "Thread", s = "completed", items = items } })
assert(not shownString("Sol review") and not shownString("cffffd100Working"), "the card stayed")
assert(shownString("Plain again").fontObject == ns.Skin.fonts.text, "a message after a card is muted")
return "ok"
`);
});

test("the strip tells the companion the game can show pictures only when the image slots were there at startup", () => {
  const frameWith = (before) =>
    runGame(
      `drawn = {}
local json = ns.Codec.Json
ns.Codec.Json = function(value) local out = json(value); drawn[#drawn + 1] = out; return out end
Link.Init()
Link.Render()
return tostring(Link.Images()) .. " " .. drawn[#drawn]`,
      { before },
    );
  const on = frameWith("");
  assert.ok(on.startsWith("true ") && on.includes('"im":1'), on);
  assert.ok(on.includes('"dc":1'), "the companion isn't told the game draws delegated tasks as cards");
  const off = frameWith(`missingTextures[ROOT .. "img\\\\here.tga"] = true`);
  assert.ok(off.startsWith("false ") && !off.includes('"im"'), off);
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

test("a table's columns are padded to one width, and cut down to fit a narrow window", () => {
  const out = run(`
local rows = { { "Name", "Size", "Note" }, { "a|b", "10", "ok" }, { "longer name", "2", "" } }
local function lines(...) return table.concat(UI.TableLines(...), "\\n") end
return table.concat({
  lines(rows, "lrc", 80),
  lines(rows, "lrc", 20),
  lines({ { "café", "b" }, { "x", "y" } }, nil, 80),
  lines({ { "abcdef", "ghijkl", "mnop" }, { "a", "b", "c" } }, nil, 10),
  lines({ { "abcdef", "ghijkl" }, { "a", "b" } }, nil, 6),
  lines({ { "abcdef", "ghijkl" }, { "a", "b" } }, nil, 4),
}, "\\n~\\n")
`);
  const [wide, narrow, accents, floor, six, four] = out.split("\n~\n");
  assert.deepEqual(wide.split("\n"), [
    "Name        │ Size │ Note",
    "────────────┼──────┼─────",
    "a|b         │   10 │  ok",
    "longer name │    2 │",
  ]);
  // 20 characters: the widest column gives way, its long cell cut short.
  assert.deepEqual(narrow.split("\n"), [
    "Name   │ Size │ Note",
    "───────┼──────┼─────",
    "a|b    │   10 │  ok",
    "longe… │    2 │",
  ]);
  for (const line of narrow.split("\n")) assert.ok([...line].length <= 20, line);
  // Characters are counted, not bytes.
  assert.deepEqual(accents.split("\n"), ["café │ b", "─────┼──", "x    │ y"]);
  // Columns keep 3 characters; past that, the last ones give way to a "…" column.
  assert.deepEqual(floor.split("\n"), ["ab… │ …", "────┼──", "a   │ …"]);
  for (const line of floor.split("\n")) assert.ok([...line].length <= 10, line);
  // Narrower still, the one column left is cut to fit; with no room for it, "…" alone.
  assert.deepEqual(six.split("\n"), ["a… │ …", "───┼──", "a  │ …"]);
  assert.deepEqual(four.split("\n"), ["…", "─", "…"]);
});

test("code, inline code and tables are drawn in the monospace font, and copying still takes the text", () => {
  run(SHOWN + `
local mono, text = ns.Skin.fonts.mono, ns.Skin.fonts.text
function shownExactly(wanted)
  for _, fs in ipairs(fontStrings) do
    if not fs.hidden and fs.text == wanted then return fs end
  end
end
local thread = { e = 1, id = "t1", t = "Thread", s = "completed", set = false, at = time(), p = "" }
local x = "Run |cffc8c8a0npm test|r:\\n|cffc8c8a0ok || fail|r\\n|| k || v ||"
local g = { "Run \\1npm test\\2:", { c = "ok | fail" }, { t = { { "k", "v" }, { "a", "1" } } }, "Done." }
local items = { { r = "u", x = "see |cffc8c8a0x|r", g = { "see \\1x\\2" } }, { r = "a", x = x, g = g } }
inboxWith(ENVS, { threads = { thread }, view = { e = 1, id = "t1", t = "Thread", s = "completed", items = items } })
UI.Select(thread)
-- Prose with inline code: a string per run, in a frame as tall as its lines.
local run, chip = shownExactly("Run"), shownExactly("npm test")
assert(run and run.fontObject == text, "the prose run isn't in the text font")
assert(chip and chip.fontObject == mono, "inline code isn't in the monospace font")
assert(shownExactly(":") and shownExactly(":").fontObject == text, "the text after the code is missing")
local holder = run.point[2]
assert(chip.point[2] == holder, "the inline code isn't laid out with its line")
-- The code block, then the table, then the closing prose, each below the one before.
local code = shownExactly("ok || fail")
assert(code and code.fontObject == mono, "the code block isn't in the monospace font")
assert(below(code, holder), "the code block isn't below the prose")
local tbl = shownString("│")
assert(tbl and tbl.fontObject == mono, "the table isn't in the monospace font")
assert(tbl.text:find("k |r", 1, true) and tbl.text:find("\\na |cff%x+│|r 1$"), tbl.text)
assert(below(tbl, code), "the table isn't below the code")
assert(below(shownExactly("Done."), tbl), "the prose after the table isn't below it")
-- Your message's inline code too.
assert(shownExactly("x") and shownExactly("x").fontObject == mono, "your inline code isn't monospace")
assert(not shownString("^see |c"), "your message was drawn from its text as well as its segments")
-- Clicking copies the message's text, as before.
local hit
for _, f in ipairs(frames) do
  if f.kind == "Button" and f.copyText == x and f.shown then hit = f end
end
assert(hit, "the message can't be clicked")
hit.scripts.OnClick(hit, "LeftButton")
local copied
for _, f in ipairs(frames) do
  if f.kind == "EditBox" and f.text == "Run npm test:\\nok | fail\\n| k | v |" then copied = f end
end
assert(copied, "the copy box doesn't hold the message's text")
-- Drawn again (a new inbox), the pieces are reused, not piled up.
local count = #fontStrings
inboxWith(ENVS, { threads = { thread }, view = { e = 1, id = "t1", t = "Thread", s = "completed", items = items } })
assert(#fontStrings == count, "a redraw made new strings: " .. (#fontStrings - count))
-- Without segments (an older companion), the message is one string again.
inboxWith(ENVS, { threads = { thread }, view = { e = 1, id = "t1", t = "Thread", s = "completed", items = { { r = "a", x = x } } } })
assert(shownExactly(x), "the text isn't drawn without segments")
assert(not shownExactly("ok || fail"), "the code block stayed")
return "ok"
`);
});

test("every segment the companion's fixtures make draws in the game", () => {
  const cases = JSON.parse(readFileSync(new URL("../companion/tests/fixtures/segments.json", import.meta.url), "utf8"));
  for (const { input, segments } of cases.filter((c) => c.segments.length > 0)) {
    run(SHOWN + `
local thread = { e = 1, id = "t1", t = "Thread", s = "completed", set = false, at = time(), p = "" }
local g = ${luaValue(segments)}
for _, mine in ipairs({ false, true }) do
  local items = { { r = mine and "u" or "a", x = "text", g = g } }
  inboxWith(ENVS, { threads = { thread }, view = { e = 1, id = "t1", t = "Thread", s = "completed", items = items } })
  UI.Select(thread)
  for _, seg in ipairs(g) do
    if type(seg) == "table" and seg.c then
      local escaped, found = seg.c:gsub("|", "||"), false
      for _, fs in ipairs(fontStrings) do
        if not fs.hidden and fs.text == escaped and fs.fontObject == ns.Skin.fonts.mono then found = true end
      end
      assert(found, "code not drawn: " .. seg.c)
    end
  end
  assert(not showing("^text$"), "drawn from its text instead")
end
return "ok"
`);
    assert.ok(true, input);
  }
});

test("Settings picks the theme, which applies at once and is saved", () => {
  {
    run(`
local Skin = ns.Skin
assert(Skin.ThemeKey() == "t3", "doesn't start in T3")
UI.ShowSettings()
assert(menuTexts("T3WoWThemeMenu") == "T3|Flat|Classic", menuTexts("T3WoWThemeMenu"))
assert(menuEntries("T3WoWThemeMenu")[1].checked, "T3 isn't checked")
pick("T3WoWThemeMenu", "Classic")
assert(Skin.ThemeKey() == "classic" and T3WoWDB.settings.theme == "classic", "Classic not applied or not saved")
assert(shown("T3WoWThemeMenu") == "Classic", shown("T3WoWThemeMenu"))
assert(menuEntries("T3WoWThemeMenu")[3].checked, "Classic isn't checked")
-- The window still draws a thread in it.
inboxWith(ENVS, { threads = { { e = 1, id = "t1", t = "Classic thread", s = "completed", set = false, at = time(), p = "" } } })
assert(showing("Classic thread"), "the list didn't draw")
return "ok"
`);
  }
});

test("the saved theme is drawn from login, and an unknown one falls back to T3", () => {
  run(`
T3WoWDB.settings = { theme = "flat" }
fire("PLAYER_LOGIN")
assert(ns.Skin.ThemeKey() == "flat", ns.Skin.ThemeKey())
T3WoWDB.settings.theme = "neon"
fire("PLAYER_LOGIN")
assert(ns.Skin.ThemeKey() == "t3", ns.Skin.ThemeKey())
return "ok"
`);
});

test("Flat takes ElvUI's colors and font when ElvUI runs", () => {
  run(`
ElvUI = { { media = { bordercolor = { 0.1, 0.2, 0.3 }, backdropcolor = { 0.05, 0.05, 0.05 },
  rgbvaluecolor = { r = 1, g = 0.5, b = 0 }, normFont = "ElvUI-PT-Sans.ttf" }, mult = 0.71 } }
ns.Skin.Set("flat")
local r, g, b = ns.Skin.Color("border")
assert(r == 0.1 and g == 0.2 and b == 0.3, "border isn't ElvUI's")
r, g, b = ns.Skin.Color("accent")
assert(r == 1 and g == 0.5 and b == 0, "accent isn't ElvUI's")
assert(ns.Skin.px == 0.71, "pixel size isn't ElvUI's")
assert(ns.Skin.Theme().fonts.regular == "ElvUI-PT-Sans.ttf", "font isn't ElvUI's")
-- Other themes ignore it.
ns.Skin.Set("t3")
r = ns.Skin.Color("border")
assert(r ~= 0.1, "T3 took ElvUI's border")
return "ok"
`);
});

test("the compact list puts each thread on one line, its machine in the tooltip", () => {
  run(`
local thread = { e = 1, id = "t1", t = "Fix the build", s = "completed", set = false, at = time(), p = "web" }
inboxWith(ENVS, { threads = { thread } })
assert(showing("^PC · web$"), "comfortable rows show the machine and project")
UI.ShowSettings()
-- The Compact thread list check is the first in Settings.
local compact
for _, f in ipairs(frames) do
  if f.kind == "CheckButton" and f.get then compact = f break end
end
compact.GetChecked = function() return true end
compact.scripts.OnClick(compact)
assert(T3WoWDB.settings.compact == true, "not saved")
assert(not showing("^PC · web$"), "compact rows still show the machine line")
assert(showing("Fix the build"), "compact rows lost the title")
return "ok"
`);
});

test("after an update installed while the game ran, the window says to restart instead of breaking", () => {
  const out = runGame(`
UI.Toggle()
UI.Toggle()
-- The rest of the addon can still call into the window.
UI.OnInbox({ k = "c1", envs = {}, threads = {} })
UI.Refresh()
return tostring(#printed) .. " " .. printed[1]
`, { ui: true, theme: false, before: `printed = {}; function print(text) printed[#printed + 1] = text end` });
  assert.match(out, /^1 .*restart WoW to load it/);
});

test("a notification plays T3 Code's sound for its outcome, and the whisper sound until the game has found it", () => {
  const out = runGame(`
played = {}
local function alert(status) UI.Alert({ id = "t9", e = "local", s = status, t = "Other thread" }) end
alert("completed")
alert("failed")
alert("approval")
alert("waiting")
soundsFound = false
alert("completed")
T3WoWDB.settings.sound = false
alert("completed")
return table.concat(played, " ")
`, { ui: true, before: `
soundsFound = true
SOUNDKIT = { TELL_MESSAGE = 3081 }
function PlaySoundFile(path, channel) played[#played + 1] = path:match("Media.(.+)$") .. "@" .. channel; return soundsFound end
function PlaySound(kit) played[#played + 1] = "kit" .. kit end` });
  assert.equal(out, [
    "notification-completion.mp3@SFX", "notification-completion.mp3@SFX",
    "notification-input.mp3@SFX", "notification-input.mp3@SFX",
    "notification-completion.mp3@SFX", "kit3081",
  ].join(" "));
});

test("the pill's Reload UI soon is a click that reloads, and only while it says so", () => {
  const out = run(`
local function pillHit()
  for _, f in ipairs(frames) do
    if f.kind == "Button" and f.parent == T3WoWFrame and f.scripts.OnDragStart then return f end
  end
end
inboxWith(ENVS)
assert(showing("^|cff40c040Connected|r$"), "not connected")
assert(not pillHit().shown, "clickable while connected")
Link.state.nextSlot = 192
UI.Refresh()
assert(showing("191/200 updates used: Reload UI soon"), "no Reload UI soon")
assert(pillHit().shown, "Reload UI soon isn't clickable")
pillHit().scripts.OnClick(pillHit(), "LeftButton")
return tostring(reloads)
`, { before: `reloads = 0; function ReloadUI() reloads = reloads + 1 end` });
  assert.equal(out, "1");
});

// Presses Ctrl+V in the message box. `pasted` is the text the game pastes, if any.
const CTRL_V = `
function ctrlV(pasted)
  local box, later = composer(), nil
  local after = C_Timer.After
  C_Timer.After = function(_, fn) later = fn end
  controlDown = true
  box.scripts.OnKeyDown(box, "V")
  controlDown = false
  C_Timer.After = after
  if pasted then box.text = box.text .. pasted end
  later()
end
function pasteChips()
  local chips = {}
  for _, f in ipairs(frames) do
    if f.kind == "Button" and rawget(f, "image") ~= nil and f.shown then chips[#chips + 1] = f end
  end
  return chips
end
THREAD = { e = 2, id = "t1", t = "Mac thread", s = "completed", set = false, at = time(), p = "" }
`;

test("Ctrl+V that pastes no text asks the companion for the clipboard's image, which shows above the box and goes with Send", () => {
  run(`${CTRL_V}
inboxWith(ENVS, { pi = true, threads = { THREAD } })
UI.Select({ e = 2, id = "t1" })
-- Text on the clipboard: the game pasted it, nothing to ask for.
local before = #queue()
ctrlV("some text")
assert(#queue() == before, "asked for an image after text was pasted")
-- Nothing pasted: the clipboard may hold an image.
typeText("what's this?")
ctrlV()
assert(sent().t == "paste" and sent().d == "mac:t1", tostring(sent().t) .. " " .. tostring(sent().d))
assert(showing("Sending…"))
-- The companion has it: drawn small above the box, from its image slot; New Thread's isn't.
local att = { { d = "mac:t1", i = 1, w = 1920, h = 1080, f = 3, tw = 142, th = 80 }, { d = "new", i = 2, w = 4, h = 4 } }
inboxWith(ENVS, { pi = true, threads = { THREAD }, att = att })
local chips = pasteChips()
assert(#chips == 1 and chips[1].image.i == 1, #chips)
assert(chips[1].picture.texturePath == ROOT .. "img\\\\0003.tga", tostring(chips[1].picture.texturePath))
assert(chips[1].picture.shown, "no small picture")
-- Send takes it along (the companion attaches what is pasted for the thread); images
-- alone are a message too.
typeText("")
click("Send")
assert(sent().t == "send" and sent().id == "t1" and sent().m == "", tostring(sent().m))
assert(showing("Sending…"))
-- New Thread shows its own, by number when the game can't show pictures.
click("New Thread")
chips = pasteChips()
assert(#chips == 1 and chips[1].image.i == 2, #chips)
assert(not chips[1].picture.shown and chips[1].label.text == "Image 1", tostring(chips[1].label.text))
-- × takes one back.
chips[1].remove.scripts.OnClick(chips[1].remove)
assert(sent().t == "unpaste" and sent().d == "new" and sent().i == 2, tostring(sent().t))
inboxWith(ENVS, { pi = true, threads = { THREAD }, att = { att[1] } })
assert(#pasteChips() == 0, "still shows the image taken back")
click("Start")
assert(#queue() == 0, "started with nothing typed and no image")
return "ok"
`);
});

test("Ctrl+V asks for an image only from a companion that pastes, and a refused paste says why", () => {
  run(`${CTRL_V}
-- An older companion: Ctrl+V only pastes text, as before.
inboxWith(ENVS, { threads = { THREAD } })
UI.Select({ e = 2, id = "t1" })
local before = #queue()
ctrlV()
assert(#queue() == before, "asked an older companion to paste")
assert(not showing("Ctrl%+V pastes images"))
-- A companion that pastes: the box says so, and a refusal comes back to the window.
inboxWith(ENVS, { pi = true, threads = { THREAD } })
assert(showing("Ctrl%+V pastes images too"))
ctrlV()
local s = sent().s
inboxWith(ENVS, { pi = true, threads = { THREAD }, out = { { s = s, ok = false, x = "T3 WoW pastes only right after you press Ctrl+V in its message box" } } })
assert(showing("Not pasted: T3 WoW pastes only right after"), "refusal not shown")
assert(not showing("your text is back"))
-- Read-only in the tray: not asked, and the window says why.
inboxWith(ENVS, { pi = true, trust = "readonly", threads = { THREAD } })
before = #queue()
ctrlV()
assert(#queue() == before)
assert(showing("Pasting images from the game is off"))
-- Eight is the most a message takes.
local att = {}
for i = 1, 8 do att[i] = { d = "mac:t1", i = i, w = 4, h = 4 } end
inboxWith(ENVS, { pi = true, threads = { THREAD }, att = att })
ctrlV()
assert(#queue() == 0)
assert(showing("A message takes 8 images at most"))
return "ok"
`);
});

test("clicking the resize grip without moving keeps the size, and the conversation hides only while you drag", () => {
  run(`
local window = T3WoWFrame
-- The window as the game places it: 940 x 580, its top-left corner at (100, 900).
local left, top = 100, 900
window.width, window.height = 940, 580
function window:GetWidth() return self.width end
function window:GetHeight() return self.height end
function window:GetLeft() return left end
function window:GetTop() return top end
function window:GetEffectiveScale() return 1 end
function window:GetPoint() return unpack(self.point) end
function UIParent:GetWidth() return 1920 end
-- Like the game, a new size runs OnSizeChanged; the conversation's width shows when it was laid out.
function window:SetSize(w, h)
  self.width, self.height = w, h
  self.scripts.OnSizeChanged(self, w, h)
end
local laidOutAt
function messageScroll.child:SetWidth(w) laidOutAt = w end
local cursorX, cursorY, held = 1032, 330, true
function GetCursorPosition() return cursorX, cursorY end
function IsMouseButtonDown() return held end
local grip
for _, f in ipairs(frames) do
  if rawget(f, "glyph") and f.scripts.OnMouseDown then grip = f end
end
assert(grip, "no grip")
grip.scripts.OnMouseDown(grip, "LeftButton")
tick(0.25)
assert(window.width == 940 and window.height == 580, "a click resized to " .. window.width .. "x" .. window.height)
assert(window.point[1] == "TOPLEFT" and window.point[4] == 100 and window.point[5] == 900, "the top-left corner moved")
assert(not messageScroll.shown, "the conversation shows while dragging")
-- Dragged 60 right and 40 down; then past the smallest size.
cursorX, cursorY = 1092, 290
tick(0.25)
assert(window.width == 1000 and window.height == 620, "dragged to " .. window.width .. "x" .. window.height)
assert(laidOutAt == nil, "the conversation was laid out mid-drag, at " .. tostring(laidOutAt))
-- The companion's updates keep coming while you drag: they don't bring the conversation back.
UI.Refresh()
assert(not messageScroll.shown, "a refresh showed the conversation mid-drag")
cursorX, cursorY = 0, 2000
tick(0.25)
assert(window.width == 720 and window.height == 420, "under the smallest size: " .. window.width .. "x" .. window.height)
cursorX, cursorY = 1092, 290
tick(0.25)
held = false
grip.scripts.OnMouseUp(grip, "LeftButton")
assert(messageScroll.shown, "the conversation stays hidden after the drag")
assert(laidOutAt == 1000 - 300 - 46, "not laid out at the new width: " .. tostring(laidOutAt))
assert(T3WoWDB.window.w == 1000 and T3WoWDB.window.h == 620 and T3WoWDB.window.point == "TOPLEFT", "the size wasn't saved")
return "ok"
`, { before: SCROLLING, db: "{}" });
});

// What the companion says each machine offers, as inbox.models carries it.
const SELECTOR_HELPERS = `
CATALOG = {
  { e = 1, id = "claudeAgent", t = "Claude", m = {
    { id = "opus", t = "Claude Opus", o = "effort", ef = { "low", "high", "xhigh" } },
    { id = "haiku", t = "Claude Haiku", o = "effort", ef = { "low" } },
  } },
  { e = 1, id = "grok", t = "Grok", rm = { "approval-required", "auto", "full-access" }, m = { { id = "grok-build", t = "Grok Build" } } },
  { e = 2, id = "codex", t = "Codex", m = { { id = "luna", t = "gpt-luna", o = "reasoningEffort", ef = { "medium" } } } },
}
SEL_ENVS = { { id = "local", name = "PC", ok = true, dm = "full-access" }, { id = "mac", name = "Mac", ok = true, dm = "approval-required" } }
function segments()
  local list = {}
  for _, f in ipairs(frames) do
    if f.parent == T3WoWPermissions and rawget(f, "mode") then list[#list + 1] = f end
  end
  return list
end
function segment(id)
  for _, s in ipairs(segments()) do if s.mode.id == id then return s end end
end
function lit()
  local on = {}
  for _, s in ipairs(segments()) do if s.lit.shown then on[#on + 1] = s.mode.id end end
  return table.concat(on, ",")
end
function openPermissions()
  if T3WoWPermissions.shown then T3WoWPermissions:Hide() end
  T3WoWPermissionMenu.scripts.OnClick(T3WoWPermissionMenu, "LeftButton")
end
`;

test("New Thread picks a model, effort and permissions, and leaves out what you left alone", () => {
  run(SELECTOR_HELPERS + `
inboxWith(SEL_ENVS, { sel = true, models = CATALOG })
click("New Thread")
assert(T3WoWModelMenu.shown and T3WoWPermissionMenu.shown, "the row is hidden")
assert(shown("T3WoWModelMenu") == "Default model", shown("T3WoWModelMenu"))
assert(not T3WoWEffortMenu.shown, "no model picked, so no efforts to list")
assert(shown("T3WoWPermissionMenu") == "Full access", "shows what a thread gets by default")
-- The list opens over the chip: the box is at the window's bottom.
assert(T3WoWModelMenu.menu.above, "should open upward")
assert(menuTexts("T3WoWModelMenu") == "Default model|Claude|Claude Opus|Claude Haiku|Grok|Grok Build", menuTexts("T3WoWModelMenu"))
-- Left alone: nothing about models or modes goes out, so T3's defaults apply.
typeText("as it comes")
click("Start")
local plain = sent()
assert(plain.t == "new" and plain.pv == nil and plain.md == nil and plain.ef == nil and plain.rm == nil, "sent a pick nobody made")

click("New Thread")
pick("T3WoWModelMenu", "Claude Opus")
assert(T3WoWEffortMenu.shown, "a picked model's efforts")
assert(shown("T3WoWEffortMenu") == "Default", shown("T3WoWEffortMenu"))
assert(menuTexts("T3WoWEffortMenu") == "Default|Low|High|Extra high", menuTexts("T3WoWEffortMenu"))
pick("T3WoWEffortMenu", "Extra high")
openPermissions()
segment("approval-required").scripts.OnClick(segment("approval-required"), "LeftButton")
assert(shown("T3WoWPermissionMenu") == "Supervised", shown("T3WoWPermissionMenu"))
typeText("carefully")
click("Start")
local picked = sent()
assert(picked.pv == "claudeAgent" and picked.md == "opus" and picked.ef == "xhigh" and picked.rm == "approval-required",
  table.concat({ tostring(picked.pv), tostring(picked.md), tostring(picked.ef), tostring(picked.rm) }, ","))
-- The message stays in its own field.
assert(picked.m == "carefully", picked.m)
return "ok"
`);
});

test("an effort another model doesn't have goes, and another machine starts over with its own models", () => {
  run(SELECTOR_HELPERS + `
inboxWith(SEL_ENVS, { sel = true, models = CATALOG })
click("New Thread")
pick("T3WoWModelMenu", "Claude Opus")
pick("T3WoWEffortMenu", "High")
-- Haiku only has low.
pick("T3WoWModelMenu", "Claude Haiku")
assert(shown("T3WoWEffortMenu") == "Default", shown("T3WoWEffortMenu"))
-- A model with no efforts has no effort to pick.
pick("T3WoWModelMenu", "Grok Build")
assert(not T3WoWEffortMenu.shown, "Grok has no efforts")
openPermissions()
segment("approval-required").scripts.OnClick(segment("approval-required"), "LeftButton")
pick("T3WoWMachineMenu", "Mac")
assert(shown("T3WoWModelMenu") == "Default model", shown("T3WoWModelMenu"))
assert(shown("T3WoWPermissionMenu") == "Supervised", "the Mac's default, not the pick made for the PC")
assert(menuTexts("T3WoWModelMenu") == "Default model|gpt-luna", menuTexts("T3WoWModelMenu"))
typeText("on the mac")
click("Start")
assert(sent().e == "mac" and sent().md == nil and sent().rm == nil, "a pick for the PC went to the Mac")
return "ok"
`);
});

test("the permissions popover lights the mode, says what each does, and dims what a provider can't run", () => {
  run(SELECTOR_HELPERS + `
inboxWith(SEL_ENVS, { sel = true, models = CATALOG })
click("New Thread")
openPermissions()
assert(T3WoWPermissions.shown, "didn't open")
assert(#segments() == 4, #segments())
assert(lit() == "full-access", lit())
assert(showing("Allow commands and edits without prompts"), "the lit mode's description")
-- Hovering one says what it does; leaving goes back to the lit one.
local edits = segment("auto-accept-edits")
edits.scripts.OnEnter(edits)
assert(showing("Apply edits without asking"), "hover text")
edits.scripts.OnLeave(edits)
assert(showing("Allow commands and edits without prompts"), "back to the lit mode")
-- It stays open as you pick, lighting the new one.
segment("auto").scripts.OnClick(segment("auto"), "LeftButton")
assert(T3WoWPermissions.shown and lit() == "auto", lit())
T3WoWPermissions:Hide()
-- Grok has no "auto-accept-edits": it can't be picked once Grok is the model.
pick("T3WoWModelMenu", "Grok Build")
openPermissions()
assert(segment("auto-accept-edits").enabled == false, "Grok can't run that mode")
assert(segment("auto").enabled == true and lit() == "auto", lit())
T3WoWPermissions:Hide()
-- Claude takes every mode; a pick Grok can't run goes back to the default on the way in.
pick("T3WoWModelMenu", "Claude Opus")
openPermissions()
segment("auto-accept-edits").scripts.OnClick(segment("auto-accept-edits"), "LeftButton")
T3WoWPermissions:Hide()
pick("T3WoWModelMenu", "Grok Build")
assert(shown("T3WoWPermissionMenu") == "Full access", shown("T3WoWPermissionMenu"))
return "ok"
`);
});

test("an open thread shows what it runs on: model and effort change, its permissions don't", () => {
  run(SELECTOR_HELPERS + `
local function view(cfg) return { e = 1, id = "t1", t = "A thread", s = "completed", items = {}, cfg = cfg } end
local thread = { { e = 1, id = "t1", t = "A thread", s = "completed", set = false, at = time(), p = "" } }
local opus = { pv = "claudeAgent", m = "opus", ef = "high", rm = "auto" }
inboxWith(SEL_ENVS, { sel = true, models = CATALOG, threads = thread })
UI.Select({ e = 1, id = "t1" })
assert(not T3WoWModelMenu.shown, "nothing to show until its read arrives")
inboxWith(SEL_ENVS, { sel = true, models = CATALOG, threads = thread, view = view(opus) })
assert(shown("T3WoWModelMenu") == "Claude Opus", shown("T3WoWModelMenu"))
assert(shown("T3WoWEffortMenu") == "High", shown("T3WoWEffortMenu"))
assert(shown("T3WoWPermissionMenu") == "Auto", shown("T3WoWPermissionMenu"))
-- A thread stays with its provider, as in T3, so there are no groups and no default row.
assert(menuTexts("T3WoWModelMenu") == "Claude Opus|Claude Haiku", menuTexts("T3WoWModelMenu"))
assert(menuTexts("T3WoWEffortMenu") == "Low|High|Extra high", menuTexts("T3WoWEffortMenu"))

-- Picking is sent at once and shows right away, ahead of the thread's next read.
pick("T3WoWEffortMenu", "Low")
local change = sent()
assert(change.t == "model" and change.e == "local" and change.id == "t1" and change.pv == "claudeAgent" and change.md == "opus" and change.ef == "low", "bad change")
assert(shown("T3WoWEffortMenu") == "Low", shown("T3WoWEffortMenu"))
inboxWith(SEL_ENVS, { sel = true, models = CATALOG, threads = thread, view = view(opus) })
assert(shown("T3WoWEffortMenu") == "Low", "fell back while the read caught up")
return "ok"
`);
});

test("a refused model change goes back and says why; one that lands stops showing as pending", () => {
  run(SELECTOR_HELPERS + `
local function view(cfg) return { e = 1, id = "t1", t = "A thread", s = "completed", items = {}, cfg = cfg } end
local thread = { { e = 1, id = "t1", t = "A thread", s = "completed", set = false, at = time(), p = "" } }
local opus = { pv = "claudeAgent", m = "opus", ef = "high", rm = "auto" }
local function update(cfg, out) inboxWith(SEL_ENVS, { sel = true, models = CATALOG, threads = thread, view = view(cfg), out = out }) end
update(opus)
UI.Select({ e = 1, id = "t1" })
update(opus)
pick("T3WoWEffortMenu", "Low")
local seq = sent().s
assert(type(seq) == "number", "no command number")
update(opus, { { s = seq, ok = false, x = "T3 said no" } })
assert(shown("T3WoWEffortMenu") == "High", shown("T3WoWEffortMenu"))
assert(showing("Model not changed: T3 said no"), "the reason")

-- Switching model keeps the effort when the new one has it, and drops it when it doesn't.
pick("T3WoWModelMenu", "Claude Haiku")
assert(sent().md == "haiku" and sent().ef == nil, "Haiku has no high")
local landed = { pv = "claudeAgent", m = "haiku", rm = "auto" }
update(landed)
assert(shown("T3WoWModelMenu") == "Claude Haiku" and shown("T3WoWEffortMenu") == "Default", shown("T3WoWEffortMenu"))
-- A later change on T3's side shows: nothing is held over.
update(opus)
assert(shown("T3WoWModelMenu") == "Claude Opus", shown("T3WoWModelMenu"))
-- A pick that never shows up stops being shown after a while.
pick("T3WoWModelMenu", "Claude Haiku")
update(opus)
assert(shown("T3WoWModelMenu") == "Claude Haiku", "too soon to give up")
clock = clock + 21
update(opus)
assert(shown("T3WoWModelMenu") == "Claude Opus", shown("T3WoWModelMenu"))
return "ok"
`);
});

test("a thread's permissions only show, with the way to change them in T3", () => {
  run(SELECTOR_HELPERS + `
local thread = { { e = 1, id = "t1", t = "A thread", s = "completed", set = false, at = time(), p = "" } }
local v = { e = 1, id = "t1", t = "A thread", s = "completed", items = {}, cfg = { pv = "claudeAgent", m = "opus", rm = "approval-required" } }
inboxWith(SEL_ENVS, { sel = true, models = CATALOG, threads = thread })
UI.Select({ e = 1, id = "t1" })
inboxWith(SEL_ENVS, { sel = true, models = CATALOG, threads = thread, view = v })
openPermissions()
assert(lit() == "approval-required", lit())
for _, s in ipairs(segments()) do assert(s.enabled == false, s.mode.id .. " can be picked") end
assert(showing("keeps the permissions it started with"), "doesn't say why")
local before = #queue()
segment("full-access").scripts.OnClick(segment("full-access"), "LeftButton")
assert(#queue() == before and lit() == "approval-required", "a thread's permissions changed from the game")
for _, f in ipairs(frames) do
  if f.kind == "Button" and f.parent == T3WoWPermissions and f.text == "Open in T3" then f.scripts.OnClick(f, "LeftButton") end
end
assert(sent().t == "t3" and sent().id == "t1", "didn't open T3")
assert(not T3WoWPermissions.shown, "stayed open")
return "ok"
`);
});

test("the tray's Read-only greys out changing the model; No new threads leaves it", () => {
  run(SELECTOR_HELPERS + `
local thread = { { e = 1, id = "t1", t = "A thread", s = "completed", set = false, at = time(), p = "" } }
local v = { e = 1, id = "t1", t = "A thread", s = "completed", items = {}, cfg = { pv = "claudeAgent", m = "opus", ef = "high", rm = "auto" } }
inboxWith(SEL_ENVS, { sel = true, models = CATALOG, threads = thread, trust = "readonly" })
UI.Select({ e = 1, id = "t1" })
inboxWith(SEL_ENVS, { sel = true, models = CATALOG, threads = thread, view = v, trust = "readonly" })
assert(T3WoWModelMenu.enabled == false and T3WoWEffortMenu.enabled == false, "still enabled")
inboxWith(SEL_ENVS, { sel = true, models = CATALOG, threads = thread, view = v, trust = "nonew" })
assert(T3WoWModelMenu.enabled == true and T3WoWEffortMenu.enabled == true, "No new threads still changes models")
return "ok"
`);
});

test("a companion that can't take the picks shows no row, and a game without the icon files shows words", () => {
  run(SELECTOR_HELPERS + `
-- An older companion: no inbox.sel, so a pick would be dropped and T3's default would run.
inboxWith(SEL_ENVS, {})
click("New Thread")
assert(not T3WoWModelMenu.shown and not T3WoWEffortMenu.shown and not T3WoWPermissionMenu.shown, "offered picks it would ignore")
-- A companion that reads no models still has the permissions to offer.
inboxWith(SEL_ENVS, { sel = true })
assert(not T3WoWModelMenu.shown and T3WoWPermissionMenu.shown, "permissions need no catalog")
for _, name in ipairs({ "lock", "pencil", "sparkle", "shield-off" }) do missingTextures[ROOT .. "Media\\\\" .. name] = true end
inboxWith(SEL_ENVS, { sel = true })
openPermissions()
local segment = segments()[1]
assert(segment.short.hidden == false and not segment.glyph.shown, "should fall back to its word")
assert(not T3WoWPermissionMenu.icon.shown, "no square on the chip")
return "ok"
`);
});
