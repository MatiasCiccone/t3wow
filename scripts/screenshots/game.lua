-- A fake WoW with real layout: anchors resolve to rectangles, text is measured with the real
-- fonts' advance widths, and DRAW() returns everything visible, in draw order.
clock, EPOCH = 1000, 1794000000
function GetTime() return clock end
function time() return EPOCH + math.floor(clock - 1000) end
function date(_, t) return tostring(t or time()) end
function GetPhysicalScreenSize() return 1920, 1080 end
function print() end
function GetLocale() return "enUS" end
function IsShiftKeyDown() return false end
function IsControlKeyDown() return false end
function IsAltKeyDown() return false end
function GetCursorPosition() return 0, 0 end
function GetCurrentKeyBoardFocus() return nil end
function ReloadUI() end
function PlaySound() end
function PlaySoundFile() end
function hooksecurefunc() end
tinsert, tremove, unpack = table.insert, table.remove, table.unpack
STANDARD_TEXT_FONT = "Fonts\\FRIZQT__.TTF"
BUILD = { }

local gen, seq = 0, 0
local function dirty() gen = gen + 1 end
local timers = {}
C_Timer = { After = function(d, fn) timers[#timers + 1] = { at = clock + (d or 0), fn = fn } end,
  NewTicker = function() return { Cancel = function() end } end, NewTimer = function() return { Cancel = function() end } end }
C_AddOns = { LoadAddOn = function() return true end, GetAddOnMetadata = function() return "0.0.0" end }

frames = {}
local M = {}
local function class(o)
  return setmetatable(o, { __index = function(_, k)
    local m = M[k]
    if m then return m end
    if type(k) ~= "string" or not k:find("^[A-Z]") then return nil end
    if k:find("^Scroll") then return nil end
    if k:find("^Get") then return function() return 0 end end
    if k:find("^Is") or k:find("^Has") then return function() return false end end
    return function(self) return self end
  end })
end

local function new(kind, parent, name)
  seq = seq + 1
  local o = class({ kind = kind, parent = parent, name = name, seq = seq, shown = true, alpha = 1, scripts = {}, anchors = {}, enabled = true })
  if name then _G[name] = o end
  frames[#frames + 1] = o
  return o
end

-- ---------------------------------------------------------------- fonts
local FONT_DEFAULT = { path = "Fonts\\ARIALN.TTF", size = 12, color = { 1, 1, 1, 1 }, sc = { 0, 0, 0, 0 }, sx = 0, sy = 0 }
function CreateFont(name)
  local f = { kind = "Font", path = FONT_DEFAULT.path, size = 12, color = { 1, 1, 1, 1 }, sc = { 0, 0, 0, 0 }, sx = 0, sy = 0 }
  function f:SetFont(path, size) self.path, self.size = path, size; dirty(); return true end
  function f:GetFont() return self.path, self.size, "" end
  function f:SetTextColor(r, g, b, a) self.color = { r, g, b, a or 1 } end
  function f:GetTextColor() return unpack(self.color) end
  function f:SetShadowColor(r, g, b, a) self.sc = { r, g, b, a or 1 } end
  function f:SetShadowOffset(x, y) self.sx, self.sy = x, y end
  function f:SetSpacing() end
  function f:SetJustifyH() end
  function f:CopyFontObject() end
  if name then _G[name] = f end
  return f
end
ChatFontNormal = CreateFont("ChatFontNormal")
GameFontNormal = CreateFont("GameFontNormal")
GameFontHighlight = CreateFont("GameFontHighlight")
GameFontHighlightSmall = CreateFont("GameFontHighlightSmall")

local function fontOf(r)
  if r.buttonOwner then
    local b = r.buttonOwner
    local fo = (not b.enabled and b.disabledFont) or b.normalFont
    if fo then return fo.path, fo.size, r.color or fo.color, fo end
  end
  local fo = r.fontObject or FONT_DEFAULT
  return r.fontPath or fo.path, r.fontSize or fo.size, r.color or fo.color, fo
end

local function fontKey(path)
  local p = path:lower()
  if p:find("semibold") then return "semi" end
  if p:find("inter") then return "inter" end
  if p:find("jetbrains") then return "mono" end
  if p:find("friz") then return "friz" end
  return "narrow"
end

local function advance(key, size, ch)
  local w = WIDTHS[key][ch]
  return (w or 600) * size / 1000
end

-- text to glyphs: { {ch=, c=hex or nil} ... }; |n and \n become { nl = true }.
local function glyphs(text)
  local out, color, i = {}, nil, 1
  local n = #text
  while i <= n do
    local c = text:sub(i, i)
    if c == "|" then
      local nx = text:sub(i + 1, i + 1)
      if nx == "c" then color = text:sub(i + 4, i + 9); i = i + 10
      elseif nx == "r" then color = nil; i = i + 2
      elseif nx == "n" then out[#out + 1] = { nl = true }; i = i + 2
      elseif nx == "T" or nx == "A" then local e = text:find("|[ta]", i + 2); i = (e or n) + 2
      elseif nx == "|" then out[#out + 1] = { ch = "|", c = color }; i = i + 2
      else out[#out + 1] = { ch = "|", c = color }; i = i + 1 end
    elseif c == "\n" then out[#out + 1] = { nl = true }; i = i + 1
    elseif c == "\r" then i = i + 1
    else
      local b, len = c:byte(), 1
      if b >= 240 then len = 4 elseif b >= 224 then len = 3 elseif b >= 192 then len = 2 end
      out[#out + 1] = { ch = text:sub(i, i + len - 1), c = color }
      i = i + len
    end
  end
  return out
end

-- The game's lines are about as tall as the font's size; the addon adds its own spacing.
local LINE = 1.08
local function lineHeight(size) return math.floor(size * LINE + 0.5) end

-- Lines of glyph arrays, wrapped to width (nil: no wrap) with their widths.
local function layoutText(text, path, size, width, wrap, nonSpace)
  local key = fontKey(path)
  local lines, cur, curW, lastBreak = {}, {}, 0, nil
  local function flush() lines[#lines + 1] = { g = cur, w = curW }; cur, curW, lastBreak = {}, 0, nil end
  for _, g in ipairs(glyphs(text or "")) do
    if g.nl then flush()
    else
      local w = advance(key, size, g.ch)
      if width and wrap and curW + w > width + 0.5 and #cur > 0 then
        if g.ch == " " then flush(); w = -1
        elseif lastBreak then
          local carried, cw = {}, 0
          for j = lastBreak + 1, #cur do carried[#carried + 1] = cur[j]; cw = cw + cur[j].w end
          local kept, kw = {}, 0
          for j = 1, lastBreak - 1 do kept[#kept + 1] = cur[j]; kw = kw + cur[j].w end
          cur, curW = kept, kw
          flush()
          cur, curW = carried, cw
        else flush() end
      end
      if w >= 0 then
        g.w = w
        cur[#cur + 1] = g
        curW = curW + w
        if g.ch == " " then lastBreak = #cur end
      end
    end
  end
  flush()
  if width and not wrap then
    for _, l in ipairs(lines) do
      if l.w > width + 0.5 then
        local dots = 3 * advance(key, size, ".")
        local kept, kw = {}, 0
        for _, g in ipairs(l.g) do
          if kw + g.w + dots > width then break end
          kept[#kept + 1] = g; kw = kw + g.w
        end
        while #kept > 0 and kept[#kept].ch == " " do kw = kw - kept[#kept].w; kept[#kept] = nil end
        local c = kept[#kept] and kept[#kept].c
        for _ = 1, 3 do local w = advance(key, size, "."); kept[#kept + 1] = { ch = ".", c = c, w = w }; kw = kw + w end
        l.g, l.w = kept, kw
      end
    end
  end
  return lines
end

local rect

local function textLines(r, width)
  local path, size = fontOf(r)
  return layoutText(r.text or "", path, size, width, r.wrap ~= false, r.nonSpace), size
end

local function stringWidth(r, bound)
  local lines = textLines(r, bound)
  local w = 0
  for _, l in ipairs(lines) do w = math.max(w, l.w) end
  return w
end

local function stringHeight(r, bound)
  local lines, size = textLines(r, bound)
  return #lines * lineHeight(size) + (#lines - 1) * (r.spacing or 0)
end

-- ---------------------------------------------------------------- layout
local FX = { TOPLEFT = 0, LEFT = 0, BOTTOMLEFT = 0, TOP = 0.5, CENTER = 0.5, BOTTOM = 0.5, TOPRIGHT = 1, RIGHT = 1, BOTTOMRIGHT = 1 }
local FY = { TOPLEFT = 1, TOP = 1, TOPRIGHT = 1, LEFT = 0.5, CENTER = 0.5, RIGHT = 0.5, BOTTOMLEFT = 0, BOTTOM = 0, BOTTOMRIGHT = 0 }

local function boundWidth(r)
  if r.width and r.width > 0 then return r.width end
end

function rect(r)
  if r._g == gen then return r._l, r._r, r._t, r._b end
  if r._busy then return nil end
  if r.isRoot then
    r._g, r._l, r._r, r._t, r._b = gen, 0, SCREEN_W, SCREEN_H, 0
    return 0, SCREEN_W, SCREEN_H, 0
  end
  r._busy = true
  local l, rt, t, b
  if r.scrollOf then
    local sl, _, st = rect(r.scrollOf)
    if sl then
      l, t = sl, st + (r.scrollOf.vscroll or 0)
      rt, b = l + (r.width or 0), t - (r.height or 0)
    end
  elseif r.thumbOf then
    local sl, sr, st, sb = rect(r.thumbOf)
    if sl then
      local lo, hi, v = r.thumbOf.min or 0, r.thumbOf.max or 1, r.thumbOf.value or 0
      local f = hi > lo and (v - lo) / (hi - lo) or 0
      local w, h = r.width or 14, r.height or 14
      if r.thumbOf.vertical then
        l = sl + ((sr - sl) - w) / 2; t = st - f * ((st - sb) - h)
      else
        l = sl + f * ((sr - sl) - w); t = (st + sb) / 2 + h / 2
      end
      rt, b = l + w, t - h
    end
  else
    local L, R, CX, T, B, CY
    for point, a in pairs(r.anchors) do
      local rel = a.rel
      if type(rel) == "string" then rel = _G[rel] end
      rel = rel or r.parent or UIParent
      local al, ar, at, ab = rect(rel)
      if al then
        local x = al + (ar - al) * FX[a.relPoint] + a.x
        local y = ab + (at - ab) * FY[a.relPoint] + a.y
        if FX[point] == 0 then L = x elseif FX[point] == 1 then R = x else CX = x end
        if FY[point] == 1 then T = y elseif FY[point] == 0 then B = y else CY = y end
      end
    end
    local isText = r.kind == "FontString"
    local bound = (L and R) and (R - L) or boundWidth(r)
    local w = (L and R) and (R - L) or r.width
    if isText and not w then w = stringWidth(r, nil) end
    w = w or 0
    if L then rt = R or (L + w); l = L elseif R then rt = R; l = R - w elseif CX then l = CX - w / 2; rt = l + w end
    local h = (T and B) and (T - B) or r.height
    if isText and not h then h = stringHeight(r, bound) end
    h = h or 0
    if T then b = B or (T - h); t = T elseif B then b = B; t = B + h elseif CY then t = CY + h / 2; b = t - h end
    if not (l and t) then l, rt, t, b = nil, nil, nil, nil end
  end
  r._busy = false
  r._g, r._l, r._r, r._t, r._b = gen, l, rt, t, b
  return l, rt, t, b
end

-- ---------------------------------------------------------------- region methods
function M:GetObjectType() return self.kind end
function M:IsObjectType(k) return self.kind == k end
function M:GetName() return self.name end
function M:GetParent() return self.parent end
function M:SetParent(p) self.parent = p; dirty() end
function M:Show() local was = self.shown; self.shown = true; if not was and self.scripts.OnShow then self.scripts.OnShow(self) end end
function M:Hide() local was = self.shown; self.shown = false; if was and self.scripts.OnHide then self.scripts.OnHide(self) end end
function M:SetShown(s) if s then self:Show() else self:Hide() end end
function M:IsShown() return self.shown end
function M:IsVisible()
  local r = self
  while r do if not r.shown then return false end; r = r.parent end
  return true
end
function M:SetAlpha(a) self.alpha = a end
function M:GetAlpha() return self.alpha end
function M:GetEffectiveAlpha() return self.alpha end
function M:GetScale() return 1 end
function M:GetEffectiveScale() return 1 end
function M:SetPoint(point, a, b, c, d)
  local rel, relPoint, x, y
  if type(a) == "number" then x, y = a, b
  elseif type(b) == "number" then rel, x, y = a, b, c
  else rel, relPoint, x, y = a, b, c, d end
  self.anchors[point] = { rel = rel, relPoint = relPoint or point, x = x or 0, y = y or 0 }
  dirty()
end
function M:SetAllPoints(rel)
  self.anchors = {}
  self:SetPoint("TOPLEFT", rel or self.parent)
  self:SetPoint("BOTTOMRIGHT", rel or self.parent)
end
function M:ClearAllPoints() self.anchors = {}; dirty() end
function M:GetNumPoints() local n = 0; for _ in pairs(self.anchors) do n = n + 1 end; return n end
function M:GetPoint()
  for point, a in pairs(self.anchors) do return point, a.rel, a.relPoint, a.x, a.y end
end
function M:SetSize(w, h) self.width, self.height = w, h; dirty() end
function M:SetWidth(w) if w == 0 and self.kind == "FontString" then w = nil end; self.width = w; dirty() end
function M:SetHeight(h) self.height = h; dirty() end
function M:GetWidth() local l, r = rect(self); return l and (r - l) or self.width or 0 end
function M:GetHeight() local l, _, t, b = rect(self); return l and (t - b) or self.height or 0 end
function M:GetSize() return self:GetWidth(), self:GetHeight() end
function M:GetLeft() local l = rect(self); return l end
function M:GetRight() local _, r = rect(self); return r end
function M:GetTop() local _, _, t = rect(self); return t end
function M:GetBottom() local _, _, _, b = rect(self); return b end
function M:GetCenter() local l, r, t, b = rect(self); if l then return (l + r) / 2, (t + b) / 2 end end
function M:IsMouseOver() return self.mouseOver == true end
function M:SetScript(e, fn) self.scripts[e] = fn end
function M:GetScript(e) return self.scripts[e] end
function M:HasScript() return true end
function M:HookScript(e, fn)
  local before = self.scripts[e]
  self.scripts[e] = before and function(...) before(...); fn(...) end or fn
end
function M:SetFrameStrata(s) self.strata = s end
function M:GetFrameStrata() return self.strata or (self.parent and self.parent:GetFrameStrata()) or "MEDIUM" end
function M:SetFrameLevel(l) self.level = l end
function M:GetFrameLevel()
  if self.level then return self.level end
  return self.parent and self.parent.GetFrameLevel and self.parent:GetFrameLevel() + 1 or 0
end
function M:SetDrawLayer(l, s) self.layer, self.sub = l, s or self.sub end
function M:GetDrawLayer() return self.layer, self.sub end

-- children
function M:CreateTexture(_, layer, _, sub)
  local t = new("Texture", self)
  t.layer, t.sub = layer or "ARTWORK", sub or 0
  return t
end
function M:CreateFontString(_, layer)
  local f = new("FontString", self)
  f.layer, f.sub, f.text = layer or "ARTWORK", 0, ""
  return f
end
function M:GetRegions()
  local list = {}
  for _, f in ipairs(frames) do
    if f.parent == self and (f.kind == "Texture" or f.kind == "FontString") then list[#list + 1] = f end
  end
  return unpack(list)
end
function M:GetChildren()
  local list = {}
  for _, f in ipairs(frames) do
    if f.parent == self and f.kind ~= "Texture" and f.kind ~= "FontString" then list[#list + 1] = f end
  end
  return unpack(list)
end

-- textures
function M:SetTexture(path) self.tex = path; self.solid = nil end
function M:GetTexture() return self.tex end
function M:GetTextureFilePath() return self.tex end
function M:SetColorTexture(r, g, b, a) self.solid = { r, g, b, a or 1 }; self.tex = nil end
function M:SetVertexColor(r, g, b, a) self.vc = { r, g, b, a or 1 } end
function M:GetVertexColor() return unpack(self.vc or { 1, 1, 1, 1 }) end
function M:SetTexCoord(...) self.uv = { ... } end
function M:SetDesaturated() end
function M:SetBlendMode() end

-- text
function M:SetText(t)
  if self.kind == "Button" or self.kind == "CheckButton" then
    if not self.fs then
      self.fs = self:CreateFontString(nil, "OVERLAY")
      self.fs.buttonOwner = self
      self.fs:SetPoint("CENTER")
    end
    self.fs.text = t or ""
  else
    self.text = t == nil and "" or tostring(t)
  end
  dirty()
end
function M:GetText()
  if self.kind == "Button" or self.kind == "CheckButton" then return self.fs and self.fs.text or "" end
  return self.text or ""
end
function M:SetFormattedText(f, ...) self:SetText(string.format(f, ...)) end
function M:SetFontObject(o) self.fontObject = o; dirty() end
function M:GetFontObject() return self.fontObject end
function M:SetFont(path, size) self.fontPath, self.fontSize = path, size; dirty(); return true end
function M:GetFont() local p, s = fontOf(self); return p, s, "" end
function M:SetTextColor(r, g, b, a) self.color = { r, g, b, a or 1 } end
function M:GetTextColor() local _, _, c = fontOf(self); return unpack(c) end
function M:SetJustifyH(j) self.justH = j end
function M:SetJustifyV(j) self.justV = j end
function M:SetWordWrap(w) self.wrap = w; dirty() end
function M:SetNonSpaceWrap(w) self.nonSpace = w; dirty() end
function M:SetSpacing(s) self.spacing = s; dirty() end
function M:GetSpacing() return self.spacing or 0 end
function M:GetLineHeight() local _, s = fontOf(self); return lineHeight(s) end
function M:GetStringWidth()
  local l, r = rect(self)
  local bound = self.width or ((l and self.anchors.LEFT and self.anchors.RIGHT) and (r - l)) or nil
  return stringWidth(self, bound)
end
function M:GetUnboundedStringWidth() return stringWidth(self, nil) end
function M:GetStringHeight()
  local l, r = rect(self)
  local bound = (l and (r - l) > 0) and (r - l) or self.width
  return stringHeight(self, bound)
end
function M:GetNumLines() local l, r = rect(self); return #textLines(self, l and (r - l) or self.width) end
function M:IsTruncated() return false end

-- buttons
function M:SetEnabled(on)
  local was = self.enabled
  self.enabled = on and true or false
  if was ~= self.enabled then
    local s = self.scripts[self.enabled and "OnEnable" or "OnDisable"]
    if s then s(self) end
  end
end
function M:Enable() self:SetEnabled(true) end
function M:Disable() self:SetEnabled(false) end
function M:IsEnabled() return self.enabled end
function M:GetFontString() return self.fs end
function M:SetFontString(fs) self.fs = fs; fs.buttonOwner = self; dirty() end
function M:SetNormalFontObject(o) self.normalFont = o; dirty() end
function M:SetHighlightFontObject() end
function M:SetDisabledFontObject(o) self.disabledFont = o; dirty() end
function M:GetTextWidth() return self.fs and stringWidth(self.fs, nil) or 0 end
function M:GetTextHeight() return self.fs and stringHeight(self.fs, nil) or 0 end
function M:Click() if self.scripts.OnClick then self.scripts.OnClick(self, "LeftButton") end end
function M:SetChecked(c) self.checked = c and true or false end
function M:GetChecked() return self.checked end

-- edit boxes
function M:Insert(t) self.text = (self.text or "") .. t; dirty() end
function M:SetMultiLine(m) self.multi = m end
function M:IsMultiLine() return self.multi end
function M:SetTextInsets(l, r, t, b) self.insets = { l, r, t, b } end
function M:GetCursorPosition() return #(self.text or "") end
function M:SetFocus() self.focus = true end
function M:ClearFocus() self.focus = false end
function M:HasFocus() return self.focus == true end
function M:GetNumLetters() return #(self.text or "") end

-- scrolling
function M:SetScrollChild(c) self.child = c; c.scrollOf = self; dirty() end
function M:GetScrollChild() return self.child end
function M:GetVerticalScrollRange()
  local c = self.child
  if not c then return 0 end
  -- A multi-line edit box is as tall as its text, as in the game.
  if c.kind == "EditBox" and c.multi then
    local inset = c.insets or { 0, 0, 0, 0 }
    local h = stringHeight(c, math.max(1, (c.width or 0) - inset[1] - inset[2])) + inset[3] + inset[4]
    if c.height ~= h then c.height = h; dirty() end
  end
  return math.max(0, (c.height or 0) - self:GetHeight())
end
-- As the game's ScrollFrame_OnScrollRangeChanged: with something to scroll the bar shows,
-- thumb and all. With nothing to scroll it hides only on a frame marked scrollBarHideable;
-- otherwise it stays as it was, its thumb still drawn.
local function syncBar(s)
  local bar = s.ScrollBar
  if bar then
    local range = math.floor(s:GetVerticalScrollRange())
    bar.min, bar.max, bar.value = 0, range, s.vscroll or 0
    if range > 0 then bar:Show() elseif s.scrollBarHideable then bar:Hide() end
  end
end
function M:UpdateScrollChildRect() syncBar(self) end
function M:SetVerticalScroll(v) self.vscroll = v; dirty(); syncBar(self) end
function M:GetVerticalScroll() return self.vscroll or 0 end
function M:SetHorizontalScroll() end

-- sliders
function M:SetMinMaxValues(a, b) self.min, self.max = a, b; dirty() end
function M:GetMinMaxValues() return self.min or 0, self.max or 1 end
function M:SetValue(v)
  self.value = v
  dirty()
  if self.scripts.OnValueChanged then self.scripts.OnValueChanged(self, v, false) end
end
function M:GetValue() return self.value or 0 end
function M:SetOrientation(o) self.vertical = o == "VERTICAL" end
function M:SetThumbTexture(p)
  if not self.thumb then
    self.thumb = new("Texture", self)
    self.thumb.layer, self.thumb.sub = "OVERLAY", 7
    self.thumb.thumbOf = self
  end
  if type(p) == "string" then self.thumb.tex = p end
end
function M:GetThumbTexture() return self.thumb end

function CreateFrame(kind, name, parent, template)
  local f = new(kind, parent or UIParent, name)
  if kind == "Button" or kind == "CheckButton" then f.wrap = true end
  if kind == "EditBox" then f.text = "" end
  if template and template:find("ScrollFrame") then
    local barName = (name or ("Scroll" .. f.seq)) .. "ScrollBar"
    local bar = new("Slider", f, barName)
    bar.vertical = true
    bar:SetThumbTexture("Interface\\Buttons\\UI-ScrollBar-Knob")
    bar.ScrollUpButton = new("Button", bar)
    bar.ScrollDownButton = new("Button", bar)
    f.ScrollBar = bar
    f.scrollBarTemplate = true
  end
  return f
end
UIParent = new("Frame", nil, "UIParent"); UIParent.isRoot = true; UIParent.strata = "MEDIUM"; UIParent.level = 0
WorldFrame = new("Frame", nil, "WorldFrame"); WorldFrame.isRoot = true
Minimap = CreateFrame("Frame", "Minimap")
UISpecialFrames = {}
GameTooltip = CreateFrame("Frame", "GameTooltip")

function fire(event, ...)
  for _, f in ipairs(frames) do
    if f.scripts.OnEvent then f.scripts.OnEvent(f, event, ...) end
  end
end

-- Runs timers, OnUpdate and OnSizeChanged until the layout stops moving.
function settle(seconds)
  local target = clock + (seconds or 1)
  while clock < target do
    clock = clock + 0.25
    local due = timers
    timers = {}
    for _, t in ipairs(due) do
      if t.at <= clock then t.fn() else timers[#timers + 1] = t end
    end
    for _, f in ipairs(frames) do
      if f.scripts.OnUpdate and f:IsVisible() then f.scripts.OnUpdate(f, 0.25) end
    end
  end
  for _ = 1, 12 do
    local changed = false
    for _, f in ipairs(frames) do
      if f.kind ~= "Texture" and f.kind ~= "FontString" then
        local w, h = f:GetWidth(), f:GetHeight()
        if f._lw and (math.abs(f._lw - w) > 0.5 or math.abs(f._lh - h) > 0.5) and f.scripts.OnSizeChanged then
          f._lw, f._lh = w, h
          f.scripts.OnSizeChanged(f, w, h)
          changed = true
        end
        f._lw, f._lh = w, h
      end
    end
    if not changed then break end
  end
  for _, f in ipairs(frames) do if f.scrollBarTemplate then syncBar(f) end end
end

-- ---------------------------------------------------------------- drawing
local STRATA = { WORLD = 0, BACKGROUND = 1, LOW = 2, MEDIUM = 3, HIGH = 4, DIALOG = 5, FULLSCREEN = 6, FULLSCREEN_DIALOG = 7, TOOLTIP = 8 }
local LAYER = { BACKGROUND = 1, BORDER = 2, ARTWORK = 3, OVERLAY = 4, HIGHLIGHT = 5 }

local function hostOf(r)
  if r.kind == "Texture" or r.kind == "FontString" then return r.parent end
  return r
end

local function effAlpha(r)
  local a, p = 1, r
  while p do a = a * (p.alpha or 1); p = p.parent end
  return a
end

local function clipOf(r)
  local l, rt, t, b
  local p = r
  while p do
    if p.scrollOf then
      local sl, sr, st, sb = rect(p.scrollOf)
      if sl then
        if not l then l, rt, t, b = sl, sr, st, sb
        else l, rt, t, b = math.max(l, sl), math.min(rt, sr), math.min(t, st), math.max(b, sb) end
      end
      p = p.scrollOf
    else
      p = p.parent
    end
  end
  return l, rt, t, b
end

local function runsOf(line)
  local runs, cur = {}, nil
  for _, g in ipairs(line.g) do
    if cur and cur.c == g.c then cur.t = cur.t .. g.ch
    else cur = { t = g.ch, c = g.c }; runs[#runs + 1] = cur end
  end
  return runs
end

function DRAW()
  local items = {}
  for _, r in ipairs(frames) do
    if (r.kind == "Texture" or r.kind == "FontString" or r.kind == "EditBox") and r:IsVisible() then
      local l, rt, t, b = rect(r)
      if l and effAlpha(r) > 0.003 then
        local host = hostOf(r)
        local item = {
          key = { STRATA[host:GetFrameStrata()] or 3, host:GetFrameLevel(), host.seq,
            r.kind == "EditBox" and 5 or (LAYER[r.layer or "ARTWORK"] or 3), r.sub or 0, r.seq },
          x = l, y = SCREEN_H - t, w = rt - l, h = t - b, a = effAlpha(r),
        }
        local cl, cr, ct, cb = clipOf(r)
        if cl then item.clip = { cl, SCREEN_H - ct, cr - cl, ct - cb } end
        local drawable = false
        if r.kind == "Texture" then
          if r.solid then item.t, item.c, drawable = "rect", r.solid, true
          elseif r.tex then
            local name = tostring(r.tex):match("Media\\([%w%-_]+)")
            local c = r.vc or { 1, 1, 1, 1 }
            if tostring(r.tex):find("WHITE8X8") then item.t, item.c, drawable = "rect", c, true
            elseif name then item.t, item.img, item.c, item.uv, drawable = "tex", name, c, r.uv, true end
          end
        else
          local text = r.kind == "EditBox" and (r.text or "") or r.text
          if text and text ~= "" then
            local path, size, color, fo = fontOf(r)
            local inset = r.insets or { 0, 0, 0, 0 }
            local width = item.w - inset[1] - inset[2]
            local wrap = r.wrap ~= false
            if r.kind == "EditBox" and not r.multi then wrap = false end
            local lines = layoutText(text, path, size, width > 0 and width or nil, wrap, r.nonSpace)
            local lh = lineHeight(size)
            local total = #lines * lh + (#lines - 1) * (r.spacing or 0)
            local y = item.y + inset[3]
            if r.kind ~= "EditBox" then
              if r.justV == "TOP" then y = item.y
              elseif r.justV == "BOTTOM" then y = item.y + item.h - total
              else y = item.y + (item.h - total) / 2 end
            elseif not r.multi then y = item.y + (item.h - total) / 2 end
            local rows = {}
            for i, line in ipairs(lines) do
              local x = item.x + inset[1]
              if r.justH == "CENTER" or (r.justH == nil and r.kind == "FontString") then x = x + (width - line.w) / 2
              elseif r.justH == "RIGHT" then x = x + width - line.w end
              rows[i] = { x = x, y = y + (i - 1) * (lh + (r.spacing or 0)), r = runsOf(line) }
            end
            item.t, item.font, item.size, item.lh, item.c, item.rows = "text", fontKey(path), size, lh, color, rows
            item.sc, item.sx, item.sy = fo.sc, fo.sx, fo.sy
            drawable = true
          end
        end
        if drawable then items[#items + 1] = item end
      end
    end
  end
  table.sort(items, function(a, b)
    for i = 1, 6 do
      if a.key[i] ~= b.key[i] then return a.key[i] < b.key[i] end
    end
    return false
  end)
  return items
end

-- ---------------------------------------------------------------- JSON
local function esc(s)
  return '"' .. s:gsub('[%c"\\]', function(c)
    if c == '"' then return '\\"' elseif c == "\\" then return "\\\\" end
    return string.format("\\u%04x", c:byte())
  end) .. '"'
end
function JSON(v)
  local t = type(v)
  if t == "table" then
    if v[1] ~= nil or next(v) == nil then
      local o = {}
      for i, x in ipairs(v) do o[i] = JSON(x) end
      return "[" .. table.concat(o, ",") .. "]"
    end
    local o = {}
    for k, x in pairs(v) do if k ~= "key" then o[#o + 1] = esc(tostring(k)) .. ":" .. JSON(x) end end
    return "{" .. table.concat(o, ",") .. "}"
  elseif t == "string" then return esc(v)
  elseif t == "number" or t == "boolean" then return tostring(v)
  end
  return "null"
end
