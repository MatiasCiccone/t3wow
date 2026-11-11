local _, ns = ...
local UI = {}
ns.UI = UI
local Skin = ns.Skin

-- Theme.lua came with this version. The game finds a new file only when it starts, so
-- after an update installed while it ran, a Reload UI loads this without it: say so, and
-- keep the rest of the addon from tripping over a window that wasn't built.
if not Skin then
    local said = false
    local function tell()
        if said then return end
        said = true
        print("|cff8f8fffT3 WoW|r was updated: restart WoW to load it (/reload isn't enough).")
    end
    setmetatable(UI, { __index = function() return function() end end })
    UI.Toggle, UI.ShowSettings = tell, tell
    tell()
    return
end

local format, floor = string.format, math.floor
local WIDTH, HEIGHT, LIST_WIDTH = 940, 580, 300
local MIN_WIDTH, MIN_HEIGHT, MAX_WIDTH, MAX_HEIGHT = 720, 420, 2400, 1600
local WORKING = { preparing = true, queued = true, starting = true, running = true }
-- A thread whose agent watches a pull request (its checks, its reviews) after its run ended:
-- T3's sidebar files it under Working as "Waiting", though there's no run to stop.
local function inWorking(status) return WORKING[status] or status == "watching" end
-- A run Stop can cancel: working, or waiting on something only T3 can answer (an approval,
-- a card), whose run goes on until it's answered. On UI: the file is out of locals.
function UI.Stoppable(status) return WORKING[status] == true or status == "approval" or status == "card" end
local STATUS = {
    preparing = { "Working", "60a5fa", "spinner" }, queued = { "Queued", "60a5fa", "spinner" }, starting = { "Working", "60a5fa", "spinner" },
    running = { "Working", "60a5fa", "spinner" }, completed = { "Finished", "40c040" }, failed = { "Failed", "ff4040" },
    -- As T3 says it: Input, in its indigo, with a question mark; "notice" is for a notification, where the one word says little.
    waiting = { "Input", "a5b4fc", "question", notice = "Awaiting input" }, approval = { "Approval", "fbbf24", "shield", notice = "Approve in T3" }, interrupted = { "Stopped", "a0a0a0" }, cancelled = { "Stopped", "a0a0a0" },
    -- A card only T3's own window can answer (a secret, a machine link): T3 says Input too.
    card = { "Input", "a5b4fc", "question", notice = "Answer in T3" },
    idle = { "Idle", "a0a0a0" }, rolled_back = { "Rolled back", "a0a0a0" }, watching = { "Waiting", "ffd100" },
}
-- A linked pull request's state, in the colors GitHub gives them. Another state keeps the
-- text's own color.
local PullRequests = { STATE = { open = { "Open", "40c040" }, merged = { "Merged", "a371f7" }, closed = { "Closed", "ff4040" } } }
function PullRequests.Colored(number, state)
    local s = PullRequests.STATE[state]
    return s and format("|cff%s%s|r", s[2], number) or number
end

-- The game can't open a browser, so the companion does.
function PullRequests.Open(envId, threadId, pr)
    ns.Link.Send({ t = "pr", e = envId, id = threadId, u = pr.u })
    UI.Notice("Opening " .. pr.n .. " in your browser…")
end

function PullRequests.TextButton(parent)
    local b = CreateFrame("Button", nil, parent)
    b:SetHeight(16)
    b.label = b:CreateFontString(nil, "OVERLAY")
    b.label:SetFontObject(Skin.fonts.small)
    b.label:SetPoint("CENTER")
    return b
end

function PullRequests.Fit(b) b:SetWidth((b.label:GetStringWidth() or 0) + 2) end

-- A pull request's number, opening it when clicked. owner() says whose it is: machine, thread.
function PullRequests.NewLink(parent, owner)
    local link = PullRequests.TextButton(parent)
    link:SetScript("OnClick", function(self)
        local envId, threadId = owner()
        PullRequests.Open(envId, threadId, self.pr)
    end)
    link:SetScript("OnEnter", function(self)
        local state = PullRequests.STATE[self.pr.s]
        Skin.Tooltip:SetOwner(self, "ANCHOR_BOTTOM")
        Skin.Tooltip:AddLine(state and (self.pr.n .. " · " .. state[1]) or self.pr.n)
        if self.pr.u then Skin.Tooltip:AddLine("Click to open in your browser") end
        Skin.Tooltip:Show()
    end)
    link:SetScript("OnLeave", function() Skin.Tooltip:Hide() end)
    return link
end

function PullRequests.SetLink(link, pr)
    link.pr = pr
    link.label:SetText(PullRequests.Colored(pr.n, pr.s))
    link:SetEnabled(pr.u ~= nil)
    PullRequests.Fit(link)
end

local inbox
-- { env = envId, id = threadId } or { new = true, env = envId, project = projectId or nil,
-- worktree = true to start it in a new worktree, and what you picked of pv, m (provider and
-- model), ef (effort) and rm (permissions), each nil for T3's default }.
-- Machines and projects are kept by id: their places in the lists change between updates.
local selected
local collapsed = { settled = true, snoozed = true }
local echo -- { id, text, s } shown until the thread view catches up
local notice
-- Commands whose outcome matters, by number, until the companion reports it (inbox.out):
-- { kind = "send" | "new" | "answer", text, at }. A failure puts the text back.
local awaiting = {}
local ANSWER_TIMEOUT = 45 -- an answer with no word back stops showing "Sending…"
-- The Stop under way, { env, id, s, at, taken }: its thread's Stop shows "Stopping…" until
-- the thread stops working, the companion says there was nothing to stop or T3 refused, or
-- STOP_TIMEOUT passes (taken: T3 accepted it, so the run is just slow to end).
local stopping
local STOP_TIMEOUT = 60

local function hexColor(hex)
    return tonumber(hex:sub(1, 2), 16) / 255, tonumber(hex:sub(3, 4), 16) / 255, tonumber(hex:sub(5, 6), 16) / 255
end

local function statusText(status)
    local s = STATUS[status] or { status or "?", "a0a0a0" }
    return format("|cff%s%s|r", s[2], s[1])
end

local function age(at)
    if not at or at == 0 then return "" end
    local d = math.max(0, time() - at)
    if d < 60 then return "now" end
    if d < 3600 then return floor(d / 60) .. "m" end
    if d < 86400 then return floor(d / 3600) .. "h" end
    return floor(d / 86400) .. "d"
end

local function env(index) return inbox and inbox.envs and inbox.envs[index] end

local function envIndexById(id)
    for index, e in ipairs(inbox and inbox.envs or {}) do
        if e.id == id then return index end
    end
end

local function envById(id) return env(envIndexById(id)) end

-- The machine id behind a list entry's machine index.
local function envIdOf(index)
    local e = env(index)
    return e and e.id
end

local function listedThread(envId, threadId)
    for _, t in ipairs(inbox and inbox.threads or {}) do
        if t.id == threadId and envIdOf(t.e) == envId then return t end
    end
end

-- The listed thread that is selected, if it is listed.
local function selectedThread()
    if not selected or selected.new then return nil end
    return listedThread(selected.env, selected.id)
end

local function isSelectedView(view)
    return view ~= nil and selected ~= nil and not selected.new and view.id == selected.id and envIdOf(view.e) == selected.env
end

-- The companion's tray setting "Commands from the game" (inbox.trust: "nonew" or
-- "readonly"; an older companion leaves it out) limits what the game may make T3 do. The
-- companion refuses those commands anyway: the window greys them out and says why.
local TRUST_NAME = { nonew = "No new threads", readonly = "Read-only" }
local CHANGES_T3 = { send = true, new = true, settle = true, unsettle = true, snooze = true, unsnooze = true, stop = true, answer = true, paste = true, model = true, script = true }
local BLOCKED_LEAD = {
    send = "Sending messages from the game is off", new = "Starting threads from the game is off",
    script = "Running scripts from the game is off",
    paste = "Pasting images from the game is off", model = "Changing the model from the game is off",
    answer = "Answering from the game is off", settle = "Settle is off", unsettle = "Unsettle is off", stop = "Stop is off",
    snooze = "Snooze is off", unsnooze = "Wake is off",
}
local function blockedReason(kind)
    local trust = inbox and inbox.trust
    if (trust == "readonly" and CHANGES_T3[kind]) or (trust == "nonew" and kind == "new") then
        return format('%s: the T3 WoW tray menu has "Commands from the game" set to %s.', BLOCKED_LEAD[kind], TRUST_NAME[trust])
    end
end

-- ---------------------------------------------------------------- frame

local frame = CreateFrame("Frame", "T3WoWFrame", UIParent)
frame:SetSize(WIDTH, HEIGHT)
frame:SetPoint("CENTER")
frame:SetFrameStrata("HIGH")
frame:SetMovable(true)
frame:EnableMouse(true)
frame:RegisterForDrag("LeftButton")
frame:SetScript("OnDragStart", frame.StartMoving)
frame:SetResizable(true)
if frame.SetResizeBounds then
    frame:SetResizeBounds(MIN_WIDTH, MIN_HEIGHT, MAX_WIDTH, MAX_HEIGHT)
else
    frame:SetMinResize(MIN_WIDTH, MIN_HEIGHT)
    frame:SetMaxResize(MAX_WIDTH, MAX_HEIGHT)
end

-- Size and position live in T3WoWDB.window, restored the first time the window opens.
local function saveLayout()
    if not T3WoWDB then return end
    local point, _, relativePoint, x, y = frame:GetPoint()
    T3WoWDB.window = { w = frame:GetWidth(), h = frame:GetHeight(), point = point, relativePoint = relativePoint, x = x, y = y }
end

local layoutRestored = false
local function restoreLayout()
    if layoutRestored or not T3WoWDB then return end
    layoutRestored = true
    local saved = T3WoWDB.window
    if not saved then return end
    frame:SetSize(math.max(MIN_WIDTH, saved.w or WIDTH), math.max(MIN_HEIGHT, saved.h or HEIGHT))
    if saved.point then
        frame:ClearAllPoints()
        frame:SetPoint(saved.point, UIParent, saved.relativePoint or saved.point, saved.x or 0, saved.y or 0)
    end
end

frame:SetScript("OnDragStop", function(self)
    self:StopMovingOrSizing()
    saveLayout()
end)
frame:SetClampedToScreen(true)
-- Sizes of the window's parts (one table: a file's main chunk may hold only 200 locals).
local SIZE = {
    title = 40, footer = 48, -- the title bar and the footer
    header = 58, -- a thread's title and status, above the conversation
    search = 36, -- the search box over the thread list, with its margins
    row = LIST_WIDTH - 26, -- a thread row's width
    bubbleX = 12, bubbleY = 8, -- the padding inside a message bubble
    card = 14, -- the padding inside the question's card
    settings = 300, -- the settings panel's width
    -- Where a labelled control (a dropdown, the slider) starts in Settings: past the widest
    -- label, "Stay on screen", which T3's Inter draws widest.
    field = 132,
    composer = 126, -- the message box and its row of selectors, with their margins
}
-- The window is drawn in the theme (Theme.lua): a panel with a title bar and a footer, the
-- thread list down the left.
Skin.Box(frame, { shape = "panel", fill = "bg", ring = "border", ringLayer = "BORDER", sub = -8 })
do
    local titleLine = Skin.Line(frame)
    titleLine:SetPoint("TOPLEFT", 1, -SIZE.title)
    titleLine:SetPoint("TOPRIGHT", -1, -SIZE.title)
    local footerLine = Skin.Line(frame)
    footerLine:SetPoint("BOTTOMLEFT", 1, SIZE.footer)
    footerLine:SetPoint("BOTTOMRIGHT", -1, SIZE.footer)
end
frame:Hide()
tinsert(UISpecialFrames, "T3WoWFrame")

-- Escape closes a popup over the window and leaves the window open. The game hides every
-- frame in UISpecialFrames at once, and listing a frame in UIMenus doesn't help: 1.60's
-- Escape goes on to UISpecialFrames after hiding it. So while it shows, a popup holds the
-- window's place in that list. Only one shows at a time: each covers the screen with a
-- click catcher. Call this after setting the popup's OnHide, on a popup that starts hidden.
local function popupTakesEscape(popup)
    local name = popup:GetName()
    -- In place, as the game may be going through the list when a popup hides.
    local function swap(from, to)
        for i, listed in ipairs(UISpecialFrames) do
            if listed == from then
                UISpecialFrames[i] = to
                return
            end
        end
    end
    local onHide = popup:GetScript("OnHide")
    popup:SetScript("OnShow", function() swap("T3WoWFrame", name) end)
    popup:SetScript("OnHide", function(self)
        swap(name, "T3WoWFrame")
        if onHide then onHide(self) end
    end)
end

local title = frame:CreateFontString(nil, "OVERLAY")
title:SetFontObject(Skin.fonts.title)
title:SetPoint("LEFT", frame, "TOPLEFT", 14, -SIZE.title / 2)
title:SetText("T3 WoW")

local close = Skin.CloseButton(frame, function() frame:Hide() end)
close:SetPoint("TOPRIGHT", -8, -8)

-- The link's state, in a pill beside the title: a dot in its color, and what it is.
local connectionPill = Skin.Box(frame, { shape = "pill", fill = "card", ring = "border", height = 20, free = true, layer = "ARTWORK" })
connectionPill:SetPoint("LEFT", title, "RIGHT", 12, 0)
connectionPill.proxy:SetHeight(20)
local connectionDot = Skin.Box(frame, { shape = "pill", fill = "dim", height = 6, free = true, layer = "ARTWORK", sub = 2 })
connectionDot:SetPoint("LEFT", connectionPill.proxy, "LEFT", 8, 0)
connectionDot.proxy:SetSize(6, 6)
local connection = frame:CreateFontString(nil, "OVERLAY")
connection:SetFontObject(Skin.fonts.muted)
connection:SetPoint("LEFT", connectionPill.proxy, "LEFT", 20, 0)
connection:SetJustifyH("LEFT")
connection:SetWordWrap(false)
-- While the pill says to Reload UI, a click on it does (shown only then: renderStatus). It
-- sits on the pill's table: the main chunk is near its 200 locals. Dragging it still moves
-- the window.
connectionPill.hit = CreateFrame("Button", nil, frame)
do
    local hit = connectionPill.hit
    hit:SetAllPoints(connectionPill.proxy)
    hit:RegisterForClicks("LeftButtonUp")
    hit:RegisterForDrag("LeftButton")
    hit:SetScript("OnClick", function() ReloadUI() end)
    hit:SetScript("OnDragStart", function() frame:StartMoving() end)
    hit:SetScript("OnDragStop", function() frame:GetScript("OnDragStop")(frame) end)
    hit:SetScript("OnEnter", function(self)
        Skin.Tooltip:SetOwner(self, "ANCHOR_BOTTOM")
        Skin.Tooltip:AddLine("Click to Reload UI")
        Skin.Tooltip:Show()
    end)
    hit:SetScript("OnLeave", function() Skin.Tooltip:Hide() end)
    hit:Hide()
end

local function makeScroll(parent, width)
    local scroll = CreateFrame("ScrollFrame", nil, parent, "UIPanelScrollFrameTemplate")
    local child = CreateFrame("Frame", nil, scroll)
    child:SetSize(width, 1)
    scroll:SetScrollChild(child)
    Skin.ScrollBar(scroll)
    return scroll, child
end

-- variant: "secondary" (the default), "primary" or "ghost" (Skin.Button).
local function button(parent, text, width, onClick, variant)
    local b = Skin.Button(CreateFrame("Button", nil, parent), variant)
    b:SetSize(width, 28)
    b:SetText(text)
    b:SetScript("OnClick", onClick)
    return b
end

-- A square button with an icon, named by its tooltip. A game started before the icon's file
-- shipped shows the short word instead; asked each time it shows, as the game can take a
-- while to say.
function UI.IconButton(parent, glyph, tip, short, onClick)
    local b = CreateFrame("Button", nil, parent)
    b:SetSize(28, 28)
    b.tip = tip
    b.glyph = Skin.Glyph(b, glyph, 16, "muted", "OVERLAY")
    b.glyph:SetPoint("CENTER")
    b.short = b:CreateFontString(nil, "OVERLAY")
    b.short:SetFontObject(Skin.fonts.small)
    b.short:SetPoint("CENTER")
    b.short:SetText(short)
    b.short:Hide()
    b:SetScript("OnClick", onClick)
    b:SetScript("OnShow", function(self)
        local drawn = UI.GlyphKnown(self.glyph)
        self.glyph:SetShown(drawn)
        self.short:SetShown(not drawn)
    end)
    b:SetScript("OnEnter", function(self)
        Skin.Tooltip:SetOwner(self, "ANCHOR_TOP")
        Skin.Tooltip:AddLine(self.tip)
        Skin.Tooltip:Show()
    end)
    b:SetScript("OnLeave", function() Skin.Tooltip:Hide() end)
    Skin.Hover(b)
    return b
end

