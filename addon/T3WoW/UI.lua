local _, ns = ...
local UI = {}
ns.UI = UI

local format, floor = string.format, math.floor
local WIDTH, HEIGHT, LIST_WIDTH = 940, 580, 300
local WORKING = { preparing = true, queued = true, starting = true, running = true }
local STATUS = {
    preparing = { "Working", "ffd100" }, queued = { "Queued", "ffd100" }, starting = { "Working", "ffd100" },
    running = { "Working", "ffd100" }, completed = { "Finished", "40c040" }, failed = { "Failed", "ff4040" },
    waiting = { "Needs you", "ff8000" }, interrupted = { "Stopped", "a0a0a0" }, cancelled = { "Stopped", "a0a0a0" },
    idle = { "Idle", "a0a0a0" }, rolled_back = { "Rolled back", "a0a0a0" },
}

local inbox
local selected -- { e = envIndex, id = threadId } or { new = true, e = envIndex, p = projectIndex }
local collapsed = { settled = true }
local echo -- { id, text } shown until the thread view catches up
local notice

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

-- ---------------------------------------------------------------- frame

local frame = CreateFrame("Frame", "T3WoWFrame", UIParent, "BackdropTemplate")
frame:SetSize(WIDTH, HEIGHT)
frame:SetPoint("CENTER")
frame:SetFrameStrata("HIGH")
frame:SetMovable(true)
frame:EnableMouse(true)
frame:RegisterForDrag("LeftButton")
frame:SetScript("OnDragStart", frame.StartMoving)
frame:SetScript("OnDragStop", frame.StopMovingOrSizing)
frame:SetClampedToScreen(true)
frame:SetBackdrop({
    bgFile = "Interface\\DialogFrame\\UI-DialogBox-Background-Dark",
    edgeFile = "Interface\\DialogFrame\\UI-DialogBox-Border",
    tile = true, tileSize = 32, edgeSize = 32,
    insets = { left = 11, right = 12, top = 12, bottom = 11 },
})
frame:Hide()
tinsert(UISpecialFrames, "T3WoWFrame")

local title = frame:CreateFontString(nil, "OVERLAY", "GameFontNormalLarge")
title:SetPoint("TOP", 0, -18)
title:SetText("T3 Code")

local close = CreateFrame("Button", nil, frame, "UIPanelCloseButton")
close:SetPoint("TOPRIGHT", -6, -6)

local connection = frame:CreateFontString(nil, "OVERLAY", "GameFontHighlightSmall")
connection:SetPoint("TOPLEFT", 20, -20)
connection:SetJustifyH("LEFT")

local function makeScroll(parent, width)
    local scroll = CreateFrame("ScrollFrame", nil, parent, "UIPanelScrollFrameTemplate")
    local child = CreateFrame("Frame", nil, scroll)
    child:SetSize(width, 1)
    scroll:SetScrollChild(child)
    return scroll, child
end

local function button(parent, text, width, onClick)
    local b = CreateFrame("Button", nil, parent, "UIPanelButtonTemplate")
    b:SetSize(width, 24)
    b:SetText(text)
    b:SetScript("OnClick", onClick)
    return b
end

-- ---------------------------------------------------------------- thread list

local listScroll, listChild = makeScroll(frame, LIST_WIDTH - 10)
listScroll:SetPoint("TOPLEFT", 18, -46)
listScroll:SetPoint("BOTTOMLEFT", 18, 48)
listScroll:SetWidth(LIST_WIDTH - 30)
local rows, headers = {}, {}

local function header(index)
    local h = headers[index]
    if not h then
        h = CreateFrame("Button", nil, listChild)
        h:SetSize(LIST_WIDTH - 40, 22)
        h.text = h:CreateFontString(nil, "OVERLAY", "GameFontNormal")
        h.text:SetPoint("LEFT", 4, 0)
        h.line = h:CreateTexture(nil, "BACKGROUND")
        h.line:SetColorTexture(1, 0.82, 0, 0.15)
        h.line:SetAllPoints()
        headers[index] = h
    end
    return h
end

