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
function sent() return T3WoWDB.queue[#T3WoWDB.queue] end
local MAC = { id = "mac", name = "Mac", ok = true }
local LOCAL = { id = "local", name = "PC", ok = true }
function inboxWith(envs, extra)
  local data = { k = "c1", ack = 0, envs = envs, threads = {}, projects = {}, notes = {}, events = {}, now = time() }
  for _, t in ipairs(extra and extra.threads or {}) do data.threads[#data.threads + 1] = t end
  for _, p in ipairs(extra and extra.projects or {}) do data.projects[#data.projects + 1] = p end
  data.view, data.out = extra and extra.view, extra and extra.out
  UI.OnInbox(data)
end
ENVS = { LOCAL, MAC }
Link.Init()
UI.Toggle()
`;

const run = (script) => runGame(UI_HELPERS + script, { ui: true });

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
local before = #T3WoWDB.queue
click("Send")
assert(#T3WoWDB.queue == before, "sent to a machine that's gone")
assert(typed() == "still there?", typed())
return "ok"
`);
});

test("New Thread keeps the machine and project it was given, and refuses once they're gone", () => {
  run(`
local projects = { { e = 1, id = "pa", t = "alpha" }, { e = 2, id = "pm", t = "mac app" } }
inboxWith(ENVS, { projects = projects })
click("New Thread")
click("Machine: PC")
assert(find("Button", "Machine: Mac"), "machine didn't change")
click("Project: No project (scratch)")
-- The local machine drops out of the list: the Mac's project is still the one picked.
inboxWith({ ENVS[2] }, { projects = { { e = 1, id = "pm", t = "mac app" } } })
typeText("build it")
click("Start")
assert(sent().t == "new" and sent().e == "mac" and sent().p == "pm", "started " .. tostring(sent().e) .. "/" .. tostring(sent().p))
-- The project is removed: Start refuses and keeps the text.
inboxWith({ ENVS[2] })
typeText("again")
local before = #T3WoWDB.queue
click("Start")
assert(#T3WoWDB.queue == before, "started with a project that's gone")
assert(typed() == "again", typed())
return "ok"
`);
});
