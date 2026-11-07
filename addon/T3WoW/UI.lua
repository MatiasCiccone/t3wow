local _, ns = ...
local UI = {}
ns.UI = UI

local format, floor = string.format, math.floor
local WIDTH, HEIGHT, LIST_WIDTH = 940, 580, 300
local MIN_WIDTH, MIN_HEIGHT, MAX_WIDTH, MAX_HEIGHT = 720, 420, 2400, 1600
local WORKING = { preparing = true, queued = true, starting = true, running = true }
local STATUS = {
    preparing = { "Working", "ffd100" }, queued = { "Queued", "ffd100" }, starting = { "Working", "ffd100" },
    running = { "Working", "ffd100" }, completed = { "Finished", "40c040" }, failed = { "Failed", "ff4040" },
    waiting = { "Needs you", "ff8000" }, approval = { "Approve in T3", "ff8000" }, interrupted = { "Stopped", "a0a0a0" }, cancelled = { "Stopped", "a0a0a0" },
    idle = { "Idle", "a0a0a0" }, rolled_back = { "Rolled back", "a0a0a0" },
}

local inbox
local selected -- { e = envIndex, id = threadId } or { new = true, e = envIndex, p = projectIndex }
local collapsed = { settled = true, snoozed = true }
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
title:SetText("T3 WoW")

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
    local groups = { working = {}, open = {}, snoozed = {}, settled = {} }
    -- Threads an agent launched sit under the thread that launched them when both are in
    -- the same group, as in T3's sidebar, and don't count in it. Away from their launcher
    -- (settled while it's open, or it isn't listed) they show on their own only while
    -- they're working or need you; otherwise T3's sidebar has no row for them either.
    local function groupOf(t) return (t.set and "settled") or (t.sn and "snoozed") or (WORKING[t.s] and "working") or "open" end
    local threads = inbox and inbox.threads or {}
    local byId, launched = {}, {}
    for _, t in ipairs(threads) do byId[t.e .. ":" .. t.id] = t end
    for _, t in ipairs(threads) do
        local parent = t.par and byId[t.e .. ":" .. t.par]
        if parent and groupOf(parent) == groupOf(t) then
            local key = t.e .. ":" .. t.par
            launched[key] = launched[key] or {}
            table.insert(launched[key], t)
        elseif not t.par or WORKING[t.s] or (t.s == "waiting" and not t.set) then
            table.insert(groups[groupOf(t)], t)
        end
    end
    local y, rowIndex, headerIndex = 0, 0, 0
    for _, group in ipairs({ { "open", "Open" }, { "working", "Working" }, { "snoozed", "Snoozed" }, { "settled", "Settled" } }) do
        local key, label = group[1], group[2]
        headerIndex = headerIndex + 1
        local h = header(headerIndex)
        -- Only recent settled threads are listed; the count is all of them, as in T3.
        local count = #groups[key]
        if key == "settled" and inbox and inbox.settledTotal then count = math.max(count, inbox.settledTotal) end
        h.text:SetText(format("%s %s (%d)", collapsed[key] and "+" or "-", label, count))
        h:SetPoint("TOPLEFT", 0, -y)
        h:SetScript("OnClick", function()
            collapsed[key] = not collapsed[key]
            renderList()
        end)
        h:Show()
        y = y + 24
        if not collapsed[key] then
            local placed = {}
            local function place(t, indent)
                local id = t.e .. ":" .. t.id
                if placed[id] then return end
                placed[id] = true
                rowIndex = rowIndex + 1
                local r = row(rowIndex)
                r.thread = t
                r.title:SetText(t.t ~= "" and t.t or "Untitled")
                local e = env(t.e)
                r.sub:SetText(format("%s%s", e and e.name or "?", t.p ~= "" and (" · " .. t.p) or ""))
                r.status:SetText(statusText(t.s))
                r.age:SetText(age(t.at))
                local isSelected = selected and not selected.new and selected.id == t.id
                r.highlight:SetColorTexture(1, 0.82, 0, isSelected and 0.25 or (indent > 0 and 0.02 or 0.04))
                r:SetPoint("TOPLEFT", indent, -y)
                r:SetWidth(LIST_WIDTH - 40 - indent)
                r:Show()
                y = y + 38
                for _, child in ipairs(launched[id] or {}) do place(child, math.min(indent + 14, 42)) end
            end
            for _, t in ipairs(groups[key]) do place(t, 0) end
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

