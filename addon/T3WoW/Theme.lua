local _, ns = ...
-- The window's look. Every theme shares one layout; a theme is colors, fonts and the art its
-- boxes are cut from. Widgets are built through the helpers here and remember what they
-- are (a card, a primary button, a pill), so Skin.Apply restyles all of them in place: a
-- theme change shows at once, and so does the saved theme once saved settings exist.
local Skin = {}
ns.Skin = Skin

local format, floor = string.format, math.floor

local MEDIA = "Interface\\AddOns\\T3WoW\\Media\\"
local WHITE = "Interface\\Buttons\\WHITE8X8"
local ARIAL_NARROW, FRIZ = "Fonts\\ARIALN.TTF", "Fonts\\FRIZQT__.TTF"
-- The game has no monospace font: code and tables are drawn in JetBrains Mono, bundled.
local MONO = MEDIA .. "JetBrainsMono-Regular.ttf"

local function hex(code, alpha)
    return { tonumber(code:sub(1, 2), 16) / 255, tonumber(code:sub(3, 4), 16) / 255, tonumber(code:sub(5, 6), 16) / 255, alpha or 1 }
end

-- Box art: a 64×64 texture whose corner is its outer 16 pixels, drawn with corners c units
-- wide ("px": one screen pixel; "half": half the box's height, for pills and dots).
local ROUND = { fill = MEDIA .. "round", ring = MEDIA .. "ring" }
local SQUARE = { fill = WHITE, ring = WHITE }
local CLASSIC = { fill = MEDIA .. "classic", ring = MEDIA .. "classic-ring" }

Skin.THEMES = {
    t3 = {
        name = "T3",
        shapes = {
            panel = { art = ROUND, c = 8 }, control = { art = ROUND, c = 8 }, pill = { art = ROUND, c = "half" },
        },
        fonts = {
            regular = MEDIA .. "Inter-Regular.ttf", strong = MEDIA .. "Inter-SemiBold.ttf", heading = MEDIA .. "Inter-SemiBold.ttf", mono = MONO,
        },
        -- Inter is wider than Arial Narrow, so its body text is a size smaller to take the same room.
        sizes = { text = 12, strong = 12, small = 11, title = 15, label = 11, button = 12, pill = 10, code = 12 },
        shadow = false,
        check = "toggle", pillAlpha = 0.14,
        rowHeight = 44, compactHeight = 24,
        colors = {
            bg = hex("0b0b0c", 0.96), panel = hex("0f0f10"), card = hex("151517"), raised = hex("1c1c1f"),
            hover = { 1, 1, 1, 0.05 }, border = hex("27272a"), line = hex("27272a"),
            text = hex("f4f4f5"), muted = hex("a1a1aa"), dim = hex("71717a"), heading = hex("f4f4f5"), label = hex("71717a"),
            accent = hex("fafafa"), primary = hex("fafafa"), primaryRing = hex("fafafa"), primaryText = hex("09090b"),
            selected = hex("1c1c1f"), selectedRing = hex("3f3f46"), input = hex("151517"), focus = hex("52525b"),
            off = hex("3f3f46"), knob = hex("a1a1aa"), knobOn = hex("09090b"), scroll = hex("2e2e33"), warn = hex("fb923c"),
            code = hex("e4e4e7"), codeBg = hex("18181b"),
        },
    },
    flat = {
        name = "Flat",
        shapes = { panel = { art = SQUARE, c = "px" }, control = { art = SQUARE, c = "px" }, pill = { art = SQUARE, c = "px" } },
        fonts = { regular = ARIAL_NARROW, strong = ARIAL_NARROW, heading = ARIAL_NARROW, mono = MONO },
        sizes = { text = 14, strong = 14, small = 11, title = 15, label = 11, button = 13, pill = 11, code = 12 },
        shadow = true,
        check = "fill", pillAlpha = 0.16,
        rowHeight = 42, compactHeight = 22,
        colors = {
            bg = hex("131313", 0.93), panel = hex("101010"), card = hex("161616"), raised = hex("1e1e1e"),
            hover = hex("8f8fff", 0.1), border = hex("000000"), line = hex("2b2b2b"),
            text = hex("e8e8e8"), muted = hex("b0b0b0"), dim = hex("8a8a8a"), heading = hex("e8e8e8"), label = hex("8a8a8a"),
            accent = hex("8f8fff"), primary = hex("23233a"), primaryRing = hex("8f8fff"), primaryText = hex("ffffff"),
            selected = hex("8f8fff", 0.16), selectedRing = hex("8f8fff"), input = hex("0c0c0c"), focus = hex("8f8fff"),
            off = hex("0c0c0c"), knob = hex("3a3a3a"), knobOn = hex("8f8fff"), scroll = hex("3a3a3a"), warn = hex("ff8000"),
            code = hex("dcdcdc"), codeBg = hex("0c0c0c"),
        },
    },
    classic = {
        name = "Classic",
        shapes = { panel = { art = CLASSIC, c = 8 }, control = { art = CLASSIC, c = 8 }, pill = { art = CLASSIC, c = "half" } },
        fonts = { regular = ARIAL_NARROW, strong = ARIAL_NARROW, heading = FRIZ, mono = MONO },
        sizes = { text = 14, strong = 13, small = 11, title = 14, label = 12, button = 12, pill = 11, code = 12 },
        shadow = true,
        check = "tick", pillAlpha = 0,
        rowHeight = 42, compactHeight = 22,
        colors = {
            bg = hex("09090e", 0.94), panel = { 0, 0, 0, 0.35 }, card = hex("141418"), raised = hex("222226"),
            hover = hex("ffd100", 0.12), border = { 1, 1, 1, 1 }, line = hex("2f2f2f"),
            text = hex("ffffff"), muted = hex("c8c8c8"), dim = hex("9d9d9d"), heading = hex("ffd100"), label = hex("ffd100"),
            accent = hex("ffd100"), primary = hex("2a2a2f"), primaryRing = hex("ffd100"), primaryText = hex("ffd100"),
            selected = hex("ffd100", 0.18), selectedRing = hex("ffd100"), input = { 0, 0, 0, 0.8 }, focus = hex("ffd100"),
            off = hex("0b0b0b"), knob = hex("7a7a7a"), knobOn = hex("ffd100"), scroll = hex("5a5a5a"), warn = hex("ff8000"),
            code = hex("e6e2c8"), codeBg = { 0, 0, 0, 0.55 },
        },
    },
}
Skin.ORDER = { "t3", "flat", "classic" }

local current = "t3"
local theme = Skin.THEMES.t3

function Skin.Theme() return theme end
function Skin.ThemeKey() return current end

-- ElvUI, when it runs: the Flat theme takes its colors, font and pixel size, so the window
-- looks like the rest of an ElvUI screen. Anything missing keeps Flat's own.
local function elvui()
    local E = type(ElvUI) == "table" and ElvUI[1]
    return type(E) == "table" and type(E.media) == "table" and E or nil
end

local function color(c)
    return type(c) == "table" and c[1] and { c[1], c[2], c[3], c[4] or 1 }
        or type(c) == "table" and c.r and { c.r, c.g, c.b, c.a or 1 } or nil
end

-- The theme in force: the saved one, with ElvUI's media layered on Flat.
local function resolve(key)
    local base = Skin.THEMES[key] or Skin.THEMES.t3
    local E = key == "flat" and elvui()
    local locale = GetLocale and GetLocale()
    if locale == "zhCN" or locale == "zhTW" or locale == "koKR" then
        -- The bundled fonts have no CJK glyphs, so the client's own font draws everything, code
        -- too: a table's columns may then not line up, but every character shows.
        local native = (ChatFontNormal and ChatFontNormal.GetFont and ChatFontNormal:GetFont()) or STANDARD_TEXT_FONT
        if native then
            local t = setmetatable({ fonts = { regular = native, strong = native, heading = native, mono = native } }, { __index = base })
            if not E then return t end
            base = t
        end
    end
    if not E then return base end
    local t = setmetatable({ colors = setmetatable({}, { __index = base.colors }), fonts = setmetatable({}, { __index = base.fonts }) },
        { __index = base })
    local m = E.media
    local backdrop, fade, border, value = color(m.backdropcolor), color(m.backdropfadecolor), color(m.bordercolor), color(m.rgbvaluecolor)
    if fade then t.colors.bg = fade end
    if backdrop then t.colors.panel, t.colors.card, t.colors.raised, t.colors.input = backdrop, backdrop, backdrop, backdrop end
    if border then t.colors.border = border end
    if value then
        t.colors.accent, t.colors.primaryRing, t.colors.selectedRing, t.colors.focus, t.colors.knobOn = value, value, value, value, value
        t.colors.selected, t.colors.hover = { value[1], value[2], value[3], 0.16 }, { value[1], value[2], value[3], 0.1 }
    end
    if type(m.normFont) == "string" then t.fonts.regular, t.fonts.strong, t.fonts.heading = m.normFont, m.normFont, m.normFont end
    t.elvMult = type(E.mult) == "number" and E.mult or nil
    return t
end

-- One screen pixel, in the window's units.
local function pixel()
    if theme.elvMult then return theme.elvMult end
    local _, height = GetPhysicalScreenSize()
    local scale = UIParent:GetEffectiveScale()
    if not height or height <= 0 or not scale or scale <= 0 then return 1 end
    return 768 / height / scale
end
Skin.px = 1

function Skin.Color(key)
    local c = type(key) == "table" and key or theme.colors[key] or theme.colors.text
    return c[1], c[2], c[3], c[4] or 1
end

-- "rrggbb" of a color, for |c codes in text.
function Skin.Hex(key)
    local r, g, b = Skin.Color(key)
    return format("%02x%02x%02x", floor(r * 255 + 0.5), floor(g * 255 + 0.5), floor(b * 255 + 0.5))
end

-- ---------------------------------------------------------------- fonts
-- Font objects by role: every string set to one changes with it.
local FONT_ROLES = {
    text = { "regular", "text", "text" }, strong = { "strong", "strong", "text" }, small = { "regular", "small", "dim" },
    muted = { "regular", "small", "muted" }, title = { "heading", "title", "heading" }, label = { "heading", "label", "label" },
    button = { "heading", "button", "text" }, buttonPrimary = { "heading", "button", "primaryText" },
    buttonGhost = { "heading", "button", "muted" }, buttonDisabled = { "heading", "button", "dim" },
    pill = { "strong", "pill", "text" }, mono = { "mono", "code", "code" },
}
Skin.fonts = {}
for role in pairs(FONT_ROLES) do Skin.fonts[role] = CreateFont("T3WoWFont_" .. role) end

local function applyFonts()
    for role, spec in pairs(FONT_ROLES) do
        local font = Skin.fonts[role]
        local path = theme.fonts[spec[1]]
        -- A font file that won't load (ElvUI's, moved) falls back to the game's own. Clients
        -- that don't say whether SetFont worked are taken at their word.
        local ok = font:SetFont(path, theme.sizes[spec[2]], "")
        if ok == false or not font:GetFont() then font:SetFont(STANDARD_TEXT_FONT or ARIAL_NARROW, theme.sizes[spec[2]], "") end
        font:SetTextColor(Skin.Color(spec[3]))
        font:SetShadowColor(0, 0, 0, theme.shadow and 1 or 0)
        font:SetShadowOffset(theme.shadow and 1 or 0, theme.shadow and -1 or 0)
    end