local function row(index)
    local r = rows[index]
    if not r then
        r = CreateFrame("Button", nil, listChild)
        r:SetSize(LIST_WIDTH - 40, 36)
        r.highlight = r:CreateTexture(nil, "BACKGROUND")
        r.highlight:SetAllPoints()
        r.title = r:CreateFontString(nil, "OVERLAY", "GameFontHighlight")
        r.title:SetPoint("TOPLEFT", 6, -4)
        r.title:SetPoint("RIGHT", -64, 0)
        r.title:SetJustifyH("LEFT")
        r.title:SetWordWrap(false)
        r.sub = r:CreateFontString(nil, "OVERLAY", "GameFontDisableSmall")
        r.sub:SetPoint("BOTTOMLEFT", 6, 4)
        r.sub:SetPoint("RIGHT", -40, 0)
        r.sub:SetJustifyH("LEFT")
        r.sub:SetWordWrap(false)
        r.status = r:CreateFontString(nil, "OVERLAY", "GameFontHighlightSmall")
        r.status:SetPoint("TOPRIGHT", -6, -5)
        r.age = r:CreateFontString(nil, "OVERLAY", "GameFontDisableSmall")
        r.age:SetPoint("BOTTOMRIGHT", -6, 5)
        r:SetScript("OnClick", function(self) UI.Select(self.thread) end)
        rows[index] = r
    end
    return r
end