-- The conversation takes whatever width the list leaves; it follows the window's size.
local function messageWidth() return frame:GetWidth() - LIST_WIDTH - 70 end
local messageScroll, messageChild = makeScroll(right, messageWidth())
messageScroll:SetPoint("TOPLEFT", 0, -40)
messageScroll:SetPoint("BOTTOMRIGHT", -24, 92)
local messageStrings = {}
local earlier = button(messageChild, "Show earlier messages", 180, function() ns.Link.Send({ t = "more" }) end)

local function messageString(index)
    local fs = messageStrings[index]
    if not fs then
        fs = messageChild:CreateFontString(nil, "OVERLAY", "ChatFontNormal")
        fs:SetWidth(messageWidth() - 8)
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

-- A thread opens scrolled to its latest message. The messages arrive a moment after the
-- click, so the wish is kept until they are drawn.
local scrollToEnd = false
local function scrollToBottom()
    messageScroll:UpdateScrollChildRect()
    messageScroll:SetVerticalScroll(messageScroll:GetVerticalScrollRange())
    -- Text heights can settle a frame later; scroll again then.
    C_Timer.After(0, function()
        messageScroll:UpdateScrollChildRect()
        messageScroll:SetVerticalScroll(messageScroll:GetVerticalScrollRange())
    end)
end

-- ---------------------------------------------------------------- questions from the agent
-- A question the agent is waiting on shows at the end of the conversation: one button per
-- option (they toggle when several may be picked) and Send answer. Text typed in the box
-- answers it too, in your own words. Approvals can't be answered from here: T3's tools
-- don't allow it, so the window says to answer them in T3.
local askStrings, askButtons = {}, {}
local askState = { k = nil, picks = {} }
local setPlaceholder -- set with the composer below
local renderView -- defined below; the option buttons redraw through it

local function pendingAsk()
    local view = inbox and inbox.view
    if view and view.ask and selected and not selected.new and view.id == selected.id then return view.ask end
end

-- Sends the picks, and typed text for the first question without a pick. The companion
-- turns the indexes back into T3's ids and labels. Returns true when it used the input.
local function sendAnswer(text)
    local ask = pendingAsk()
    if not ask then return false end
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
    local e = env(selected.e)
    ns.Link.Send({ t = "answer", e = e and e.id or "", id = selected.id, k = ask.k, a = answers })
    askState.sent = ask.k
    UI.Notice("Sending your answer…")
    return true
end

-- Brings T3 to the front on a thread: the companion opens T3's own link for it.
local function openInT3(threadId, envId)
    ns.Link.Send({ t = "t3", e = envId or "", id = threadId or "" })
    UI.Notice("Opening T3…")
end

local approvalOpen = button(messageChild, "Open in T3", 120, function()
    local e = selected and env(selected.e)
    if selected then openInT3(selected.id, e and e.id) end
end)
approvalOpen:Hide()

local askSend = button(messageChild, "Send answer", 120, function()
    sendAnswer(nil)
    UI.Refresh()
end)
askSend:Hide()

local function askString(index)
    local fs = askStrings[index]
    if not fs then
        fs = messageChild:CreateFontString(nil, "OVERLAY", "ChatFontNormal")
        fs:SetJustifyH("LEFT")
        fs:SetWordWrap(true)
        fs:SetSpacing(2)
        askStrings[index] = fs
    end
    return fs
end

