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

// Scroll frames that keep their offset and child height, with a 300 px view; the
// conversation's one is the frame the copy box hides with.
const SCROLLING = `
local createFrame = CreateFrame
function CreateFrame(kind, ...)
  local f = createFrame(kind, ...)
  if kind == "ScrollFrame" then
    f.offset = 0
    function f:SetScrollChild(child)
      self.child = child
      function child:SetHeight(h) self.h = h end
    end
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
