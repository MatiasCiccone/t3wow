// Switching the companion to another T3 sign-in (`t3wow.exe login --origin`, or a sign-out):
// each update carries the sign-in's stamp (lid), and the addon drops what it had from the
// previous one, in the link (waiting commands, alerts) and in the window.
import assert from "node:assert/strict";
import { test } from "node:test";
import { runGame } from "./game.js";

// Loads `inbox` as the next slot right away; the frames the strip draws are kept in `drawn`.
const HELPERS = `
drawn = {}
local json = ns.Codec.Json
ns.Codec.Json = function(value)
  local out = json(value)
  drawn[#drawn + 1] = out
  return out
end
function load(inbox)
  Link.state.pollAt = 0
  inboxes[string.format("T3WoW_S%03d", Link.state.nextSlot)] = inbox
  tick(0.5)
end
function kinds()
  local out = {}
  for _, c in ipairs(Link.Queue()) do out[#out + 1] = c.t end
  return table.concat(out, ",")
end
function lastFrame() return drawn[#drawn] or "" end
`;

test("a new sign-in's stamp drops the previous one's waiting commands and alerts; the first one drops nothing", () => {
  runGame(`${HELPERS}
Link.Init()
Link.Send({ t = "send", id = "t1", m = "for the first T3" })
-- The first stamp this game sees (a fresh install, or an addon from before stamps): nothing to drop.
load({ k = "c1", u = Link.Session(), lid = "aaaa", ack = 0, now = time(), events = {
  { n = time() * 1000, e = "mac", id = "t1", t = "Fix tests", s = "completed" } } })
assert(T3WoWDB.login == "aaaa", tostring(T3WoWDB.login))
assert(forgotten == nil, "dropped on the first stamp")
assert(kinds() == "hello,send", kinds())
assert(#Link.Status().recentAlerts == 1)
-- The strip says which sign-in its commands were meant for.
Link.Render()
assert(lastFrame():find('"lid":"aaaa"', 1, true), lastFrame())

Link.Send({ t = "open", e = "mac", id = "t1" })
Link.Send({ t = "refresh" })
-- The companion moved to another T3.
load({ k = "c1", u = Link.Session(), lid = "bbbb", ack = 0, now = time() })
assert(forgotten == 1, "not dropped")
assert(T3WoWDB.login == "bbbb", tostring(T3WoWDB.login))
assert(kinds() == "hello,refresh", "kept " .. kinds())
assert(#Link.Status().recentAlerts == 0, "alerts kept")
assert(lastFrame():find('"lid":"bbbb"', 1, true), lastFrame())

-- A slot left over from before the move (another UI session, older): not shown, nothing dropped.
load({ k = "c1", u = "an earlier session", lid = "aaaa", ack = 0, now = time() - 600, threads = { { id = "old" } } })
assert(T3WoWDB.login == "bbbb" and forgotten == 1, "a leftover took over")
assert(Link.state.inbox.lid == "bbbb", "a leftover is shown")
-- An older companion's update (no stamp) changes nothing either.
load({ k = "c1", u = Link.Session(), ack = 0, now = time() })
assert(T3WoWDB.login == "bbbb" and forgotten == 1)
return "ok"
`);
});

test("the stamp keeps across /reload: a move while the game was closed drops the saved queue", () => {
  runGame(`${HELPERS}
Link.Init()
-- Saved at the last logout: a message for the first T3 that never went out.
assert(kinds() == "send,hello", kinds())
assert(lastFrame():find('"lid":"aaaa"', 1, true), "the saved stamp isn't drawn: " .. lastFrame())
-- The companion moved while the game was closed; its first update for this session.
load({ k = "c1", u = Link.Session(), lid = "cccc", ack = 0, now = time() })
assert(kinds() == "hello", kinds())
assert(forgotten == 1)
return "ok"
`, { db: `{ client = "c1", seq = 3, queue = { { s = 3, t = "send", id = "t1", m = "unsent" } }, login = "aaaa", loginAt = 1794000000 }` });
});

// The window, with the real UI.
const UI_HELPERS = `${HELPERS}
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
function said(pattern)
  local n = 0
  for _, line in ipairs(printed) do if line:find(pattern) then n = n + 1 end end
  return n
end
-- Shown text, leaving out the notifications that are gone (the simulated game hides a
-- frame but not the text on it). A notification is a frame with a head line of its own.
function onScreen(pattern)
  local gone = {}
  for _, f in ipairs(frames) do
    if rawget(f, "head") and not f.shown then gone[f.head], gone[f.title], gone[f.sub] = true, true, true end
  end
  for _, fs in ipairs(fontStrings) do
    if not gone[fs] and not fs.hidden and type(fs.text) == "string" and fs.text:find(pattern) then return true end
  end
  return false
end
function toastsShown()
  local n = 0
  for _, f in ipairs(frames) do if rawget(f, "head") and f.shown then n = n + 1 end end
  return n
end
Link.Init()
UI.Toggle()
`;

test("in the window, the previous T3's threads, open thread and notifications go; what you typed stays", () => {
  runGame(`${UI_HELPERS}
local MAC = { id = "mac", name = "Mac", ok = true }
local thread = { e = 1, id = "t1", t = "Mac thread", s = "running", set = false, at = time(), p = "" }
load({ k = "c1", u = Link.Session(), lid = "aaaa", ack = 0, now = time(), envs = { MAC }, threads = { thread },
  projects = {}, notes = {}, events = { { n = time() * 1000, e = "mac", id = "t1", t = "Mac thread", s = "completed" } } })
assert(showing("Mac thread"), "the first T3's thread isn't listed")
assert(said("connected to T3") == 1)
assert(toastsShown() == 1, "no notification for the first T3's thread")
UI.Select(thread)
load({ k = "c1", u = Link.Session(), lid = "aaaa", ack = 0, now = time(), envs = { MAC }, threads = { thread }, projects = {},
  notes = {}, events = {}, view = { e = 1, id = "t1", t = "Mac thread", s = "running", items = { { r = "a", x = "the first T3's answer" } } } })
assert(showing("the first T3's answer"), "the open thread isn't shown")
typeText("half a thought")

load({ k = "c1", u = Link.Session(), lid = "bbbb", ack = 0, now = time(), envs = { { id = "box", name = "Box", ok = true } },
  threads = { { e = 1, id = "b1", t = "Box thread", s = "completed", set = false, at = time(), p = "" } }, projects = {}, notes = {}, events = {} })
assert(showing("Box thread"), "the new T3's thread isn't listed")
assert(toastsShown() == 0, "the previous T3's notification still shows")
assert(not onScreen("Mac thread"), "the previous T3's thread still shows")
assert(not onScreen("the first T3's answer"), "the previous T3's open thread still shows")
assert(typed() == "half a thought", "the typed text went: " .. typed())
-- Nothing is selected: Send has nowhere to go.
local before = #Link.Queue()
click("Send")
assert(#Link.Queue() == before, "sent to the previous T3's thread")
assert(said("connected to T3") == 2, "the new T3 isn't announced")
return "ok"
`, { ui: true });
});