local function askButton(index)
    local b = askButtons[index]
    if not b then
        b = CreateFrame("Button", nil, messageChild, "UIPanelButtonTemplate")
        b:SetHeight(22)
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
        b:SetScript("OnEnter", function(self)
            if self.desc and self.desc ~= "" and self.desc ~= self.label then
                GameTooltip:SetOwner(self, "ANCHOR_RIGHT")
                GameTooltip:SetText(self.desc, 1, 1, 1, 1, true)
                GameTooltip:Show()
            end
        end)
        b:SetScript("OnLeave", function() GameTooltip:Hide() end)
        askButtons[index] = b
    end
    return b
end

local function hideAsk()
    for _, fs in ipairs(askStrings) do fs:Hide() end
    for _, b in ipairs(askButtons) do b:Hide() end
    askSend:Hide()
    approvalOpen:Hide()
end

-- Draws the question (or the approval note) from y down; returns the new y.
local function renderAsk(view, y)
    hideAsk()
    local ask = view.ask
    if setPlaceholder then setPlaceholder(ask ~= nil) end
    if not ask then
        if view.approval then
            local fs = askString(1)
            fs:SetWidth(messageWidth() - 8)
            fs:SetText("|cffff8000Waiting for your approval.|r Approve or deny it in T3: by design, T3 doesn't take approvals from anywhere else. Open in T3 brings it up on this thread; Stop cancels the run from here.")
            fs:ClearAllPoints()
            fs:SetPoint("TOPLEFT", 4, -y)
            fs:Show()
            y = y + fs:GetStringHeight() + 8
            approvalOpen:ClearAllPoints()
            approvalOpen:SetPoint("TOPLEFT", 4, -y)
            approvalOpen:Show()
            y = y + 32
        end
        return y
    end
    if askState.k ~= ask.k then askState = { k = ask.k, picks = {} } end
    local width = messageWidth() - 16
    local si, bi = 0, 0
    for qi, q in ipairs(ask.qs or {}) do
        si = si + 1
        local fs = askString(si)
        fs:SetWidth(width)
        fs:SetText(format("|cffff8000%s|r%s\n%s", (q.h and q.h ~= "") and q.h or "Question",
            q.m and " |cffa0a0a0(pick any)|r" or "", q.q or ""))
        fs:ClearAllPoints()
        fs:SetPoint("TOPLEFT", 4, -y)
        fs:Show()
        y = y + fs:GetStringHeight() + 6
        for oi, o in ipairs(q.o or {}) do
            bi = bi + 1
            local b = askButton(bi)
            b.qi, b.oi, b.multi, b.desc, b.label = qi, oi, q.m, o.d, o.l
            local picked = askState.picks[qi] and askState.picks[qi][oi]
            local mark = q.m and (picked and "[x] " or "[  ] ") or ""
            b:SetText(picked and ("|cff40ff40" .. mark .. o.l .. "|r") or (mark .. o.l))
            b:SetWidth(math.min(width, math.max(140, b:GetFontString():GetStringWidth() + 32)))
            if picked then b:LockHighlight() else b:UnlockHighlight() end
            b:ClearAllPoints()
            b:SetPoint("TOPLEFT", 12, -y)
            b:Show()
            y = y + 25
        end
        y = y + 8
    end
    local sending = askState.sent == ask.k
    askSend:ClearAllPoints()
    askSend:SetPoint("TOPLEFT", 12, -y)
    askSend:SetText(sending and "Sending…" or "Send answer")
    askSend:SetEnabled(not sending)
    askSend:Show()
    si = si + 1
    local hint = askString(si)
    hint:SetWidth(math.max(80, width - 150))
    hint:SetText("|cffa0a0a0or type your own answer below and press Enter|r")
    hint:ClearAllPoints()
    hint:SetPoint("LEFT", askSend, "RIGHT", 10, 0)
    hint:Show()
    return y + 34
end