end

-- ---------------------------------------------------------------- boxes
-- A box is a rectangle filled (and optionally outlined) in the theme's art, cut into nine
-- pieces so its corners keep their shape at any size. It is drawn with textures on its host
-- (in a draw layer), around an invisible anchor region: Box:SetPoint places that, so a box
-- can sit behind a host's own text (a message's bubble, the question's card).
local Box = {}
Box.__index = Box
local boxes = {}

local COORDS = { -- left, right, top, bottom of each piece, as fractions of the art (corner m)
    function(m) return 0, m, 0, m end, function(m) return m, 1 - m, 0, m end, function(m) return 1 - m, 1, 0, m end,
    function(m) return 0, m, m, 1 - m end, function(m) return m, 1 - m, m, 1 - m end, function(m) return 1 - m, 1, m, 1 - m end,
    function(m) return 0, m, 1 - m, 1 end, function(m) return m, 1 - m, 1 - m, 1 end, function(m) return 1 - m, 1, 1 - m, 1 end,
}

local function pieces(host, layer, sub, proxy)
    local p = {}
    for i = 1, 9 do p[i] = host:CreateTexture(nil, layer, nil, sub) end
    p[1]:SetPoint("TOPLEFT", proxy)
    p[3]:SetPoint("TOPRIGHT", proxy)
    p[7]:SetPoint("BOTTOMLEFT", proxy)
    p[9]:SetPoint("BOTTOMRIGHT", proxy)
    p[2]:SetPoint("TOPLEFT", p[1], "TOPRIGHT")
    p[2]:SetPoint("BOTTOMRIGHT", p[3], "BOTTOMLEFT")
    p[8]:SetPoint("TOPLEFT", p[7], "TOPRIGHT")
    p[8]:SetPoint("BOTTOMRIGHT", p[9], "BOTTOMLEFT")
    p[4]:SetPoint("TOPLEFT", p[1], "BOTTOMLEFT")
    p[4]:SetPoint("BOTTOMRIGHT", p[7], "TOPRIGHT")
    p[6]:SetPoint("TOPLEFT", p[3], "BOTTOMLEFT")
    p[6]:SetPoint("BOTTOMRIGHT", p[9], "TOPRIGHT")
    p[5]:SetPoint("TOPLEFT", p[1], "BOTTOMRIGHT")
    p[5]:SetPoint("BOTTOMRIGHT", p[9], "TOPLEFT")
    return p
