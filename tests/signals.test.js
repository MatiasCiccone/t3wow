import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import fengari from "fengari";

const { lua, lauxlib, lualib, to_luastring } = fengari;
const read = (name) => readFileSync(new URL(`../addon/T3WoW/${name}`, import.meta.url), "utf8");

// Runs the real addon Link.lua in a small simulated game: frames whose OnUpdate we drive,
// fonts that load a while after SetFont (and only if the file was real when asked), slot
// addons whose Inbox we hand over, and two clocks (GetTime and time()) moving together.
const GAME = `
clock = 1000
EPOCH = 1794000000
LOAD_DELAY = 1.5
real = {}
firstAsked = {}
inboxes = {}
loads = {}
alerts = {}
function GetTime() return clock end
function time() return EPOCH + math.floor(clock - 1000) end
function GetPhysicalScreenSize() return 1920, 1080 end
function date(_, t) return tostring(t or time()) end
STANDARD_TEXT_FONT = "Fonts\\\\STANDARD.TTF"

-- Like the game (as traces showed): a font string whose font load failed never loads a
-- font afterwards, whatever it is asked for.
local function fontString()
  local fs = { font = STANDARD_TEXT_FONT, pending = false, poisoned = false }
  function fs:SetFont(path)
    if self.pending and not self.pending.real then self.poisoned = true end
    if path == STANDARD_TEXT_FONT then self.font, self.pending = path, false; return true end
    firstAsked[path] = firstAsked[path] or clock
    self.pending = { path = path, at = clock, real = real[path] }
  end
  function fs:GetFont()
    local p = self.pending
    if p and p.real and not self.poisoned and (clock - p.at >= LOAD_DELAY or clock - firstAsked[p.path] >= LOAD_DELAY) then
      self.font, self.pending = p.path, false
    end
    return self.font
  end
  return setmetatable(fs, { __index = function() return function() end end })
end

frames = {}
local function frame()
  local f = { scripts = {}, shown = true }
  function f:SetScript(name, fn) self.scripts[name] = fn end
  function f:CreateFontString() return fontString() end
  function f:Show() self.shown = true end
  function f:Hide() self.shown = false end
  function f:IsShown() return self.shown end
  frames[#frames + 1] = f
  return setmetatable(f, { __index = function() return function() return f end end })
end
function CreateFrame() return frame() end
UIParent, WorldFrame = frame(), frame()
C_AddOns = { LoadAddOn = function(name) loads[#loads + 1] = name; T3WoW_Inbox = inboxes[name]; return true end }

function tick(seconds)
  for _ = 1, math.floor(seconds / 0.25 + 0.5) do
    clock = clock + 0.25
    for _, f in ipairs(frames) do
      if f.scripts.OnUpdate and f.shown then f.scripts.OnUpdate(f, 0.25) end
    end
  end
end
`;

function runGame(script) {
  const L = lauxlib.luaL_newstate();
  lualib.luaL_openlibs(L);
  const chunk = `${GAME}
local ns = { UI = {
  OnInbox = function() end, Refresh = function() end, Notice = function() end,
  IsShown = function() return false end,
  Alert = function(event) alerts[#alerts + 1] = event end,
} }
assert(load(${JSON.stringify(read("Codec.lua"))}, "Codec.lua"))("T3WoW", ns)
assert(load(${JSON.stringify(read("Link.lua"))}, "Link.lua"))("T3WoW", ns)
T3WoWDB = { client = "c1", seq = 0, queue = {} }
Link = ns.Link
${script}`;
  if (lauxlib.luaL_loadstring(L, to_luastring(chunk)) !== lua.LUA_OK || lua.lua_pcall(L, 0, 1, 0) !== lua.LUA_OK) {
    throw new Error(lua.lua_tojsstring(L, -1));
  }
  return lua.lua_tojsstring(L, -1);
}

test("a slow-loading signal font after five quiet minutes still brings the update in", () => {
  const result = runGame(`
real["Interface\\\\AddOns\\\\T3WoW\\\\ctl\\\\probe\\\\valid.ttf"] = true
Link.Init()
-- Fonts check out, then the first polled slot names the wait: file 100, starting now.
inboxes.T3WoW_S001 = { k = "c1", ack = 1, ns = 100, nt = time() }
tick(6)
assert(Link.Status().signals == "on", "signals " .. tostring(Link.Status().signals))
assert(#loads == 1, "polled loads " .. #loads)

-- Five quiet minutes: no slot is loaded, the watched file moves along with the clock.
tick(300)
assert(#loads == 1, "loaded while idle: " .. #loads)
local watched = Link.Status().nextSignal
assert(watched == 100 + math.floor((time() - (EPOCH + 6)) / 25), "watching " .. watched)

-- The companion publishes: the file it computed and its neighbours become real.
for n = watched - 1, watched + 1 do real[string.format("Interface\\\\AddOns\\\\T3WoW\\\\sig\\\\%04d.ttf", n)] = true end
inboxes.T3WoW_S002 = { k = "c1", ack = 1, ns = watched + 2, nt = time(), events = { { n = 7, e = "", id = "t1", t = "Done", s = "completed" } } }
local before = clock
for _ = 1, 40 do
  tick(0.25)
  if #alerts > 0 then break end
end
assert(#alerts == 1, "no alert after " .. (clock - before) .. "s")
assert(loads[2] == "T3WoW_S002", tostring(loads[2]))
assert(Link.Status().recentAlerts[1].via == "a signal", Link.Status().recentAlerts[1].via)
local trace = table.concat(Link.Trace(), " | ")
assert(trace:find("signal %d+ noticed"), trace)
assert(trace:find("load T3WoW_S002 via a signal"), trace)
return string.format("%.2f", clock - before)
`);
  // Load delay 1.5 s plus at most one check interval.
  assert.ok(Number(result) <= 2.5, `took ${result}s`);
});

test("a slow first font load right after the game starts is retried, not taken as 'signals off'", () => {
  runGame(`
LOAD_DELAY = 6 -- longer than the addon's 4 s check: a cold start
real["Interface\\\\AddOns\\\\T3WoW\\\\ctl\\\\probe\\\\valid.ttf"] = true
Link.Init()
tick(5)
assert(Link.Status().signals == "checking", "after 5 s: " .. Link.Status().signals)
tick(10)
assert(Link.Status().signals == "on", "after 15 s: " .. Link.Status().signals .. " | " .. table.concat(Link.Trace(), " | "))
local trace = table.concat(Link.Trace(), " | ")
assert(trace:find("retrying"), trace)
return "ok"
`);
});