function renderView()
    -- Stay at the end while new messages come in, unless you scrolled up to read.
    local stickToBottom = scrollToEnd
        or messageScroll:GetVerticalScroll() >= messageScroll:GetVerticalScrollRange() - 8
    for _, fs in ipairs(messageStrings) do fs:Hide() end
    hideAsk()
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
        -- Your message is in once any recent message of yours matches it: the agent may
        -- already have answered, so it needn't be the last one. Compare letters and digits
        -- only, since the thread view reformats text (markdown, escaped pipes).
        local function key(text)
            return (text or ""):gsub("||", ""):gsub("|c%x%x%x%x%x%x%x%x", ""):gsub("|r", ""):gsub("[^%w]", ""):lower():sub(1, 40)
        end
        local wanted, arrived = key(echo.text), false
        for i = count, math.max(1, count - 7), -1 do
            if items[i].r == "u" and key(items[i].x) == wanted then arrived = true break end
        end
        if arrived then
            echo = nil
        else
            local fs = messageString(count + 1)
            fs:SetText("|cff7fbfffYou|r |cffa0a0a0(sending)|r\n" .. echo.text:gsub("|", "||"))
            fs:SetPoint("TOPLEFT", 4, -y)
            fs:Show()
            y = y + fs:GetStringHeight() + 14
        end
    end
    y = renderAsk(view, y)
    if WORKING[status] then
        local fs = messageString(count + 2)
        fs:SetText("|cffffd100Working…|r")
        fs:SetPoint("TOPLEFT", 4, -y)
        fs:Show()
        y = y + fs:GetStringHeight() + 14
    end
    messageChild:SetHeight(math.max(y, 1))
    if stickToBottom then
        scrollToEnd = false
        scrollToBottom()
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
input:SetWidth(messageWidth() - 30)
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
setPlaceholder = function(asking)
    placeholder:SetText(asking and "Type your own answer, or pick one above"
        or "Message the agent (Shift+Enter for a new line)")
end
input:SetScript("OnCursorChanged", function(_, _, y, _, height)
    local offset = inputScroll:GetVerticalScroll()
    if -y < offset then inputScroll:SetVerticalScroll(-y)
    elseif -y + height > offset + inputScroll:GetHeight() then inputScroll:SetVerticalScroll(-y + height - inputScroll:GetHeight()) end
end)

local function sendCurrent()
    local text = input:GetText():gsub("^%s+", ""):gsub("%s+$", "")
    -- While the agent waits on a question, Send (and Enter) answers it.
    if pendingAsk() then
        if sendAnswer(text) and text ~= "" then input:SetText("") end
        input:ClearFocus()
        UI.Refresh()
        return
    end
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
        connected = "|cffff4040Companion not answering: is T3 WoW running in the system tray?|r"
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
    send:SetText((selected and selected.new and "Start") or (pendingAsk() and "Answer") or "Send")
    send:SetEnabled(selected ~= nil)
end

-- ---------------------------------------------------------------- settings

