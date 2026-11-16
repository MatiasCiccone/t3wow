// A small simulated game for running the real addon files in fengari: frames whose
// OnUpdate we drive, fonts that load a while after SetFont (and only if the file was real
// when asked), slot addons whose Inbox we hand over, and two clocks (GetTime and time())
// moving together. Not a test file itself: the *.test.js files use it.
import { readFileSync } from "node:fs";
import fengari from "fengari";

const { lua, lauxlib, lualib, to_luastring } = fengari;
export const read = (name) => readFileSync(new URL(`../addon/T3WoW/${name}`, import.meta.url), "utf8");

export const GAME = `
clock = 1000
EPOCH = 1794000000
LOAD_DELAY = 1.5
real = {}
firstAsked = {}
inboxes = {}
loads = {}
alerts = {}
printed = {}
missingTextures = {}
function GetTime() return clock end
function time() return EPOCH + math.floor(clock - 1000) end
function GetPhysicalScreenSize() return 1920, 1080 end
function date(_, t) return tostring(t or time()) end
function print(text) printed[#printed + 1] = text end
STANDARD_TEXT_FONT = "Fonts\\\\STANDARD.TTF"
ROOT = "Interface\\\\AddOns\\\\T3WoW\\\\"
PROBE_VALID = ROOT .. "ctl\\\\probe\\\\valid.ttf"
function sigPath(n) return string.format("%ssig\\\\%04d.ttf", ROOT, n) end

-- Like the game (as traces showed): a font string whose font load failed never loads a
-- font afterwards, whatever it is asked for.
local function fontString()
  local fs = { font = STANDARD_TEXT_FONT, pending = false, poisoned = false, hidden = false, text = "" }
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
  function fs:SetText(text) self.text = text end
  function fs:SetFontObject(font) self.fontObject = font end
  function fs:GetText() return self.text or "" end
  function fs:Show() self.hidden = false end
  function fs:Hide() self.hidden = true end
  function fs:IsShown() return not self.hidden end
  function fs:SetShown(shown) self.hidden = not shown end
  function fs:SetPoint(...) self.point = { ... } end
  function fs:ClearAllPoints() self.point = nil end
  fontStrings[#fontStrings + 1] = fs
  return setmetatable(fs, { __index = function() return function() return 0 end end })
end

-- Whether any shown text matches the Lua pattern.
fontStrings = {}
function showing(pattern)
  for _, fs in ipairs(fontStrings) do
    if not fs.hidden and type(fs.text) == "string" and fs.text:find(pattern) then return true end
  end
  return false
end

-- Widgets: text, shown state and scripts are kept; other setters do nothing, and other
-- getters answer 0 (sizes, scroll ranges).
frames = {}
local function frame(kind, name)
  local f = { scripts = {}, shown = true, text = "", enabled = true, kind = kind, name = name, alpha = 1 }
  function f:SetScript(event, fn) self.scripts[event] = fn end
  function f:GetScript(event) return self.scripts[event] end
  function f:HookScript(event, fn)
    local before = self.scripts[event]
    self.scripts[event] = before and function(...) before(...); fn(...) end or fn
  end
  function f:CreateFontString() return fontString() end
  function f:CreateTexture() return frame("Texture") end
  function f:GetName() return self.name end
  function f:Show()
    local was = self.shown
    self.shown = true
    if not was and self.scripts.OnShow then self.scripts.OnShow(self) end
  end
  function f:Hide()
    local was = self.shown
    self.shown = false
    if was and self.scripts.OnHide then self.scripts.OnHide(self) end
  end
  function f:SetShown(shown) if shown then self:Show() else self:Hide() end end
  function f:IsShown() return self.shown end
  function f:IsVisible() return self.shown end
  function f:SetText(text) self.text = text or "" end
  function f:GetText() return self.text end
  function f:Insert(text) self.text = self.text .. text end
  function f:GetCursorPosition() return #self.text end
  function f:SetEnabled(on) self.enabled = on end
  function f:SetAlpha(a) self.alpha = a end
  function f:GetAlpha() return self.alpha end
  function f:EnableMouse(on) self.mouse = on end
  function f:GetFontString() return fontString() end
  function f:GetPoint() return "CENTER", nil, "CENTER", 0, 0 end
  function f:SetPoint(...) self.point = { ... } end
  function f:ClearAllPoints() self.point = nil end
  function f:SetSize(w, h) self.width, self.height = w, h end
  function f:SetHeight(h) self.height = h end
  function f:SetParent(parent) self.parent = parent end
  function f:GetParent() return self.parent end
  -- Textures: the file asked for, and its file id as the game gives it (none for a file
  -- the game didn't find at startup, listed in missingTextures).
  function f:SetTexture(path) self.texturePath = path end
  function f:GetTextureFilePath()
    if self.texturePath == nil or missingTextures[self.texturePath] then return nil end
    return -4186
  end
  frames[#frames + 1] = f
  if name then _G[name] = f end
  return setmetatable(f, { __index = function(_, key)
    if type(key) == "string" and key:find("^Get") then return function() return 0 end end
    if type(key) == "string" and key:find("^Is") then return function() return false end end
    return function() return f end
  end })
end
function CreateFont(name) return frame("Font", name) end
function CreateFrame(kind, name, parent)
  local f = frame(kind, name)
  f.parent = parent
  return f
end
UIParent, WorldFrame, Minimap = frame(), frame(), frame()
UISpecialFrames = {}
-- What Escape does to addon windows in the game (UIParentPanelManager.lua).
function CloseSpecialWindows()
  local found
  for _, name in pairs(UISpecialFrames) do
    local f = _G[name]
    if f and f:IsShown() then
      f:Hide()
      found = 1
    end
  end
  return found
end
tinsert = table.insert
unpack = unpack or table.unpack
ChatFontNormal = {}
C_Timer = { After = function(_, fn) fn() end }
function IsShiftKeyDown() return false end
controlDown = false
function IsControlKeyDown() return controlDown end
function ReloadUI() end
function PlaySound() end
C_AddOns = { LoadAddOn = function(name) loads[#loads + 1] = name; T3WoW_Inbox = inboxes[name]; return true end }

-- Sends a game event (PLAYER_LOGOUT, say) to every frame that has an OnEvent script.
function fire(event, ...)
  for _, f in ipairs(frames) do
    if f.scripts.OnEvent then f.scripts.OnEvent(f, event, ...) end
  end
end

function tick(seconds)
  for _ = 1, math.floor(seconds / 0.25 + 0.5) do
    clock = clock + 0.25
    for _, f in ipairs(frames) do
      if f.scripts.OnUpdate and f.shown then f.scripts.OnUpdate(f, 0.25) end
    end
  end
end
`;