-- A dropdown: click it to open a list, pick one. Every dropdown is built here, so a reskin
-- restyles them in one place. menu.entries() returns the list, asked again each time it
-- opens: { text, value } entries can be picked (menu.pick(value), and menu.isSelected(value)
-- checks one), { text, note = true } lines can't (an empty list's note, a group's name). The
-- widget's SetLabel(text) sets what it shows closed; menu.above opens the list over it.
-- The list is the addon's own, drawn in the theme: Blizzard's menus draw their own art,
-- shared with every other addon, and UIDropDownMenu can taint the game's menus. One list
-- serves every dropdown and opens under the one clicked; past 15 rows it scrolls.
local dropdown
do
    local ROWS, ROW_HEIGHT, PAD = 15, 20, 4
    local list = CreateFrame("Frame", "T3WoWDropdownList", UIParent)
    list:SetFrameStrata("FULLSCREEN_DIALOG")
    list:SetFrameLevel(20)
    list:SetClampedToScreen(true)
    list:EnableMouse(true)
    list:Hide()
    Skin.Box(list, { shape = "control", fill = "card", ring = "border", ringLayer = "BORDER" })
    local scroll, child = makeScroll(list, 100)
    scroll:SetPoint("TOPLEFT", PAD, -PAD)
    -- A click anywhere else closes it, as the game's menus do.
    local catcher = CreateFrame("Button", nil, UIParent)
    catcher:SetFrameStrata("FULLSCREEN_DIALOG")
    catcher:SetFrameLevel(10)
    catcher:SetAllPoints(UIParent)
    catcher:RegisterForClicks("AnyUp")
    catcher:SetScript("OnClick", function() list:Hide() end)
    catcher:Hide()

    local rows, owner = {}, nil
    list.rows = rows
    list:SetScript("OnHide", function()
        catcher:Hide()
        owner = nil
    end)
    popupTakesEscape(list)

    local function row(index)
        local r = rows[index]
        if r then return r end
        r = CreateFrame("Button", nil, child)
        r:SetHeight(ROW_HEIGHT)
        local fs = r:CreateFontString(nil, "OVERLAY")
        fs:SetFontObject(Skin.fonts.text)
        fs:SetPoint("LEFT", 24, 0)
        fs:SetPoint("RIGHT", -8, 0)
        fs:SetJustifyH("LEFT")
        fs:SetWordWrap(false)
        r:SetFontString(fs)
        r:SetNormalFontObject(Skin.fonts.text)
        r:SetHighlightFontObject(Skin.fonts.text)
        r:SetDisabledFontObject(Skin.fonts.small)
        r.check = Skin.Glyph(r, "check", 14, "accent", "OVERLAY")
        r.check:SetPoint("LEFT", 6, 0)
        r:SetScript("OnClick", function(self)
            local menu, value = owner and owner.menu, self.entry and self.entry.value
            list:Hide()
            if menu then menu.pick(value) end
        end)
        Skin.Hover(r)
        rows[index] = r
        return r
    end

    local function open(dd)
        owner = dd
        local entries = dd.menu.entries()
        local width = dd:GetWidth() or 0
        for index, entry in ipairs(entries) do
            local r = row(index)
            r.entry = entry
            r:SetText(entry.text)
            r.checked = not entry.note and dd.menu.isSelected(entry.value) and true or false
            r.check:SetShown(r.checked)
            r:SetEnabled(not entry.note)
            r:ClearAllPoints()
            r:SetPoint("TOPLEFT", child, "TOPLEFT", 0, -(index - 1) * ROW_HEIGHT)
            r:SetPoint("TOPRIGHT", child, "TOPRIGHT", 0, -(index - 1) * ROW_HEIGHT)
            r:Show()
            width = math.max(width, (r:GetFontString():GetStringWidth() or 0) + 36)
        end
        for index = #entries + 1, #rows do rows[index]:Hide() end
        local scrolls = #entries > ROWS
        local shown = math.min(#entries, ROWS)
        list.scrollHeight = scrolls and shown * ROW_HEIGHT or false
        local bar = scroll.ScrollBar
        if type(bar) == "table" and bar.SetShown then bar:SetShown(scrolls) end
        child:SetSize(width, math.max(1, #entries * ROW_HEIGHT))
        -- Room on the right for the scroll bar, which sits just outside the scroll frame.
        scroll:SetPoint("BOTTOMRIGHT", list, "BOTTOMRIGHT", -PAD - (scrolls and 12 or 0), PAD)
        list:SetSize(width + 2 * PAD + (scrolls and 12 or 0), math.max(1, shown) * ROW_HEIGHT + 2 * PAD)
        scroll:SetVerticalScroll(0)
        list:ClearAllPoints()
        if dd.menu.above then
            list:SetPoint("BOTTOMLEFT", dd, "TOPLEFT", 0, 2)
        else
            list:SetPoint("TOPLEFT", dd, "BOTTOMLEFT", 0, -2)
        end
        catcher:Show()
        list:Show()
    end

    -- The same list, opened from any other button.
    function UI.OpenMenu(anchor, menu)
        anchor.menu = menu
        open(anchor)
    end

    function dropdown(parent, name, width, menu)
        local dd = CreateFrame("Button", name, parent)
        dd:SetSize(width, 26)
        dd.menu = menu
        Skin.Box(dd, { shape = "control", fill = "input", ring = "border" })
        local chevron = Skin.Glyph(dd, "chevron", 14, "muted", "OVERLAY")
        chevron:SetPoint("RIGHT", -8, 0)
        dd.label = dd:CreateFontString(nil, "OVERLAY")
        dd.label:SetFontObject(Skin.fonts.text)
        dd.label:SetPoint("LEFT", 9, 0)
        dd.label:SetPoint("RIGHT", chevron, "LEFT", -4, 0)
        dd.label:SetJustifyH("LEFT")
        dd.label:SetWordWrap(false)
        dd:SetScript("OnClick", function(self)
            if owner == self and list:IsShown() then list:Hide() else open(self) end
        end)
        dd:SetScript("OnHide", function(self) if owner == self then list:Hide() end end)
        Skin.Hover(dd)
        -- An open list stays as it is under the cursor; it's asked for again next time.
        function dd:SetLabel(text)
            self.text = text
            self.label:SetText(text)
        end
        return dd
    end
end

-- ---------------------------------------------------------------- thread list

-- The list sits on its own panel down the left, divided from the conversation by a line.
do
    local listPanel = Skin.Box(frame, { shape = "panel", fill = "panel", free = true, sub = -7 })
    listPanel:SetPoint("TOPLEFT", frame, "TOPLEFT", 1, -SIZE.title - 1)
    listPanel:SetPoint("BOTTOMRIGHT", frame, "BOTTOMLEFT", LIST_WIDTH, SIZE.footer + 1)
    local listLine = Skin.Line(frame)
    listLine.vertical = true
    listLine:SetPoint("TOPLEFT", frame, "TOPLEFT", LIST_WIDTH, -SIZE.title)
    listLine:SetPoint("BOTTOMLEFT", frame, "BOTTOMLEFT", LIST_WIDTH, SIZE.footer)
end

-- Open runs down from under the search box. Working, Snoozed and Settled are docked at the
-- bottom, as in T3's sidebar: opening one pushes the dock up into the list. Each scrolls on
-- its own, so the dock stays in reach however long Open is.
local listScroll, listChild = makeScroll(frame, SIZE.row)
local List = { headers = {}, openHeight = 0, dockHeight = 0 }
List.dock, List.dockChild = makeScroll(frame, SIZE.row)
List.dock:SetPoint("BOTTOMLEFT", 8, SIZE.footer + 6)
List.dock:SetSize(SIZE.row, 1)
listScroll:SetPoint("TOPLEFT", 8, -SIZE.title - SIZE.search - 6)
listScroll:SetPoint("BOTTOMLEFT", List.dock, "TOPLEFT", 0, 4)
listScroll:SetWidth(SIZE.row)

-- The dock is as tall as its groups, up to what Open leaves; when both want more than half
-- the list, each gets half. A scroll bar shows only when there's something to scroll.
function List.FitDock()
    local room = frame:GetHeight() - SIZE.title - SIZE.search - SIZE.footer - 16
    local height = math.max(math.min(List.dockHeight, math.max(room / 2, room - List.openHeight)), 1)
    List.dock:SetHeight(height)
    local function showBar(scroll, scrolls)
        local bar = scroll.ScrollBar
        if type(bar) == "table" and bar.SetShown then bar:SetShown(scrolls) end
    end
    showBar(List.dock, List.dockHeight > height)
    showBar(listScroll, List.openHeight > room - height)
end

local rows = {}
local search -- the search box over the list (built after renderList)

-- Compact (a setting): one line per thread, its machine and project in the tooltip.
local function compactList() return T3WoWDB ~= nil and T3WoWDB.settings ~= nil and T3WoWDB.settings.compact == true end

-- Statuses with nothing to say on a row: as in T3, it shows the thread's age instead.
UI.AtRest = { completed = true, idle = true, interrupted = true, cancelled = true, rolled_back = true }

-- The game finds a new file only when it starts: a game started before this version shows
-- no icon rather than a square. Asked each time, as the answer can come a while after SetTexture.
function UI.GlyphKnown(glyph)
    local file = not glyph.GetTextureFilePath or glyph:GetTextureFilePath()
    return file ~= nil and file ~= ""
end

-- Headers and rows are reused by position, so one can move between the list and the dock.
local function header(index, parent)
    local h = List.headers[index]
    if h and h:GetParent() ~= parent then h:SetParent(parent) end
    if not h then
        h = CreateFrame("Button", nil, parent)
        h:SetSize(SIZE.row, 26)
        h.chevron = Skin.Glyph(h, "chevron", 12, "label")
        h.chevron:SetPoint("LEFT", 4, 0)
        h.text = h:CreateFontString(nil, "OVERLAY")
        h.text:SetFontObject(Skin.fonts.label)
        h.text:SetPoint("LEFT", 20, 0)
        h.count = h:CreateFontString(nil, "OVERLAY")
        h.count:SetFontObject(Skin.fonts.label)
        h.count:SetPoint("RIGHT", -8, 0)
        List.headers[index] = h
    end
    return h
end

local function rowTooltip(self)
    local t = self.thread
    if not t or not compactList() then return end
    local e = env(t.e)
    Skin.Tooltip:SetOwner(self, "ANCHOR_RIGHT")
    Skin.Tooltip:AddLine(t.t ~= "" and t.t or "Untitled")
    Skin.Tooltip:AddLine(format("%s%s", e and e.name or "?", t.p ~= "" and (" · " .. t.p) or ""))
    Skin.Tooltip:AddLine(statusText(t.s))
    Skin.Tooltip:Show()
end

local function row(index, parent)
    local r = rows[index]
    if r and r:GetParent() ~= parent then r:SetParent(parent) end
    if not r then
        r = CreateFrame("Button", nil, parent)
        r:SetSize(SIZE.row, 44)
        r.box = Skin.Box(r, { shape = "control", fill = false, ring = false })
        r.project = r:CreateFontString(nil, "OVERLAY")
        r.project:SetFontObject(Skin.fonts.rowMeta)
        r.project:SetJustifyH("LEFT")
        r.project:SetWordWrap(false)
        r.title = r:CreateFontString(nil, "OVERLAY")
        r.title:SetFontObject(Skin.fonts.strong)
        r.title:SetJustifyH("LEFT")
        r.title:SetWordWrap(false)
        r.sub = r:CreateFontString(nil, "OVERLAY")
        r.sub:SetFontObject(Skin.fonts.rowMeta)
        r.sub:SetJustifyH("LEFT")
        r.sub:SetWordWrap(false)
        r.status = Skin.Pill(r)
        r.dot = Skin.Box(r, { shape = "pill", fill = "dim", height = 6, free = true, layer = "ARTWORK" })
        r.dot.proxy:SetSize(6, 6)
        r.age = r:CreateFontString(nil, "OVERLAY")
        r.age:SetFontObject(Skin.fonts.small)
        r.prLink = PullRequests.NewLink(r, function() return envIdOf(r.thread.e), r.thread.id end)
        r.prMore = PullRequests.TextButton(r)
        r.prMore:SetScript("OnEnter", function(self) PullRequests.ShowAll(self, r.thread) end)
        r:SetScript("OnClick", function(self) UI.Select(self.thread) end)
        r:SetScript("OnEnter", function(self)
            rowTooltip(self)
            UI.RowActions.Hover(self)
        end)
        r:SetScript("OnLeave", function(self)
            Skin.Tooltip:Hide()
            UI.RowActions.Hover(self)
        end)
        r.hoverBox = Skin.Hover(r)
        UI.RowActions.Build(r)
        rows[index] = r
    end
    return r
end

-- Comfortable, as in T3: the project, the title, then the machine with the linked pull
-- request across from it. A thread with no project has no first line. Top right is one
-- place: the status while there's something to say (T3 shows no status for a thread at
-- rest), else the age, and Settle and Snooze while the pointer is on the row.
-- Compact: a status dot, the title, the pull request and the age on one line.
local function layoutRow(r, compact, hasPr, hasMore)
    r.isCompact, r.hasPr, r.hasMore = compact, hasPr, hasMore
    local hasProject = not compact and (r.thread.p or "") ~= ""
    local rowStatus = UI.RowStatus(r.thread)
    local showStatus = not compact and (not UI.AtRest[r.thread.s] or rowStatus == UI.DONE or rowStatus == UI.WOKE)
    r.project:ClearAllPoints()
    r.title:ClearAllPoints()
    r.sub:ClearAllPoints()
    r.age:ClearAllPoints()
    r.actions:ClearAllPoints()
    r.status:ClearAllPoints()
    r.dot:ClearAllPoints()
    r.prLink:ClearAllPoints()
    r.prMore:ClearAllPoints()
    local slot = (r.hovering and r.actions) or (showStatus and r.status.box.proxy) or r.age
    r.prLink:SetShown(hasPr)
    r.prMore:SetShown(hasMore)
    local leftmostPr = (hasPr and r.prLink) or (hasMore and r.prMore) or nil
    if compact then
        r.prMore:SetPoint("RIGHT", slot, "LEFT", -6, 0)
        r.prLink:SetPoint("RIGHT", hasMore and r.prMore or slot, "LEFT", hasMore and -2 or -6, 0)
        r.dot:SetPoint("LEFT", r, "LEFT", 9, 0)
        r.title:SetPoint("LEFT", 22, 0)
        r.title:SetPoint("RIGHT", leftmostPr or slot, "LEFT", -6, 0)
        r.age:SetPoint("RIGHT", -8, 0)
        r.actions:SetPoint("RIGHT", -5, 0)
    else
        r.status:SetPoint("TOPRIGHT", r, "TOPRIGHT", -8, -5)
        r.age:SetPoint("TOPRIGHT", -8, -7)
        r.actions:SetPoint("TOPRIGHT", -5, -4)
        local firstLine = hasProject and r.project or r.title
        firstLine:SetPoint("TOPLEFT", 10, -7)
        firstLine:SetPoint("RIGHT", slot, "LEFT", -6, 0)
        if hasProject then
            r.title:SetPoint("TOPLEFT", 10, -23)
            r.title:SetPoint("TOPRIGHT", -8, -23)
        end
        r.prMore:SetPoint("BOTTOMRIGHT", -8, 4)
        if hasMore then r.prLink:SetPoint("RIGHT", r.prMore, "LEFT", -2, 0) else r.prLink:SetPoint("BOTTOMRIGHT", -8, 4) end
        r.sub:SetPoint("BOTTOMLEFT", 10, 7)
        if leftmostPr then r.sub:SetPoint("RIGHT", leftmostPr, "LEFT", -6, 0) else r.sub:SetPoint("BOTTOMRIGHT", -8, 7) end
    end
    r.project:SetShown(hasProject)
    r.sub:SetShown(not compact)
    r.status:SetShown(showStatus and not r.hovering)
    r.dot:SetShown(compact)
    r.age:SetShown(not showStatus and not r.hovering)
    r.actions:SetShown(r.hovering)
end

-- Settle and Snooze on a row, as in T3's sidebar. Snooze asks for how long; T3 takes a time,
-- which the companion works out, since the game's clock may not be the PC's. On a snoozed
-- thread the clock wakes it. A settled thread has only Unsettle. As in T3 0.18.0, a thread
-- waiting on a question can be snoozed (the question stays open; a newer one wakes it), but
-- one waiting on an approval, or on a turn still starting, has only Settle.
do
    local Actions = {}
    UI.RowActions = Actions
    local SNOOZE_FOR = { { "1 hour", 60 }, { "4 hours", 240 }, { "1 day", 1440 }, { "1 week", 10080 } }
    Actions.UNSNOOZABLE = { approval = true, preparing = true, queued = true, starting = true }

    local function command(kind, t, minutes)
        local blocked = blockedReason(kind)
        if blocked then return UI.Notice(blocked) end
        ns.Link.Send({ t = kind, e = envIdOf(t.e), id = t.id, mn = minutes })
    end

    local snoozeMenu = { isSelected = function() return false end }
    function snoozeMenu.entries()
        local list = {}
        for _, choice in ipairs(SNOOZE_FOR) do list[#list + 1] = { text = choice[1], value = choice[2] } end
        return list
    end
    -- The thread is the one clicked: its row may list another by the time one is picked.
    function snoozeMenu.pick(minutes) command("snooze", snoozeMenu.thread, minutes) end

    local function action(r, glyph, onClick)
        local b = CreateFrame("Button", nil, r.actions)
        b:SetSize(16, 16)
        b.glyph = Skin.Glyph(b, glyph, 12, "muted", "OVERLAY")
        b.glyph:SetPoint("CENTER")
        b:SetScript("OnClick", onClick)
        b:SetScript("OnEnter", function(self)
            r.hoverBox:Show() -- the row was just left for this button, but the pointer is still on it
            Skin.Tooltip:SetOwner(self, "ANCHOR_TOP")
            Skin.Tooltip:AddLine(self.tip)
            Skin.Tooltip:Show()
        end)
        b:SetScript("OnLeave", function() Skin.Tooltip:Hide() end)
        Skin.Hover(b)
        return b
    end

    function Actions.Build(r)
        r.hovering = false
        r.actions = CreateFrame("Frame", nil, r)
        r.actions:SetHeight(16)
        r.actions:Hide()
        -- However the pointer left (a menu's click-away layer swallows the row's OnLeave).
        r.actions:SetScript("OnUpdate", function()
            if not r:IsMouseOver() then Actions.Hover(r) end
        end)
        r.settleAction = action(r, "check", function()
            local t = r.thread
            if t then command(t.set and "unsettle" or "settle", t) end
        end)
        r.settleAction:SetPoint("RIGHT", r.actions, "RIGHT", 0, 0)
        -- The check and the word, as in T3.
        r.settleAction.glyph:ClearAllPoints()
        r.settleAction.glyph:SetPoint("LEFT", 2, 0)
        r.settleAction.label = r.settleAction:CreateFontString(nil, "OVERLAY")
        r.settleAction.label:SetFontObject(Skin.fonts.muted)
        r.settleAction.label:SetPoint("LEFT", r.settleAction.glyph, "RIGHT", 2, 0)
        r.snoozeAction = action(r, "clock", function(self)
            local t = r.thread
            if not t then return end
            if t.sn then return command("unsnooze", t) end
            if blockedReason("snooze") then return UI.Notice(blockedReason("snooze")) end
            snoozeMenu.thread = t
            UI.OpenMenu(self, snoozeMenu)
        end)
        r.snoozeAction:SetPoint("RIGHT", r.settleAction, "LEFT", -2, 0)
        -- What it says when the game has no icon file yet.
        r.snoozeAction.short = r.snoozeAction:CreateFontString(nil, "OVERLAY")
        r.snoozeAction.short:SetFontObject(Skin.fonts.small)
        r.snoozeAction.short:SetPoint("CENTER")
        r.snoozeAction.short:SetText("Zz")
    end

    local function paint(b, kind, name)
        local blocked = blockedReason(kind)
        b.tip = blocked or name
        b:SetEnabled(not blocked)
        Skin.SetGlyphColor(b.glyph, blocked and "dim" or "muted")
    end

    -- Whether the pointer is on the row, and what its two buttons do for this thread.
    function Actions.Refresh(r)
        local t = r.thread
        r.hovering = (t ~= nil and r:IsMouseOver()) and true or false
        if not r.hovering then return end
        paint(r.settleAction, t.set and "unsettle" or "settle", t.set and "Unsettle thread" or "Settle thread")
        paint(r.snoozeAction, t.sn and "unsnooze" or "snooze", t.sn and "Wake thread" or "Snooze thread")
        local drawn = UI.GlyphKnown(r.snoozeAction.glyph)
        r.snoozeAction.glyph:SetShown(drawn)
        r.snoozeAction.short:SetShown(not drawn)
        local snoozable = t.sn or not (t.set or Actions.UNSNOOZABLE[t.s])
        r.snoozeAction:SetShown(snoozable and true or false)
        r.settleAction.label:SetText(t.set and "Unsettle" or "Settle")
        local width = math.ceil(r.settleAction.label:GetStringWidth() or 0) + 19
        r.settleAction:SetWidth(width)
        r.actions:SetWidth(width + (snoozable and 18 or 0))
    end

    function Actions.Hover(r)
        Actions.Refresh(r)
        layoutRow(r, r.isCompact, r.hasPr, r.hasMore)
    end
end

-- A row's pull requests: one shows its number, several just +total, listed on hover. An
-- older companion lists none, so its number and +n only show. Without a page to open, a
-- click on them goes through to the row. Returns whether each is there, for layoutRow.
function PullRequests.FillRow(r, t)
    local number, more = (t.pr or ""):match("^(%S+)%s*(.*)$")
    local prs = t.prs or {}
    if prs[2] then number, more = nil, "+" .. #prs end
    local only = not prs[2] and prs[1] or { n = number or "", s = number and t.ps }
    PullRequests.SetLink(r.prLink, only)
    r.prLink:EnableMouse(only.u ~= nil)
    r.prMore.label:SetText(more or "")
    r.prMore:EnableMouse(prs[2] ~= nil)
    PullRequests.Fit(r.prMore)
    return number ~= nil, more ~= nil and more ~= ""
end

-- What a row's +total stands for, listed on hovering it: the thread's pull requests, each
-- opening when clicked. A tooltip can't be clicked, so this is a frame of its own, open
-- while the pointer is on the +total or on it, with a moment to cross from one to the other.
do
    local LINGER, ENTRY_HEIGHT, PAD = 0.2, 18, 4
    local popup = CreateFrame("Frame", "T3WoWPullRequests", frame)
    popup:SetFrameStrata("FULLSCREEN_DIALOG")
    popup:SetFrameLevel(20)
    popup:SetClampedToScreen(true)
    popup:EnableMouse(true)
    Skin.Box(popup, { shape = "control", fill = "card", ring = "border", ringLayer = "BORDER" })
    popup.entries = {}
    popup:Hide()
    -- Closed with the window, it stays closed when the window comes back.
    popup:SetScript("OnHide", function(self) self:Hide() end)
    popup:SetScript("OnUpdate", function(self, elapsed)
        if self:IsMouseOver() or self.anchor:IsMouseOver() then
            self.away = 0
        else
            self.away = self.away + elapsed
            if self.away >= LINGER then self:Hide() end
        end
    end)

    local function entryAt(index)
        local entry = popup.entries[index]
        if not entry then
            entry = PullRequests.TextButton(popup)
            entry:SetHeight(ENTRY_HEIGHT)
            entry:SetPoint("TOPLEFT", PAD, -PAD - (index - 1) * ENTRY_HEIGHT)
            entry.label:ClearAllPoints()
            entry.label:SetPoint("LEFT", 6, 0)
            entry:SetScript("OnClick", function(self)
                PullRequests.Open(popup.env, popup.threadId, self.pr)
                popup:Hide()
            end)
            Skin.Hover(entry)
            popup.entries[index] = entry
        end
        return entry
    end

    function PullRequests.ShowAll(anchor, thread)
        local prs = thread.prs or {}
        popup.anchor, popup.away = anchor, 0
        popup.env, popup.threadId = envIdOf(thread.e), thread.id
        local count, width = #prs, 0
        for index = 1, count do
            local pr, entry = prs[index], entryAt(index)
            local state = PullRequests.STATE[pr.s]
            entry.pr = pr
            entry.label:SetText(PullRequests.Colored(pr.n, pr.s) .. (state and format("  |cff%s%s|r", Skin.Hex("dim"), state[1]) or ""))
            entry:SetEnabled(pr.u ~= nil)
            entry:Show()
            width = math.max(width, entry.label:GetStringWidth() or 0)
        end
        for index = count + 1, #popup.entries do
            popup.entries[index]:Hide()
            popup.entries[index].label:SetText("")
        end
        for index = 1, count do popup.entries[index]:SetWidth(width + 12) end
        popup:SetSize(width + 12 + 2 * PAD, count * ENTRY_HEIGHT + 2 * PAD)
        popup:ClearAllPoints()
        popup:SetPoint("TOPRIGHT", anchor, "BOTTOMRIGHT", 0, -2)
        popup:Show()
    end

    -- The list is drawn again on every update: what's open follows its thread's row, and
    -- closes with it.
    function PullRequests.FollowList()
        if not popup:IsShown() then return end
        for _, r in ipairs(rows) do
            local t = r.thread
            if r:IsShown() and t and t.id == popup.threadId and envIdOf(t.e) == popup.env and t.prs and t.prs[2] then
                return PullRequests.ShowAll(r.prMore, t)
            end
        end
        popup:Hide()
    end
end

-- A finished thread you haven't opened: T3's Done. T3's tools don't say what you've read,
-- so this is what the game saw finish and hasn't opened since; reading it in T3 doesn't clear it.
UI.DONE = { "Done", "34d399", "done" }
function UI.Unread()
    T3WoWDB = T3WoWDB or {}
    T3WoWDB.unread = T3WoWDB.unread or {}
    return T3WoWDB.unread
end

-- A snoozed thread that came back and you haven't opened since: T3's Woke. T3's tools don't
-- say when a thread woke or what you've read, so this is what the game saw snoozed and then
-- awake: a thread it never saw snoozed can't say Woke, and opening one in T3 doesn't clear it.
UI.WOKE = { "Woke", "fbbf24", "alarm" }
function UI.Woke()
    T3WoWDB = T3WoWDB or {}
    T3WoWDB.woke = T3WoWDB.woke or {}
    return T3WoWDB.woke
end

function UI.RowStatus(t)
    local key = UI.Waits.Key(envIdOf(t.e), t.id)
    if UI.AtRest[t.s] and UI.Woke()[key] then return UI.WOKE end
    if t.s == "completed" and not t.set and UI.Unread()[key] then return UI.DONE end
    return STATUS[t.s] or { t.s or "?", "a0a0a0" }
end

-- A run's status says how long it has been going, once that's a minute or more.
function UI.PaintRowStatus(r, t)
    local s = UI.RowStatus(t)
    local elapsed = WORKING[t.s] and t.st and age(t.st) or ""
    r.status:Set((elapsed ~= "" and elapsed ~= "now") and (s[1] .. " " .. elapsed) or s[1], s[2], s[3])
    r.dot:SetColors({ hexColor(s[2]) })
end

-- Which threads just finished, from what each update says against the one before. An
-- update with no threads says nothing: the lists haven't arrived.
function UI.TrackUnread(data)
    local threads = data.threads or {}
    if #threads == 0 then return end
    T3WoWDB = T3WoWDB or {}
    local before, unread, now = T3WoWDB.lastStatus or {}, UI.Unread(), {}
    for _, t in ipairs(threads) do
        local id = data.envs and data.envs[t.e] and data.envs[t.e].id
        if id then
            local key = UI.Waits.Key(id, t.id)
            now[key] = t.s
            if t.s ~= "completed" then
                unread[key] = nil
            elseif before[key] and before[key] ~= "completed" and not UI.Waits.Reading(id, t.id) then
                unread[key] = true
            end
        end
    end
    for key in pairs(unread) do
        if not now[key] then unread[key] = nil end
    end
    T3WoWDB.lastStatus = now
end

-- Which threads just woke. The companion reads each machine's lists in turn, so for a moment
-- a thread can be in two of them (still snoozed wins) or in none: T3WoWDB.snoozed holds it
-- until a list says it's awake, and across a logout, as it may wake meanwhile.
function UI.TrackWoke(data)
    local threads = data.threads or {}
    if #threads == 0 then return end
    T3WoWDB = T3WoWDB or {}
    T3WoWDB.snoozed = T3WoWDB.snoozed or {}
    local snoozed, woke, listed, asleep = T3WoWDB.snoozed, UI.Woke(), {}, {}
    local function envIdFor(t) return data.envs and data.envs[t.e] and data.envs[t.e].id end
    for _, t in ipairs(threads) do
        local id = envIdFor(t)
        if id then
            local key = UI.Waits.Key(id, t.id)
            listed[key] = true
            asleep[key] = asleep[key] or t.sn
        end
    end
    for _, t in ipairs(threads) do
        local id = envIdFor(t)
        local key = id and UI.Waits.Key(id, t.id)
        if key and asleep[key] then
            snoozed[key], woke[key] = true, nil
        elseif key then
            if snoozed[key] and not UI.Waits.Reading(id, t.id) then woke[key] = true end
            snoozed[key] = nil
            if t.set then woke[key] = nil end
        end
    end
    for key in pairs(woke) do
        if not listed[key] then woke[key] = nil end
    end
end

-- Show more, at the end of a group with more threads than the companion listed: the next
-- updates list more of it, for this UI session. Working and Needs you come from the open
-- list, so Open's covers them.
local moreButtons = {}
local function moreButton(key)
    local b = moreButtons[key]
    if not b then
        b = button(key == "open" and listChild or List.dockChild, "Show more", SIZE.row - 8, function(self) ns.Link.Send({ t = "list", g = self.group }) end, "ghost")
        b.group = key
        moreButtons[key] = b
    end
    return b
end

local function renderList()
    for _, r in ipairs(rows) do r:Hide() end
    for _, h in ipairs(List.headers) do h:Hide() end
    for _, b in pairs(moreButtons) do b:Hide() end
    local groups = { working = {}, open = {}, snoozed = {}, settled = {} }
    -- Threads an agent launched sit under the thread that launched them when both are in
    -- the same group, as in T3's sidebar, and don't count in it. In another group than
    -- their launcher (settled while it's open, say) they stand on their own in theirs.
    -- With their launcher not listed at all, settled ones still show under Settled, and
    -- others only while they're working or need you; T3's sidebar has no row for them.
    -- The same goes for a thread the companion marks "ag": delegated from another machine,
    -- which T3's sidebar leaves out too. Other agent-made threads are listed like any.
    local function groupOf(t) return (t.set and "settled") or (t.sn and "snoozed") or (inWorking(t.s) and "working") or "open" end
    -- A search keeps the threads whose title, project or machine has every word typed,
    -- case aside. A launched thread whose launcher doesn't match, or isn't listed, stands on its own.
    local words = {}
    for word in (search and search:GetText() or ""):lower():gmatch("%S+") do words[#words + 1] = word end
    local searching = #words > 0
    local function matches(t)
        local e = env(t.e)
        local text = format("%s\n%s\n%s", t.t or "", t.p or "", e and e.name or ""):lower()
        for _, word in ipairs(words) do
            if not text:find(word, 1, true) then return false end
        end
        return true
    end
    local threads = inbox and inbox.threads or {}
    local byId, launched = {}, {}
    for _, t in ipairs(threads) do byId[t.e .. ":" .. t.id] = t end
    for _, t in ipairs(threads) do
        local parent = t.par and byId[t.e .. ":" .. t.par]
        if not matches(t) then
            -- left out by the search
        elseif parent and matches(parent) and groupOf(parent) == groupOf(t) then
            local key = t.e .. ":" .. t.par
            launched[key] = launched[key] or {}
            table.insert(launched[key], t)
        elseif searching or not (t.par or t.ag) or parent or t.set or inWorking(t.s) or t.s == "waiting" or t.s == "approval" or t.s == "card" then
            table.insert(groups[groupOf(t)], t)
        end
    end
    local compact = compactList()
    local rowHeight = compact and Skin.Theme().compactHeight or Skin.Theme().rowHeight
    local y, rowIndex, headerIndex = 0, 0, 0
    List.openHeight = 0 -- a search can leave Open out
    -- While searching, every group opens on its matches (folding one lasts for this search),
    -- and a group with none and nothing more to list hides.
    local folded = searching and search.folded or collapsed
    local more = inbox and type(inbox.more) == "table" and inbox.more or {} -- an older companion sends none
    local shownGroups = {}
    for _, group in ipairs({ { "open", "Open" }, { "working", "Working" }, { "snoozed", "Snoozed" }, { "settled", "Settled" } }) do
        if not searching or #groups[group[1]] > 0 or more[group[1]] then shownGroups[#shownGroups + 1] = group end
    end
    search.none:SetShown(#shownGroups == 0)
    for _, group in ipairs(shownGroups) do
        local key, label = group[1], group[2]
        local parent = key == "open" and listChild or List.dockChild
        headerIndex = headerIndex + 1
        local h = header(headerIndex, parent)
        -- Only recent settled threads are listed; the count is all of them, as in T3.
        local count = #groups[key]
        if key == "settled" and not searching and inbox and inbox.settledTotal then count = math.max(count, inbox.settledTotal) end
        h.text:SetText(label)
        h.count:SetText(count)
        Skin.PointGlyph(h.chevron, folded[key])
        h:SetPoint("TOPLEFT", 0, -y)
        h:SetScript("OnClick", function()
            folded[key] = not folded[key]
            renderList()
        end)
        h:Show()
        y = y + 28
        if not folded[key] then
            local placed = {}
            local function place(t, indent)
                local id = t.e .. ":" .. t.id
                if placed[id] then return end
                placed[id] = true
                rowIndex = rowIndex + 1
                local r = row(rowIndex, parent)
                r.thread = t
                if selected and not selected.new and selected.id == t.id and selected.env == envIdOf(t.e) then
                    local key = UI.Waits.Key(selected.env, t.id)
                    UI.Unread()[key], UI.Woke()[key] = nil, nil
                end
                UI.RowActions.Refresh(r)
                layoutRow(r, compact, PullRequests.FillRow(r, t))
                r.title:SetText(t.t ~= "" and t.t or "Untitled")
                local e = env(t.e)
                r.project:SetText(t.p or "")
                r.sub:SetText(e and e.name or "?")
                UI.PaintRowStatus(r, t)
                -- `la`: when T3's sidebar ages the row from (an older companion sends only `at`).
                r.age:SetText(age(t.la or t.at))
                local isSelected = selected and not selected.new and selected.id == t.id and selected.env == envIdOf(t.e)
                r.box:SetColors(isSelected and "selected" or false, isSelected and "selectedRing" or false)
                r:SetPoint("TOPLEFT", indent, -y)
                -- A thread with no project has no line for it.
                local height = (compact or (t.p or "") ~= "") and rowHeight or rowHeight - 15
                r:SetSize(SIZE.row - indent, height)
                r:Show()
                y = y + height + (compact and 1 or 2)
                for _, child in ipairs(launched[id] or {}) do place(child, math.min(indent + 14, 42)) end
            end
            for _, t in ipairs(groups[key]) do place(t, 0) end
            if more[key] then
                local b = moreButton(key)
                b:SetPoint("TOPLEFT", 4, -y - 2)
                b:Show()
                y = y + 32
            end
        end
        -- Open ends here: the dock's groups count from the top of the dock.
        if key == "open" then List.openHeight, y = y, 0 end
    end
    List.dockHeight = y
    listChild:SetHeight(math.max(List.openHeight, 1))
    List.dockChild:SetHeight(math.max(y, 1))
    List.FitDock()
    PullRequests.FollowList()
end

-- While the window is open: running threads' elapsed time.
do
    local ticker, since = CreateFrame("Frame", nil, frame), 0
    ticker:SetScript("OnUpdate", function(_, elapsed)
        since = since + elapsed
        if since < 15 then return end
        since = 0
        for _, r in ipairs(rows) do
            if r:IsShown() and r.thread and WORKING[r.thread.s] then UI.PaintRowStatus(r, r.thread) end
        end
    end)
end

-- Search, over the list as in T3's sidebar: typing narrows the list (renderList), Enter
-- opens the first thread it shows, Escape clears it. It searches the threads the companion
-- listed; Show more lists more of a group.
search = CreateFrame("EditBox", "T3WoWSearch", frame)
search:SetPoint("TOPLEFT", 10, -SIZE.title - 8)
search:SetSize(LIST_WIDTH - 50, 26) -- New Thread is beside it
search:SetAutoFocus(false)
search:SetMaxLetters(80)
search:SetFontObject(Skin.fonts.text)
search:SetTextInsets(28, 24, 0, 0)
search.folded = {} -- groups folded during this search
search.box = Skin.Box(search, { shape = "control", fill = "input", ring = "border" })
search.glyph = Skin.Glyph(search, "search", 14, "muted", "OVERLAY")
search.glyph:SetPoint("LEFT", 9, 0)
-- A game started before this version found no search.tga: no glyph, rather than a square.
-- Asked each time the window shows, as the answer can come a while after SetTexture.
search:SetScript("OnShow", function(self)
    local known = not self.glyph.GetTextureFilePath or self.glyph:GetTextureFilePath()
    self.glyph:SetShown(known ~= nil and known ~= "")
end)
search.placeholder = search:CreateFontString(nil, "OVERLAY")
search.placeholder:SetFontObject(Skin.fonts.muted)
search.placeholder:SetPoint("LEFT", 28, 0)
search.placeholder:SetText("Search")
-- A small x to clear what is typed, there only while something is.
search.clear = Skin.CloseButton(search, function()
    search:SetText("")
    search:ClearFocus()
end)
search.clear:SetSize(18, 18)
search.clear.glyph:SetSize(12, 12)
search.clear:SetPoint("RIGHT", -4, 0)
search.clear:Hide()
search.none = listChild:CreateFontString(nil, "OVERLAY")
search.none:SetFontObject(Skin.fonts.muted)
search.none:SetPoint("TOPLEFT", 10, -8)
search.none:SetText("No threads match")
search.none:Hide()
search:SetScript("OnEditFocusGained", function(self) self.box:SetColors(nil, "focus") end)
search:SetScript("OnEditFocusLost", function(self) self.box:SetColors(nil, "border") end)
search:SetScript("OnTextChanged", function(self)
    local text = self:GetText()
    self.placeholder:SetShown(text == "")
    self.clear:SetShown(text ~= "")
    if not text:find("%S") then self.folded = {} end
    listScroll:SetVerticalScroll(0)
    List.dock:SetVerticalScroll(0)
    renderList()
end)
search:SetScript("OnEscapePressed", function(self)
    self:SetText("")
    self:ClearFocus()
end)
search:SetScript("OnEnterPressed", function(self)
    self:ClearFocus()
    local first = rows[1]
    if self:GetText():find("%S") and first and first:IsShown() then UI.Select(first.thread) end
end)

-- ---------------------------------------------------------------- conversation

local right = CreateFrame("Frame", nil, frame)
right:SetPoint("TOPLEFT", LIST_WIDTH + 1, -SIZE.title - 1)
right:SetPoint("BOTTOMRIGHT", -1, SIZE.footer + 1)

-- The thread's title, then its machine, age and branch; a line under them. Its status is in the list.
local viewTitle = right:CreateFontString(nil, "OVERLAY")
viewTitle:SetFontObject(Skin.fonts.title)
viewTitle:SetPoint("TOPLEFT", 18, -12)
viewTitle:SetPoint("RIGHT", -18, 0)
viewTitle:SetJustifyH("LEFT")
viewTitle:SetWordWrap(false)
local viewMeta = right:CreateFontString(nil, "OVERLAY")
viewMeta:SetFontObject(Skin.fonts.small)
viewMeta:SetJustifyH("LEFT")
-- One line: a long branch is cut short with "…" at the panel's edge.
viewMeta:SetWordWrap(false)
do
    local headerLine = Skin.Line(right)
    headerLine:SetPoint("TOPLEFT", 0, -SIZE.header)
    headerLine:SetPoint("TOPRIGHT", 0, -SIZE.header)
end

-- The open thread's pull requests, at the right of the line under the title.
do
    local links = {}
    local function linkAt(index)
        local link = links[index]
        if not link then
            link = PullRequests.NewLink(right, function() return selected.env, selected.id end)
            links[index] = link
        end
        return link
    end

    -- Laid out from the right edge, level with the line under the title, so what's before them is
    -- what gets cut short. Returns the leftmost, or nil when there are none. Off the title,
    -- not the text beside them: that text is anchored to these, and the game refuses a loop.
    function PullRequests.Place(prs)
        local leftmost
        for i = #prs, 1, -1 do
            local link = linkAt(i)
            PullRequests.SetLink(link, prs[i])
            link:ClearAllPoints()
            if leftmost then
                link:SetPoint("RIGHT", leftmost, "LEFT", -6, 0)
            else
                link:SetPoint("TOPRIGHT", viewTitle, "BOTTOMRIGHT", PullRequests.titleInset, -6)
            end
            link:Show()
            leftmost = link
        end
        for i = #prs + 1, #links do
            links[i]:Hide()
            links[i].label:SetText("")
        end
        return leftmost
    end
end

-- What the header says under the title; prs is the thread's pull requests, if it has any.
local function setViewMeta(text, prs)
    viewMeta:ClearAllPoints()
    viewMeta:SetPoint("TOPLEFT", viewTitle, "BOTTOMLEFT", 0, -8)
    local leftmost = PullRequests.Place(prs or {})
    if leftmost then
        viewMeta:SetPoint("RIGHT", leftmost, "LEFT", -10, 0)
    else
        viewMeta:SetPoint("RIGHT", right, "RIGHT", -18, 0)
    end
    viewMeta:SetText(text or "")
end

-- The conversation takes whatever width the list leaves; it follows the window's size.
local function messageWidth() return frame:GetWidth() - LIST_WIDTH - 46 end
local messageScroll, messageChild = makeScroll(right, messageWidth())
messageScroll:SetPoint("TOPLEFT", 18, -SIZE.header - 12)
messageScroll:SetPoint("BOTTOMRIGHT", -26, SIZE.composer)
-- By index: a message's, then (sending) at count + 1 and Working… at count + 2. That can
-- leave a gap (Working… with no echo before it), so go through them with pairs: ipairs
-- stopped at the gap and left the last Working… shown under the next reply.
local messageStrings = {}
local earlier = button(messageChild, "Show earlier messages", 180, function() ns.Link.Send({ t = "more" }) end, "ghost")
local openCopy -- the copy box, made after the composer
-- Pictures: pages an agent showed and images a message embeds, drawn by the companion
-- into image slot files (img\NNNN.tga). A message item with `im` = { f = slot, w, h }.
local Pictures = { used = 0, pool = {} }

-- Each message has a button over it: clicking a message opens it in the copy box. Your
-- messages sit in a bubble on the right; the agent's run the full width.
local function messageString(index)
    local fs = messageStrings[index]
    if not fs then
        fs = messageChild:CreateFontString(nil, "OVERLAY")
        fs:SetFontObject(Skin.fonts.text)
        fs:SetWidth(messageWidth() - 8)
        fs:SetJustifyH("LEFT")
        fs:SetWordWrap(true)
        fs:SetSpacing(2)
        -- The bubble, the button and the hover tint are placed around the message as it is
        -- drawn (anchorMessage): its text alone, or the pieces of a message with code.
        fs.bubble = Skin.Box(messageChild, { shape = "panel", fill = "raised", ring = "border", free = true })
        fs.bubble:Hide()
        local hit = CreateFrame("Button", nil, messageChild)
        -- The hover tint sits behind the text, on the conversation itself.
        fs.hover = Skin.Box(messageChild, { shape = "control", fill = "hover", free = true, sub = -7 })
        fs.hover:Hide()
        hit:SetScript("OnClick", function(self)
            if self.work then return UI.Work.Toggle(self.work) end
            if self.picture then return Pictures.Zoom(self.picture) end
            openCopy(self.copyText)
        end)
        hit:SetScript("OnEnter", function(self)
            Skin.Tooltip:SetOwner(self, "ANCHOR_RIGHT")
            if self.work then
                Skin.Tooltip:AddLine(UI.Work.open[self.work.l] and "Click to collapse" or "Click to expand")
            else
                Skin.Tooltip:AddLine(self.picture and "Click to enlarge" or "Click to copy")
            end
            Skin.Tooltip:Show()
        end)
        hit:SetScript("OnLeave", function() Skin.Tooltip:Hide() end)
        hit:HookScript("OnEnter", function() if not fs.bubble.shown then fs.hover:Show() end end)
        hit:HookScript("OnLeave", function() fs.hover:Hide() end)
        hit:HookScript("OnHide", function() fs.hover:Hide() end)
        hit:Hide()
        hit.picture = false
        hit.work = false
        fs.hit = hit
        messageStrings[index] = fs
    end
    return fs
end

-- The conversation is laid out top to bottom, each piece hung from the bottom of the one
-- before it, so the game places it by the height its text really has. A height read just
-- after SetText can still be the string's old text's (text is laid out later), which once
-- drew "Working…" over a long reply that had taken the string "Working…" used before.
local flow = {}
local function flowStart(top) flow.last, flow.x, flow.gap = nil, 0, top end
-- Places a region x from the left, below the last one; `after` is the space under it.
local function place(region, x, after)
    region:ClearAllPoints()
    region:SetPoint("TOPLEFT", flow.last or messageChild, flow.last and "BOTTOMLEFT" or "TOPLEFT", x - flow.x, -flow.gap)
    region:Show()
    flow.last, flow.x, flow.gap = region, x, after
end

-- ---------------------------------------------------------------- code and tables
-- A message with code or a table also comes cut into segments (the companion's
-- wow_segments): prose (game text, inline code between \1 and \2), code blocks ({ c }) and
-- tables ({ t = rows, the header first; a = column alignments }), code and cells as plain
-- text. Each is a piece of its own in the flow: code and tables in the monospace font on a
-- code box, so each theme styles them. A string has one font, so prose with inline code is
-- laid out run by run. Pieces come from pools and are handed out again on every render.
local Mono = { pools = {}, used = {} }
do
    local WHITE = "Interface\\Buttons\\WHITE8X8"
    local PAD, CHIP = 8, 3 -- inside a code box; around inline code

    local function take(kind, make)
        local list = Mono.pools[kind]
        if not list then
            list = {}
            Mono.pools[kind] = list
        end
        local n = (Mono.used[kind] or 0) + 1
        Mono.used[kind] = n
        local obj = list[n]
        if not obj then
            obj = make()
            list[n] = obj
        end
        obj:ClearAllPoints()
        obj:Show()
        return obj
    end

    -- Hides every piece, before a render hands them out again.
    function Mono.Release()
        for _, list in pairs(Mono.pools) do
            for _, obj in ipairs(list) do
                obj:Hide()
                if obj.box then obj.box:Hide() end
                if obj.chip then obj.chip:Hide() end
            end
        end
        Mono.used = {}
    end

    -- Widths are measured on a string that is never seen, with no width of its own. A
    -- font object brings its text color with it, alpha and all, so the color is cleared
    -- after each one: the game showed the last measured text otherwise.
    local meter = messageChild:CreateFontString(nil, "OVERLAY")
    meter:SetAlpha(0)
    meter:SetWordWrap(false)
    meter:SetPoint("TOPLEFT")
    local function measure(font, text)
        meter:SetFontObject(font)
        meter:SetTextColor(0, 0, 0, 0)
        meter:SetText(text)
        return meter:GetStringWidth() or 0
    end

    local function fontSize(font, fallback)
        local _, size = font:GetFont()
        return type(size) == "number" and size > 0 and size or fallback
    end

    -- One character of the monospace font (an estimate until the font has loaded).
    local function charWidth()
        local w = measure(Skin.fonts.mono, "0000000000") / 10
        return w > 0 and w or fontSize(Skin.fonts.mono, 12) * 0.6
    end

    local function widestLine(font, text)
        local widest = 0
        for line in (text .. "\n"):gmatch("(.-)\n") do widest = math.max(widest, measure(font, line)) end
        return widest
    end

    local function chars(s)
        local out = {}
        for ch in s:gmatch("[^\128-\191][\128-\191]*") do out[#out + 1] = ch end
        return out
    end

    -- A table as lines of plain text, its columns padded to one width, "│" between them and
    -- a rule under the header, in at most `room` characters: when it doesn't fit, the widest
    -- columns are cut down until it does (to 3 characters at least, then the last columns
    -- give way to a "…" column), and a cell cut short ends in "…".
    function UI.TableLines(rows, align, room)
        local n = 0
        for _, row in ipairs(rows) do n = math.max(n, type(row) == "table" and #row or 0) end
        if n == 0 then return {} end
        local cells, widths = {}, {}
        for j = 1, n do widths[j] = 1 end
        for i, row in ipairs(rows) do
            if type(row) ~= "table" then row = {} end
            cells[i] = {}
            for j = 1, n do
                cells[i][j] = chars(tostring(row[j] or ""))
                widths[j] = math.max(widths[j], #cells[i][j])
            end
        end
        local cap = 0
        for j = 1, n do cap = math.max(cap, widths[j]) end
        local function total()
            local sum = 3 * (n - 1)
            for j = 1, n do sum = sum + math.min(widths[j], cap) end
            return sum
        end
        while cap > 3 and total() > room do cap = cap - 1 end
        -- Still too wide: the last columns go, and a "…" column says so.
        local more = total() > room
        if more then
            while n > 1 and total() + 4 > room do n = n - 1 end
            if total() + 4 > room then
                if room >= 5 then cap = room - 4 else n = 0 end -- no room for a column: "…" alone
            end
        end
        local lines = {}
        for i, row in ipairs(cells) do
            local out = {}
            for j = 1, n do
                local c, w, a = row[j], math.min(widths[j], cap), (align or ""):sub(j, j)
                local text = #c > w and (table.concat(c, "", 1, w - 1) .. "…") or table.concat(c)
                local pad = w - math.min(#c, w)
                if a == "r" then
                    text = (" "):rep(pad) .. text
                elseif a == "c" then
                    text = (" "):rep(floor(pad / 2)) .. text .. (" "):rep(pad - floor(pad / 2))
                else
                    text = text .. (" "):rep(pad)
                end
                out[j] = text
            end
            if more then out[n + 1] = "…" end
            lines[#lines + 1] = (table.concat(out, " │ "):gsub(" +$", ""))
            if i == 1 then
                local rule = {}
                for j = 1, n do rule[j] = ("─"):rep(math.min(widths[j], cap)) end
                if more then rule[n + 1] = "─" end
                lines[#lines + 1] = table.concat(rule, "─┼─")
            end
        end
        return lines
    end

    -- The table as game text: the header in the heading color, rules and bars dimmed.
    local function tableText(seg, room)
        local lines = UI.TableLines(type(seg.t) == "table" and seg.t or {}, seg.a, room)
        local dim, head = "|cff" .. Skin.Hex("dim"), "|cff" .. Skin.Hex("heading")
        for i, line in ipairs(lines) do
            line = line:gsub("|", "||")
            if i == 1 then
                line = head .. line:gsub("│", "|r" .. dim .. "│|r" .. head) .. "|r"
            elseif i == 2 then
                line = dim .. line .. "|r"
            else
                line = line:gsub("│", dim .. "│|r")
            end
            lines[i] = line
        end
        return table.concat(lines, "\n")
    end

    local function prose()
        local fs = messageChild:CreateFontString(nil, "OVERLAY")
        fs:SetFontObject(Skin.fonts.text)
        fs:SetJustifyH("LEFT")
        fs:SetWordWrap(true)
        fs:SetSpacing(2)
        fs.box, fs.chip = false, false -- what Release hides with a piece: none here
        return fs
    end

    local function codeBlock()
        local fs = messageChild:CreateFontString(nil, "OVERLAY")
        fs:SetFontObject(Skin.fonts.mono)
        fs:SetJustifyH("LEFT")
        fs:SetWordWrap(true)
        fs:SetNonSpaceWrap(true) -- a long path or URL breaks instead of running past the box
        fs:SetSpacing(2)
        fs.chip = false
        fs.box = Skin.Box(messageChild, { shape = "control", fill = "codeBg", ring = "line", free = true, sub = -4 })
        fs.box:SetPoint("TOPLEFT", fs, "TOPLEFT", -PAD, PAD)
        fs.box:SetPoint("BOTTOMRIGHT", fs, "BOTTOMRIGHT", PAD, -PAD)
        return fs
    end

    local function run(font)
        return function()
            local fs = messageChild:CreateFontString(nil, "OVERLAY")
            fs:SetFontObject(font)
            fs:SetWordWrap(false)
            fs.box, fs.chip = false, false
            if font == Skin.fonts.mono then
                fs.chip = messageChild:CreateTexture(nil, "BACKGROUND", nil, -2)
                fs.chip:SetTexture(WHITE)
                fs.chip:SetPoint("TOPLEFT", fs, "TOPLEFT", -CHIP, 1)
                fs.chip:SetPoint("BOTTOMRIGHT", fs, "BOTTOMRIGHT", CHIP, -1)
            end
            return fs
        end
    end
    local textRun, monoRun = run(Skin.fonts.text), run(Skin.fonts.mono)

    -- Prose with inline code, `width` wide: each run of text or code is a string of its own,
    -- set after the one before and wrapped at spaces (anywhere, when a word is wider than
    -- the line), in a frame as tall as its lines.
    local function inlineBlock(text, width)
        local holder = take("holder", function()
            local f = CreateFrame("Frame", nil, messageChild)
            f.box, f.chip = false, false
            return f
        end)
        holder:SetWidth(width)
        local textSize, monoSize = fontSize(Skin.fonts.text, 13), fontSize(Skin.fonts.mono, 12)
        local lineHeight = math.max(textSize, monoSize) + 5
        local space = { [false] = math.max(1, measure(Skin.fonts.text, "a a") - measure(Skin.fonts.text, "aa")), [true] = charWidth() }
        local x, row = 0, 0
        -- How wide s is drawn (spaces at its ends count, which a string's width may not).
        local function span(s, mono)
            local lead, body, trail = s:match("^( *)(.-)( *)$")
            local w = (#lead + #trail) * space[mono] + (body ~= "" and measure(mono and Skin.fonts.mono or Skin.fonts.text, body) or 0)
            return w + (mono and 2 * CHIP or 0)
        end
        -- The end of the longest start of s that fits in room: whole words, or with `anywhere`
        -- (at a line's start) as many characters as fit, one at least. nil when none fit.
        local function cut(s, mono, room, anywhere)
            local best, pos = nil, 1
            while true do
                local _, e = s:find("^ *[^ ]+", pos)
                if not e or span(s:sub(1, e), mono) > room then break end
                best, pos = e, e + 1
            end
            if best or not anywhere then return best end
            local e = 0
            repeat
                local unit = s:sub(e + 1, e + 2) == "||" and "||" or s:match("^[^\128-\191][\128-\191]*", e + 1) or s:sub(e + 1, e + 1)
                if e > 0 and span(s:sub(1, e + #unit), mono) > room then break end
                e = e + #unit
            until e >= #s
            return e
        end
        local function put(s, mono, color)
            local lead, body = s:match("^( *)(.-) *$")
            if body ~= "" then
                local fs = take(mono and "mono" or "text", mono and monoRun or textRun)
                fs:SetText(color and (color .. body .. "|r") or body)
                local size = mono and monoSize or textSize
                fs:SetPoint("TOPLEFT", holder, "TOPLEFT", x + #lead * space[mono] + (mono and CHIP or 0),
                    -row * lineHeight - floor((lineHeight - size) / 2))
                if fs.chip then
                    fs.chip:SetVertexColor(Skin.Color("codeBg"))
                    fs.chip:Show()
                end
            end
            x = x + span(s, mono)
        end
        local function flowRun(s, mono)
            local color
            if not mono then
                local c, inner = s:match("^(|c%x%x%x%x%x%x%x%x)(.*)|r$")
                if c then color, s = c, inner end
            end
            while s ~= "" do
                local e = cut(s, mono, width - x, x == 0)
                if e then
                    put(s:sub(1, e), mono, color)
                    s = s:sub(e + 1)
                end
                -- Only spaces left: the gap before the next run, not a reason for a new line.
                if not s:find("[^ ]") then
                    x = x + #s * space[mono]
                    break
                end
                x, row = 0, row + 1
                s = s:gsub("^ +", "")
            end
        end
        local firstLine = true
        for line in (text .. "\n"):gmatch("(.-)\n") do
            if not firstLine then x, row = 0, row + 1 end
            firstLine = false
            for plain, code in (line .. "\1\2"):gmatch("(.-)\1(.-)\2") do
                flowRun(plain, false)
                flowRun((code:gsub("\1", "")), true)
            end
        end
        holder:SetHeight((row + 1) * lineHeight)
        return holder
    end

    -- Lays the segments out as one message, yours in a bubble (as wide as its widest piece
    -- needs) or the agent's at the full width. Returns where it starts and ends ({ region,
    -- inset }) and its width, or nothing when no segment could be drawn.
    function Mono.Place(segments, mine)
        local full = messageWidth() - 8
        local most = mine and floor(full * 0.75) or full
        local cw = charWidth()
        local width = full
        if mine then
            width = 40
            for _, seg in ipairs(segments) do
                local w = 0
                if type(seg) == "string" then
                    w = widestLine(Skin.fonts.text, (seg:gsub("[\1\2]", "")))
                elseif type(seg) == "table" and seg.c then
                    w = widestLine(Skin.fonts.mono, (tostring(seg.c):gsub("|", "||"))) + 2 * PAD
                elseif type(seg) == "table" and seg.t then
                    w = widestLine(Skin.fonts.mono, tableText(seg, floor((most - 2 * PAD - 2) / cw))) + 2 * PAD
                end
                width = math.max(width, math.ceil(w + 1))
            end
            width = math.min(width, most)
        end
        -- What runs and table columns are fitted to (some width, should the window claim none).
        local room = width > 20 and width or 120
        local x = mine and (full + 4 - width - SIZE.bubbleX) or 4
        local first, last
        for _, seg in ipairs(segments) do
            local region, inset
            if type(seg) == "string" and seg:find("\1", 1, true) then
                region, inset = inlineBlock(seg, room), 0
            elseif type(seg) == "string" then
                region, inset = take("prose", prose), 0
                region:SetWidth(width)
                region:SetText(seg)
            elseif type(seg) == "table" and (seg.c or seg.t) then
                region, inset = take("code", codeBlock), PAD
                region:SetWidth(width - 2 * PAD)
                region:SetText(seg.c and (tostring(seg.c):gsub("|", "||")) or tableText(seg, floor((room - 2 * PAD - 2) / cw)))
                region.box:Show()
            end
            if region then
                if not first and mine then flow.gap = flow.gap + SIZE.bubbleY end
                flow.gap = flow.gap + inset
                place(region, x + inset, inset + 10)
                first = first or { region, inset }
                last = { region, inset }
            end
        end
        if not first then return end
        flow.gap = last[2] + 18 + (mine and SIZE.bubbleY or 0)
        return first, last, width
    end
end

-- The bubble, the button and the hover tint go around a message from the top left of its
-- first piece to the bottom of its last, `width` wide (a piece inset by its padding).
local function anchorMessage(fs, first, firstInset, last, lastInset, width)
    local function around(region, left, top, right, bottom)
        region:ClearAllPoints()
        region:SetPoint("TOPLEFT", first, "TOPLEFT", -firstInset - left, firstInset + top)
        region:SetPoint("BOTTOMRIGHT", last, "BOTTOMLEFT", width - lastInset + right, -lastInset - bottom)
    end
    around(fs.bubble, SIZE.bubbleX, SIZE.bubbleY, SIZE.bubbleX, SIZE.bubbleY)
    around(fs.hit, 4, 4, 4, 4)
    around(fs.hover, 6, 5, 6, 5)
end

-- A message: yours in a bubble against the right edge, as wide as its text needs up to
-- three quarters of the conversation; the agent's the full width. With segments (a message
-- with code or a table), it is drawn from those, piece by piece.
local function placeMessage(fs, text, mine, segments)
    local full = messageWidth() - 8
    fs:SetFontObject(Skin.fonts.text)
    fs.bubble:SetShown(mine)
    if type(segments) == "table" and #segments > 0 then
        local first, last, width = Mono.Place(segments, mine)
        if first then
            fs:Hide()
            anchorMessage(fs, first[1], first[2], last[1], last[2], width)
            return
        end
    end
    local width = full
    if mine then
        local most = floor(full * 0.75)
        fs:SetWidth(most)
        fs:SetText(text)
        width = math.min(most, math.ceil((fs:GetStringWidth() or most) + 1))
        if width < 40 then width = 40 end
        fs:SetWidth(width)
        flow.gap = flow.gap + SIZE.bubbleY
        place(fs, full + 4 - width - SIZE.bubbleX, 18 + SIZE.bubbleY)
    else
        fs:SetWidth(full)
        fs:SetText(text)
        place(fs, 4, 18)
    end
    anchorMessage(fs, fs, 0, fs, 0, width)
end

-- A task the agent delegated, on a card as T3 draws it: its title, with its status (and
-- the time it ended) on the right, and the start of its result below, in the muted font.
-- d = { t = title, s = status, at = when it ended, b = result }, from the companion.
local function placeCard(fs, d)
    local pad = SIZE.card
    local inner = messageWidth() - 8 - 2 * pad
    if not rawget(fs, "card") then
        fs.card = Skin.Box(messageChild, { shape = "panel", fill = "card", ring = "border", free = true, sub = -8 })
        fs.head = messageChild:CreateFontString(nil, "OVERLAY")
        fs.head:SetFontObject(Skin.fonts.strong)
        fs.head:SetJustifyH("LEFT")
        fs.head:SetWordWrap(true)
        fs.state = messageChild:CreateFontString(nil, "OVERLAY")
        fs.state:SetFontObject(Skin.fonts.small)
        fs.state:SetJustifyH("RIGHT")
        fs.state:SetWordWrap(false)
    end
    fs.bubble:Hide()
    local s = STATUS[d.s] or { tostring(d.s or "?"), "a0a0a0" }
    local ended = d.s ~= "running" and type(d.at) == "number" and d.at > 0 and type(date) == "function"
    fs.state:SetText(format("|cff%s%s|r%s", WORKING[d.s] and Skin.Hex("muted") or s[2], s[1], ended and ("  " .. date("%H:%M", d.at)) or ""))
    fs.head:SetWidth(math.max(40, inner - math.ceil(fs.state:GetStringWidth() or 0) - 12))
    fs.head:SetText(type(d.t) == "string" and d.t or "Delegated task")
    local body = type(d.b) == "string" and d.b ~= "" and d.b or nil
    flow.gap = flow.gap + pad
    place(fs.head, 4 + pad, body and 6 or (pad + 18))
    fs.state:ClearAllPoints()
    fs.state:SetPoint("TOPRIGHT", fs.head, "TOPLEFT", inner, 0)
    fs.state:Show()
    local last = fs.head
    if body then
        fs:SetFontObject(Skin.fonts.muted)
        fs:SetWidth(inner)
        fs:SetText(body)
        place(fs, 4 + pad, pad + 18)
        last = fs
    else
        fs:Hide()
    end
    fs.card:ClearAllPoints()
    fs.card:SetPoint("TOPLEFT", fs.head, "TOPLEFT", -pad, pad)
    fs.card:SetPoint("BOTTOMRIGHT", last, "BOTTOMLEFT", inner + pad, -pad)
    fs.card:Show()
    anchorMessage(fs, fs.head, 0, last, 0, inner)
end

-- ---------------------------------------------------------------- pictures
do
    local IMG = "Interface\\AddOns\\T3WoW\\img\\"
    local PASTED_MAX_WIDTH, PASTED_MAX_HEIGHT = 360, 200
    local function path(im) return format("%s%04d.tga", IMG, im.f) end

    -- Screen pixels per UI unit of `region`: a picture fits the conversation pixel for pixel
    -- when it can, and is scaled down to its width when it can't.
    local function pixelsPerUnit(region)
        local _, height = GetPhysicalScreenSize()
        local scale = region.GetEffectiveScale and region:GetEffectiveScale() or 1
        if type(height) ~= "number" or height <= 0 or type(scale) ~= "number" or scale <= 0 then return 1 end
        return height / 768 * scale
    end

    -- The size a picture `w` x `h` pixels is drawn at: `scale` times its own size (1 when
    -- left out), at most `room` wide.
    local function fit(im, room, region, scale)
        local w, h = tonumber(im.w) or 0, tonumber(im.h) or 0
        if w <= 0 or h <= 0 then return end
        local width = math.min(room, (scale or 1) * w / pixelsPerUnit(region))
        return width, width * h / w
    end

    function Pictures.Release()
        for _, texture in ipairs(Pictures.pool) do texture:Hide() end
        Pictures.used = 0
    end

    -- Draws a message that is a picture in the flow, yours against the right edge;
    -- false when it can't be one (then the message's text is drawn instead).
    function Pictures.Place(fs, im, mine)
        if type(im) ~= "table" or type(im.f) ~= "number" then return false end
        local full = messageWidth() - 8
        local width, height = fit(im, full, messageChild)
        if not width then return false end
        -- An image you pasted stays small in the conversation; a click enlarges it.
        if im.sm then
            local shrink = math.min(1, PASTED_MAX_WIDTH / width, PASTED_MAX_HEIGHT / height)
            width, height = width * shrink, height * shrink
        end
        local n = (Pictures.used or 0) + 1
        Pictures.used = n
        local texture = Pictures.pool[n]
        if not texture then
            texture = messageChild:CreateTexture(nil, "ARTWORK")
            Pictures.pool[n] = texture
        end
        texture:SetTexture(path(im))
        texture:SetSize(width, height)
        fs:Hide()
        fs.bubble:Hide()
        place(texture, mine and (full + 4 - width) or 4, 18)
        anchorMessage(fs, texture, 0, texture, 0, width)
        return true
    end

    -- Where a picture couldn't be loaded (its file was emptied, by a reinstall say): a small
    -- card saying so, yours against the right edge.
    function Pictures.Lost(fs, mine)
        local PAD, ICON, GAP = 8, 16, 6
        if not rawget(fs, "lostBox") then
            fs.lostBox = Skin.Box(messageChild, { shape = "panel", fill = "card", ring = "border", free = true, sub = -8 })
            fs.lostIcon = Skin.Glyph(messageChild, "image-off", ICON, "muted", "OVERLAY")
        end
        local icon = fs.lostIcon
        local full = messageWidth() - 8
        fs.bubble:Hide()
        fs:SetFontObject(Skin.fonts.muted)
        fs:SetWidth(full)
        fs:SetText("Image couldn't be loaded")
        local textWidth = math.ceil(fs:GetStringWidth() or 0) + 1
        fs:SetWidth(textWidth)
        local width = PAD + ICON + GAP + textWidth + PAD
        flow.gap = flow.gap + PAD
        place(icon, (mine and (full + 4 - width) or 4) + PAD, PAD + 18)
        -- The game finds a new file only when it starts: one started before this version
        -- draws no icon rather than a square. Faded, not hidden: the text hangs from it.
        local file = not icon.GetTextureFilePath or icon:GetTextureFilePath()
        icon:SetAlpha((file ~= nil and file ~= "") and 1 or 0)
        fs:ClearAllPoints()
        fs:SetPoint("LEFT", icon, "RIGHT", GAP, 0)
        fs:Show()
        fs.lostBox:ClearAllPoints()
        fs.lostBox:SetPoint("TOPLEFT", icon, "TOPLEFT", -PAD, PAD)
        fs.lostBox:SetPoint("BOTTOMRIGHT", icon, "BOTTOMLEFT", width - PAD, -PAD)
        fs.lostBox:Show()
    end

    -- A picture enlarged over the screen, up to twice its own size; a click or Escape
    -- closes it.
    local zoom, typingIn
    function Pictures.Zoom(im)
        if not zoom then
            zoom = CreateFrame("Button", "T3WoWPictureZoom", UIParent)
            zoom:SetAllPoints(UIParent)
            zoom:SetFrameStrata("FULLSCREEN_DIALOG")
            zoom:EnableMouse(true)
            local shade = zoom:CreateTexture(nil, "BACKGROUND")
            shade:SetAllPoints()
            shade:SetColorTexture(0, 0, 0, 0.85)
            zoom.picture = zoom:CreateTexture(nil, "ARTWORK")
            zoom.picture:SetPoint("CENTER")
            local help = zoom:CreateFontString(nil, "OVERLAY")
            help:SetFontObject(Skin.fonts.small)
            help:SetPoint("BOTTOM", 0, 24)
            help:SetText("Click or press Escape to close.")
            zoom:SetScript("OnClick", function(self) self:Hide() end)
            -- Back to the box you were typing in, once it closes.
            zoom:SetScript("OnHide", function()
                local box = typingIn
                typingIn = nil
                if box and frame:IsShown() then box:SetFocus() end
            end)
            popupTakesEscape(zoom)
            zoom:Hide()
        end
        local room = math.min(UIParent:GetWidth() * 0.92, (UIParent:GetHeight() * 0.88) * (tonumber(im.w) or 1) / math.max(1, tonumber(im.h) or 1))
        local width, height = fit(im, room, zoom, 2)
        if not width then return end
        zoom.picture:SetTexture(path(im))
        zoom.picture:SetSize(width, height)
        -- A box being typed in would take the Escape meant for the picture.
        typingIn = GetCurrentKeyBoardFocus and GetCurrentKeyBoardFocus() or nil
        if typingIn then typingIn:ClearFocus() end
        zoom:Show()
    end
end

local newPanel = CreateFrame("Frame", nil, right)
newPanel:SetPoint("TOPLEFT", 18, -SIZE.header - 12)
newPanel:SetPoint("BOTTOMRIGHT", -26, SIZE.composer)
local newHelp = newPanel:CreateFontString(nil, "OVERLAY")
newHelp:SetFontObject(Skin.fonts.muted)
newHelp:SetPoint("TOPLEFT", 4, -8)
newHelp:SetPoint("RIGHT", -4, 0)
newHelp:SetJustifyH("LEFT")
-- The project dropdown; its list is the machine's picked under the message box.
local function composing() return selected ~= nil and selected.new end
-- As in T3: the question over the project, over the message box, all in the middle
-- (placed by Selectors.PlaceBox, once the box exists).
newPanel.heading = newPanel:CreateFontString(nil, "OVERLAY")
newPanel.heading:SetFontObject(Skin.fonts.hero)
newPanel.heading:SetText("What should we work on?")
local projectMenu = dropdown(newPanel, "T3WoWProjectMenu", 240, {
    entries = function()
        local list = { { text = "No project" } }
        local projects = composing() and UI.ProjectsFor(envIndexById(selected.env)) or {}
        for _, p in ipairs(projects) do list[#list + 1] = { text = p.t, value = p.id } end
        if #projects == 0 then list[2] = { text = "No projects on this machine", note = true } end
        return list
    end,
    isSelected = function(id) return composing() and selected.project == id end,
    pick = function(id)
        if not composing() then return end
        -- A project picked again keeps the workspace chosen for it; another starts at its default.
        if selected.project ~= id then
            selected.project, selected.worktree = id, false
            for _, p in ipairs(UI.ProjectsFor(envIndexById(selected.env))) do
                if p.id == id then selected.worktree = p.w == true end
            end
        end
        UI.Refresh()
    end,
})
-- Where a project's thread runs, as T3 asks: a new worktree (from the checkout's current
-- branch) or the project's checkout itself. A scratch thread has neither, so it's hidden.
local WORKSPACE_NAME = { worktree = "New worktree", checkout = "Current checkout" }
local workspaceMenu = dropdown(newPanel, "T3WoWWorkspaceMenu", 200, {
    entries = function()
        return { { text = WORKSPACE_NAME.checkout, value = "checkout" }, { text = WORKSPACE_NAME.worktree, value = "worktree" } }
    end,
    isSelected = function(mode)
        return composing() and (selected.worktree and "worktree" or "checkout") == mode
    end,
    pick = function(mode)
        if not composing() then return end
        selected.worktree = mode == "worktree"
        UI.Refresh()
    end,
})

-- The New Thread composer's project, if it is still listed.
local function selectedProject()
    for _, p in ipairs(UI.ProjectsFor(envIndexById(selected.env))) do
        if p.id == selected.project then return p end
    end
end

function UI.ProjectsFor(envIndex)
    local list = {}
    for _, p in ipairs(inbox and inbox.projects or {}) do
        if p.e == envIndex then list[#list + 1] = p end
    end
    table.sort(list, function(a, b) return a.t:lower() < b.t:lower() end)
    return list
end

-- A thread opens scrolled to its latest message. The messages arrive a moment after the
-- click, so the wish is kept until they are drawn.
local scrollToEnd = false
-- The scroll child is as tall as what is laid out in it, measured from where the game put
-- the last piece (nil while the game can't say yet).
local function contentHeight()
    local top, bottom = messageChild:GetTop(), flow.last and flow.last:GetBottom()
    -- The space under the last piece counts: a bubble or the question's card reaches into it.
    if top and bottom then return math.max(top - bottom + math.max(8, flow.gap or 0), 1) end
    if not flow.last then return math.max(flow.gap or 0, 1) end
end

-- The child's height as last set, and the offsets the conversation can scroll through.
-- Both are kept here: the game's own range can lag a frame behind a new height, and an
-- offset set from a stale range scrolled a short thread past its end, into empty space.
local childHeight = 1
local function scrollRange() return math.max(0, childHeight - (messageScroll:GetHeight() or 0)) end

-- Scrolls to the end, or keeps the offset within the conversation, with the scroll bar in step.
local function applyScroll(toEnd)
    local range = scrollRange()
    local bar = messageScroll.ScrollBar
    if type(bar) == "table" and bar.SetMinMaxValues then bar:SetMinMaxValues(0, range) end
    local offset = messageScroll:GetVerticalScroll() or 0
    if toEnd or offset > range then messageScroll:SetVerticalScroll(toEnd and range or math.min(offset, range)) end
end

local function setChildHeight(height)
    childHeight = height
    messageChild:SetHeight(height)
    messageScroll:UpdateScrollChildRect()
end

-- Sizes the conversation to what is laid out, then, if asked, scrolls to its end.
local function fit(stickToBottom)
    local height = contentHeight()
    if height then setChildHeight(height) end
    applyScroll(stickToBottom)
end

-- Text is laid out after it is set, and again once a font that was still loading arrives,
-- so the height measured while drawing can be a previous message's. Every frame the
-- conversation is shown, it is measured again: a new height resizes it, and a view that
-- was at the end stays there; one scrolled up keeps its place, within the conversation.
local settle = CreateFrame("Frame", nil, messageScroll)
settle:SetScript("OnUpdate", function()
    local height = contentHeight()
    if height and math.abs(height - childHeight) > 0.5 then
        local atEnd = (messageScroll:GetVerticalScroll() or 0) >= scrollRange() - 8
        setChildHeight(height)
        applyScroll(atEnd)
    elseif (messageScroll:GetVerticalScroll() or 0) > scrollRange() + 0.5 then
        applyScroll(false)
    end
end)

-- ---------------------------------------------------------------- questions from the agent
-- A question the agent is waiting on shows at the end of the conversation: one button per
-- option (they toggle when several may be picked) and Send answer. Text typed in the box
-- answers it too, in your own words. Approvals can't be answered from here: T3's tools
-- don't allow it, so the window says to answer them in T3.
local askStrings, askButtons = {}, {}
local askState = { k = nil, picks = {} }
local setPlaceholder -- set with the composer below
local renderView -- defined below; the option buttons redraw through it
-- While the resize grip is held, `Sizing.from` holds where the drag started (see resizing).
local Sizing = {}

local function pendingAsk()
    local view = inbox and inbox.view
    if isSelectedView(view) and view.ask then return view.ask end
end

-- Sends the picks, and typed text for the first question without a pick. The companion
-- turns the indexes back into T3's ids and labels. Returns true when it used the input.
local function sendAnswer(text)
    local ask = pendingAsk()
    if not ask then return false end
    local why = blockedReason("answer")
    if why then
        UI.Notice(why)
        return false -- what you typed stays
    end
    local answers, any, typedUsed = {}, false, false
    for qi, q in ipairs(ask.qs or {}) do
        local list = {}
        for oi = 1, #(q.o or {}) do
            if askState.picks[qi] and askState.picks[qi][oi] then list[#list + 1] = oi end
        end
        if #list > 0 then
            answers[qi] = q.m and list or list[1]
            any = true
        elseif text and text ~= "" and not typedUsed then
            answers[qi] = text
            typedUsed, any = true, true
        else
            answers[qi] = false -- keeps the list in step with the questions
        end
    end
    if not any then
        UI.Notice("Pick an answer, or type one below.")
        return true
    end
    local command = { t = "answer", e = selected.env, id = selected.id, k = ask.k, a = answers }
    if not ns.Link.Fits(command) then
        UI.Notice("That answer is too long to send from the game: shorten it.")
        return false -- what you typed stays
    end
    local s = ns.Link.Send(command)
    if s then awaiting[s] = { kind = "answer", text = typedUsed and text or nil, at = GetTime() } end
    askState.sent, askState.sentAt = ask.k, GetTime()
    UI.Notice("Sending your answer…")
    return true
end

-- Brings T3 to the front on a thread: the companion opens T3's own link for it.
local function openInT3(threadId, envId)
    ns.Link.Send({ t = "t3", e = envId or "", id = threadId or "" })
    UI.Notice("Opening T3…")
end

-- The open thread's project's saved scripts (view.scr: { id, n }), at the top right of the
-- header. T3's tools can't run one, so picking it asks the thread's agent to, which is not
-- what T3's own dropdown does: the list and the tooltip say so.
do
    local WIDTH_SCRIPTS = 92
    -- What the title gives up for the button: the pull requests under it stay at the edge.
    PullRequests.titleInset = 0
    local function openScripts()
        local view = inbox and inbox.view
        return isSelectedView(view) and view.scr or {}
    end
    -- The open thread is on a machine whose scripts T3's tools don't tell (view.sna).
    local function unavailable()
        local view = inbox and inbox.view
        return isSelectedView(view) and view.sna == true
    end
    local menu = dropdown(right, "T3WoWScriptsMenu", WIDTH_SCRIPTS, {
        entries = function()
            if unavailable() then return { { text = "Not available for threads on another machine", note = true } } end
            local list = { { text = "Asks this thread's agent, not T3's terminal", note = true } }
            for _, script in ipairs(openScripts()) do
                list[#list + 1] = { text = script.n, value = script }
            end
            return list
        end,
        isSelected = function() return false end,
        pick = function(script)
            if pendingAsk() then return UI.Notice("Answer the agent's question first.") end
            local s = ns.Link.Send({ t = "script", e = selected.env, id = selected.id, sc = script.id })
            if s then awaiting[s] = { kind = "script", at = GetTime() } end
            UI.Notice(format('Asking the agent to run "%s"…', script.n))
        end,
    })
    menu:SetHeight(22)
    menu:SetPoint("TOPRIGHT", right, "TOPRIGHT", -18, -8)
    menu.label:SetFontObject(Skin.fonts.muted)
    menu:SetLabel("Scripts")
    menu:Hide()
    -- Hooked: the dropdown's hover highlight is on these already.
    menu:HookScript("OnEnter", function(self)
        Skin.Tooltip:SetOwner(self, "ANCHOR_BOTTOM")
        Skin.Tooltip:AddLine("Saved scripts")
        Skin.Tooltip:AddLine(unavailable() and "T3 keeps a machine's scripts in that machine's settings and doesn't share them with T3 WoW. Run them from T3." or "Picking one messages this thread's agent to run it. It takes a turn, with the thread's permissions, and a busy thread steers or queues as with any message. The result shows in the conversation, not in T3's terminal.")
        local blocked = blockedReason("script")
        if blocked then Skin.Tooltip:AddLine("|cffff8000" .. blocked .. "|r") end
        Skin.Tooltip:Show()
    end)
    menu:HookScript("OnLeave", function() Skin.Tooltip:Hide() end)

    function UI.RenderScripts()
        local inset = (unavailable() or #openScripts() > 0) and WIDTH_SCRIPTS + 8 or 0
        menu:SetShown(inset > 0)
        menu:SetEnabled(not blockedReason("script"))
        viewTitle:SetPoint("RIGHT", right, "RIGHT", -18 - inset, 0)
        PullRequests.titleInset = inset
    end
end

local approvalOpen = button(messageChild, "Open in T3", 120, function()
    if selected then openInT3(selected.id, selected.env) end
end)
approvalOpen:Hide()

local askSend = button(messageChild, "Submit answer", 112, function() UI.SendCurrent() end, "primary")
askSend:Hide()

-- The question (or the approval note) sits on a card, its content inset by SIZE.card.
local askCard = Skin.Box(messageChild, { shape = "panel", fill = "card", ring = "border", free = true, sub = -8 })
askCard:Hide()

local function askString(index)
    local fs = askStrings[index]
    if not fs then
        fs = messageChild:CreateFontString(nil, "OVERLAY")
        fs:SetFontObject(Skin.fonts.text)
        fs:SetJustifyH("LEFT")
        fs:SetWordWrap(true)
        fs:SetSpacing(2)
        askStrings[index] = fs
    end
    return fs
end

-- An option is a row, as in T3: a round mark for one pick (a square one where several may
-- be picked), its label with what it means under it, and its number at the right.
local function askButton(index)
    local b = askButtons[index]
    if not b then
        b = Skin.Button(CreateFrame("Button", nil, messageChild), "ghost")
        b:GetFontString():SetJustifyH("LEFT")
        b:GetFontString():SetWordWrap(false)
        b.mark = Skin.Box(b, { shape = "pill", fill = "off", ring = "border", height = 12, free = true, layer = "ARTWORK" })
        b.mark.proxy:SetSize(12, 12)
        b.about = b:CreateFontString(nil, "OVERLAY")
        b.about:SetFontObject(Skin.fonts.muted)
        b.about:SetJustifyH("LEFT")
        b.about:SetWordWrap(true)
        b.badge = Skin.Box(b, { shape = "control", fill = "raised", height = 18, free = true, layer = "ARTWORK" })
        b.badge:SetPoint("RIGHT", b, "RIGHT", -8, 0)
        b.badge.proxy:SetSize(20, 18)
        b.number = b:CreateFontString(nil, "OVERLAY")
        b.number:SetFontObject(Skin.fonts.small)
        b.number:SetPoint("CENTER", b.badge.proxy, "CENTER", 0, 0)
        b.dot = Skin.Box(b, { shape = "pill", fill = "accent", height = 6, free = true, layer = "ARTWORK", sub = 2 })
        b.dot:SetPoint("CENTER", b.mark.proxy, "CENTER")
        b.dot.proxy:SetSize(6, 6)
        b.tick = Skin.Glyph(b, "check", 14, "accent", "OVERLAY")
        b.tick:SetPoint("CENTER", b.mark.proxy, "CENTER", 1, 1)
        b:SetScript("OnClick", function(self)
            local picks = askState.picks[self.qi] or {}
            if self.multi then
                picks[self.oi] = not picks[self.oi]
            else
                picks = { [self.oi] = true }
            end
            askState.picks[self.qi] = picks
            renderView()
        end)
        askButtons[index] = b
    end
    return b
end

local function hideAsk()
    for _, fs in ipairs(askStrings) do fs:Hide() end
    for _, b in ipairs(askButtons) do b:Hide() end
    askSend:Hide()
    approvalOpen:Hide()
    askCard:Hide()
end

-- The card spans the conversation's width, from the top of `first` to the bottom of `last`
-- (both placed SIZE.card in from its left).
local function showCard(first, last)
    local inner = messageWidth() - 8 - 2 * SIZE.card
    askCard:ClearAllPoints()
    askCard:SetPoint("TOPLEFT", first, "TOPLEFT", -SIZE.card, SIZE.card)
    askCard:SetPoint("BOTTOMRIGHT", last, "BOTTOMLEFT", inner + SIZE.card, -SIZE.card)
    askCard:Show()
end

-- Draws the question (or the approval note) below what is laid out so far.
local function renderAsk(view)
    hideAsk()
    local ask = view.ask
    if setPlaceholder then setPlaceholder() end
    local x, width = 4 + SIZE.card, messageWidth() - 8 - 2 * SIZE.card
    local warn = Skin.Hex("warn")
    if not ask then
        if view.approval then
            local fs = askString(1)
            fs:SetFontObject(Skin.fonts.text)
            fs:SetWidth(width)
            fs:SetText("|cff" .. warn .. "Waiting for your approval.|r Approve or deny it in T3: by design, T3 doesn't take approvals from anywhere else. Open in T3 brings it up on this thread; Stop cancels the run from here.")
            flow.gap = flow.gap + SIZE.card
            place(fs, x, 10)
            place(approvalOpen, x, 18 + SIZE.card)
            showCard(fs, approvalOpen)
        elseif view.card then
            -- The game never takes the answer: what goes between it and the companion sits
            -- in files on disk, so a secret typed here would land there in plain text.
            local fs = askString(1)
            fs:SetFontObject(Skin.fonts.text)
            fs:SetWidth(width)
            local label = (view.card.l and view.card.l ~= "") and view.card.l or nil
            local asks = view.card.k == "link" and ("T3 asks you to link " .. (label or "a machine") .. ".")
                or ("T3 asks you for a secret" .. (label and (": " .. label) or "") .. ".")
            fs:SetText("|cff" .. warn .. asks .. "|r Answer it in T3: the game never takes it, so don't type it in the message box. Open in T3 brings it up on this thread; Stop cancels the run from here.")
            flow.gap = flow.gap + SIZE.card
            place(fs, x, 10)
            place(approvalOpen, x, 18 + SIZE.card)
            showCard(fs, approvalOpen)
        end
        return
    end
    if askState.k ~= ask.k then askState = { k = ask.k, picks = {} } end
    local si, bi = 0, 0
    local first, last
    flow.gap = flow.gap + SIZE.card
    local blocked = blockedReason("answer")
    for qi, q in ipairs(ask.qs or {}) do
        si = si + 1
        local head = askString(si)
        head:SetFontObject(Skin.fonts.muted)
        head:SetWidth(width)
        head:SetText(((q.h and q.h ~= "") and q.h or "Question") .. (q.m and " (pick any)" or ""))
        place(head, x, 8)
        first = first or head
        si = si + 1
        local question = askString(si)
        question:SetFontObject(Skin.fonts.strong)
        question:SetWidth(width)
        question:SetText(q.q or "")
        place(question, x, 8)
        last = question
        for oi, o in ipairs(q.o or {}) do
            bi = bi + 1
            local b = askButton(bi)
            b.qi, b.oi, b.multi = qi, oi, q.m
            local picked = askState.picks[qi] and askState.picks[qi][oi] and true or false
            local about = type(o.d) == "string" and o.d ~= "" and o.d ~= o.l and o.d or nil
            local label = b:GetFontString()
            b:SetText(o.l)
            b:SetWidth(width)
            label:SetWidth(width - 62)
            label:ClearAllPoints()
            b.mark:ClearAllPoints()
            b.about:ClearAllPoints()
            if about then
                b.mark:SetPoint("TOPLEFT", b, "TOPLEFT", 6, -8)
                label:SetPoint("TOPLEFT", 24, -7)
                b.about:SetPoint("TOPLEFT", label, "BOTTOMLEFT", 0, -2)
                b.about:SetWidth(width - 62)
                b.about:SetText(about)
                b:SetHeight(25 + math.max(12, b.about:GetStringHeight() or 0))
            else
                b.mark:SetPoint("LEFT", b, "LEFT", 6, 0)
                label:SetPoint("LEFT", 24, 0)
                b.about:SetText("")
                b:SetHeight(30)
            end
            b.about:SetShown(about ~= nil)
            b.number:SetText(oi)
            Skin.SetVariant(b, picked and "picked" or "ghost")
            b.mark.shape = q.m and "control" or "pill"
            b.mark:SetColors("off", picked and "accent" or "border")
            b.dot:SetShown(picked and not q.m)
            b.tick:SetShown(picked and q.m and true or false)
            b:SetEnabled(not blocked)
            place(b, x, 2)
            last = b
        end
        flow.gap = flow.gap + 12
    end
    if not first then return end
    -- Submitting is in the message box, where Send is, as in T3.
    local sending = askState.sent == ask.k and GetTime() - (askState.sentAt or 0) < ANSWER_TIMEOUT
    askSend:SetText(sending and "Submitting…" or (#ask.qs > 1 and "Submit answers" or "Submit answer"))
    askSend:SetEnabled(not sending and not blocked)
    askSend:Show()
    if blocked then
        si = si + 1
        local hint = askString(si)
        hint:SetFontObject(Skin.fonts.small)
        hint:SetWidth(width)
        hint:SetText("|cffff8000" .. blocked .. " Answer it in T3.|r")
        place(hint, x, 18 + SIZE.card)
        showCard(first, hint)
    else
        flow.gap = 18 + SIZE.card
        showCard(first, last)
    end
end

-- The line T3 draws where a thread moved to another provider or its context was compacted:
-- its text in the middle, a rule each side. A message that moved provider (item.ho) gets it
-- above itself; a compaction (item.rule) is one in the list.
local Handoff = { pool = {}, used = 0 }

function Handoff.Release()
    for _, line in ipairs(Handoff.pool) do
        line.text:Hide()
        line.left:Hide()
        line.right:Hide()
    end
    Handoff.used = 0
end

function Handoff.Place(text)
    Handoff.used = Handoff.used + 1
    local line = Handoff.pool[Handoff.used]
    if not line then
        line = { text = messageChild:CreateFontString(nil, "OVERLAY"), left = Skin.Line(messageChild), right = Skin.Line(messageChild) }
        line.text:SetFontObject(Skin.fonts.muted)
        line.text:SetJustifyH("CENTER")
        line.text:SetWordWrap(false)
        -- The theme sizes its lines when it's applied, and that was before this one existed.
        line.left:SetHeight(Skin.px)
        line.right:SetHeight(Skin.px)
        Handoff.pool[Handoff.used] = line
    end
    line.text:SetWidth(messageWidth() - 8)
    line.text:SetText(text)
    place(line.text, 4, 18)
    local room = math.ceil((line.text:GetStringWidth() or 0) / 2) + 8
    line.left:ClearAllPoints()
    line.left:SetPoint("LEFT", line.text, "LEFT", 0, 0)
    line.left:SetPoint("RIGHT", line.text, "CENTER", -room, 0)
    line.left:Show()
    line.right:ClearAllPoints()
    line.right:SetPoint("LEFT", line.text, "CENTER", room, 0)
    line.right:SetPoint("RIGHT", line.text, "RIGHT", 0, 0)
    line.right:Show()
end

-- What the agent did between its messages, as T3 draws it: a dimmed line with a chevron,
-- collapsed until clicked. item.w = { p, l }: the row's first and last positions in the
-- thread (l names the row). What's in it is read when it's first opened and kept here, so
-- closing and opening it again costs nothing. view.det = { { l, b = segments, bad } }.
UI.Work = { open = {}, bodies = {}, asked = {}, retry = {} }
do
    local Work = UI.Work
    local ROW_GAP, GROUP_GAP = 10, 14
    local WORK_ICON = { b = "brain", c = "terminal", t = "hammer", q = "message", r = "eye", e = "pencil", s = "search", w = "globe" }

    -- How long since `since` (a time from the companion, on this PC's clock), as T3 says
    -- it: seconds show whenever minutes do ("12m 0s"), hours alone stay bare ("1h").
    local function elapsed(since)
        local seconds = math.max(0, time() - since)
        if seconds < 60 then return seconds .. "s" end
        local h, m, s = floor(seconds / 3600), floor(seconds % 3600 / 60), seconds % 60
        local parts = {}
        if h > 0 then parts[#parts + 1] = h .. "h" end
        if m > 0 then parts[#parts + 1] = m .. "m" end
        if s > 0 or m > 0 then parts[#parts + 1] = s .. "s" end
        return table.concat(parts, " ")
    end

    -- Every second: the running turn's clock. Kept here, so it spends no updates.
    function Work.Tick()
        if Work.live then Work.live:SetText("Working for " .. elapsed(Work.liveSince)) end
    end

    -- Takes in the contents that have arrived. Another thread starts over.
    function Work.Take(view)
        if Work.thread ~= view.id then
            Work.thread, Work.open, Work.bodies, Work.asked, Work.retry = view.id, {}, {}, {}, {}
        end
        for _, d in ipairs(type(view.det) == "table" and view.det or {}) do
            if type(d) == "table" and type(d.b) == "table" and (Work.asked[d.l] or Work.retry[d.l]) then
                Work.bodies[d.l], Work.asked[d.l], Work.retry[d.l] = d.b, nil, d.bad == 1 or nil
            end
        end
    end

    function Work.Toggle(w)
        local opening = not Work.open[w.l]
        Work.open[w.l] = opening or nil
        if opening and not w.fold and (Work.retry[w.l] or not Work.bodies[w.l]) and not Work.asked[w.l] then
            Work.bodies[w.l], Work.retry[w.l], Work.asked[w.l] = nil, nil, true
            ns.Link.Send({ t = "detail", e = selected.env, id = selected.id, p = w.p, l = w.l })
        end
        renderView()
    end

    function Work.Place(fs, w, text)
        if not rawget(fs, "chevron") then
            fs.chevron = Skin.Glyph(messageChild, "chevron", 12, "dim", "OVERLAY")
            fs.workText = messageChild:CreateFontString(nil, "OVERLAY")
            fs.workText:SetFontObject(Skin.fonts.muted)
            fs.workText:SetJustifyH("LEFT")
            fs.workText:SetWordWrap(false)
            fs.workIcon = Skin.Glyph(messageChild, "message", 14, "muted", "OVERLAY")
            fs.workAnswer = messageChild:CreateFontString(nil, "OVERLAY")
            fs.workAnswer:SetFontObject(Skin.fonts.strong)
            fs.workAnswer:SetJustifyH("LEFT")
            fs.workAnswer:SetWordWrap(false)
            fs.workTime = messageChild:CreateFontString(nil, "OVERLAY")
            fs.workTime:SetFontObject(Skin.fonts.small)
            fs.workTime:SetWordWrap(false)
        end
        fs.bubble:Hide()
        local open = Work.open[w.l] == true
        local width = messageWidth() - 8
        Skin.PointGlyph(fs.chevron, not open)
        -- Rows sit close together, and a group as far from the message above as from the one
        -- below. A message leaves 18 under it (more around a bubble or a code box: kept).
        if flow.last then flow.gap = flow.rowLast == flow.last and ROW_GAP or flow.gap - (18 - GROUP_GAP) end
        -- The turn still going, which opens nothing and tells its own time.
        if w.live then
            fs.workText:SetWidth(width)
            fs.workText:SetText("Working for " .. elapsed(w.live))
            place(fs.workText, 4, GROUP_GAP)
            flow.rowLast = fs.workText
            Work.live, Work.liveSince = fs.workText, w.live
            anchorMessage(fs, fs.workText, 0, fs.workText, 0, width)
            return
        end
        -- "Worked for": its text, with the chevron right after it. What it folds is drawn
        -- (or not) as the conversation's own items.
        if w.fold then
            fs.workText:SetWidth(width - 18)
            fs.workText:SetText(text)
            local textWidth = math.min(width - 18, math.ceil(fs.workText:GetStringWidth() or 0) + 1)
            fs.workText:SetWidth(textWidth)
            place(fs.workText, 4, GROUP_GAP)
            flow.rowLast = fs.workText
            fs.chevron:ClearAllPoints()
            fs.chevron:SetPoint("LEFT", fs.workText, "RIGHT", 6, 0)
            fs.chevron:Show()
            anchorMessage(fs, fs.workText, 0, fs.workText, 0, textWidth + 18)
            return
        end
        -- An icon for what it was on the left, the chevron at the right edge. A question
        -- also says what you answered and when.
        local q = type(w.q) == "table" and w.q or nil
        local first = fs.workIcon
        first:SetTexture("Interface\\AddOns\\T3WoW\\Media\\" .. (q and "message" or WORK_ICON[w.k] or "hammer"))
        place(first, 4, open and ROW_GAP or GROUP_GAP)
        flow.rowLast = first
        -- The game finds a new file only when it starts: one started before this version
        -- draws no icon rather than a square. Faded, not hidden: the row hangs from it.
        local file = not first.GetTextureFilePath or first:GetTextureFilePath()
        first:SetAlpha((file ~= nil and file ~= "") and 1 or 0)
        fs.chevron:ClearAllPoints()
        fs.chevron:SetPoint("LEFT", first, "LEFT", width - 12, 0)
        -- A question still waiting opens nothing: its card is further down.
        fs.chevron:SetShown(not w.still)
        local textX, textWidth = 20, width - 40
        if q then
            fs.workTime:SetText(type(q.at) == "number" and q.at > 0 and type(date) == "function" and date("%H:%M", q.at) or "")
            local timeWidth = math.ceil(fs.workTime:GetStringWidth() or 0) + 1
            fs.workTime:SetWidth(timeWidth)
            fs.workTime:ClearAllPoints()
            fs.workTime:SetPoint("LEFT", first, "LEFT", width - 20 - timeWidth, 0)
            fs.workTime:Show()
            local answerWidth = 0
            local answer = type(q.a) == "string" and q.a or ""
            if answer ~= "" then
                local most = floor(width * 0.3)
                fs.workAnswer:SetWidth(most)
                fs.workAnswer:SetText(answer)
                answerWidth = math.min(most, math.ceil(fs.workAnswer:GetStringWidth() or 0) + 1)
                fs.workAnswer:SetWidth(answerWidth)
            end
            textWidth = width - 20 - 20 - timeWidth - 10 - (answerWidth > 0 and answerWidth + 10 or 0)
            if answerWidth > 0 then
                fs.workAnswer:ClearAllPoints()
                fs.workAnswer:SetPoint("LEFT", first, "LEFT", textX + textWidth + 10, 0)
                fs.workAnswer:Show()
            end
        end
        fs.workText:ClearAllPoints()
        fs.workText:SetPoint("LEFT", first, "LEFT", textX, 0)
        fs.workText:SetWidth(math.max(20, textWidth))
        fs.workText:SetText(text)
        fs.workText:Show()
        anchorMessage(fs, first, 0, first, 0, width)
        if not open then return end
        local color, shown = "|cff" .. Skin.Hex("muted"), {}
        for i, seg in ipairs(Work.bodies[w.l] or { "Loading…" }) do
            shown[i] = type(seg) == "string" and (color .. seg .. "|r") or seg
        end
        Mono.Place(shown, false)
        flow.gap = flow.gap - (18 - GROUP_GAP)
    end
end

function renderView()
    -- The conversation is hidden while the window is resized, and drawn when you let go.
    if Sizing.from then return end
    -- Stay at the end while new messages come in, unless you scrolled up to read.
    local stickToBottom = scrollToEnd
        or messageScroll:GetVerticalScroll() >= scrollRange() - 8
    for _, fs in pairs(messageStrings) do
        fs:Hide()
        fs.hit:Hide()
        fs.bubble:Hide()
        if rawget(fs, "card") then
            fs.card:Hide()
            fs.head:Hide()
            fs.state:Hide()
        end
        if rawget(fs, "lostBox") then
            fs.lostBox:Hide()
            fs.lostIcon:Hide()
        end
        if rawget(fs, "chevron") then
            fs.chevron:Hide()
            fs.workText:Hide()
            fs.workIcon:Hide()
            fs.workAnswer:Hide()
            fs.workTime:Hide()
        end
    end
    UI.Work.live = nil
    Mono.Release()
    Pictures.Release()
    Handoff.Release()
    hideAsk()
    earlier:Hide()
    UI.RenderScripts()
    if selected and selected.new then
        messageScroll:Hide()
        newPanel:Show()
        local project = selectedProject()
        viewTitle:SetText("New thread")
        setViewMeta("")
        -- Only what the layout can't say: it's turned off, or the picks wouldn't be heard.
        local blocked = blockedReason("new")
        newHelp:SetText(blocked and ("|cffff8000" .. blocked .. "|r")
            or (inbox and inbox.sel and "")
            or "The thread uses the project's default model and mode.")
        projectMenu:SetLabel(project and project.t
            or (selected.project and "|cffff4040gone: pick another|r" or "No project"))
        workspaceMenu:SetShown(selected.project ~= nil)
        workspaceMenu:SetLabel(WORKSPACE_NAME[selected.worktree and "worktree" or "checkout"])
        return
    end
    newPanel:Hide()
    messageScroll:Show()
    if not selected then
        viewTitle:SetText("Pick a thread")
        setViewMeta("")
        flowStart(0)
        fit(true)
        return
    end
    local view = inbox and inbox.view
    if not isSelectedView(view) then view = nil end
    local listed = selectedThread()
    viewTitle:SetText((view and view.t) or (listed and listed.t) or "")
    local status = (listed and listed.s) or (view and view.s)
    local e = envById(selected.env)
    local meta = format("%s · %s", e and e.name or "?", age(listed and (listed.la or listed.at)))
    -- The branch its checkout is on, last so a long one is what gets cut short. Only the
    -- open thread's read carries it (T3's list doesn't), so the list rows go without.
    if view and view.br and view.br ~= "" then
        meta = meta .. " · " .. view.br
    end
    setViewMeta(meta, view and view.prs)
    if not view then
        flowStart(0)
        placeMessage(messageString(1), "|cff" .. Skin.Hex("dim") .. "Loading…|r", false)
        fit(true)
        return
    end
    UI.Work.Take(view)
    flowStart(0)
    if view.more then
        earlier:SetPoint("TOP", messageChild, "TOP", 0, 0)
        earlier:Show()
        flowStart(30)
    end
    local items = view.items or {}
    local count = #items
    -- Its row says the turn is going, in place of the line at the end.
    local liveRow = false
    for i, item in ipairs(items) do
        liveRow = liveRow or (type(item.w) == "table" and item.w.live ~= nil)
        -- Inside a finished turn's "Worked for" row (f names it), while that is closed.
        if type(item.f) ~= "string" or UI.Work.open[item.f] then
            local fs = messageString(i)
            if item.ho then Handoff.Place("Context handoff · " .. item.ho) end
            local rule = item.rule == 1
            local work = type(item.w) == "table" and item.w or false
            local card = type(item.d) == "table"
            local lost = not card and item.lost == 1
            local picture = not card and item.im ~= nil and Pictures.Place(fs, item.im, item.r == "u")
            if work then
                UI.Work.Place(fs, work, item.x)
            elseif rule then
                Handoff.Place(item.x)
            elseif card then
                placeCard(fs, item.d)
            elseif lost then
                Pictures.Lost(fs, item.r == "u")
            elseif not picture then
                placeMessage(fs, item.x, item.r == "u", item.g)
            end
            -- Copying takes the message's text, not its segments: the code with its markdown.
            -- cx is that text with each model T3 names written as T3 has it (model:...).
            fs.hit.copyText = item.cx or item.x
            -- A picture enlarges instead.
            fs.hit.picture = picture and item.im or false
            -- A row opens instead.
            fs.hit.work = work
            -- A picture that couldn't be loaded has nothing to enlarge or copy; a delegated
            -- task's card isn't a message to copy.
            fs.hit:SetShown(not lost and not rule and not card and not (work and work.still))
        end
    end
    if echo and echo.id == selected.id then
        -- Your message is in once any recent message of yours matches it: the agent may
        -- already have answered, so it needn't be the last one. Compare letters and digits
        -- only, since the thread view reformats text (markdown, escaped pipes). A message that
        -- names a model shows its name; its copy text (cx) has the model:... you typed.
        local function key(text)
            return (text or ""):gsub("||", ""):gsub("|c%x%x%x%x%x%x%x%x", ""):gsub("|r", ""):gsub("[^%w]", ""):lower():sub(1, 40)
        end
        local wanted, arrived = key(echo.text), false
        for i = count, math.max(1, count - 7), -1 do
            local item = items[i]
            if item.r == "u" and (key(item.x) == wanted or (item.cx and key(item.cx) == wanted)) then arrived = true break end
        end
        if arrived then
            echo = nil
        else
            local fs = messageString(count + 1)
            local pictures = (echo.images or 0) > 0 and format("[%d %s]\n", echo.images, echo.images == 1 and "image" or "images") or ""
            placeMessage(fs, "|cff" .. Skin.Hex("dim") .. "(sending)|r\n" .. pictures .. echo.text:gsub("|", "||"), true)
        end
    end
    renderAsk(view)
    if WORKING[status] and not liveRow then
        placeMessage(messageString(count + 2), "|cff" .. Skin.Hex("muted") .. "Working…|r", false)
    end
    -- Sized now from what the game has laid out; `settle` sizes it again as text heights
    -- settle.
    if stickToBottom then scrollToEnd = false end
    fit(stickToBottom)
end

-- ---------------------------------------------------------------- composer

-- The box is a card; its outline takes the focus color while you type.
local inputScroll = CreateFrame("ScrollFrame", nil, right, "UIPanelScrollFrameTemplate")
-- It leaves room inside the card, under the text, for the model, effort and Send or Stop.
-- In New Thread the card moves to the middle (Selectors.PlaceBox).
inputScroll:SetPoint("BOTTOMLEFT", 28, 48)
inputScroll:SetPoint("BOTTOMRIGHT", -40, 48)
inputScroll:SetHeight(60)
Skin.ScrollBar(inputScroll)
local inputBg = CreateFrame("Frame", nil, right)
inputBg:SetPoint("TOPLEFT", inputScroll, -10, 10)
inputBg:SetPoint("BOTTOMRIGHT", inputScroll, 26, -36)
inputBg:SetFrameLevel(math.max(0, inputScroll:GetFrameLevel() - 1))
local inputBox = Skin.Box(inputBg, { shape = "panel", fill = "input", ring = "border" })

-- The list that opens as you type / $ or @ (filled in further down, once what it reads exists).
-- On UI, not a local of its own: the file has all of Lua's 200 locals in one chunk.
UI.Picker = {}

local input = CreateFrame("EditBox", nil, inputScroll)
input:SetMultiLine(true)
input:SetAutoFocus(false)
input:SetFontObject(Skin.fonts.text)
input:SetTextInsets(0, 0, 2, 2)
input:SetWidth(messageWidth() - 24)
-- The companion looks for Ctrl+V only while you're typing here, so it hears when you are.
input:SetScript("OnEditFocusGained", function()
    inputBox:SetColors(nil, "focus")
    ns.Link.Send({ t = "ui", o = true, f = true })
end)
input:SetScript("OnEditFocusLost", function()
    inputBox:SetColors(nil, "border")
    -- A window closing says so itself.
    if frame:IsShown() then ns.Link.Send({ t = "ui", o = true, f = false }) end
    -- Not at once: a click on one of the picker's rows may be what took the focus.
    C_Timer.After(0.2, function()
        if not input:HasFocus() then UI.Picker.Close() end
    end)
end)
input:SetMaxBytes(2400)
inputScroll:SetScrollChild(input)
input:SetScript("OnEscapePressed", function(self)
    if not UI.Picker.Close() then self:ClearFocus() end
end)
inputScroll:SetScript("OnMouseDown", function() input:SetFocus() end)
local placeholder = input:CreateFontString(nil, "OVERLAY")
placeholder:SetFontObject(Skin.fonts.small)
placeholder:SetPoint("TOPLEFT", 0, -3)
placeholder:SetText("Message the agent (Shift+Enter for a new line)")
input:SetScript("OnTextChanged", function(self, userInput)
    placeholder:SetShown(self:GetText() == "")
    UI.Picker.Update(userInput)
    UI.ComposerButton()
end)
-- What the empty box says: what Enter does, or that the tray setting turned it off.
setPlaceholder = function()
    local asking = pendingAsk() ~= nil
    local blocked = blockedReason((selected and selected.new) and "new" or (asking and "answer" or "send"))
    local view = inbox and inbox.view
    local card = not asking and isSelectedView(view) and view.card
    placeholder:SetText(blocked or (asking and "Type your own answer, or pick one above")
        or (card and "Answer T3's card in T3, not here (a message goes to the agent)")
        or (inbox and inbox.pi and "Message the agent (Shift+Enter for a new line, Ctrl+V pastes images too)")
        or "Message the agent (Shift+Enter for a new line)")
end
input:SetScript("OnCursorChanged", function(_, _, y, _, height)
    local offset = inputScroll:GetVerticalScroll()
    if -y < offset then inputScroll:SetVerticalScroll(-y)
    elseif -y + height > offset + inputScroll:GetHeight() then inputScroll:SetVerticalScroll(-y + height - inputScroll:GetHeight()) end
end)

-- ---------------------------------------------------------------- pasting images
-- The game can't read the clipboard, and Ctrl+V pastes only text into the box. When it
-- pastes nothing, the clipboard may hold an image: the companion reads it, and keeps it
-- for the message being written (a thread's, or New Thread's), drawn small above the box
-- (inbox.att: { d = the message, i = its number, w, h, f, tw, th for the small picture's
-- image slot and size, bf, bw, bh for the large one's }). It takes the paste only right
-- after a real Ctrl+V in WoW, so no other addon can paste for you. A click enlarges one,
-- each × takes one back, and Send sends them along.
-- One table, not a local each: the file is near Lua's 200 locals in one chunk.
local Paste = {}
do
    local MAX_PASTED = 8
    local PASTE_HEIGHT = 44
    Paste.rowHeight = PASTE_HEIGHT + 6

    -- The message being written: "new" in New Thread, else "<machine>:<thread>".
    function Paste.DraftKey()
        if not selected then return nil end
        if selected.new then return "new" end
        return (selected.env or "") .. ":" .. tostring(selected.id)
    end

    function Paste.For(key)
        local list = {}
        for _, image in ipairs(key and inbox and type(inbox.att) == "table" and inbox.att or {}) do
            if type(image) == "table" and image.d == key then list[#list + 1] = image end
        end
        return list
    end

    local function requestPaste()
        local key = Paste.DraftKey()
        -- An older companion can't paste: Ctrl+V just pastes text, as it always did.
        if not key or not (inbox and inbox.pi) then return end
        local blocked = blockedReason("paste")
        if blocked then return UI.Notice(blocked) end
        if #Paste.For(key) >= MAX_PASTED then return UI.Notice(format("A message takes %d images at most.", MAX_PASTED)) end
        local s = ns.Link.Send({ t = "paste", d = key })
        if s then awaiting[s] = { kind = "paste", at = GetTime() } end
        UI.Refresh()
    end

    input:SetScript("OnKeyDown", function(self, key)
        UI.Picker.Key(self, key)
        if key ~= "V" or not (IsControlKeyDown() or (IsMetaKeyDown and IsMetaKeyDown())) then return end
        -- Once the game has pasted: no text came in, so look for an image.
        local before = self:GetText()
        C_Timer.After(0, function()
            if self:GetText() == before then requestPaste() end
        end)
    end)

    local pasteRow = CreateFrame("Frame", nil, right)
    pasteRow:SetPoint("BOTTOMLEFT", inputBg, "TOPLEFT", 0, 6)
    pasteRow:SetPoint("BOTTOMRIGHT", inputBg, "TOPRIGHT", 0, 6)
    pasteRow:SetHeight(PASTE_HEIGHT)
    pasteRow:Hide()
    local pasteChips = {}

    local function chipTooltip(self)
        Skin.Tooltip:SetOwner(self, "ANCHOR_TOP")
        Skin.Tooltip:AddLine(format("Pasted image, %d x %d", tonumber(self.image.w) or 0, tonumber(self.image.h) or 0))
        if type(self.image.bf) == "number" then Skin.Tooltip:AddLine("Click to enlarge") end
        Skin.Tooltip:AddLine("It goes with your next message. × takes it back.")
        Skin.Tooltip:Show()
    end

    -- Enlarges the large picture the companion keeps of it (bf, bw, bh: its image slot and
    -- size); one that has none, from an older companion, does nothing.
    local function chipClick(self)
        local image = self.image
        if image and type(image.bf) == "number" then
            Pictures.Zoom({ f = image.bf, w = image.bw, h = image.bh })
        end
    end

    local function pasteChip(index)
        local chip = pasteChips[index]
        if chip then return chip end
        chip = CreateFrame("Button", nil, pasteRow)
        chip.box = Skin.Box(chip, { shape = "panel", fill = "card", ring = "border" })
        chip.picture = chip:CreateTexture(nil, "ARTWORK")
        chip.picture:SetPoint("CENTER")
        chip.label = chip:CreateFontString(nil, "OVERLAY")
        chip.label:SetFontObject(Skin.fonts.small)
        chip.label:SetPoint("CENTER")
        chip.remove = Skin.CloseButton(chip, function()
            local image = chip.image
            if image then ns.Link.Send({ t = "unpaste", d = image.d, i = image.i }) end
        end)
        chip.remove:SetSize(18, 18)
        chip.remove:SetPoint("TOPRIGHT", 4, 4)
        chip:SetScript("OnEnter", chipTooltip)
        chip:SetScript("OnLeave", function() Skin.Tooltip:Hide() end)
        chip:SetScript("OnClick", chipClick)
        pasteChips[index] = chip
        return chip
    end

    -- T3's /clear: the open thread's next message starts a fresh provider session (the agent
    -- won't see the thread's earlier messages). Armed per thread, as T3 keeps it, with a
    -- notice above the box until that message goes; its × keeps the current session.
    -- Offered where the thread's T3 can do it (fc on its machine, from the companion).
    Paste.fresh = {}
    local FRESH_HEIGHT = 42
    local freshRow = CreateFrame("Frame", "T3WoWFreshNotice", right)
    freshRow:SetHeight(FRESH_HEIGHT)
    freshRow:Hide()
    Skin.Box(freshRow, { shape = "panel", fill = "card", ring = "border" })
    freshRow.title = freshRow:CreateFontString(nil, "OVERLAY")
    freshRow.title:SetFontObject(Skin.fonts.strong)
    freshRow.title:SetPoint("TOPLEFT", 10, -7)
    freshRow.title:SetPoint("RIGHT", -30, 0)
    freshRow.title:SetJustifyH("LEFT")
    freshRow.title:SetText("Your next message starts a fresh session")
    freshRow.note = freshRow:CreateFontString(nil, "OVERLAY")
    freshRow.note:SetFontObject(Skin.fonts.muted)
    freshRow.note:SetPoint("TOPLEFT", freshRow.title, "BOTTOMLEFT", 0, -3)
    freshRow.note:SetPoint("RIGHT", -30, 0)
    freshRow.note:SetJustifyH("LEFT")
    freshRow.note:SetText("The agent won't see earlier messages in this thread.")
    freshRow.keep = Skin.CloseButton(freshRow, function() Paste.Arm(false) end)
    freshRow.keep:SetPoint("TOPRIGHT", -4, -4)
    freshRow.keep:SetScript("OnEnter", function(self)
        Skin.Tooltip:SetOwner(self, "ANCHOR_TOP")
        Skin.Tooltip:AddLine("Keep the current session")
        Skin.Tooltip:Show()
    end)
    freshRow.keep:SetScript("OnLeave", function() Skin.Tooltip:Hide() end)

    -- Whether the open thread's T3 starts fresh sessions.
    function Paste.CanFresh()
        if not selected or selected.new then return false end
        local e = envById(selected.env)
        return e ~= nil and e.fc == true
    end

    -- Whether the open thread's next message starts fresh.
    function Paste.Fresh()
        local key = Paste.DraftKey()
        return key ~= nil and Paste.fresh[key] == true and Paste.CanFresh()
    end

    function Paste.Arm(on)
        local key = Paste.DraftKey()
        if not key or (on and not Paste.CanFresh()) then return end
        Paste.fresh[key] = on or nil
        UI.Refresh()
    end

    -- The images pasted for the message being written, in a row above the box; the
    -- conversation (or New Thread's choices) ends above them while there are any.
    function Paste.Render()
        local list = Paste.For(Paste.DraftKey())
        local x = 0
        for i, image in ipairs(list) do
            local chip = pasteChip(i)
            chip.image = image
            local height = PASTE_HEIGHT - 4
            local width = 64
            if type(image.f) == "number" and tonumber(image.tw) and tonumber(image.th) and tonumber(image.th) > 0 then
                width = math.min(120, math.max(24, height * image.tw / image.th))
                chip.picture:SetTexture(format("Interface\\AddOns\\T3WoW\\img\\%04d.tga", image.f))
                chip.picture:SetSize(width - 4, height - 4)
                chip.picture:Show()
                chip.label:SetText("")
            else
                chip.picture:Hide()
                chip.label:SetText(format("Image %d", i))
            end
            chip:SetSize(width, height)
            chip:ClearAllPoints()
            chip:SetPoint("BOTTOMLEFT", pasteRow, "BOTTOMLEFT", x, 0)
            chip:Show()
            x = x + width + 8
        end
        for i = #list + 1, #pasteChips do pasteChips[i]:Hide() end
        pasteRow:SetShown(#list > 0)
        local fresh = Paste.Fresh()
        local under = #list > 0 and pasteRow or inputBg
        freshRow:ClearAllPoints()
        freshRow:SetPoint("BOTTOMLEFT", under, "TOPLEFT", 0, 6)
        freshRow:SetPoint("BOTTOMRIGHT", under, "TOPRIGHT", 0, 6)
        freshRow:SetShown(fresh)
        local bottom = SIZE.composer + (#list > 0 and (PASTE_HEIGHT + 6) or 0) + (fresh and (FRESH_HEIGHT + 6) or 0)
        messageScroll:SetPoint("BOTTOMRIGHT", -26, bottom)
        newPanel:SetPoint("BOTTOMRIGHT", -26, bottom)
    end
end

-- ---------------------------------------------------------------- model, effort, permissions
-- The row inside the box, as in T3's composer: the model, its effort and, on the right, the
-- permissions. New Thread picks all three (inbox.models says what each machine offers, and
-- envs[i].dm the permissions a thread gets when none is picked); what you leave alone is
-- T3's default. An open thread's model and effort change as you pick them. T3's tools can't
-- change a thread's permissions once it exists, so there they only show. A companion
-- without inbox.sel ignores these picks: the row stays hidden rather than promise them.
-- One table, not a local each: the file is near Lua's 200 locals in one chunk.
local Selectors = {}
do
    -- A change just picked is shown until the thread's read agrees, T3 refuses it, or this passes.
    local PENDING_TIMEOUT = 20
    local MODES = {
        { id = "approval-required", name = "Supervised", icon = "lock", short = "Ask", text = "Ask before running commands and editing files." },
        { id = "auto-accept-edits", name = "Auto-accept edits", icon = "pencil", short = "Edits", text = "Apply edits without asking, and ask before running commands." },
        { id = "auto", name = "Auto", icon = "sparkle", short = "Auto", text = "Let the agent decide when it needs to ask." },
        { id = "full-access", name = "Full access", icon = "shield-off", short = "Full", text = "Allow commands and edits without prompts." },
    }
    local MODE = {}
    for _, mode in ipairs(MODES) do MODE[mode.id] = mode end
    local EFFORT_NAME = {
        low = "Low", medium = "Medium", high = "High", xhigh = "Extra high", max = "Max",
        ultra = "Ultra", ultracode = "Ultracode", ultrathink = "Ultrathink", auto = "Auto",
    }
    local function effortName(value) return EFFORT_NAME[value] or tostring(value):gsub("|", "||") end
    local pending -- { env, id, s, pv, m, ef, at }: a change sent for an open thread

    local function providersOn(envId)
        local index, list = envIndexById(envId), {}
        for _, p in ipairs(inbox and type(inbox.models) == "table" and inbox.models or {}) do
            if type(p) == "table" and p.e == index then list[#list + 1] = p end
        end
        return list
    end

    -- The model as the machine offers it (nil for one it doesn't list), and its provider.
    local function find(envId, pv, m)
        for _, p in ipairs(providersOn(envId)) do
            if p.id == pv then
                for _, model in ipairs(p.m or {}) do
                    if model.id == m then return model, p end
                end
                return nil, p
            end
        end
    end

    local function offers(model, effort)
        for _, value in ipairs(model and model.ef or {}) do
            if value == effort then return true end
        end
        return false
    end

    local function allows(provider, mode)
        if not provider or type(provider.rm) ~= "table" then return true end
        for _, id in ipairs(provider.rm) do
            if id == mode then return true end
        end
        return false
    end

    -- What the row shows: { new, pv, m, ef, rm }, or nil when it has nothing to show.
    local function current()
        if not selected or not inbox or not inbox.sel then return nil end
        if selected.new then
            local e = envById(selected.env)
            if not e then return nil end
            return { new = true, pv = selected.pv, m = selected.m, ef = selected.ef, rm = selected.rm or e.dm }
        end
        local view = inbox.view
        if not isSelectedView(view) or type(view.cfg) ~= "table" then return nil end
        local cfg = view.cfg
        if pending and pending.env == selected.env and pending.id == selected.id then
            local arrived = pending.pv == cfg.pv and pending.m == cfg.m and pending.ef == cfg.ef
            if not arrived and GetTime() - pending.at < PENDING_TIMEOUT then
                return { pv = pending.pv, m = pending.m, ef = pending.ef, rm = cfg.rm }
            end
            pending = nil
        end
        return cfg
    end

    Selectors.ProvidersOn = providersOn

    -- The provider the message being written goes to, when that's known.
    function Selectors.Provider()
        local c = current()
        return c and c.pv
    end

    -- Sends a new model or effort for the open thread.
    local function change(pv, m, ef)
        local blocked = blockedReason("model")
        if blocked then return UI.Notice(blocked) end
        local s = ns.Link.Send({ t = "model", e = selected.env, id = selected.id, pv = pv, md = m, ef = ef })
        if s then awaiting[s] = { kind = "model", at = GetTime() } end
        pending = { env = selected.env, id = selected.id, s = s, pv = pv, m = m, ef = ef, at = GetTime() }
        UI.Refresh()
    end

    local function chip(name, width, menu)
        menu.above = true
        local dd = dropdown(inputBg, name, width, menu)
        dd:SetHeight(22)
        dd.label:SetFontObject(Skin.fonts.muted)
        dd:Hide()
        return dd
    end

    local modelMenu = chip("T3WoWModelMenu", 150, {
        entries = function()
            local c, list = current(), {}
            if not c then return list end
            if c.new then list[1] = { text = "Default model" } end
            local providers, listed = providersOn(selected.env), false
            -- Read-only: a started thread's models only show, and the menu still opens for its permissions.
            local locked = not c.new and blockedReason("model") ~= nil
            for _, p in ipairs(providers) do
                if #providers > 1 then list[#list + 1] = { text = p.t, note = true } end
                for _, model in ipairs(p.m or {}) do
                    list[#list + 1] = { text = model.t, value = { pv = p.id, m = model.id }, note = locked }
                    listed = listed or (p.id == c.pv and model.id == c.m)
                end
            end
            -- T3 lists its usual models only; the one a thread is on stays shown.
            if not c.new and not listed and c.m then
                table.insert(list, 1, { text = c.mt or c.m:gsub("|", "||"), value = { pv = c.pv, m = c.m }, note = locked })
            end
            if #list == 0 then list[1] = { text = "No models on this machine", note = true } end
            -- As in T3, a thread that has started shows its permissions here.
            if not c.new then
                list[#list + 1] = { text = "Permissions", note = true }
                list[#list + 1] = { text = MODE[c.rm] and MODE[c.rm].name or "Default", value = { access = true } }
            end
            return list
        end,
        isSelected = function(value)
            local c = current()
            if not c or (value and value.access) then return false end
            if not value then return c.new and c.m == nil end
            return value.pv == c.pv and value.m == c.m
        end,
        pick = function(value)
            if value and value.access then return Selectors.OpenAccess(Selectors.modelMenu) end
            local c = current()
            if not c then return end
            local model, provider
            if value then model, provider = find(selected.env, value.pv, value.m) end
            local effort = offers(model, c.ef) and c.ef or nil
            if c.new then
                selected.pv, selected.m, selected.ef = value and value.pv, value and value.m, effort
                -- A mode this provider doesn't run goes back to the default.
                if selected.rm and not allows(provider, selected.rm) then selected.rm = nil end
                UI.Refresh()
            elseif value and (value.pv ~= c.pv or value.m ~= c.m) then
                change(value.pv, value.m, effort)
            end
        end,
    })
    Selectors.modelMenu = modelMenu

    -- New Thread's machine, under the box as in T3. A thread that has started stays on its own.
    local machineMenu = chip("T3WoWMachineMenu", 190, {
        entries = function()
            local list = {}
            for _, e in ipairs(inbox and inbox.envs or {}) do list[#list + 1] = { text = e.name, value = e.id } end
            if #list == 0 then list[1] = { text = "No machines linked: link one in T3", note = true } end
            return list
        end,
        isSelected = function(id) return composing() and selected.env == id end,
        pick = function(id)
            if not composing() or selected.env == id then return end
            -- Another machine offers other models: what was picked for this one goes.
            selected.env, selected.project = id, nil
            selected.pv, selected.m, selected.ef, selected.rm = nil, nil, nil, nil
            UI.Refresh()
        end,
    })
    machineMenu:SetPoint("TOPLEFT", inputBg, "BOTTOMLEFT", 2, -6)

    local EFFORT_WIDTH, PERMISSION_WIDTH = 96, 148
    local effortMenu = chip("T3WoWEffortMenu", EFFORT_WIDTH, {
        entries = function()
            local c, list = current(), {}
            local model = c and find(selected.env, c.pv, c.m)
            if not model then return list end
            if c.new then list[1] = { text = "Default" } end
            for _, value in ipairs(model.ef or {}) do
                list[#list + 1] = { text = effortName(value), value = value }
            end
            return list
        end,
        isSelected = function(value)
            local c = current()
            return c ~= nil and c.ef == value
        end,
        pick = function(value)
            local c = current()
            if not c then return end
            if c.new then
                selected.ef = value
                UI.Refresh()
            elseif value ~= c.ef then
                change(c.pv, c.m, value)
            end
        end,
    })

    -- Model, effort, then Send or Stop run along the bottom of the card, from its right edge.
    function Selectors.PlaceRow(button)
        local effort = Selectors.effortShown
        effortMenu:ClearAllPoints()
        effortMenu:SetPoint("RIGHT", button, "LEFT", -6, 0)
        modelMenu:ClearAllPoints()
        modelMenu:SetPoint("RIGHT", effort and effortMenu or button, "LEFT", -6, 0)
        local used = 22 + button:GetWidth() + (effort and (EFFORT_WIDTH + 6) or 0)
        modelMenu:SetWidth(math.min(220, math.max(110, inputBg:GetWidth() - used)))
    end

    local permissionMenu = chip("T3WoWPermissionMenu", PERMISSION_WIDTH, {})
    permissionMenu:SetPoint("TOPRIGHT", inputBg, "BOTTOMRIGHT", -2, -6)
    permissionMenu.icon = Skin.Glyph(permissionMenu, "lock", 14, "muted", "OVERLAY")
    permissionMenu.icon:SetPoint("LEFT", 8, 0)

    local known = UI.GlyphKnown

    local POPOVER_WIDTH, SEGMENT_HEIGHT = 300, 28
    local popover = CreateFrame("Frame", "T3WoWPermissions", UIParent)
    popover:SetFrameStrata("FULLSCREEN_DIALOG")
    popover:SetFrameLevel(20)
    popover:SetClampedToScreen(true)
    popover:EnableMouse(true)
    popover:SetWidth(POPOVER_WIDTH)
    Skin.Box(popover, { shape = "panel", fill = "card", ring = "border", ringLayer = "BORDER" })
    popover:Hide()
    -- A click anywhere else closes it, as with the dropdown lists.
    local catcher = CreateFrame("Button", nil, UIParent)
    catcher:SetFrameStrata("FULLSCREEN_DIALOG")
    catcher:SetFrameLevel(10)
    catcher:SetAllPoints(UIParent)
    catcher:RegisterForClicks("AnyUp")
    catcher:SetScript("OnClick", function() popover:Hide() end)
    catcher:Hide()
    popover:SetScript("OnHide", function() catcher:Hide() end)
    popupTakesEscape(popover)

    local heading = popover:CreateFontString(nil, "OVERLAY")
    heading:SetFontObject(Skin.fonts.muted)
    heading:SetPoint("TOPLEFT", 12, -10)
    heading:SetText("Permissions")
    local name = popover:CreateFontString(nil, "OVERLAY")
    name:SetFontObject(Skin.fonts.strong)
    name:SetPoint("TOPLEFT", 12, -66)
    name:SetJustifyH("LEFT")
    local description = popover:CreateFontString(nil, "OVERLAY")
    description:SetFontObject(Skin.fonts.muted)
    description:SetPoint("TOPLEFT", 12, -84)
    description:SetWidth(POPOVER_WIDTH - 24)
    description:SetJustifyH("LEFT")
    description:SetWordWrap(true)
    local locked = popover:CreateFontString(nil, "OVERLAY")
    locked:SetFontObject(Skin.fonts.small)
    locked:SetPoint("TOPLEFT", 12, -118)
    locked:SetWidth(POPOVER_WIDTH - 24)
    locked:SetJustifyH("LEFT")
    locked:SetWordWrap(true)
    locked:SetText("A thread keeps the permissions it started with here. To change them, use T3.")
    local openT3 = button(popover, "Open in T3", 110, function()
        popover:Hide()
        if selected and not selected.new then openInT3(selected.id, selected.env) end
    end)
    openT3:SetPoint("BOTTOMRIGHT", -12, 10)

    local function describe(mode)
        name:SetText(mode and mode.name or "Default")
        description:SetText(mode and mode.text or "T3 sets it from this machine and your sign-in.")
    end

    local function describeCurrent()
        local c = current()
        describe(c and MODE[c.rm])
    end

    local segments = {}
    local segmentWidth = (POPOVER_WIDTH - 24) / #MODES
    for index, mode in ipairs(MODES) do
        local segment = CreateFrame("Button", nil, popover)
        segment:SetSize(segmentWidth - 2, SEGMENT_HEIGHT)
        segment:SetPoint("TOPLEFT", 12 + (index - 1) * segmentWidth + 1, -30)
        segment.mode = mode
        segment.lit = Skin.Box(segment, { shape = "control", fill = "selected", ring = "selectedRing" })
        segment.lit:Hide()
        segment.glyph = Skin.Glyph(segment, mode.icon, 16, "muted", "OVERLAY")
        segment.glyph:SetPoint("CENTER")
        -- What it says when the game has no icon file yet.
        segment.short = segment:CreateFontString(nil, "OVERLAY")
        segment.short:SetFontObject(Skin.fonts.muted)
        segment.short:SetPoint("CENTER")
        segment.short:SetText(mode.short)
        segment.short:Hide()
        segment:SetScript("OnClick", function(self)
            if not selected or not selected.new then return end
            selected.rm = self.mode.id
            UI.Refresh()
        end)
        segment:SetScript("OnEnter", function(self) describe(self.mode) end)
        segment:SetScript("OnLeave", describeCurrent)
        Skin.Hover(segment)
        segments[index] = segment
    end

    local function renderPopover(c)
        local provider = select(2, find(selected.env, c.pv, c.m))
        for _, segment in ipairs(segments) do
            local on = segment.mode.id == c.rm
            local open = c.new and allows(provider, segment.mode.id)
            segment.lit:SetShown(on)
            segment:SetEnabled(open and true or false)
            local drawn = known(segment.glyph)
            segment.glyph:SetShown(drawn)
            segment.short:SetShown(not drawn)
            Skin.SetGlyphColor(segment.glyph, on and "text" or (open and "muted" or "dim"))
        end
        describe(MODE[c.rm])
        locked:SetShown(not c.new)
        openT3:SetShown(not c.new)
        popover:SetHeight(c.new and 122 or 192)
    end

    -- Opens over what was clicked: the chip in New Thread, the model's menu in a thread.
    function Selectors.OpenAccess(anchor)
        if popover:IsShown() then return popover:Hide() end
        local c = current()
        if not c then return end
        popover:ClearAllPoints()
        popover:SetPoint("BOTTOMRIGHT", anchor, "TOPRIGHT", 0, 4)
        renderPopover(c)
        catcher:Show()
        popover:Show()
    end
    permissionMenu:SetScript("OnClick", function(self) Selectors.OpenAccess(self) end)
    permissionMenu:SetScript("OnHide", function() popover:Hide() end)
    modelMenu:HookScript("OnHide", function() popover:Hide() end)

    -- The box at the bottom of a thread; in New Thread, in the middle under T3's question,
    -- no wider than it reads well. The card reaches past the text area (see inputBg), so
    -- the offsets are for the card and what's over and under it to be centered as a whole.
    function Selectors.PlaceBox(centered)
        inputScroll:ClearAllPoints()
        if not centered then
            inputScroll:SetPoint("BOTTOMLEFT", 28, 48)
            inputScroll:SetPoint("BOTTOMRIGHT", -40, 48)
            input:SetWidth(messageWidth() - 24)
            return
        end
        local textWidth = math.max(320, math.min(680, right:GetWidth() - 60)) - 36
        inputScroll:SetWidth(textWidth)
        inputScroll:SetPoint("CENTER", right, "CENTER", -8, -43)
        input:SetWidth(textWidth)
        -- Pasted images sit right over the box: the project moves up for them.
        local raise = 14 + (#Paste.For("new") > 0 and Paste.rowHeight or 0)
        local both = selected.project ~= nil
        projectMenu:SetWidth(both and 200 or 240)
        projectMenu:ClearAllPoints()
        workspaceMenu:ClearAllPoints()
        if both then
            projectMenu:SetPoint("BOTTOMRIGHT", inputBg, "TOP", -3, raise)
            workspaceMenu:SetPoint("BOTTOMLEFT", inputBg, "TOP", 3, raise)
        else
            projectMenu:SetPoint("BOTTOM", inputBg, "TOP", 0, raise)
        end
        newPanel.heading:ClearAllPoints()
        newPanel.heading:SetPoint("BOTTOM", inputBg, "TOP", 0, raise + 38)
    end

    function Selectors.Render()
        local composingNew = composing() and true or false
        Selectors.PlaceBox(composingNew)
        machineMenu:SetShown(composingNew)
        if composingNew then
            local e = envById(selected.env)
            local noMachines = #(inbox and inbox.envs or {}) == 0
            machineMenu:SetLabel(e and e.name or (noMachines and "|cffff4040none linked|r" or "|cffff4040no longer linked: pick another|r"))
        end
        local c = current()
        if not c then
            modelMenu:Hide()
            effortMenu:Hide()
            permissionMenu:Hide()
            Selectors.effortShown = false
            return
        end
        local model = find(selected.env, c.pv, c.m)
        -- A started thread's permissions are in this menu, so it shows even with no models.
        modelMenu:SetShown(not c.new or c.m ~= nil or #providersOn(selected.env) > 0)
        modelMenu:SetLabel(model and model.t or c.mt or (c.m and c.m:gsub("|", "||")) or "Default model")
        Selectors.effortShown = model ~= nil and type(model.ef) == "table" and #model.ef > 0
        effortMenu:SetShown(Selectors.effortShown)
        effortMenu:SetLabel(c.ef and effortName(c.ef) or "Default")
        local changeable = c.new or not blockedReason("model")
        effortMenu:SetEnabled(changeable and true or false)
        local mode = MODE[c.rm]
        permissionMenu:SetShown(c.new and true or false)
        permissionMenu:SetLabel(mode and mode.name or "Default")
        Skin.SetGlyphFile(permissionMenu.icon, mode and mode.icon or "lock")
        local drawn = known(permissionMenu.icon)
        permissionMenu.icon:SetShown(drawn)
        permissionMenu.label:SetPoint("LEFT", drawn and 26 or 9, 0)
        if popover:IsShown() then renderPopover(c) end
    end

    -- T3 refused the change: back to what the thread really runs on.
    function Selectors.Refused(s)
        if pending and pending.s == s then pending = nil end
    end

    function Selectors.Forget() pending = nil end
end

-- ---------------------------------------------------------------- copying
-- The game can't write to the clipboard, so a clicked message opens over the conversation
-- in a read-only box, highlighted: Ctrl+C copies it (or the part you select), and closes
-- the box, as does Escape.
local copyBox = CreateFrame("Frame", nil, right)
copyBox:SetPoint("TOPLEFT", messageScroll, -8, 8)
copyBox:SetPoint("BOTTOMRIGHT", messageScroll, 24, -8)
Skin.Box(copyBox, { shape = "panel", fill = "card", ring = "focus" })
copyBox:SetFrameLevel(messageScroll:GetFrameLevel() + 20)
copyBox:EnableMouse(true)
copyBox:Hide()
local copyHelp = copyBox:CreateFontString(nil, "OVERLAY")
copyHelp:SetFontObject(Skin.fonts.small)
copyHelp:SetPoint("TOPLEFT", 12, -12)
copyHelp:SetText("Ctrl+C copies the highlighted text. Escape closes.")
local copyClose = Skin.CloseButton(copyBox, function() copyBox:Hide() end)
copyClose:SetPoint("TOPRIGHT", -4, -4)
local copyScroll = CreateFrame("ScrollFrame", nil, copyBox, "UIPanelScrollFrameTemplate")
copyScroll:SetPoint("TOPLEFT", 12, -32)
copyScroll:SetPoint("BOTTOMRIGHT", -24, 10)
Skin.ScrollBar(copyScroll)
local copyEdit = CreateFrame("EditBox", nil, copyScroll)
copyEdit:SetMultiLine(true)
copyEdit:SetAutoFocus(false)
copyEdit:SetFontObject(Skin.fonts.text)
copyEdit:SetMaxLetters(0)
copyEdit:SetWidth(messageWidth() - 8)
copyScroll:SetScrollChild(copyEdit)
copyEdit:SetScript("OnEscapePressed", function() copyBox:Hide() end)
-- Read-only: whatever is typed puts the message back.
copyEdit:SetScript("OnTextChanged", function(self, userInput)
    if userInput then
        self:SetText(copyBox.text or "")
        self:HighlightText()
    end
end)
copyEdit:SetScript("OnKeyDown", function(_, key)
    if key == "C" and (IsControlKeyDown() or (IsMetaKeyDown and IsMetaKeyDown())) then
        C_Timer.After(0.1, function() copyBox:Hide() end) -- after the game has copied
    end
end)
copyBox:SetScript("OnHide", function() copyEdit:ClearFocus() end)
messageScroll:HookScript("OnHide", function() copyBox:Hide() end)

-- Messages come as game text: color codes go, and doubled pipes are single again.
function openCopy(text)
    copyBox.text = ((text or ""):gsub("||", "\1"):gsub("|c%x%x%x%x%x%x%x%x", ""):gsub("|r", ""):gsub("\1", "|"))
    copyEdit:SetWidth(messageWidth() - 8)
    copyEdit:SetText(copyBox.text)
    copyBox:Show()
    copyScroll:SetVerticalScroll(0)
    copyEdit:SetFocus()
    copyEdit:HighlightText()
end

local function sendCurrent()
    local text = input:GetText():gsub("^%s+", ""):gsub("%s+$", "")
    -- /clear on its own is T3's: it arms a fresh session for the next message, sending
    -- nothing (never an answer to a question).
    if text == "/clear" and Paste.CanFresh() then
        input:SetText("")
        Paste.Arm(true)
        return
    end
    -- While the agent waits on a question, Send (and Enter) answers it.
    if pendingAsk() then
        if sendAnswer(text) and text ~= "" then input:SetText("") end
        UI.Refresh()
        return
    end
    -- Pasted images can go on their own.
    local images = #Paste.For(Paste.DraftKey())
    if (text == "" and images == 0) or not selected then return end
    local fresh = Paste.Fresh()
    if fresh and images > 0 then
        UI.Notice("A fresh session can't start with pasted images: send words first, or × the notice to keep this session.")
        return
    end
    local blocked = blockedReason(selected.new and "new" or "send")
    if blocked then
        UI.Notice(blocked)
        return
    end
    -- What was picked may have gone since (a machine unlinked, a project removed): then
    -- nothing is sent and the text stays.
    if not envById(selected.env) then
        local noMachines = #(inbox and inbox.envs or {}) == 0
        UI.Notice(not selected.new and "That thread's machine isn't linked any more."
            or noMachines and "No machine to start it on: link one in T3."
            or "That machine isn't linked any more: pick another with Machine.")
        return
    end
    if selected.new and selected.project and not selectedProject() then
        UI.Notice("That project is gone: pick another with Project.")
        return
    end
    local command = selected.new
        and { t = "new", e = selected.env, p = selected.project, w = (selected.project and selected.worktree) or nil, m = UI.Picker.Expand(text),
            pv = selected.pv, md = selected.m, ef = selected.ef, rm = selected.rm }
        or { t = "send", e = selected.env, id = selected.id, m = UI.Picker.Expand(text), fc = fresh or nil }
    if not ns.Link.Fits(command) then
        UI.Notice("That message is too long to send from the game: shorten it.")
        return
    end
    local s = ns.Link.Send(command)
    if s then awaiting[s] = { kind = command.t, text = text, at = GetTime(), fresh = fresh and Paste.DraftKey() or nil } end
    -- Spent on this message; it comes back armed if T3 refuses it.
    if fresh then Paste.fresh[Paste.DraftKey()] = nil end
    if selected.new then
        UI.Notice("Starting a new thread…")
    elseif text ~= "" then
        echo = { id = selected.id, text = text, s = s, images = images }
    end
    input:SetText("")
    UI.Refresh()
end

UI.SendCurrent = sendCurrent

-- ---------------------------------------------------------------- the / $ and @ picker
-- As in T3's composer: / at the start of a word lists the provider's commands and skills,
-- $ lists its skills, and @ lists your other machines and the machine's models. What you pick goes into the
-- box as text, the way T3's own composer writes it, and T3 finds it there. MCP doesn't say what
-- a provider offers, so inbox.picks comes from T3's cache on this PC ({ e = machine, pv =
-- provider, c = { { n = name, d = description, c = runs as /name, s = runs as $name } } });
-- other machines have no list. Each update costs one of the session's 200, so the list comes
-- once and is filtered here. It follows the end of the text only: where the game counts the
-- cursor in bytes or in letters doesn't matter that way, and a pick needs no cursor math.
do
    local Picker = UI.Picker
    local MAX_ROWS, ROW_HEIGHT, PAD = 8, 22, 4
    local list = CreateFrame("Frame", "T3WoWPicker", frame)
    list:SetFrameStrata("FULLSCREEN_DIALOG")
    list:SetPoint("BOTTOMLEFT", inputBg, "TOPLEFT", 0, 4)
    list:SetPoint("BOTTOMRIGHT", inputBg, "TOPRIGHT", 0, 4)
    list:EnableMouse(true)
    list:Hide()
    Skin.Box(list, { shape = "control", fill = "card", ring = "border", ringLayer = "BORDER" })

    local rows, shown, active, tokenBytes, owner = {}, {}, 1, 0, nil

    local function paint()
        for index, item in ipairs(shown) do
            local r = rows[index]
            r.label:SetText(item.text)
            r.note:SetText(item.note or "")
            if index == active then r.lit:Show() else r.lit:Hide() end
            r:Show()
        end
        for index = #shown + 1, #rows do rows[index]:Hide() end
        list:SetHeight(#shown * ROW_HEIGHT + 2 * PAD)
    end

    local function row(index)
        if rows[index] then return rows[index] end
        local r = CreateFrame("Button", nil, list)
        r:SetHeight(ROW_HEIGHT)
        r:SetPoint("TOPLEFT", PAD, -PAD - (index - 1) * ROW_HEIGHT)
        r:SetPoint("TOPRIGHT", -PAD, -PAD - (index - 1) * ROW_HEIGHT)
        r.lit = Skin.Box(r, { shape = "control", fill = "hover", layer = "BORDER", sub = 0 })
        r.lit:Hide()
        r.label = r:CreateFontString(nil, "OVERLAY")
        r.label:SetFontObject(Skin.fonts.text)
        r.label:SetPoint("LEFT", 8, 0)
        r.label:SetWordWrap(false)
        r.note = r:CreateFontString(nil, "OVERLAY")
        r.note:SetFontObject(Skin.fonts.small)
        r.note:SetPoint("LEFT", r.label, "RIGHT", 10, 0)
        r.note:SetPoint("RIGHT", -8, 0)
        r.note:SetJustifyH("LEFT")
        r.note:SetWordWrap(false)
        r:SetScript("OnEnter", function()
            active = index
            paint()
        end)
        r:SetScript("OnClick", function() Picker.Accept(index) end)
        rows[index] = r
        return r
    end

    -- Your other machines. Only from a thread on this T3's own machine: a linked machine's
    -- agent may have no link back to the others.
    local function machines()
        local found, own = {}, inbox and inbox.envs and inbox.envs[1]
        if not selected or not own or selected.env ~= own.id then return found end
        for _, e in ipairs(inbox.envs) do
            if e ~= own and e.ok and e.id ~= "" then found[#found + 1] = e end
        end
        return found
    end

    -- Further into a message a skill is written as $name, which T3 runs from anywhere.
    local function candidates(mark, leading)
        local found, seen = {}, {}
        if mark == "@" then
            for _, e in ipairs(machines()) do
                found[#found + 1] = { key = e.name, text = "@" .. e.name, note = "Machine" }
            end
            return found
        end
        local index, provider = envIndexById(selected.env), Selectors.Provider()
        -- T3's /clear stands in for the provider's, which would clear a session T3 still
        -- treats as live. Only at the start, as a command; not in New Thread, which starts
        -- fresh anyway.
        local machine = envById(selected.env)
        local fresh = machine ~= nil and machine.fc == true
        if Paste.CanFresh() and mark == "/" and leading then
            found[#found + 1] = { key = "clear", text = "/clear", note = "Start a fresh session; earlier messages stay visible", fresh = true }
        end
        for _, p in ipairs(inbox and type(inbox.picks) == "table" and inbox.picks or {}) do
            if type(p) == "table" and p.e == index and (not provider or p.pv == provider) then
                for _, c in ipairs(type(p.c) == "table" and p.c or {}) do
                    if not seen[c.n] and (mark == "/" or c.s) and not (fresh and c.n == "clear" and not c.s) then
                        seen[c.n] = true
                        local slash = mark == "/" and c.c and (leading or not c.s) and not (fresh and c.n == "clear")
                        found[#found + 1] = { key = c.n, text = (slash and "/" or "$") .. c.n, note = c.d }
                    end
                end
            end
        end
        return found
    end

    -- What's being typed at the end of the text, if it asks for the list.
    local function parse(text)
        local token = text:match("(%S*)$")
        local mark, query = token:sub(1, 1), token:sub(2)
        -- A / splits the words of a model query (@claude/opus), as in T3.
        if not query:find(mark == "@" and "^[%w:/_%.%-]*$" or "^[%w:_%.%-]*$") then return nil end
        if mark == "$" or mark == "@" or mark == "/" then return mark, query, token, text:find("^%s*%S*$") ~= nil end
    end

    -- Starting with what you typed first, then containing it; alphabetical within each.
    local function matching(items, query)
        local q, found = query:lower(), {}
        for _, item in ipairs(items) do
            local at = item.key:lower():find(q, 1, true)
            if at then
                item.rank = at == 1 and 1 or 2
                found[#found + 1] = item
            end
        end
        table.sort(found, function(a, b)
            if a.rank ~= b.rank then return a.rank < b.rank end
            return a.key < b.key
        end)
        for index = #found, MAX_ROWS + 1, -1 do found[index] = nil end
        return found
    end

    -- T3's score for one word against one field (searchRanking's scoreQueryMatch with the
    -- model picker's bases, position ignored): lower is better, nil for no match.
    local function fieldScore(field, word, base)
        if field == "" then return nil end
        if field == word then return base end
        if field:sub(1, #word) == word then return base + 2 end
        for _, marker in ipairs({ " ", "-", "_", "/" }) do
            if field:find(marker .. word, 1, true) then return base + 4 end
        end
        if field:find(word, 1, true) then return base + 6 end
        if #word < 3 then return nil end
        -- In order, with gaps: three for each letter skipped between, one for the span.
        local at, first, previous, gaps = 1, nil, nil, 0
        for i = 1, #field do
            if field:sub(i, i) == word:sub(at, at) then
                first = first or i
                if previous then gaps = gaps + i - previous - 1 end
                previous, at = i, at + 1
                if at > #word then return base + 100 + gaps * 3 + (i - first + 1 - #word) end
            end
        end
    end

    -- Whether word starts a word of text: at its start, or after something not a letter or digit.
    local function startsWord(text, word)
        local from = 1
        while true do
            local at = text:find(word, from, true)
            if not at then return false end
            local before = at > 1 and text:byte(at - 1)
            if not before or (before < 128 and not string.char(before):find("%w")) then return true end
            from = at + 1
        end
    end

    -- A reference T3 would read back: an instance id as T3 makes them, a model id without
    -- spaces, quotes, commas or the game's escape character.
    local function referable(pv, id)
        return type(pv) == "string" and pv:find("^%a[%w_%-]*$") ~= nil
            and type(id) == "string" and id ~= "" and not id:find("[%s,\"'|]")
    end

    -- The models of a machine's providers (inbox.models) an @ query asks for, best first, at
    -- most MAX_ROWS: { pv, id, name, provider }. Pure, for the tests.
    function Picker.Models(providers, query)
        local trim = function(s) return (s:gsub("^%s+", ""):gsub("%s+$", "")) end
        local spaced = trim(query:gsub("[:/]+", " "))
        local unhyphenated = trim(spaced:gsub("%-+", " "))
        if #unhyphenated:gsub("%s+", "") < 2 then return {} end
        local candidates = {}
        for _, p in ipairs(providers or {}) do
            for _, model in ipairs(type(p) == "table" and type(p.m) == "table" and p.m or {}) do
                if referable(p.id, model.id) then
                    local name = tostring(model.t or model.id):gsub("||", "|")
                    candidates[#candidates + 1] = {
                        pv = p.id, id = model.id, name = tostring(model.t or model.id), provider = tostring(p.t or p.id),
                        fields = { name:lower(), tostring(p.dk or p.id):lower(), tostring(p.t or p.id):gsub("||", "|"):lower() },
                    }
                end
            end
        end
        for _, words in ipairs(unhyphenated == spaced and { spaced } or { spaced, unhyphenated }) do
            local list = {}
            for w in words:lower():gmatch("%S+") do list[#list + 1] = w end
            local ranked = {}
            for order, c in ipairs(candidates) do
                local haystack = table.concat(c.fields, " ")
                local fields = { c.fields[1], c.fields[2], c.fields[3], haystack }
                local literal = true
                for _, w in ipairs(list) do literal = literal and startsWord(haystack, w) end
                if literal then
                    local score = 0
                    if table.concat(list, " ") == c.fields[1]:gsub("%s+", " ") then
                        score = -1000
                    else
                        for _, w in ipairs(list) do
                            local best
                            for index, field in ipairs(fields) do
                                local s = fieldScore(field, w, (index - 1) * 10)
                                if s and (not best or s < best) then best = s end
                            end
                            score = score + best
                        end
                    end
                    c.score, c.order = score, order
                    ranked[#ranked + 1] = c
                end
            end
            if #ranked > 0 then
                table.sort(ranked, function(a, b)
                    if a.score ~= b.score then return a.score < b.score end
                    return a.order < b.order
                end)
                for index = #ranked, MAX_ROWS + 1, -1 do ranked[index] = nil end
                return ranked
            end
        end
        return {}
    end

    -- The thread's machine's models for an @ query, best first, as T3 0.18 ranks them
    -- (composer-model-mentions): every word typed starts a word of the model's or provider's
    -- name, and two letters at least. A pick writes model:<instance>/<model>, which names a
    -- model for the agent and switches nothing.
    local function models(query)
        local list = Picker.Models(Selectors.ProvidersOn(selected.env), query)
        for _, m in ipairs(list) do
            m.text, m.insert, m.note = m.name, "model:" .. m.pv .. "/" .. m.id, m.provider
        end
        return list
    end

    -- Says whether it was open.
    function Picker.Close()
        local was = list:IsShown()
        list:Hide()
        return was
    end

    -- Only for what you type: the box filled back in after a failed send opens nothing.
    function Picker.Update(userInput)
        local text = input:GetText()
        -- Off by one at most is still the end: the game may not have moved the cursor yet.
        local away = input:GetCursorPosition() < (strlenutf8 or string.len)(text) - 1
        if not userInput or not selected or pendingAsk() or away then return Picker.Close() end
        local mark, query, token, leading = parse(text)
        local found = mark and matching(candidates(mark, leading), query) or {}
        if mark == "@" then
            -- Machines first, then models, as in T3.
            for _, m in ipairs(models(query)) do
                if #found >= MAX_ROWS then break end
                found[#found + 1] = m
            end
        end
        if #found == 0 then return Picker.Close() end
        shown, active, tokenBytes, owner = found, 1, #token, Paste.DraftKey()
        for index = 1, #shown do row(index) end
        paint()
        list:Show()
    end

    -- Moving to another thread leaves the list behind.
    function Picker.Sync()
        if owner ~= Paste.DraftKey() then Picker.Close() end
    end

    function Picker.Move(step)
        if not list:IsShown() then return end
        active = (active - 1 + step) % #shown + 1
        paint()
    end

    -- Puts a row's text in place of what was being typed. False when there's nothing to pick.
    function Picker.Accept(index)
        local item = list:IsShown() and shown[index or active]
        if not item then return false end
        local text = input:GetText()
        Picker.Close()
        -- /clear is done once picked: it arms the next message, and leaves no text.
        if item.fresh then
            input:SetText(text:sub(1, #text - tokenBytes))
            Paste.Arm(true)
            input:SetFocus()
            return true
        end
        -- Insert leaves the cursor after it; a SetText might not.
        input:SetText("")
        input:Insert(text:sub(1, #text - tokenBytes) .. (item.insert or item.text) .. " ")
        input:SetFocus()
        return true
    end

    -- An @machine goes out as T3 stores one: a link it reads for the chip, the machine's id in it.
    function Picker.Expand(text)
        local others = machines()
        -- The longest first: "@AIQ Mac" isn't "@AIQ" followed by a word.
        table.sort(others, function(a, b) return #a.name > #b.name end)
        for _, e in ipairs(others) do
            local token, from = "@" .. e.name, 1
            local label = e.name:gsub("%]", "")
            local reference = format("[%s](t3-context://v1/environment/environment_%s)", label, e.id)
            while true do
                local first, last = text:find(token, from, true)
                if not first then break end
                local before, after = text:sub(first - 1, first - 1), text:sub(last + 1, last + 1)
                if (before == "" or before:find("%s")) and (after == "" or after:find("%s")) then
                    text = text:sub(1, first - 1) .. reference .. text:sub(last + 1)
                    from = first + #reference
                else
                    from = last + 1
                end
            end
        end
        return text
    end

    input:SetScript("OnTabPressed", function() Picker.Accept() end)
    -- The arrow keys, seen as key presses: a box of several lines keeps them for its cursor
    -- and the game never fires OnArrowPressed on it.
    function Picker.Key(box, key)
        if not list:IsShown() then return end
        if key == "UP" or key == "DOWN" then
            Picker.Move(key == "DOWN" and 1 or -1)
            -- The cursor moved too: back to the end, where the list works.
            C_Timer.After(0, function() box:SetCursorPosition(#box:GetText()) end)
        elseif key == "LEFT" or key == "RIGHT" then
            Picker.Close()
        end
    end
end

input:SetScript("OnEnterPressed", function(self)
    if IsShiftKeyDown() then
        self:Insert("\n")
    elseif not UI.Picker.Accept() then
        sendCurrent()
    end
end)

-- ---------------------------------------------------------------- buttons and status

-- The footer starts with Settings (made with its panel, below) and then Refresh.
local refresh = UI.IconButton(frame, "refresh", "Refresh", "Ref", function() ns.Link.Send({ t = "refresh" }) end)
refresh:SetPoint("BOTTOMLEFT", 44, 10)
-- As in T3, beside the search box.
local newThread = UI.IconButton(frame, "compose", "New Thread", "New", function()
    local first = inbox and inbox.envs and inbox.envs[1]
    selected = { new = true, env = first and first.id or "" }
    UI.History.pending = nil
    ns.Link.Send({ t = "open" })
    UI.Refresh()
    input:SetFocus()
end)
newThread:SetSize(26, 26)
newThread:SetPoint("LEFT", search, "RIGHT", 4, 0)
local reload = button(frame, "Reload UI", 100, function() ReloadUI() end)
reload:SetPoint("LEFT", refresh, "RIGHT", 6, 0)

-- Send and Stop share one place in the message box, next to the model, as in T3.
local send = button(inputBg, "Send", 64, sendCurrent, "primary")
send:SetHeight(24)
send:SetPoint("BOTTOMRIGHT", inputBg, "BOTTOMRIGHT", -8, 6)
local stop = button(inputBg, "Stop", 76, function()
    if not selected or selected.new then return end
    if blockedReason("stop") then return UI.Notice(blockedReason("stop")) end
    local s = ns.Link.Send({ t = "stop", e = selected.env, id = selected.id })
    stopping = { env = selected.env, id = selected.id, s = s, at = GetTime() }
    UI.Refresh()
end)
stop:SetHeight(24)
stop:SetPoint("BOTTOMRIGHT", inputBg, "BOTTOMRIGHT", -8, 6)
stop:Hide()
-- An agent's question is answered from the same place.
askSend:SetParent(inputBg)
askSend:SetHeight(24)
askSend:ClearAllPoints()
askSend:SetPoint("BOTTOMRIGHT", inputBg, "BOTTOMRIGHT", -8, 6)

-- A run shows Stop, until there's something to send: then it's Send, which steers or queues.
function UI.ComposerButton()
    local listed = selectedThread()
    local isStopping = stopping ~= nil and selected ~= nil and stopping.env == selected.env and stopping.id == selected.id
    local writing = input:GetText():find("%S") ~= nil or #Paste.For(Paste.DraftKey()) > 0
    local answering = pendingAsk() ~= nil
    local showStop = ((listed ~= nil and UI.Stoppable(listed.s)) or isStopping) and not writing and not answering
    stop:SetShown(showStop)
    send:SetShown(not showStop and not answering)
    Selectors.PlaceRow((answering and askSend) or (showStop and stop) or send)
end

local statusLine = frame:CreateFontString(nil, "OVERLAY")
statusLine:SetFontObject(Skin.fonts.small)
statusLine:SetPoint("LEFT", reload, "RIGHT", 12, 0)
-- Level with the footer's buttons (10 up, 28 tall).
statusLine:SetPoint("RIGHT", frame, "BOTTOMRIGHT", -22, 24)
statusLine:SetJustifyH("LEFT")
statusLine:SetWordWrap(false)

-- The companion names the addon version on disk (inbox.av; older companions leave it out).
-- WoW reads addon files only when it starts, so a newer one there waits for a full
-- restart of WoW: /reload isn't enough. The version this addon was loaded as is read now.
local addonName = ...
local getMetadata = (C_AddOns and C_AddOns.GetAddOnMetadata) or GetAddOnMetadata
local LOADED_VERSION = getMetadata and getMetadata(addonName, "Version")
local restartSaid = false -- said in chat this UI session

-- A test build's label ("0.9.1-dev+57ee2ec") doesn't count here: it is 0.9.1.
local function versionNumbers(v)
    if type(v) ~= "string" then return nil end
    local core = v:match("^(%d+[%d%.]*)$") or v:match("^(%d+[%d%.]*)[-+]")
    if not core then return nil end
    local numbers = {}
    for n in core:gmatch("%d+") do numbers[#numbers + 1] = tonumber(n) end
    return numbers
end

-- What to say when the addon on disk is newer than this one, or another build of the same
-- version (the companion put its own build there), nil otherwise.
local function restartNote()
    local disk, loaded = versionNumbers(inbox and inbox.av), versionNumbers(LOADED_VERSION)
    if not disk or not loaded then return nil end
    local note = format("T3 WoW %s is installed: restart WoW to load it (/reload isn't enough)", inbox.av)
    for i = 1, math.max(#disk, #loaded) do
        local a, b = disk[i] or 0, loaded[i] or 0
        if a ~= b then
            return a > b and note or nil
        end
    end
    local labelled = inbox.av:find("[-+]") or LOADED_VERSION:find("[-+]")
    return labelled and inbox.av ~= LOADED_VERSION and note or nil
end

local function renderStatus()
    local link = ns.Link.Status()
    local connected
    if ns.Link.Exhausted() then
        connected = "|cffff8000All 200 updates of this session used: Reload UI|r"
    elseif link.loadError then
        connected = "|cffff4040Slot addons blocked (" .. tostring(link.loadError) .. "): enable T3 WoW slots in AddOns|r"
    elseif ns.Link.Stalled() then
        connected = "|cffff4040Companion not answering: is T3 WoW running in the system tray?|r"
    elseif not inbox then
        connected = "|cffa0a0a0Waiting for the companion…|r"
    elseif inbox.auth then
        connected = "|cffff4040" .. inbox.auth .. "|r"
    elseif restartNote() then
        connected = "|cffff8000" .. restartNote() .. "|r"
    elseif ns.Link.SlotsLow() then
        connected = format("|cffffd100%d/%d updates used: Reload UI soon|r", ns.Link.SlotsUsed())
    elseif inbox.trust and TRUST_NAME[inbox.trust] then
        connected = format("|cff40c040Connected|r |cffa0a0a0(%s: see Commands from the game in the T3 WoW tray menu)|r", TRUST_NAME[inbox.trust])
    else
        connected = "|cff40c040Connected|r"
    end
    connection:SetWidth(0)
    connection:SetText(connected)
    -- The pill fits the text, up to the room the title bar has; the dot takes its color.
    local room = math.max(120, frame:GetWidth() - (title:GetStringWidth() or 60) - 170)
    local width = math.min(room, math.ceil(connection:GetStringWidth() or 0) + 1)
    connection:SetWidth(width)
    connectionPill.proxy:SetWidth(width + 30)
    local dot = connected:match("^|cff(%x%x%x%x%x%x)")
    connectionDot:SetColors(dot and { hexColor(dot) } or "dim")
    local showReload = (ns.Link.Exhausted() or ns.Link.SlotsLow() or link.loadError ~= nil) and true or false
    reload:SetShown(showReload)
    statusLine:SetPoint("LEFT", showReload and reload or refresh, "RIGHT", 8, 0)
    connectionPill.hit:SetShown(connected:find("Reload UI", 1, true) ~= nil)
    local line = notice
    if not line and inbox and inbox.notes and #inbox.notes > 0 then
        local last = inbox.notes[#inbox.notes]
        if time() - last.at < 30 then line = last.x end
    end
    if link.queue > 0 then line = "Sending…" end
    statusLine:SetText(line or "")
    local listed = selectedThread()
    local isStopping = stopping ~= nil and selected ~= nil and stopping.env == selected.env and stopping.id == selected.id
    stop:SetText(isStopping and "Stopping…" or "Stop")
    stop:SetEnabled(not isStopping and listed ~= nil and UI.Stoppable(listed.s) and not blockedReason("stop"))
    UI.ComposerButton()
    send:SetText((selected and selected.new and "Start") or "Send")
    send:SetEnabled(selected ~= nil and not blockedReason((selected.new and "new") or "send"))
    setPlaceholder()
end

-- ---------------------------------------------------------------- settings

local DEFAULTS = {
    toasts = true, chat = true, sound = true, duration = 8, maxToasts = 4, approvalOpensT3 = false,
    onCompleted = true, onWaiting = true, onFailed = true, onStopped = false,
}
local EVENT_SETTING = {
    completed = "onCompleted", idle = "onCompleted", rolled_back = "onCompleted",
    waiting = "onWaiting", approval = "onWaiting", card = "onWaiting", failed = "onFailed", interrupted = "onStopped", cancelled = "onStopped",
}
local DURATIONS = { 5, 8, 15, 30, 0 } -- 0: until clicked

local function settings()
    T3WoWDB = T3WoWDB or {}
    local s = T3WoWDB.settings or {}
    T3WoWDB.settings = s
    for key, value in pairs(DEFAULTS) do
        if s[key] == nil then s[key] = value end
    end
    return s
end

local selectThread -- defined with the API below

-- The threads opened since the UI loaded, which the mouse's side buttons step through as
-- they do in T3. Works like a browser's history, except that New Thread isn't a stop.
-- On UI, not a local of its own: the file has all of Lua's 200 locals in one chunk.
UI.History = { list = {}, at = 0 }

function UI.History.Visit(envId, threadId)
    UI.History.pending = nil -- a thread opened any other way wins over presses still waiting
    local here = UI.History.list[UI.History.at]
    if here and here.env == envId and here.id == threadId then return end
    for i = #UI.History.list, UI.History.at + 1, -1 do UI.History.list[i] = nil end
    UI.History.at = UI.History.at + 1
    UI.History.list[UI.History.at] = { env = envId, id = threadId }
end

-- Quick presses add up: only the thread they end on is opened, once they stop, so the
-- companion isn't asked for each one on the way.
function UI.History.Step(step)
    -- From the composer, Back is the thread you left for it, which is still the current stop.
    local pending = UI.History.pending
    local leaving = step < 0 and composing() and not pending
    local to = (pending and pending.to or UI.History.at) + (leaving and 0 or step)
    if not UI.History.list[to] then return end
    UI.History.pending = { to = to, due = GetTime() + 0.25 }
end

-- Every frame: opens the thread the presses ended on, once they've stopped.
function UI.History.Land()
    local pending = UI.History.pending
    if not pending or GetTime() < pending.due then return end
    UI.History.at, UI.History.pending = pending.to, nil
    local stop = UI.History.list[pending.to]
    if selected and not selected.new and selected.env == stop.env and selected.id == stop.id then return end
    selectThread(stop.env, stop.id) -- its Visit finds this is already the current stop
end

function UI.OpenThread(threadId, envId)
    frame:Show()
    selectThread(envId or "", threadId)
end

-- ---------------------------------------------------------------- notifications

local TOAST_WIDTH, TOAST_HEIGHT, TOAST_GAP, TOAST_LIMIT = 320, 60, 6, 10
-- A notification's icon: a circle in the status color with a check, a cross or a "?".
local STATUS_ICON = {
    completed = "check", idle = "check", rolled_back = "check", waiting = "?", approval = "?", card = "?",
    failed = "close", interrupted = "close", cancelled = "close",
}

-- Notifications stack down from this anchor; "Move notifications" in settings shows it.
-- It has a notification's exact size and place, so where you drop it is where the first
-- notification appears.
local toastAnchor = CreateFrame("Frame", "T3WoWToastAnchor", UIParent)
toastAnchor:SetSize(TOAST_WIDTH, TOAST_HEIGHT)
toastAnchor:SetFrameStrata("DIALOG")
toastAnchor:SetClampedToScreen(true)
toastAnchor:SetMovable(true)
toastAnchor:RegisterForDrag("LeftButton")
Skin.Box(toastAnchor, { shape = "panel", fill = "card", ring = "accent" })
toastAnchor:SetAlpha(0)
toastAnchor:EnableMouse(false)
local anchorTitle = toastAnchor:CreateFontString(nil, "OVERLAY")
anchorTitle:SetFontObject(Skin.fonts.strong)
anchorTitle:SetPoint("CENTER", 0, 8)
anchorTitle:SetText("T3 WoW notifications appear here")
local anchorHint = toastAnchor:CreateFontString(nil, "OVERLAY")
anchorHint:SetFontObject(Skin.fonts.small)
anchorHint:SetPoint("CENTER", 0, -9)
anchorHint:SetText("Drag to move, then click Done moving")
toastAnchor:SetScript("OnDragStart", toastAnchor.StartMoving)
toastAnchor:SetScript("OnDragStop", function(self)
    self:StopMovingOrSizing()
    local point, _, relativePoint, x, y = self:GetPoint()
    settings().toastPoint = { point, relativePoint, x, y }
end)

local function placeAnchor()
    local p = settings().toastPoint
    toastAnchor:ClearAllPoints()
    if p then
        toastAnchor:SetPoint(p[1], UIParent, p[2], p[3], p[4])
    else
        toastAnchor:SetPoint("TOP", UIParent, "TOP", 0, -150)
    end
end

local toasts, toastPool = {}, {}

-- The waits you've been told about, kept across /reload (machine id by "machine:thread").
-- A notification counts once it ran its time or was clicked: one a reload wiped, one
-- pushed off by newer ones or one never shown doesn't.
-- On UI, not a local of its own: the file has all of Lua's 200 locals in one chunk.
UI.Waits = { caughtUp = {} }
function UI.Waits.Seen()
    T3WoWDB = T3WoWDB or {}
    T3WoWDB.seenWaiting = T3WoWDB.seenWaiting or {}
    return T3WoWDB.seenWaiting
end
function UI.Waits.Key(envId, threadId) return tostring(envId) .. ":" .. tostring(threadId) end
function UI.Waits.NeedsYou(status) return EVENT_SETTING[status] == "onWaiting" end
function UI.Waits.MarkSeen(toast)
    if toast.waitKey then UI.Waits.Seen()[toast.waitKey] = toast.envId end
end
function UI.Waits.Reading(envId, threadId)
    return frame:IsShown() and selected and selected.id == threadId and selected.env == envId
end

local function layoutToasts()
    for index, toast in ipairs(toasts) do
        toast:ClearAllPoints()
        toast:SetPoint("TOP", toastAnchor, "TOP", 0, -(index - 1) * (TOAST_HEIGHT + TOAST_GAP))
    end
end

local function removeToast(toast)
    for index, other in ipairs(toasts) do
        if other == toast then
            table.remove(toasts, index)
            break
        end
    end
    toast:SetScript("OnUpdate", nil)
    toast:Hide()
    toastPool[#toastPool + 1] = toast
    layoutToasts()
end

local function trimToasts()
    local limit = math.max(1, math.min(TOAST_LIMIT, tonumber(settings().maxToasts) or 4))
    while #toasts > limit do removeToast(toasts[#toasts]) end
end

local function newToast()
    local toast = table.remove(toastPool)
    if toast then return toast end
    toast = CreateFrame("Button", nil, UIParent)
    toast:SetSize(TOAST_WIDTH, TOAST_HEIGHT)
    toast:SetFrameStrata("DIALOG")
    toast.box = Skin.Box(toast, { shape = "panel", fill = "card", ring = "border" })
    toast.circle = Skin.Box(toast, { shape = "pill", fill = "raised", height = 28, free = true, layer = "ARTWORK" })
    toast.circle:SetPoint("LEFT", toast, "LEFT", 14, 0)
    toast.circle.proxy:SetSize(28, 28)
    toast.icon = Skin.Glyph(toast, "check", 18, "text", "OVERLAY")
    toast.icon:SetPoint("CENTER", toast.circle.proxy, "CENTER")
    toast.mark = toast:CreateFontString(nil, "OVERLAY")
    toast.mark:SetFontObject(Skin.fonts.title)
    toast.mark:SetPoint("CENTER", toast.circle.proxy, "CENTER", 0, 0)
    toast.mark:SetText("?")
    toast.head = toast:CreateFontString(nil, "OVERLAY")
    toast.head:SetFontObject(Skin.fonts.small)
    toast.head:SetPoint("TOPLEFT", toast.circle.proxy, "TOPRIGHT", 12, 3)
    toast.head:SetPoint("RIGHT", -12, 0)
    toast.head:SetJustifyH("LEFT")
    toast.title = toast:CreateFontString(nil, "OVERLAY")
    toast.title:SetFontObject(Skin.fonts.strong)
    toast.title:SetPoint("TOPLEFT", toast.head, "BOTTOMLEFT", 0, -3)
    toast.title:SetPoint("RIGHT", -12, 0)
    toast.title:SetJustifyH("LEFT")
    toast.title:SetWordWrap(false)
    toast.sub = toast:CreateFontString(nil, "OVERLAY")
    toast.sub:SetFontObject(Skin.fonts.small)
    toast.sub:SetPoint("TOPLEFT", toast.title, "BOTTOMLEFT", 0, -3)
    toast.sub:SetPoint("RIGHT", -12, 0)
    toast.sub:SetJustifyH("LEFT")
    toast:EnableMouse(true)
    toast:RegisterForClicks("LeftButtonUp", "RightButtonUp")
    toast:SetScript("OnClick", function(self, mouse)
        -- Left-click opens the thread in the T3 WoW window (an approval or a card, with the
        -- setting on: in T3, the only place it can be answered); right-click only dismisses.
        local threadId, envId, inT3 = self.threadId, self.envId, self.inT3
        UI.Waits.MarkSeen(self)
        removeToast(self)
        if mouse == "LeftButton" and threadId then
            if inT3 then openInT3(threadId, envId) else UI.OpenThread(threadId, envId) end
        end
    end)
    toast:SetScript("OnEnter", function(self) self.hovered = true end)
    toast:SetScript("OnLeave", function(self) self.hovered = false end)
    return toast
end

local function toastUpdate(self)
    local now = GetTime()
    local shown = now - self.shownAt
    if shown < 0.2 then return self:SetAlpha(shown / 0.2) end
    if self.duration == 0 then return self:SetAlpha(1) end -- "until clicked" never fades
    if self.hovered then
        -- Hovering holds it on screen.
        self.shownAt = math.max(self.shownAt, now - self.duration)
        return self:SetAlpha(1)
    end
    local fading = shown - self.duration
    if fading <= 0 then return self:SetAlpha(1) end
    if fading >= 0.6 then
        UI.Waits.MarkSeen(self)
        return removeToast(self)
    end
    self:SetAlpha(1 - fading / 0.6)
end

local function showToast(event)
    placeAnchor()
    local s = STATUS[event.s] or { event.s or "Done", "ffffff" }
    local toast = newToast()
    local machineIndex = envIndexById(event.e)
    local machine = machineIndex and env(machineIndex)
    -- One notification per thread: a newer one replaces it.
    for _, other in ipairs(toasts) do
        if event.id and other.threadId == event.id and other.envId == event.e then
            removeToast(other)
            break
        end
    end
    toast.threadId, toast.envId, toast.inT3 = event.id, event.e, (event.s == "approval" or event.s == "card") and settings().approvalOpensT3 == true
    toast.waitKey = event.id and UI.Waits.NeedsYou(event.s) and UI.Waits.Key(event.e, event.id) or nil
    local icon = STATUS_ICON[event.s] or "check"
    local r, g, b = hexColor(s[2])
    toast.circle:SetColors({ r, g, b, 0.18 })
    toast.icon:SetTexture("Interface\\AddOns\\T3WoW\\Media\\" .. (icon == "?" and "check" or icon))
    Skin.SetGlyphColor(toast.icon, { r, g, b })
    toast.icon:SetShown(icon ~= "?")
    toast.mark:SetTextColor(r, g, b)
    toast.mark:SetShown(icon == "?")
    toast.head:SetText(format("T3 WoW · |cff%s%s|r", s[2], s.notice or s[1]))
    toast.title:SetText(event.t ~= "" and event.t or "Thread")
    toast.sub:SetText((machine and machine.name or "") .. (event.id and (toast.inT3 and "  ·  click to open in T3" or "  ·  click to open") or ""))
    -- Classic outlines a notification in its status color, as the game's own alerts do.
    toast.box:SetColors(nil, Skin.ThemeKey() == "classic" and { r, g, b, 1 } or "border")
    toast.duration = settings().duration
    toast.shownAt = GetTime()
    toast.hovered = false
    table.insert(toasts, 1, toast)
    -- At most the number set in Settings: the oldest goes first.
    trimToasts()
    layoutToasts()
    toast:SetAlpha(0)
    toast:SetScript("OnUpdate", toastUpdate)
    toast:Show()
end

-- T3 Code's own two sounds (Media\notification-*.mp3, see Media\Sounds.txt), so a
-- notification sounds like the app and not like a whisper: one for a thread that needs you,
-- one for the rest. The game only finds a file added while it ran after a restart; until
-- then the whisper sound stands in.
local SOUND = "Interface\\AddOns\\T3WoW\\Media\\notification-%s.mp3"
local function playSound(status)
    local which = EVENT_SETTING[status] == "onWaiting" and "input" or "completion"
    if PlaySoundFile and PlaySoundFile(format(SOUND, which), "SFX") then return end
    if SOUNDKIT and SOUNDKIT.TELL_MESSAGE then PlaySound(SOUNDKIT.TELL_MESSAGE) end
end

-- Returns what happened, for /t3 status.
function UI.Alert(event)
    local s = settings()
    -- Nothing to announce about the thread you're reading.
    if UI.Waits.Reading(event.e, event.id) then
        return "not shown: you were reading that thread"
    end
    if not s[EVENT_SETTING[event.s] or "onCompleted"] then return "not shown: turned off in Settings" end
    local status = STATUS[event.s] or { event.s, "ffffff" }
    if s.chat then
        print(format("|cff8f8fffT3|r %s: |cff%s%s|r", event.t ~= "" and event.t or "Thread", status[2], status.notice or status[1]))
    end
    if s.sound then playSound(event.s) end
    if s.toasts then showToast(event) end
    return s.toasts and "shown" or "chat line only (notifications off in Settings)"
end

-- ---------------------------------------------------------------- settings panel

local refreshThemeControls -- set with the settings panel's controls
local settingsPanel = CreateFrame("Frame", "T3WoWSettings", frame)
settingsPanel:SetSize(SIZE.settings, 560)
settingsPanel:SetPoint("TOPLEFT", frame, "TOPRIGHT", 6, 0)
Skin.Box(settingsPanel, { shape = "panel", fill = "bg", ring = "border", ringLayer = "BORDER", sub = -8 })
settingsPanel:EnableMouse(true)
settingsPanel:Hide()

do
    local settingsTitle = settingsPanel:CreateFontString(nil, "OVERLAY")
    settingsTitle:SetFontObject(Skin.fonts.title)
    settingsTitle:SetPoint("LEFT", settingsPanel, "TOPLEFT", 16, -SIZE.title / 2)
    settingsTitle:SetText("Settings")
    local settingsClose = Skin.CloseButton(settingsPanel, function() settingsPanel:Hide() end)
    settingsClose:SetPoint("TOPRIGHT", -8, -8)
    local settingsLine = Skin.Line(settingsPanel)
    settingsLine:SetPoint("TOPLEFT", 1, -SIZE.title)
    settingsLine:SetPoint("TOPRIGHT", -1, -SIZE.title)

    local checks = {}
    local layoutY = -SIZE.title - 12

    local function sectionLabel(text)
        local label = settingsPanel:CreateFontString(nil, "OVERLAY")
        label:SetFontObject(Skin.fonts.label)
        label:SetPoint("TOPLEFT", 16, layoutY)
        label:SetText(text)
        layoutY = layoutY - 22
    end

    -- get/set default to the settings table entry named key.
    local function checkbox(text, key, get, set)
        local check = Skin.Check(settingsPanel)
        check:SetPoint("TOPLEFT", 16, layoutY)
        local label = settingsPanel:CreateFontString(nil, "OVERLAY")
        label:SetFontObject(Skin.fonts.text)
        label:SetPoint("LEFT", check, "RIGHT", 8, 0)
        label:SetText(text)
        check.get = get or function() return settings()[key] end
        check.set = set or function(value) settings()[key] = value end
        check:SetScript("OnClick", function(self)
            self.set(self:GetChecked() and true or false)
            Skin.UpdateCheck(self)
        end)
        checks[#checks + 1] = check
        layoutY = layoutY - 26
        return check
    end

    -- A label on the left of a row, its control at SIZE.field.
    local function fieldLabel(text)
        local label = settingsPanel:CreateFontString(nil, "OVERLAY")
        label:SetFontObject(Skin.fonts.text)
        label:SetPoint("TOPLEFT", 16, layoutY - 7)
        label:SetText(text)
        return label
    end

    -- Greys a dropdown out (and back).
    local function setDropdownEnabled(dd, on)
        dd:SetEnabled(on)
        dd:SetAlpha(on and 1 or 0.5)
    end

    local function placeDropdown(dd)
        dd:SetPoint("TOPLEFT", settingsPanel, "TOPLEFT", SIZE.field, layoutY - 2)
    end

    local function durationName(d)
        return d == 0 and "Until clicked" or (d .. " seconds")
    end

    local refreshToastControls -- set below, once the controls exist

    -- The look: the theme, and how tightly the thread list is packed. Both apply at once.
    sectionLabel("Appearance")
    fieldLabel("Theme")
    local themeMenu = dropdown(settingsPanel, "T3WoWThemeMenu", 150, {
        entries = function()
            local list = {}
            for _, key in ipairs(Skin.ORDER) do
                local name = Skin.THEMES[key].name
                if key == "flat" and type(ElvUI) == "table" then name = name .. " (ElvUI colors)" end
                list[#list + 1] = { text = name, value = key }
            end
            return list
        end,
        isSelected = function(key) return Skin.ThemeKey() == key end,
        pick = function(key) Skin.Set(key) end,
    })
    placeDropdown(themeMenu)
    layoutY = layoutY - 34
    checkbox("Compact thread list", "compact", nil, function(value)
        settings().compact = value
        UI.Refresh()
    end)
    layoutY = layoutY - 8

    sectionLabel("Notifications")
    checkbox("Line in chat", "chat")
    checkbox("Sound", "sound")
    -- Last, right above the options it switches on and off.
    checkbox("On-screen notification", "toasts", nil, function(value)
        settings().toasts = value
        refreshToastControls()
    end)

    -- How long notifications stay: a dropdown.
    local durationLabel = fieldLabel("Stay on screen")
    local durationMenu = dropdown(settingsPanel, "T3WoWDurationMenu", 150, {
        entries = function()
            local list = {}
            for _, d in ipairs(DURATIONS) do list[#list + 1] = { text = durationName(d), value = d } end
            return list
        end,
        -- Asked while the addon loads too, before saved settings exist: read them without
        -- creating anything.
        isSelected = function(d)
            local saved = T3WoWDB and T3WoWDB.settings
            return saved ~= nil and saved.duration == d
        end,
        pick = function(d)
            settings().duration = d
            refreshToastControls()
        end,
    })
    placeDropdown(durationMenu)
    layoutY = layoutY - 34

    -- How many stay on screen at once: a slider, 1 to 10. The oldest makes room.
    local maxLabel = fieldLabel("At most")
    local maxSlider = CreateFrame("Slider", "T3WoWMaxToasts", settingsPanel)
    maxSlider:SetOrientation("HORIZONTAL")
    maxSlider:SetSize(70, 16)
    maxSlider:SetPoint("TOPLEFT", settingsPanel, "TOPLEFT", SIZE.field + 4, layoutY - 6)
    maxSlider:SetMinMaxValues(1, TOAST_LIMIT)
    maxSlider:SetValueStep(1)
    if maxSlider.SetObeyStepOnDrag then maxSlider:SetObeyStepOnDrag(true) end
    maxSlider:EnableMouseWheel(true)
    Skin.Slider(maxSlider)
    local maxValue = settingsPanel:CreateFontString(nil, "OVERLAY")
    maxValue:SetFontObject(Skin.fonts.small)
    maxValue:SetPoint("LEFT", maxSlider, "RIGHT", 10, 0)
    local function maxName(n) return n .. " at a time" end
    maxSlider:SetScript("OnValueChanged", function(self, value)
        local n = math.floor(value + 0.5)
        maxValue:SetText(maxName(n))
        Skin.UpdateSlider(self)
        if settings().maxToasts ~= n then
            settings().maxToasts = n
            trimToasts()
        end
    end)
    maxSlider:SetScript("OnMouseWheel", function(self, delta) self:SetValue(self:GetValue() + delta) end)
    layoutY = layoutY - 32

    local moveButton = button(settingsPanel, "Move notifications", (SIZE.settings - 38) / 2, nil)
    moveButton:SetPoint("TOPLEFT", 16, layoutY)
    local testButton = button(settingsPanel, "Test", (SIZE.settings - 38) / 2, function()
        -- Uses your most recent thread, so clicking the sample opens a real thread too.
        local latest = inbox and inbox.threads and inbox.threads[1]
        local latestEnv = latest and env(latest.e)
        showToast({
            s = "completed",
            t = latest and ("Test: " .. latest.t) or "Test notification",
            id = latest and latest.id,
            e = latestEnv and latestEnv.id,
        })
    end)
    testButton:SetPoint("LEFT", moveButton, "RIGHT", 6, 0)
    layoutY = layoutY - 42

    local function setAnchorUnlocked(unlocked)
        placeAnchor()
        toastAnchor:SetAlpha(unlocked and 1 or 0)
        toastAnchor:EnableMouse(unlocked)
        moveButton:SetText(unlocked and "Done moving" or "Move notifications")
    end
    moveButton:SetScript("OnClick", function() setAnchorUnlocked(toastAnchor:GetAlpha() == 0) end)

    -- The duration, moving and testing only matter for on-screen notifications.
    refreshToastControls = function()
        local on = settings().toasts and true or false
        durationMenu:SetLabel(durationName(settings().duration))
        maxSlider:SetValue(settings().maxToasts)
        maxValue:SetText(maxName(settings().maxToasts))
        Skin.UpdateSlider(maxSlider)
        maxSlider:EnableMouse(on)
        maxSlider:EnableMouseWheel(on)
        maxSlider:SetAlpha(on and 1 or 0.4)
        maxLabel:SetAlpha(on and 1 or 0.5)
        maxValue:SetAlpha(on and 1 or 0.5)
        durationLabel:SetAlpha(on and 1 or 0.5)
        setDropdownEnabled(durationMenu, on)
        if not on then setAnchorUnlocked(false) end
        moveButton:SetEnabled(on)
        testButton:SetEnabled(on)
    end

    sectionLabel("Notify when a thread")
    checkbox("Finishes", "onCompleted")
    checkbox("Needs you", "onWaiting")
    checkbox("Fails", "onFailed")
    checkbox("Is stopped", "onStopped")
    -- Off by default: a stray click mid-fight shouldn't pull you out of the game. Off, an
    -- approval's notification opens the thread here, and its Open in T3 button goes on.
    -- It goes for a card only T3 can answer (a secret, a machine link) too.
    checkbox("Clicking an approval opens T3", "approvalOpensT3")
    layoutY = layoutY - 8

    sectionLabel("Minimap")
    checkbox("Show minimap button", nil,
        function() return not (T3WoWDB and T3WoWDB.minimap and T3WoWDB.minimap.hide) end,
        function(value) if ns.Minimap and ns.Minimap.Show then ns.Minimap.Show(value) end end)
    settingsPanel:SetHeight(-layoutY + 10)

    refreshThemeControls = function()
        themeMenu:SetLabel(Skin.Theme().name)
    end

    settingsPanel:SetScript("OnShow", function()
        for _, check in ipairs(checks) do
            check:SetChecked(check.get() and true or false)
            Skin.UpdateCheck(check)
        end
        refreshThemeControls()
        refreshToastControls()
    end)
    settingsPanel:SetScript("OnHide", function() setAnchorUnlocked(false) end)

    local settingsButton = UI.IconButton(frame, "gear", "Settings", "Set", function() settingsPanel:SetShown(not settingsPanel:IsShown()) end)
    settingsButton:SetPoint("BOTTOMLEFT", 12, 10)
end

function UI.ShowSettings()
    frame:Show()
    settingsPanel:Show()
end

-- ---------------------------------------------------------------- API used by Link

function UI.Refresh()
    if not frame:IsShown() then return end
    renderList()
    renderView()
    Paste.Render()
    Selectors.Render()
    UI.Picker.Sync()
    renderStatus()
end

-- The first update of a UI session says in chat that the companion reached the game.
local announced = false
local function announce(data)
    if announced then return end
    announced = true
    local tag = "|cff8f8fffT3 WoW|r"
    if data.auth then
        print(tag .. " companion connected. |cffff4040" .. tostring(data.auth) .. "|r")
        return
    end
    local waiting = 0
    for _, t in ipairs(data.threads or {}) do
        if t.s == "waiting" and not t.set then waiting = waiting + 1 end
    end
    print(tag .. " connected to T3." .. (waiting > 0 and format(" |cffff8000%d %s you.|r", waiting, waiting == 1 and "thread needs" or "threads need") or ""))
end

-- Alerts come from a thread changing status, so a thread already waiting when the UI
-- session starts gets none. The first update that lists a machine's threads in this
-- session shows a notification, without a sound, for each wait you haven't been told
-- about. Only this session's updates: one left in a slot can be out of date.
function UI.Waits.CatchUp(data)
    if not ns.Link.IsCurrent(data) then return end
    local s = settings()
    local seen = UI.Waits.Seen()
    local threads = data.threads or {}
    local listed, waiting = {}, {}
    for _, t in ipairs(threads) do
        local envId = envIdOf(t.e)
        if envId then
            listed[envId] = true
            if UI.Waits.NeedsYou(t.s) then waiting[UI.Waits.Key(envId, t.id)] = true end
        end
    end
    -- A wait that's over is forgotten, so the thread's next one notifies.
    for key, envId in pairs(seen) do
        if listed[envId] and not waiting[key] then seen[key] = nil end
    end
    for _, toast in ipairs(toasts) do
        if listed[toast.envId] and not waiting[toast.waitKey] then toast.waitKey = nil end
    end
    if s.toasts and s.onWaiting then
        -- From the end of the list: the newest are shown last, so they stay if there are too many.
        for index = #threads, 1, -1 do
            local t = threads[index]
            local envId = envIdOf(t.e)
            if envId and not UI.Waits.caughtUp[envId] and UI.Waits.NeedsYou(t.s) and not t.set and not t.sn
                and not seen[UI.Waits.Key(envId, t.id)] and not UI.Waits.Reading(envId, t.id) then
                local onScreen = false
                for _, toast in ipairs(toasts) do
                    onScreen = onScreen or (toast.threadId == t.id and toast.envId == envId)
                end
                if not onScreen then showToast({ s = t.s, t = t.t or "", id = t.id, e = envId }) end
            end
        end
    end
    for envId in pairs(listed) do UI.Waits.caughtUp[envId] = true end
end

-- What the companion says about commands sent from here: a failed one gives its text
-- back (and drops its "(sending)" echo, or the "Sending…" on Send answer); a started
-- thread opens, if you are still in the New Thread composer.
local function takeOutcomes(data)
    local said
    -- Only this session's outcomes: an earlier session's reuse command numbers.
    local outcomes = ns.Link.IsCurrent(data) and type(data.out) == "table" and data.out or {}
    for _, o in ipairs(outcomes) do
        if type(o) == "table" and stopping and o.s == stopping.s then
            if o.ok and o.st == "stopping" then
                stopping.taken = true
            else
                stopping = nil
                said = o.ok and "Nothing to stop: the run had already ended." or format("|cffff4040Couldn't stop: %s|r", tostring(o.x or "failed"))
            end
        end
        local sent = type(o) == "table" and awaiting[o.s]
        if sent then
            awaiting[o.s] = nil
            if o.ok then
                if sent.kind == "new" and o.id and selected and selected.new then
                    selected = { env = o.env or selected.env, id = o.id }
                    UI.History.Visit(selected.env, selected.id)
                    scrollToEnd = true
                end
            else
                if sent.text and input:GetText() == "" then input:SetText(sent.text) end
                if sent.fresh then Paste.fresh[sent.fresh] = true end
                if echo and echo.s == o.s then echo = nil end
                if sent.kind == "answer" then askState.sent = nil end
                if sent.kind == "model" then Selectors.Refused(o.s) end
                local what = sent.kind == "new" and "Couldn't start the thread" or (sent.kind == "answer" and "Answer not sent")
                    or (sent.kind == "paste" and "Not pasted") or (sent.kind == "model" and "Model not changed") or "Not sent"
                said = format("|cffff4040%s: %s|r%s", what, tostring(o.x or "failed"), sent.text and " (your text is back in the box)" or "")
            end
        end
    end
    -- An older companion (no session, no outcomes) opens a thread it started without saying
    -- so: once it has acked the Start, a thread in its view is the new one.
    if data.u == nil and data.out == nil and ns.Link.IsMine(data) and selected and selected.new
        and type(data.view) == "table" and data.view.id then
        for s, sent in pairs(awaiting) do
            if sent.kind == "new" and type(data.ack) == "number" and data.ack >= s then
                awaiting[s] = nil
                selected = { env = envIdOf(data.view.e) or selected.env, id = data.view.id }
                UI.History.Visit(selected.env, selected.id)
                scrollToEnd = true
                break
            end
        end
    end
    -- An older companion never reports: don't keep them forever.
    for s, sent in pairs(awaiting) do
        if GetTime() - sent.at > 300 then awaiting[s] = nil end
    end
    return said
end

function UI.OnInbox(data)
    announce(data)
    inbox = data
    UI.TrackUnread(data)
    UI.TrackWoke(data)
    UI.Waits.CatchUp(data)
    if not restartSaid and restartNote() then
        restartSaid = true
        print("|cff8f8fffT3 WoW|r |cffff8000" .. restartNote() .. ".|r")
    end
    notice = takeOutcomes(data)
    -- The run ended (or the thread dropped off the list's working states): Stop is done.
    local t = stopping and listedThread(stopping.env, stopping.id)
    if t and not UI.Stoppable(t.s) then stopping = nil end
    UI.Refresh()
end

-- The companion moved to another T3 sign-in (see Link.ForgetLogin): the previous one's
-- threads, open thread (its Stopping… and a message being copied), question,
-- notifications and waiting outcomes go. What you typed stays in the box, and chat says
-- "connected to T3" again once the new one's update is in.
function UI.ForgetLogin()
    inbox, selected, echo, notice, stopping = nil, nil, nil, nil, nil
    UI.History.list, UI.History.at, UI.History.pending = {}, 0, nil
    copyBox:Hide()
    awaiting = {}
    Paste.fresh = {}
    askState.k, askState.picks, askState.sent, askState.sentAt = nil, {}, nil, nil
    Selectors.Forget()
    announced = false
    UI.Waits.caughtUp = {}
    T3WoWDB.seenWaiting = nil
    T3WoWDB.unread, T3WoWDB.lastStatus, T3WoWDB.snoozed, T3WoWDB.woke = nil, nil, nil, nil
    while #toasts > 0 do removeToast(toasts[#toasts]) end
    UI.Refresh()
end

function UI.Notice(text)
    notice = text
    UI.Refresh()
end

function UI.IsShown() return frame:IsShown() end

function selectThread(envId, threadId)
    selected = { env = envId, id = threadId }
    UI.History.Visit(envId, threadId)
    scrollToEnd = true
    copyBox:Hide()
    ns.Link.Send({ t = "open", e = envId, id = threadId })
    UI.Refresh()
end

-- A thread from the list (its machine is an index into inbox.envs).
function UI.Select(thread) selectThread(envIdOf(thread.e) or "", thread.id) end

-- ---------------------------------------------------------------- resizing

-- Text wraps to the conversation's width, so a new width means laying the messages out again.
local function applyWidth()
    local width = messageWidth()
    messageChild:SetWidth(width)
    for _, fs in pairs(messageStrings) do fs:SetWidth(width - 8) end
    input:SetWidth(width - 24)
    copyEdit:SetWidth(width - 8)
end

-- A new theme (or the saved one, at login) changes sizes and colors drawn per thread: lay
-- everything out again.
Skin.OnApply(function()
    applyWidth()
    refreshThemeControls()
    UI.Refresh()
end)

-- The window follows the cursor itself rather than through StartSizing: from a window
-- placed by its center, StartSizing jumped past the cursor on the first click. And laying
-- the conversation out again at every new size made dragging stutter, so the conversation
-- is hidden while you drag and laid out once, when you let go.

frame:SetScript("OnSizeChanged", function()
    List.FitDock()
    if Sizing.from then
        input:SetWidth(messageWidth() - 24)
        return
    end
    applyWidth()
    if frame:IsShown() then renderView() end
end)

local function cursorPosition()
    local scale = frame:GetEffectiveScale()
    local x, y = GetCursorPosition()
    return x / scale, y / scale
end

function Sizing.Stop()
    if not Sizing.from then return end
    Sizing.from = nil
    Sizing.grip:SetScript("OnUpdate", nil)
    saveLayout()
    applyWidth()
    if frame:IsShown() then UI.Refresh() end -- shows the conversation again
end

-- Every frame of the drag: the bottom-right corner goes where the cursor moved, within the
-- size bounds and the screen (so being kept on screen never shoves the window).
function Sizing.Follow()
    if not IsMouseButtonDown("LeftButton") then return Sizing.Stop() end
    local from = Sizing.from
    local x, y = cursorPosition()
    local maxWidth = math.max(MIN_WIDTH, math.min(MAX_WIDTH, UIParent:GetWidth() - frame:GetLeft()))
    local maxHeight = math.max(MIN_HEIGHT, math.min(MAX_HEIGHT, frame:GetTop()))
    local width = math.floor(math.min(maxWidth, math.max(MIN_WIDTH, from.w + x - from.x)) + 0.5)
    local height = math.floor(math.min(maxHeight, math.max(MIN_HEIGHT, from.h + from.y - y)) + 0.5)
    if width ~= from.lastW or height ~= from.lastH then
        from.lastW, from.lastH = width, height
        frame:SetSize(width, height)
    end
end

local grip = CreateFrame("Button", nil, frame)
Sizing.grip = grip
grip:SetSize(16, 16)
grip:SetPoint("BOTTOMRIGHT", -2, 2)
grip:SetFrameLevel(frame:GetFrameLevel() + 10)
grip.glyph = Skin.Glyph(grip, "grip", 16, "dim")
grip.glyph:SetAllPoints()
grip:SetScript("OnMouseDown", function(self, mouse)
    if mouse ~= "LeftButton" then return end
    -- Pin the top-left corner where it is, so only the bottom-right one moves.
    local left, top = frame:GetLeft(), frame:GetTop()
    frame:ClearAllPoints()
    frame:SetPoint("TOPLEFT", UIParent, "BOTTOMLEFT", left, top)
    local x, y = cursorPosition()
    Sizing.from = { x = x, y = y, w = frame:GetWidth(), h = frame:GetHeight() }
    messageScroll:Hide()
    self:SetScript("OnUpdate", Sizing.Follow)
end)
grip:SetScript("OnMouseUp", Sizing.Stop)
grip:SetScript("OnHide", Sizing.Stop)
grip:SetScript("OnEnter", function(self)
    Skin.SetGlyphColor(self.glyph, "text")
    Skin.Tooltip:SetOwner(self, "ANCHOR_RIGHT")
    Skin.Tooltip:AddLine("Drag to resize")
    Skin.Tooltip:AddLine("Right-click to reset the size")
    Skin.Tooltip:Show()
end)
grip:SetScript("OnLeave", function(self)
    Skin.SetGlyphColor(self.glyph, "dim")
    Skin.Tooltip:Hide()
end)
grip:RegisterForClicks("RightButtonUp")
grip:SetScript("OnClick", function()
    frame:SetSize(WIDTH, HEIGHT)
    saveLayout()
    UI.Refresh()
end)

frame:SetScript("OnShow", function()
    restoreLayout()
    ns.Link.Send({ t = "ui", o = true, f = input:HasFocus() and true or false })
    scrollToEnd = true -- reopening the window also lands on the latest message
    UI.Refresh()
end)
frame:SetScript("OnHide", function() ns.Link.Send({ t = "ui", o = false }) end)

-- The mouse's side buttons, with the pointer on the window. This is the game-wide click
-- event because the rows and the message box keep clicks from the window's OnMouseDown.
frame:RegisterEvent("GLOBAL_MOUSE_DOWN")
frame:SetScript("OnEvent", function(_, _, mouse)
    local step = (mouse == "Button4" and -1) or (mouse == "Button5" and 1)
    if not step or not frame:IsVisible() or not frame:IsMouseOver() then return end
    if T3WoWPictureZoom and T3WoWPictureZoom:IsShown() then return end -- it covers the window
    UI.History.Step(step)
end)

local ticker = 0
frame:SetScript("OnUpdate", function(_, delta)
    UI.History.Land()
    ticker = ticker + delta
    if ticker < 1 then return end
    ticker = 0
    UI.Work.Tick()
    -- No word on an answer for a while: let it be sent again.
    if askState.sent and GetTime() - (askState.sentAt or 0) >= ANSWER_TIMEOUT then
        askState.sent = nil
        renderView()
    end
    if stopping and GetTime() - stopping.at >= STOP_TIMEOUT then
        notice = stopping.taken and "Still running after Stop: press it again, or stop it in T3."
            or "No word back on Stop: press it again."
        stopping = nil
    end
    renderStatus()
end)

function UI.Toggle() frame:SetShown(not frame:IsShown()) end