local function renderList()
    for _, r in ipairs(rows) do r:Hide() end
    for _, h in ipairs(headers) do h:Hide() end
    local groups = { working = {}, open = {}, settled = {} }
    for _, t in ipairs(inbox and inbox.threads or {}) do
        local key = t.set and "settled" or (WORKING[t.s] and "working") or "open"
        table.insert(groups[key], t)
    end
    local y, rowIndex, headerIndex = 0, 0, 0
    for _, group in ipairs({ { "working", "Working" }, { "open", "Open" }, { "settled", "Settled" } }) do
        local key, label = group[1], group[2]
        headerIndex = headerIndex + 1
        local h = header(headerIndex)
        h.text:SetText(format("%s %s (%d)", collapsed[key] and "+" or "-", label, #groups[key]))
        h:SetPoint("TOPLEFT", 0, -y)
        h:SetScript("OnClick", function()
            collapsed[key] = not collapsed[key]
            renderList()
        end)
        h:Show()
        y = y + 24
        if not collapsed[key] then
            for _, t in ipairs(groups[key]) do
                rowIndex = rowIndex + 1
                local r = row(rowIndex)
                r.thread = t
                r.title:SetText(t.t ~= "" and t.t or "Untitled")
                local e = env(t.e)
                r.sub:SetText(format("%s%s", e and e.name or "?", t.p ~= "" and (" · " .. t.p) or ""))
                r.status:SetText(statusText(t.s))
                r.age:SetText(age(t.at))
                local isSelected = selected and not selected.new and selected.id == t.id
                r.highlight:SetColorTexture(1, 0.82, 0, isSelected and 0.25 or 0.04)
                r:SetPoint("TOPLEFT", 0, -y)
                r:Show()
                y = y + 38
            end
        end
    end
    listChild:SetHeight(math.max(y, 1))
end

-- ---------------------------------------------------------------- conversation

local right = CreateFrame("Frame", nil, frame)
right:SetPoint("TOPLEFT", LIST_WIDTH + 10, -46)
right:SetPoint("BOTTOMRIGHT", -18, 48)

local viewTitle = right:CreateFontString(nil, "OVERLAY", "GameFontNormal")
viewTitle:SetPoint("TOPLEFT", 4, 0)
viewTitle:SetPoint("RIGHT", -4, 0)
viewTitle:SetJustifyH("LEFT")
viewTitle:SetWordWrap(false)
local viewMeta = right:CreateFontString(nil, "OVERLAY", "GameFontHighlightSmall")
viewMeta:SetPoint("TOPLEFT", viewTitle, "BOTTOMLEFT", 0, -3)
viewMeta:SetJustifyH("LEFT")

local MESSAGE_WIDTH = WIDTH - LIST_WIDTH - 70
local messageScroll, messageChild = makeScroll(right, MESSAGE_WIDTH)
messageScroll:SetPoint("TOPLEFT", 0, -40)
messageScroll:SetPoint("BOTTOMRIGHT", -24, 92)
local messageStrings = {}
local earlier = button(messageChild, "Show earlier messages", 180, function() ns.Link.Send({ t = "more" }) end)

local function messageString(index)
    local fs = messageStrings[index]
    if not fs then
        fs = messageChild:CreateFontString(nil, "OVERLAY", "ChatFontNormal")
        fs:SetWidth(MESSAGE_WIDTH - 8)
        fs:SetJustifyH("LEFT")
        fs:SetWordWrap(true)
        fs:SetSpacing(2)
        messageStrings[index] = fs
    end
    return fs
end

local newPanel = CreateFrame("Frame", nil, right)
newPanel:SetPoint("TOPLEFT", 0, -40)
newPanel:SetPoint("BOTTOMRIGHT", -24, 92)
local newHelp = newPanel:CreateFontString(nil, "OVERLAY", "GameFontHighlight")
newHelp:SetPoint("TOPLEFT", 4, -8)
newHelp:SetPoint("RIGHT", -4, 0)
newHelp:SetJustifyH("LEFT")
local envButton = button(newPanel, "Machine", 260, function()
    local count = inbox and #inbox.envs or 0
    if count == 0 then return end
    selected.e = selected.e % count + 1
    selected.p = 0
    UI.Refresh()
end)
envButton:SetPoint("TOPLEFT", newHelp, "BOTTOMLEFT", 0, -12)
local projectButton = button(newPanel, "Project", 260, function()
    local list = UI.ProjectsFor(selected.e)
    selected.p = (selected.p + 1) % (#list + 1)
    UI.Refresh()
end)
projectButton:SetPoint("TOPLEFT", envButton, "BOTTOMLEFT", 0, -6)

function UI.ProjectsFor(envIndex)
    local list = {}
    for _, p in ipairs(inbox and inbox.projects or {}) do
        if p.e == envIndex then list[#list + 1] = p end
    end
    table.sort(list, function(a, b) return a.t:lower() < b.t:lower() end)
    return list
end

local shownThread
local function renderView()
    local range = messageScroll:GetVerticalScrollRange()
    local stickToBottom = messageScroll:GetVerticalScroll() >= range - 8 or shownThread ~= (selected and selected.id)
    shownThread = selected and selected.id
    for _, fs in ipairs(messageStrings) do fs:Hide() end
    earlier:Hide()
    if selected and selected.new then
        messageScroll:Hide()
        newPanel:Show()
        local e = env(selected.e)
        local projects = UI.ProjectsFor(selected.e)
        local project = projects[selected.p]
        viewTitle:SetText("New thread")
        viewMeta:SetText("Type the task below and press Send.")
        newHelp:SetText("The thread uses the project's default model and mode.")
        envButton:SetText("Machine: " .. (e and e.name or "?"))
        projectButton:SetText("Project: " .. (project and project.t or "No project (scratch)"))
        return
    end
    newPanel:Hide()
    messageScroll:Show()
    if not selected then
        viewTitle:SetText("Pick a thread")
        viewMeta:SetText("")
        return
    end
    local view = inbox and inbox.view
    local listed
    for _, t in ipairs(inbox and inbox.threads or {}) do
        if t.id == selected.id then listed = t end
    end
    viewTitle:SetText((view and view.id == selected.id and view.t) or (listed and listed.t) or "")
    local status = (listed and listed.s) or (view and view.s)
    local e = env(selected.e)
    viewMeta:SetText(format("%s · %s · %s", statusText(status), e and e.name or "?", age(listed and listed.at)))
    if not view or view.id ~= selected.id then
        local fs = messageString(1)
        fs:SetText("|cffa0a0a0Loading…|r")
        fs:SetPoint("TOPLEFT", 4, 0)
        fs:Show()
        return
    end
    local y = 0
    if view.more then
        earlier:SetPoint("TOP", messageChild, "TOP", 0, 0)
        earlier:Show()
        y = 30
    end
    local items = view.items or {}
    local count = #items
    for i, item in ipairs(items) do
        local fs = messageString(i)
        local who = item.r == "u" and "|cff7fbfffYou|r" or "|cff40c0c0Agent|r"
        fs:SetText(who .. "\n" .. item.x)
        fs:SetPoint("TOPLEFT", 4, -y)
        fs:Show()
        y = y + fs:GetStringHeight() + 14
    end
    if echo and echo.id == selected.id then
        local last = items[count]
        if last and last.r == "u" and last.x:sub(1, 20) == echo.text:sub(1, 20) then
            echo = nil
        else
            local fs = messageString(count + 1)
            fs:SetText("|cff7fbfffYou|r |cffa0a0a0(sending)|r\n" .. echo.text:gsub("|", "||"))
            fs:SetPoint("TOPLEFT", 4, -y)
            fs:Show()
            y = y + fs:GetStringHeight() + 14
        end
    end
    if WORKING[status] then
        local fs = messageString(count + 2)
        fs:SetText("|cffffd100Working…|r")
        fs:SetPoint("TOPLEFT", 4, -y)
        fs:Show()
        y = y + fs:GetStringHeight() + 14
    end
    messageChild:SetHeight(math.max(y, 1))
    if stickToBottom then
        C_Timer.After(0, function() messageScroll:SetVerticalScroll(messageScroll:GetVerticalScrollRange()) end)
    end
end

-- ---------------------------------------------------------------- composer

local inputScroll = CreateFrame("ScrollFrame", nil, right, "UIPanelScrollFrameTemplate")
inputScroll:SetPoint("BOTTOMLEFT", 4, 4)
inputScroll:SetPoint("BOTTOMRIGHT", -26, 4)
inputScroll:SetHeight(76)
local inputBg = CreateFrame("Frame", nil, right, "BackdropTemplate")
inputBg:SetPoint("TOPLEFT", inputScroll, -6, 6)
inputBg:SetPoint("BOTTOMRIGHT", inputScroll, 26, -6)
inputBg:SetBackdrop({
    bgFile = "Interface\\Tooltips\\UI-Tooltip-Background",
    edgeFile = "Interface\\Tooltips\\UI-Tooltip-Border",
    tile = true, tileSize = 16, edgeSize = 14,
    insets = { left = 3, right = 3, top = 3, bottom = 3 },
})
inputBg:SetBackdropColor(0, 0, 0, 0.8)
inputBg:SetFrameLevel(inputScroll:GetFrameLevel() - 1)

local input = CreateFrame("EditBox", nil, inputScroll)
input:SetMultiLine(true)
input:SetAutoFocus(false)
input:SetFontObject(ChatFontNormal)
input:SetWidth(MESSAGE_WIDTH - 30)
input:SetMaxBytes(2400)
inputScroll:SetScrollChild(input)
input:SetScript("OnEscapePressed", input.ClearFocus)
inputScroll:SetScript("OnMouseDown", function() input:SetFocus() end)
local placeholder = input:CreateFontString(nil, "OVERLAY", "GameFontDisable")
placeholder:SetPoint("TOPLEFT", 2, -2)
placeholder:SetText("Message the agent (Shift+Enter for a new line)")
input:SetScript("OnTextChanged", function(self)
    placeholder:SetShown(self:GetText() == "")
end)
input:SetScript("OnCursorChanged", function(_, _, y, _, height)
    local offset = inputScroll:GetVerticalScroll()
    if -y < offset then inputScroll:SetVerticalScroll(-y)
    elseif -y + height > offset + inputScroll:GetHeight() then inputScroll:SetVerticalScroll(-y + height - inputScroll:GetHeight()) end
end)

local function sendCurrent()
    local text = input:GetText():gsub("^%s+", ""):gsub("%s+$", "")
    if text == "" or not selected then return end
    if selected.new then
        local e = env(selected.e)
        local project = UI.ProjectsFor(selected.e)[selected.p]
        ns.Link.Send({ t = "new", e = e and e.id or "", p = project and project.id or nil, m = text })
        UI.Notice("Starting a new thread…")
    else
        local e = env(selected.e)
        ns.Link.Send({ t = "send", e = e and e.id or "", id = selected.id, m = text })
        echo = { id = selected.id, text = text }
    end
    input:SetText("")
    input:ClearFocus()
    UI.Refresh()
end

input:SetScript("OnEnterPressed", function(self)
    if IsShiftKeyDown() then self:Insert("\n") else sendCurrent() end
end)

-- ---------------------------------------------------------------- buttons and status

local refresh = button(frame, "Refresh", 90, function() ns.Link.Send({ t = "refresh" }) end)
refresh:SetPoint("BOTTOMLEFT", 18, 16)
local newThread = button(frame, "New Thread", 110, function()
    selected = { new = true, e = 1, p = 0 }
    ns.Link.Send({ t = "open" })
    UI.Refresh()
    input:SetFocus()
end)
newThread:SetPoint("LEFT", refresh, "RIGHT", 6, 0)
local reload = button(frame, "Reload UI", 100, function() ReloadUI() end)
reload:SetPoint("LEFT", newThread, "RIGHT", 6, 0)

local send = button(frame, "Send", 90, sendCurrent)
send:SetPoint("BOTTOMRIGHT", -18, 16)
local settle = button(frame, "Settle", 90, function()
    if not selected or selected.new then return end
    local listed
    for _, t in ipairs(inbox and inbox.threads or {}) do if t.id == selected.id then listed = t end end
    local e = env(selected.e)
    ns.Link.Send({ t = (listed and listed.set) and "unsettle" or "settle", e = e and e.id or "", id = selected.id })
end)
settle:SetPoint("RIGHT", send, "LEFT", -6, 0)
local stop = button(frame, "Stop", 80, function()
    if not selected or selected.new then return end
    local e = env(selected.e)
    ns.Link.Send({ t = "stop", e = e and e.id or "", id = selected.id })
end)
stop:SetPoint("RIGHT", settle, "LEFT", -6, 0)

local statusLine = frame:CreateFontString(nil, "OVERLAY", "GameFontHighlightSmall")
statusLine:SetPoint("LEFT", reload, "RIGHT", 12, 0)
statusLine:SetPoint("RIGHT", stop, "LEFT", -12, 0)
statusLine:SetJustifyH("LEFT")
statusLine:SetWordWrap(false)

local function renderStatus()
    local link = ns.Link.Status()
    local connected
    if ns.Link.Exhausted() then
        connected = "|cffff8000Out of update slots: Reload UI|r"
    elseif link.loadError then
        connected = "|cffff4040Slot addons blocked (" .. tostring(link.loadError) .. "): enable T3 WoW slots in AddOns|r"
    elseif ns.Link.Stalled() then
        connected = "|cffff4040Companion not answering: is t3wow running?|r"
    elseif not inbox then
        connected = "|cffa0a0a0Waiting for the companion…|r"
    elseif inbox.auth then
        connected = "|cffff4040" .. inbox.auth .. "|r"
    else
        connected = "|cff40c040Connected|r"
    end
    connection:SetText(connected)
    reload:SetShown(ns.Link.Exhausted() or link.loadError ~= nil)
    local line = notice
    if not line and inbox and inbox.notes and #inbox.notes > 0 then
        local last = inbox.notes[#inbox.notes]
        if time() - last.at < 30 then line = last.x end
    end
    if link.queue > 0 then line = "Sending…" end
    statusLine:SetText(line or "")
    local listed
    if selected and not selected.new then
        for _, t in ipairs(inbox and inbox.threads or {}) do if t.id == selected.id then listed = t end end
    end
    settle:SetText(listed and listed.set and "Unsettle" or "Settle")
    settle:SetEnabled(listed ~= nil)
    stop:SetEnabled(listed ~= nil and WORKING[listed.s] == true)
    send:SetText(selected and selected.new and "Start" or "Send")
    send:SetEnabled(selected ~= nil)
end

-- ---------------------------------------------------------------- API used by Link

function UI.Refresh()
    if not frame:IsShown() then return end
    renderList()
    renderView()
    renderStatus()
end

function UI.OnInbox(data)
    inbox = data
    notice = nil
    UI.Refresh()
end

function UI.Notice(text)
    notice = text
    UI.Refresh()
end

function UI.Alert(event)
    local s = STATUS[event.s] or { event.s, "ffffff" }
    print(format("|cff8f8fffT3|r %s: |cff%s%s|r", event.t ~= "" and event.t or "Thread", s[2], s[1]))
    if SOUNDKIT and SOUNDKIT.TELL_MESSAGE then PlaySound(SOUNDKIT.TELL_MESSAGE) end
end

function UI.IsShown() return frame:IsShown() end

function UI.Select(thread)
    selected = { e = thread.e, id = thread.id }
    local e = env(thread.e)
    ns.Link.Send({ t = "open", e = e and e.id or "", id = thread.id })
    UI.Refresh()
end

frame:SetScript("OnShow", function()
    ns.Link.Send({ t = "ui", o = true })
    UI.Refresh()
end)
frame:SetScript("OnHide", function() ns.Link.Send({ t = "ui", o = false }) end)

local ticker = 0
frame:SetScript("OnUpdate", function(_, delta)
    ticker = ticker + delta
    if ticker < 1 then return end
    ticker = 0
    renderStatus()
end)

function UI.Toggle() frame:SetShown(not frame:IsShown()) end