// The addon's UI as a stub that records what Link asks of it.
const STUB_UI = `ns.UI = {
  OnInbox = function() end, Refresh = function() end, Notice = function() end,
  IsShown = function() return false end,
  Alert = function(event) alerts[#alerts + 1] = event end,
  ForgetLogin = function() forgotten = (forgotten or 0) + 1 end,
}`;

// Runs `script` after loading Codec.lua and Link.lua (and UI.lua with { ui: true }); its
// return value comes back as a string. T3WoWDB starts as `db` (a Lua table literal), and
// `before` runs ahead of the addon's files (to give the game an API it doesn't have here).
// theme: false leaves Theme.lua out, as a game started before an update added it would.
// key: the hex Key.lua would put in the addon's namespace (none: no Key.lua, nothing signed).
export function runGame(script, { ui = false, db = `{ client = "c1", seq = 0, queue = {} }`, before = "", theme = true, key } = {}) {
  const L = lauxlib.luaL_newstate();
  lualib.luaL_openlibs(L);
  const load = (name) => `assert(load(${JSON.stringify(read(name))}, "${name}"))("T3WoW", ns)`;
  const chunk = `${GAME}
${before}
local ns = {}
${key ? `ns.key = "${key}"` : ""}
${ui ? "" : STUB_UI}
${load("Codec.lua")}
${ui && theme ? load("Theme.lua") : ""}
${ui ? load("UI.lua") : ""}
${load("Link.lua")}
T3WoWDB = ${db}
Link, UI = ns.Link, ns.UI
${script}`;
  if (lauxlib.luaL_loadstring(L, to_luastring(chunk)) !== lua.LUA_OK || lua.lua_pcall(L, 0, 1, 0) !== lua.LUA_OK) {
    throw new Error(lua.lua_tojsstring(L, -1));
  }
  return lua.lua_tojsstring(L, -1);
}