local DEFAULTS = {
    toasts = true, chat = true, sound = true, duration = 8, maxToasts = 4, approvalOpensT3 = false,
    onCompleted = true, onWaiting = true, onFailed = true, onStopped = false,
}
local EVENT_SETTING = {
    completed = "onCompleted", idle = "onCompleted", rolled_back = "onCompleted",
    waiting = "onWaiting", approval = "onWaiting", failed = "onFailed", interrupted = "onStopped", cancelled = "onStopped",
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

local function hexColor(hex)
    return tonumber(hex:sub(1, 2), 16) / 255, tonumber(hex:sub(3, 4), 16) / 255, tonumber(hex:sub(5, 6), 16) / 255
end

local function envIndexById(id)
    for index, e in ipairs(inbox and inbox.envs or {}) do
        if e.id == id then return index end
    end
end

function UI.OpenThread(threadId, envId)
    frame:Show()
    UI.Select({ e = envIndexById(envId) or 1, id = threadId })
end

-- ---------------------------------------------------------------- notifications

local TOAST_WIDTH, TOAST_HEIGHT, TOAST_GAP, TOAST_LIMIT = 320, 60, 6, 10
local STATUS_ICON = {
    completed = "Interface\\RaidFrame\\ReadyCheck-Ready",
    waiting = "Interface\\RaidFrame\\ReadyCheck-Waiting",
    approval = "Interface\\RaidFrame\\ReadyCheck-Waiting",
    failed = "Interface\\RaidFrame\\ReadyCheck-NotReady",
    interrupted = "Interface\\RaidFrame\\ReadyCheck-NotReady",
    cancelled = "Interface\\RaidFrame\\ReadyCheck-NotReady",
}

-- Notifications stack down from this anchor; "Move notifications" in settings shows it.
-- It has a notification's exact size and place, so where you drop it is where the first
-- notification appears.
local toastAnchor = CreateFrame("Frame", "T3WoWToastAnchor", UIParent, "BackdropTemplate")
toastAnchor:SetSize(TOAST_WIDTH, TOAST_HEIGHT)
toastAnchor:SetFrameStrata("DIALOG")
toastAnchor:SetClampedToScreen(true)
toastAnchor:SetMovable(true)
toastAnchor:RegisterForDrag("LeftButton")
toastAnchor:SetBackdrop({
    bgFile = "Interface\\Tooltips\\UI-Tooltip-Background",
    edgeFile = "Interface\\Tooltips\\UI-Tooltip-Border",
    tile = true, tileSize = 16, edgeSize = 16,
    insets = { left = 4, right = 4, top = 4, bottom = 4 },
})
toastAnchor:SetBackdropColor(0.1, 0.4, 0.8, 0.85)
toastAnchor:SetAlpha(0)
toastAnchor:EnableMouse(false)
local anchorTitle = toastAnchor:CreateFontString(nil, "OVERLAY", "GameFontNormal")
anchorTitle:SetPoint("CENTER", 0, 8)
anchorTitle:SetText("T3 WoW notifications appear here")
local anchorHint = toastAnchor:CreateFontString(nil, "OVERLAY", "GameFontHighlightSmall")
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
    toast = CreateFrame("Button", nil, UIParent, "BackdropTemplate")
    toast:SetSize(TOAST_WIDTH, TOAST_HEIGHT)
    toast:SetFrameStrata("DIALOG")
    toast:SetBackdrop({
        bgFile = "Interface\\Tooltips\\UI-Tooltip-Background",
        edgeFile = "Interface\\Tooltips\\UI-Tooltip-Border",
        tile = true, tileSize = 16, edgeSize = 16,
        insets = { left = 4, right = 4, top = 4, bottom = 4 },
    })
    toast:SetBackdropColor(0.04, 0.04, 0.07, 0.94)
    toast.icon = toast:CreateTexture(nil, "ARTWORK")
    toast.icon:SetSize(32, 32)
    toast.icon:SetPoint("LEFT", 12, 0)
    toast.head = toast:CreateFontString(nil, "OVERLAY", "GameFontNormalSmall")
    toast.head:SetPoint("TOPLEFT", toast.icon, "TOPRIGHT", 10, 2)
    toast.head:SetPoint("RIGHT", -10, 0)
    toast.head:SetJustifyH("LEFT")
    toast.title = toast:CreateFontString(nil, "OVERLAY", "GameFontHighlight")
    toast.title:SetPoint("TOPLEFT", toast.head, "BOTTOMLEFT", 0, -3)
    toast.title:SetPoint("RIGHT", -10, 0)
    toast.title:SetJustifyH("LEFT")
    toast.title:SetWordWrap(false)
    toast.sub = toast:CreateFontString(nil, "OVERLAY", "GameFontDisableSmall")
    toast.sub:SetPoint("TOPLEFT", toast.title, "BOTTOMLEFT", 0, -3)
    toast.sub:SetPoint("RIGHT", -10, 0)
    toast.sub:SetJustifyH("LEFT")
    toast:EnableMouse(true)
    toast:RegisterForClicks("LeftButtonUp", "RightButtonUp")
    toast:SetScript("OnClick", function(self, mouse)
        -- Left-click opens the thread in the T3 WoW window (an approval: in T3, the only
        -- place it can be given); right-click only dismisses.
        local threadId, envId, inT3 = self.threadId, self.envId, self.inT3
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
    if fading >= 0.6 then return removeToast(self) end
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
    toast.threadId, toast.envId, toast.inT3 = event.id, event.e, event.s == "approval" and settings().approvalOpensT3 == true
    toast.icon:SetTexture(STATUS_ICON[event.s] or STATUS_ICON.completed)
    toast.head:SetText(format("T3 WoW · |cff%s%s|r", s[2], s[1]))
    toast.title:SetText(event.t ~= "" and event.t or "Thread")
    toast.sub:SetText((machine and machine.name or "") .. (event.id and (toast.inT3 and "  ·  click to open in T3" or "  ·  click to open") or ""))
    toast:SetBackdropBorderColor(hexColor(s[2]))
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

-- Returns what happened, for /t3 status.
function UI.Alert(event)
    local s = settings()
    -- Nothing to announce about the thread you're reading.
    if frame:IsShown() and selected and selected.id == event.id then return "not shown: you were reading that thread" end
    if not s[EVENT_SETTING[event.s] or "onCompleted"] then return "not shown: turned off in Settings" end
    local status = STATUS[event.s] or { event.s, "ffffff" }
    if s.chat then
        print(format("|cff8f8fffT3|r %s: |cff%s%s|r", event.t ~= "" and event.t or "Thread", status[2], status[1]))
    end
    if s.sound and SOUNDKIT and SOUNDKIT.TELL_MESSAGE then PlaySound(SOUNDKIT.TELL_MESSAGE) end
    if s.toasts then showToast(event) end
    return s.toasts and "shown" or "chat line only (notifications off in Settings)"
end

-- ---------------------------------------------------------------- settings panel

local settingsPanel = CreateFrame("Frame", "T3WoWSettings", frame, "BackdropTemplate")
settingsPanel:SetSize(270, 474)
settingsPanel:SetPoint("TOPLEFT", frame, "TOPRIGHT", -4, 0)
settingsPanel:SetBackdrop({
    bgFile = "Interface\\DialogFrame\\UI-DialogBox-Background-Dark",
    edgeFile = "Interface\\DialogFrame\\UI-DialogBox-Border",
    tile = true, tileSize = 32, edgeSize = 32,
    insets = { left = 11, right = 12, top = 12, bottom = 11 },
})
settingsPanel:EnableMouse(true)
settingsPanel:Hide()

local settingsTitle = settingsPanel:CreateFontString(nil, "OVERLAY", "GameFontNormalLarge")
settingsTitle:SetPoint("TOP", 0, -18)
settingsTitle:SetText("Settings")
local settingsClose = CreateFrame("Button", nil, settingsPanel, "UIPanelCloseButton")
settingsClose:SetPoint("TOPRIGHT", -6, -6)

local checks = {}
local layoutY = -48

local function sectionLabel(text)
    local label = settingsPanel:CreateFontString(nil, "OVERLAY", "GameFontNormal")
    label:SetPoint("TOPLEFT", 20, layoutY)
    label:SetText(text)
    layoutY = layoutY - 22
end

-- get/set default to the settings table entry named key.
local function checkbox(text, key, get, set)
    local check = CreateFrame("CheckButton", nil, settingsPanel, "UICheckButtonTemplate")
    check:SetSize(24, 24)
    check:SetPoint("TOPLEFT", 18, layoutY)
    local label = settingsPanel:CreateFontString(nil, "OVERLAY", "GameFontHighlight")
    label:SetPoint("LEFT", check, "RIGHT", 4, 0)
    label:SetText(text)
    check.get = get or function() return settings()[key] end
    check.set = set or function(value) settings()[key] = value end
    check:SetScript("OnClick", function(self) self.set(self:GetChecked() and true or false) end)
    checks[#checks + 1] = check
    layoutY = layoutY - 26
    return check
end

local function durationName(d)
    return d == 0 and "Until clicked" or (d .. " seconds")
end

local refreshToastControls -- set below, once the controls exist

sectionLabel("Notifications")
checkbox("Line in chat", "chat")
checkbox("Sound", "sound")
-- Last, right above the options it switches on and off.
checkbox("On-screen notification", "toasts", nil, function(value)
    settings().toasts = value
    refreshToastControls()
end)

-- How long notifications stay: a dropdown.
local durationLabel = settingsPanel:CreateFontString(nil, "OVERLAY", "GameFontHighlight")
durationLabel:SetPoint("TOPLEFT", 24, layoutY - 8)
durationLabel:SetText("Stay on screen")
local durationMenu = CreateFrame("Frame", "T3WoWDurationMenu", settingsPanel, "UIDropDownMenuTemplate")
durationMenu:SetPoint("LEFT", durationLabel, "RIGHT", -6, -2)
UIDropDownMenu_SetWidth(durationMenu, 110)
UIDropDownMenu_Initialize(durationMenu, function(_, level)
    for _, d in ipairs(DURATIONS) do
        local info = UIDropDownMenu_CreateInfo()
        info.text = durationName(d)
        -- UIDropDownMenu_Initialize also runs this once while the addon loads, before
        -- saved settings exist, so read them without creating anything.
        local saved = T3WoWDB and T3WoWDB.settings
        info.checked = saved ~= nil and saved.duration == d
        info.func = function()
            settings().duration = d
            UIDropDownMenu_SetText(durationMenu, durationName(d))
            CloseDropDownMenus()
        end
        UIDropDownMenu_AddButton(info, level)
    end
end)
layoutY = layoutY - 34

-- How many stay on screen at once: a slider, 1 to 10. The oldest makes room.
local maxLabel = settingsPanel:CreateFontString(nil, "OVERLAY", "GameFontHighlight")
maxLabel:SetPoint("TOPLEFT", 24, layoutY - 4)
maxLabel:SetText("At most")
local maxSlider = CreateFrame("Slider", "T3WoWMaxToasts", settingsPanel, "BackdropTemplate")
maxSlider:SetOrientation("HORIZONTAL")
maxSlider:SetSize(110, 16)
maxSlider:SetPoint("LEFT", maxLabel, "RIGHT", 12, 0)
maxSlider:SetBackdrop({
    bgFile = "Interface\\Buttons\\UI-SliderBar-Background",
    edgeFile = "Interface\\Buttons\\UI-SliderBar-Border",
    tile = true, tileSize = 8, edgeSize = 8,
    insets = { left = 3, right = 3, top = 6, bottom = 6 },
})
maxSlider:SetThumbTexture("Interface\\Buttons\\UI-SliderBar-Button-Horizontal")
maxSlider:SetMinMaxValues(1, TOAST_LIMIT)
maxSlider:SetValueStep(1)
if maxSlider.SetObeyStepOnDrag then maxSlider:SetObeyStepOnDrag(true) end
maxSlider:EnableMouseWheel(true)
local maxValue = settingsPanel:CreateFontString(nil, "OVERLAY", "GameFontHighlight")
maxValue:SetPoint("LEFT", maxSlider, "RIGHT", 8, 0)
local function maxName(n) return n .. " at a time" end
maxSlider:SetScript("OnValueChanged", function(self, value)
    local n = math.floor(value + 0.5)
    maxValue:SetText(maxName(n))
    if settings().maxToasts ~= n then
        settings().maxToasts = n
        trimToasts()
    end
end)
maxSlider:SetScript("OnMouseWheel", function(self, delta) self:SetValue(self:GetValue() + delta) end)
layoutY = layoutY - 28

local moveButton = button(settingsPanel, "Move notifications", 110, nil)
moveButton:SetPoint("TOPLEFT", 22, layoutY - 2)
local testButton = button(settingsPanel, "Test", 110, function()
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
layoutY = layoutY - 38

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
    UIDropDownMenu_SetText(durationMenu, durationName(settings().duration))
    maxSlider:SetValue(settings().maxToasts)
    maxValue:SetText(maxName(settings().maxToasts))
    local grey = on and 1 or 0.5
    maxSlider:EnableMouse(on)
    maxSlider:EnableMouseWheel(on)
    maxSlider:SetAlpha(on and 1 or 0.4)
    maxLabel:SetTextColor(grey, grey, grey)
    maxValue:SetTextColor(grey, grey, grey)
    if on then
        UIDropDownMenu_EnableDropDown(durationMenu)
        durationLabel:SetTextColor(1, 1, 1)
    else
        UIDropDownMenu_DisableDropDown(durationMenu)
        durationLabel:SetTextColor(0.5, 0.5, 0.5)
        setAnchorUnlocked(false)
    end
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
checkbox("Clicking an approval opens T3", "approvalOpensT3")
layoutY = layoutY - 8

sectionLabel("Minimap")
checkbox("Show minimap button", nil,
    function() return not (T3WoWDB and T3WoWDB.minimap and T3WoWDB.minimap.hide) end,
    function(value) if ns.Minimap and ns.Minimap.Show then ns.Minimap.Show(value) end end)

settingsPanel:SetScript("OnShow", function()
    for _, check in ipairs(checks) do check:SetChecked(check.get() and true or false) end
    refreshToastControls()
end)
settingsPanel:SetScript("OnHide", function() setAnchorUnlocked(false) end)

local settingsButton = button(frame, "Settings", 80, function() settingsPanel:SetShown(not settingsPanel:IsShown()) end)
settingsButton:SetPoint("TOPRIGHT", -36, -12)

function UI.ShowSettings()
    frame:Show()
    settingsPanel:Show()
end

-- ---------------------------------------------------------------- API used by Link

function UI.Refresh()
    if not frame:IsShown() then return end
    renderList()
    renderView()
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

function UI.OnInbox(data)
    announce(data)
    inbox = data
    notice = nil
    UI.Refresh()
end

function UI.Notice(text)
    notice = text
    UI.Refresh()
end

function UI.IsShown() return frame:IsShown() end

function UI.Select(thread)
    selected = { e = thread.e, id = thread.id }
    scrollToEnd = true
    local e = env(thread.e)
    ns.Link.Send({ t = "open", e = e and e.id or "", id = thread.id })
    UI.Refresh()
end

-- ---------------------------------------------------------------- resizing

-- Text wraps to the conversation's width, so a new width means laying the messages out again.
local function applyWidth()
    local width = messageWidth()
    messageChild:SetWidth(width)
    for _, fs in ipairs(messageStrings) do fs:SetWidth(width - 8) end
    input:SetWidth(width - 30)
end

frame:SetScript("OnSizeChanged", function()
    applyWidth()
    if frame:IsShown() then renderView() end
end)

local grip = CreateFrame("Button", nil, frame)
grip:SetSize(16, 16)
grip:SetPoint("BOTTOMRIGHT", -2, 2)
grip:SetFrameLevel(frame:GetFrameLevel() + 10)
grip:SetNormalTexture("Interface\\ChatFrame\\UI-ChatIM-SizeGrabber-Up")
grip:SetHighlightTexture("Interface\\ChatFrame\\UI-ChatIM-SizeGrabber-Highlight")
grip:SetPushedTexture("Interface\\ChatFrame\\UI-ChatIM-SizeGrabber-Down")
grip:SetScript("OnMouseDown", function(_, mouse)
    if mouse == "LeftButton" then frame:StartSizing("BOTTOMRIGHT") end
end)
grip:SetScript("OnMouseUp", function()
    frame:StopMovingOrSizing()
    saveLayout()
    UI.Refresh()
end)
grip:SetScript("OnEnter", function(self)
    GameTooltip:SetOwner(self, "ANCHOR_RIGHT")
    GameTooltip:AddLine("Drag to resize")
    GameTooltip:AddLine("Right-click to reset the size", 0.7, 0.7, 0.7)
    GameTooltip:Show()
end)
grip:SetScript("OnLeave", function() GameTooltip:Hide() end)
grip:RegisterForClicks("RightButtonUp")
grip:SetScript("OnClick", function()
    frame:SetSize(WIDTH, HEIGHT)
    saveLayout()
    UI.Refresh()
end)

frame:SetScript("OnShow", function()
    restoreLayout()
    ns.Link.Send({ t = "ui", o = true })
    scrollToEnd = true -- reopening the window also lands on the latest message
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