end

local function paint(p, texture, c, r, g, b, a)
    for i, piece in ipairs(p) do
        piece:SetTexture(texture)
        piece:SetTexCoord(COORDS[i](0.25))
        piece:SetVertexColor(r, g, b, a)
    end
    for _, i in ipairs({ 1, 3, 7, 9 }) do p[i]:SetSize(c, c) end
end

local function showPieces(p, shown, ring)
    for i, piece in ipairs(p) do piece:SetShown(shown and not (ring and i == 5)) end
end

-- opts: shape ("panel", "control" or "pill"), fill and ring (color keys or { r, g, b, a };
-- no ring: none drawn), layer and sub (draw layer and sublevel), ringLayer (the ring's
-- layer, when it must draw over other boxes on the host), height (a pill's, for its
-- corners), free (don't cover the host: place it with SetPoint).
function Skin.Box(host, opts)
    opts = opts or {}
    local layer, sub = opts.layer or "BACKGROUND", opts.sub or -6
    local box = setmetatable({ shape = opts.shape or "panel", fill = opts.fill or "card", ring = opts.ring, height = opts.height, shown = true }, Box)
    box.proxy = host:CreateTexture(nil, layer, nil, sub)
    if not opts.free then box.proxy:SetAllPoints(host) end
    box.fills = pieces(host, layer, sub, box.proxy)
    if opts.ring ~= nil then
        box.rings = pieces(host, opts.ringLayer or layer, opts.ringLayer and -8 or sub + 1, box.proxy)
    end
    boxes[#boxes + 1] = box
    box:Apply()
    return box
end

function Box:Apply()
    local shape = theme.shapes[self.shape] or theme.shapes.panel
    local c = shape.c
    if c == "px" then c = Skin.px elseif c == "half" then c = (self.height or 16) / 2 end
    paint(self.fills, shape.art.fill, c, Skin.Color(self.fill))
    if self.rings then
        if self.ring then paint(self.rings, shape.art.ring, c, Skin.Color(self.ring)) end
        showPieces(self.rings, self.shown and self.ring ~= false, true)
    end
    showPieces(self.fills, self.shown and self.fill ~= false)
end

-- Recolors it: fill and ring as for Skin.Box; nil keeps one, false hides it.
function Box:SetColors(fill, ring)
    if fill ~= nil then self.fill = fill end
    if ring ~= nil then self.ring = ring end
    self:Apply()
end

function Box:SetHeight(height)
    self.height = height
    self:Apply()
end

function Box:SetShown(shown)
    self.shown = shown and true or false
    self:Apply()
end
function Box:Show() self:SetShown(true) end
function Box:Hide() self:SetShown(false) end
function Box:SetPoint(...) self.proxy:SetPoint(...) end
function Box:ClearAllPoints() self.proxy:ClearAllPoints() end
function Box:SetAllPoints(region) self.proxy:SetAllPoints(region) end

-- ---------------------------------------------------------------- glyphs
-- A small white shape (close, chevron, check, grip) tinted with a theme color.
local glyphs = {}
function Skin.Glyph(host, name, size, colorKey, layer)
    local tex = host:CreateTexture(nil, layer or "ARTWORK")
    tex:SetTexture(MEDIA .. name)
    tex:SetSize(size, size)
    tex.colorKey = colorKey or "muted"
    tex:SetVertexColor(Skin.Color(tex.colorKey))
    glyphs[#glyphs + 1] = tex
    return tex
end

-- Another shape on a glyph already made.
function Skin.SetGlyphFile(tex, name)
    tex:SetTexture(MEDIA .. name)
end

function Skin.SetGlyphColor(tex, colorKey)
    tex.colorKey = colorKey
    tex:SetVertexColor(Skin.Color(colorKey))
end

-- A hover highlight behind the frame's text (the HIGHLIGHT layer would draw over it),
-- shown while the mouse is over an enabled frame. Hooked, so call it after any SetScript
-- of OnEnter or OnLeave.
function Skin.Hover(f)
    local box = Skin.Box(f, { shape = "control", fill = "hover", layer = "BORDER", sub = 0 })
    box:Hide()
    f:HookScript("OnEnter", function(self)
        if not self.IsEnabled or self:IsEnabled() then box:Show() end
    end)
    f:HookScript("OnLeave", function() box:Hide() end)
    f:HookScript("OnHide", function() box:Hide() end)
    return box
end

-- A one-pixel divider in the theme's line color; place it with SetPoint.
function Skin.Line(host, layer)
    local tex = host:CreateTexture(nil, layer or "BORDER")
    tex:SetTexture(WHITE)
    tex.colorKey, tex.isLine = "line", true
    tex:SetVertexColor(Skin.Color("line"))
    glyphs[#glyphs + 1] = tex
    return tex
end

-- A chevron pointing down, or right when collapsed (turned by its texture coordinates).
function Skin.PointGlyph(tex, right)
    if right then tex:SetTexCoord(1, 0, 0, 0, 1, 1, 0, 1) else tex:SetTexCoord(0, 1, 0, 1) end
end

-- ---------------------------------------------------------------- buttons
-- A button's variant: "secondary" (raised, outlined), "primary" (the accent), "ghost"
-- (text only until hovered), "chip" (an option: outlined; picked, outlined in the accent).
local VARIANT = {
    secondary = { fill = "raised", ring = "border", font = "button" },
    primary = { fill = "primary", ring = "primaryRing", font = "buttonPrimary" },
    ghost = { fill = false, ring = false, font = "buttonGhost" },
    chip = { fill = "raised", ring = "border", font = "button" },
    picked = { fill = "selected", ring = "accent", font = "button" },
}
local buttons = {}

local function styleButton(b)
    local enabled = b:IsEnabled()
    local v = VARIANT[b.variant] or VARIANT.secondary
    if not enabled and b.variant == "primary" then v = VARIANT.secondary end
    b.box:SetColors(v.fill, v.ring)
    b.box.proxy:SetAlpha(1)
    for _, piece in ipairs(b.box.fills) do piece:SetAlpha(enabled and 1 or 0.45) end
    for _, piece in ipairs(b.box.rings) do piece:SetAlpha(enabled and 1 or 0.45) end
    b:SetNormalFontObject(Skin.fonts[v.font])
    b:SetHighlightFontObject(Skin.fonts[v.font])
end

-- Makes a Button (made without a template) a themed one.
function Skin.Button(b, variant)
    b.variant = variant or "secondary"
    b.box = Skin.Box(b, { shape = "control", fill = "raised", ring = "border", sub = -6 })
    b.hover = Skin.Hover(b)
    local fs = b:GetFontString() or b:CreateFontString(nil, "OVERLAY")
    fs:SetFontObject(Skin.fonts.button)
    fs:SetPoint("CENTER")
    b:SetFontString(fs)
    b:SetDisabledFontObject(Skin.fonts.buttonDisabled)
    b:HookScript("OnEnable", styleButton)
    b:HookScript("OnDisable", styleButton)
    buttons[#buttons + 1] = b
    styleButton(b)
    return b
end

function Skin.SetVariant(b, variant)
    if b.variant == variant then return end
    b.variant = variant
    styleButton(b)
end

-- A close button: the close glyph, brighter when hovered.
function Skin.CloseButton(parent, onClick)
    local b = CreateFrame("Button", nil, parent)
    b:SetSize(24, 24)
    b.hover = Skin.Hover(b)
    b.glyph = Skin.Glyph(b, "close", 16, "muted")
    b.glyph:SetPoint("CENTER")
    b:SetScript("OnClick", onClick)
    return b
end

-- ---------------------------------------------------------------- checkboxes
-- A CheckButton drawn as the theme's check: a toggle switch (T3), a box filled in the
-- accent (Flat) or a box with a tick (Classic). Skin.UpdateCheck redraws it after a change.
local checks = {}
function Skin.Check(parent)
    local cb = CreateFrame("CheckButton", nil, parent)
    cb:SetSize(28, 18)
    cb.track = Skin.Box(cb, { shape = "pill", fill = "off", ring = "border", height = 16, free = true })
    cb.knob = Skin.Box(cb, { shape = "pill", fill = "knob", height = 12, free = true, layer = "ARTWORK" })
    cb.tick = Skin.Glyph(cb, "check", 16, "accent", "OVERLAY")
    checks[#checks + 1] = cb
    Skin.UpdateCheck(cb)
    return cb
end

function Skin.UpdateCheck(cb)
    local on = cb:GetChecked() and true or false
    local style = theme.check
    cb:SetWidth(style == "toggle" and 28 or (style == "tick" and 16 or 12))
    cb.track:ClearAllPoints()
    cb.knob:ClearAllPoints()
    cb.tick:ClearAllPoints()
    if style == "toggle" then
        cb.track:SetPoint("LEFT", cb, "LEFT", 0, 0)
        cb.track.proxy:SetSize(28, 16)
        cb.track.shape, cb.track.height = "pill", 16
        cb.track:SetColors(on and "accent" or "off", false)
        cb.knob.proxy:SetSize(12, 12)
        cb.knob.height = 12
        cb.knob:SetPoint("LEFT", cb, "LEFT", on and 14 or 2, 0)
        cb.knob:SetColors(on and "knobOn" or "knob")
        cb.knob:Show()
        cb.tick:Hide()
    else
        local size = style == "tick" and 16 or 12
        cb.track:SetPoint("LEFT", cb, "LEFT", 0, 0)
        cb.track.proxy:SetSize(size, size)
        cb.track.shape = "control"
        cb.track:SetColors("off", "border")
        cb.knob.proxy:SetSize(size - 6, size - 6)
        cb.knob:SetPoint("CENTER", cb.track.proxy, "CENTER")
        cb.knob.shape = "control"
        cb.knob:SetColors("accent")
        cb.knob:SetShown(on and style == "fill")
        cb.tick:SetPoint("CENTER", cb.track.proxy, "CENTER", 1, 1)
        cb.tick:SetShown(on and style == "tick")
    end
    cb.track:Show()
end

-- ---------------------------------------------------------------- sliders
-- A Slider drawn as a thin track filled up to a round (or, in Flat, square) thumb.
local sliders = {}
function Skin.Slider(slider)
    slider.track = Skin.Box(slider, { shape = "pill", fill = "off", height = 4, free = true })
    slider.track:SetPoint("LEFT", slider, "LEFT", 0, 0)
    slider.track:SetPoint("RIGHT", slider, "RIGHT", 0, 0)
    slider.track.proxy:SetHeight(4)
    slider.fillBar = Skin.Box(slider, { shape = "pill", fill = "accent", height = 4, free = true, sub = -4 })
    slider.fillBar:SetPoint("LEFT", slider, "LEFT", 0, 0)
    slider.fillBar.proxy:SetHeight(4)
    slider:SetThumbTexture(MEDIA .. "round")
    sliders[#sliders + 1] = slider
    Skin.UpdateSlider(slider)
    return slider
end

function Skin.UpdateSlider(slider)
    local thumb = slider:GetThumbTexture()
    local square = theme.check == "fill" or theme.check == "tick"
    if type(thumb) == "table" then
        thumb:SetTexture(square and WHITE or MEDIA .. "round")
        thumb:SetTexCoord(0, 1, 0, 1)
        thumb:SetSize(square and 6 or 14, 14)
        thumb:SetVertexColor(Skin.Color(square and "muted" or "accent"))
    end
    local low, high = slider:GetMinMaxValues()
    local share = (high and low and high > low) and (slider:GetValue() - low) / (high - low) or 0
    slider.fillBar.proxy:SetWidth(math.max(1, (slider:GetWidth() or 0) * share))
end

-- ---------------------------------------------------------------- scroll bars
-- The scroll bar of a UIPanelScrollFrameTemplate frame, thinned to a bare thumb beside it.
local scrollBars = {}
function Skin.ScrollBar(scroll)
    local name = scroll:GetName()
    local bar = scroll.ScrollBar or (name and _G[name .. "ScrollBar"])
    if type(bar) ~= "table" or not bar.GetThumbTexture then return end
    local barName = bar:GetName()
    for _, key in ipairs({ "ScrollUpButton", "ScrollDownButton" }) do
        local arrow = bar[key] or (barName and _G[barName .. key])
        if type(arrow) == "table" then
            arrow:SetAlpha(0)
            arrow:EnableMouse(false)
            arrow:SetSize(1, 1)
        end
    end
    bar:ClearAllPoints()
    bar:SetPoint("TOPLEFT", scroll, "TOPRIGHT", 4, 0)
    bar:SetPoint("BOTTOMLEFT", scroll, "BOTTOMRIGHT", 4, 0)
    bar:SetWidth(6)
    local thumb = bar:GetThumbTexture()
    for _, region in ipairs({ bar:GetRegions() }) do
        if region ~= thumb and region.IsObjectType and region:IsObjectType("Texture") then region:SetAlpha(0) end
    end
    bar.t3Skinned = true
    scrollBars[#scrollBars + 1] = bar
    Skin.UpdateScrollBar(bar)
end

function Skin.UpdateScrollBar(bar)
    local square = theme.check == "fill"
    bar:SetThumbTexture(square and WHITE or MEDIA .. "round")
    local thumb = bar:GetThumbTexture()
    if type(thumb) == "table" then
        thumb:SetTexCoord(0, 1, 0, 1)
        thumb:SetSize(6, 36)
        thumb:SetVertexColor(Skin.Color("scroll"))
    end
end

-- ---------------------------------------------------------------- pills
-- A status pill: its text in the status color on a faint fill of it. Pill:Set sizes it to
-- the text.
local Pill = {}
Pill.__index = Pill
local pills = {}
function Skin.Pill(host)
    local pill = setmetatable({}, Pill)
    pill.box = Skin.Box(host, { shape = "pill", fill = { 1, 1, 1, 0 }, height = 16, free = true, layer = "ARTWORK", sub = 0 })
    pill.text = host:CreateFontString(nil, "OVERLAY")
    pill.text:SetFontObject(Skin.fonts.pill)
    pill.text:SetPoint("CENTER", pill.box.proxy, "CENTER", 0, 0)
    pill.box.proxy:SetHeight(16)
    pills[#pills + 1] = pill
    return pill
end

function Pill:Set(text, hexColor)
    self.label, self.hexColor = text, hexColor
    self.text:SetText(text or "")
    local r, g, b = tonumber(hexColor:sub(1, 2), 16) / 255, tonumber(hexColor:sub(3, 4), 16) / 255, tonumber(hexColor:sub(5, 6), 16) / 255
    self.text:SetTextColor(r, g, b)
    self.box:SetColors({ r, g, b, theme.pillAlpha })
    local pad = theme.pillAlpha > 0 and 14 or 0
    self.box.proxy:SetWidth((self.text:GetStringWidth() or 0) + pad)
end

function Pill:SetPoint(...) self.box:SetPoint(...) end
function Pill:ClearAllPoints() self.box:ClearAllPoints() end
function Pill:SetShown(shown)
    self.box:SetShown(shown)
    self.text:SetShown(shown)
end
function Pill:Width() return self.box.proxy:GetWidth() or 0 end

-- ---------------------------------------------------------------- applying a theme

local listeners = {}
-- fn runs after every Apply: layout that depends on the theme (row heights, widths).
function Skin.OnApply(fn) listeners[#listeners + 1] = fn end

function Skin.Apply()
    local saved = T3WoWDB and T3WoWDB.settings and T3WoWDB.settings.theme
    current = Skin.THEMES[saved] and saved or "t3"
    theme = resolve(current)
    Skin.px = pixel()
    applyFonts()
    for _, box in ipairs(boxes) do box:Apply() end
    for _, tex in ipairs(glyphs) do
        tex:SetVertexColor(Skin.Color(tex.colorKey))
        if tex.isLine then
            if tex.vertical then tex:SetWidth(Skin.px) else tex:SetHeight(Skin.px) end
        end
    end
    for _, b in ipairs(buttons) do styleButton(b) end
    for _, cb in ipairs(checks) do Skin.UpdateCheck(cb) end
    for _, slider in ipairs(sliders) do Skin.UpdateSlider(slider) end
    for _, bar in ipairs(scrollBars) do Skin.UpdateScrollBar(bar) end
    for _, pill in ipairs(pills) do if pill.hexColor then pill:Set(pill.label, pill.hexColor) end end
    for _, fn in ipairs(listeners) do fn() end
end

function Skin.Set(key)
    if not Skin.THEMES[key] then return end
    T3WoWDB = T3WoWDB or {}
    T3WoWDB.settings = T3WoWDB.settings or {}
    T3WoWDB.settings.theme = key
    Skin.Apply()
end

applyFonts()

-- Saved settings (and ElvUI's media) exist from login on: draw in the saved theme then.
local events = CreateFrame("Frame")
events:RegisterEvent("PLAYER_LOGIN")
events:RegisterEvent("UI_SCALE_CHANGED")
events:RegisterEvent("DISPLAY_SIZE_CHANGED")
events:SetScript("OnEvent", function() Skin.Apply() end)
