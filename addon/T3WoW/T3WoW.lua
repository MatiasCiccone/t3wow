local addonName, ns = ...

BINDING_HEADER_T3WOW = "T3 WoW"
BINDING_NAME_T3WOW_TOGGLE = "Toggle T3 WoW"
function T3WoW_Toggle() ns.UI.Toggle() end

-- ---------------------------------------------------------------- minimap button
-- Registered through LibDBIcon when another addon has loaded it, so button collectors
-- (MinimapButtonButton and the like) pick it up with the rest; otherwise a plain
-- minimap button whose name those collectors also recognise.

-- The T3 logo, shipped with the addon (Media\logo.tga, 64x64 with alpha).
local ICON = "Interface\\AddOns\\T3WoW\\Media\\logo"
local LDB_NAME = "T3 WoW"
local WORKING = { preparing = true, queued = true, starting = true, running = true }

local function onClick(_, button)
    if button == "RightButton" then
        ns.UI.ShowSettings()
    else
        ns.UI.Toggle()
    end
end

local function fillTooltip(tooltip)
    tooltip:AddLine("T3 WoW")
    local inbox = ns.Link.state.inbox
    if inbox and inbox.threads then
        local working = 0
        for _, t in ipairs(inbox.threads) do
            if not t.set and WORKING[t.s] then working = working + 1 end
        end
        tooltip:AddLine(working == 1 and "1 thread working" or (working .. " threads working"), 1, 1, 1)
    else
        tooltip:AddLine("Waiting for the companion", 0.6, 0.6, 0.6)
    end
    tooltip:AddLine("Click to open, right-click for settings", 0.6, 0.6, 0.6)
end

local function ownButton(settings)
    local button = CreateFrame("Button", "T3WoWMinimapButton", Minimap)
    button:SetSize(31, 31)
    button:SetFrameStrata("MEDIUM")
    button:SetFrameLevel(8)
    button:RegisterForClicks("LeftButtonUp", "RightButtonUp")
    button:RegisterForDrag("LeftButton")
    button:SetHighlightTexture("Interface\\Minimap\\UI-Minimap-ZoomButton-Highlight")
    local background = button:CreateTexture(nil, "BACKGROUND")
    background:SetTexture("Interface\\Minimap\\UI-Minimap-Background")
    background:SetSize(20, 20)
    background:SetPoint("TOPLEFT", 7, -5)
    local icon = button:CreateTexture(nil, "ARTWORK")
    icon:SetTexture(ICON)
    icon:SetSize(20, 20)
    icon:SetPoint("TOPLEFT", 6, -5)
    local border = button:CreateTexture(nil, "OVERLAY")
    border:SetTexture("Interface\\Minimap\\MiniMap-TrackingBorder")
    border:SetSize(53, 53)
    border:SetPoint("TOPLEFT")

    local function place()
        local angle = math.rad(settings.minimapPos or 220)
        local radius = Minimap:GetWidth() / 2 + 10
        button:ClearAllPoints()
        button:SetPoint("CENTER", Minimap, "CENTER", math.cos(angle) * radius, math.sin(angle) * radius)
    end
    local function follow()
        local mx, my = Minimap:GetCenter()
        local scale = Minimap:GetEffectiveScale()
        local cx, cy = GetCursorPosition()
        settings.minimapPos = math.deg(math.atan2(cy / scale - my, cx / scale - mx)) % 360
        place()
    end
    button:SetScript("OnDragStart", function(self) self:SetScript("OnUpdate", follow) end)
    button:SetScript("OnDragStop", function(self) self:SetScript("OnUpdate", nil) end)
    button:SetScript("OnClick", onClick)
    button:SetScript("OnEnter", function(self)
        GameTooltip:SetOwner(self, "ANCHOR_LEFT")
        fillTooltip(GameTooltip)
        GameTooltip:Show()
    end)
    button:SetScript("OnLeave", function() GameTooltip:Hide() end)
    place()
    button:SetShown(not settings.hide)
    return button
end

local minimap = {}
ns.Minimap = minimap

function minimap.Init()
    local settings = T3WoWDB.minimap or {}
    T3WoWDB.minimap = settings
    local LDB = LibStub and LibStub:GetLibrary("LibDataBroker-1.1", true)
    local DBIcon = LibStub and LibStub:GetLibrary("LibDBIcon-1.0", true)
    if LDB and DBIcon then
        local launcher = LDB:GetDataObjectByName(LDB_NAME) or LDB:NewDataObject(LDB_NAME, {
            type = "launcher",
            label = LDB_NAME,
            icon = ICON,
            OnClick = onClick,
            OnTooltipShow = fillTooltip,
        })
        if not DBIcon:IsRegistered(LDB_NAME) then DBIcon:Register(LDB_NAME, launcher, settings) end
        minimap.Show = function(shown)
            settings.hide = not shown
            if shown then DBIcon:Show(LDB_NAME) else DBIcon:Hide(LDB_NAME) end
        end
    else
        local button = ownButton(settings)
        minimap.Show = function(shown)
            settings.hide = not shown
            button:SetShown(shown)
        end
    end
end

-- ---------------------------------------------------------------- probe
-- Looks for a file type whose change the game notices while it runs (the sound check
-- doesn't work on every client). For each type it compares blank and real files, then
-- asks the companion to turn the blank "flip" file real and watches for that.

local PROBE = "Interface\\AddOns\\T3WoW\\ctl\\probe\\"
local CHECK_TIMEOUT, WATCH_SECONDS = 2, 20
-- Textures may only load while drawn, so the probe frame is a speck on screen.
local probeFrame = CreateFrame("Frame", nil, UIParent)
probeFrame:SetSize(2, 2)
probeFrame:SetPoint("BOTTOMRIGHT")
probeFrame:SetAlpha(0.05)
probeFrame:Hide()
local probeText = probeFrame:CreateFontString(nil, "OVERLAY")
local probeSize = 10

local function say(text) print("|cff8f8fffT3 probe|r " .. text) end

-- begin(path) starts one check and returns a function that answers true or false once
-- it knows, nil while it doesn't; checks still undecided after CHECK_TIMEOUT count as false.
local function nextSize()
    probeSize = probeSize % 40 + 10 -- a new size each time, in case the game caches by size
    return probeSize
end
-- A font another addon ships and the game is known to load: if a method says "no" to it,
-- the method is what's broken, not our files.
local KNOWN_FONT = "Interface\\AddOns\\Details\\fonts\\Expressway.TTF"
local FONT_REALS = { "valid", "valid2", KNOWN_FONT }

-- Each font check gets its own font string so a slow load can't be overwritten.
local function fontString(size)
    local text = probeFrame:CreateFontString(nil, "OVERLAY")
    text:SetPoint("BOTTOMRIGHT")
    text:SetFont(STANDARD_TEXT_FONT, size, "")
    text:SetText("MWMWMWMW iiii 0000")
    return text
end

local METHODS = {
    { name = "font (return value)", ext = "ttf", reals = FONT_REALS, begin = function(path)
        local ok, result = pcall(probeText.SetFont, probeText, path, nextSize(), "")
        return function() return ok and result and true or false end
    end },
    { name = "font (GetFont, waits)", ext = "ttf", reals = FONT_REALS, begin = function(path)
        local text = fontString(nextSize())
        local before = text:GetFont()
        pcall(text.SetFont, text, path, probeSize, "")
        return function()
            local after = text:GetFont()
            if after ~= nil and after ~= before then return true end
        end
    end },
    { name = "font (text width, waits)", ext = "ttf", reals = FONT_REALS, begin = function(path)
        local size = nextSize()
        local text = fontString(size)
        local before = text:GetStringWidth()
        pcall(text.SetFont, text, path, size, "")
        return function()
            local width = text:GetStringWidth()
            if width and width > 0 and math.abs(width - before) > 0.5 then return true end
        end
    end },
    { name = "texture (loaded)", ext = "tga", reals = { "valid" }, begin = function(path)
        local texture = probeFrame:CreateTexture(nil, "OVERLAY")
        texture:SetAllPoints()
        texture:SetTexture(path)
        return function()
            if not texture.IsObjectLoaded then return false end
            if texture:IsObjectLoaded() then return true end
        end
    end },
}

local function yesNo(value) return value and "|cff40c040yes|r" or "|cffff4040no|r" end

local pending = {} -- { check, startedAt, done(result) }
local watch -- per-frame hook while watching the flip files

local function begin(method, file, done)
    local path = file:find("\\", 1, true) and file or (PROBE .. file .. "." .. method.ext)
    pending[#pending + 1] = { check = method.begin(path), at = GetTime(), done = done }
end

probeFrame:SetScript("OnUpdate", function(self)
    local now = GetTime()
    for index = #pending, 1, -1 do
        local item = pending[index]
        local result = item.check()
        if result == nil and now - item.at > CHECK_TIMEOUT then result = false end
        if result ~= nil then
            table.remove(pending, index)
            item.done(result)
        end
    end
    if watch then watch(now) end
    if #pending == 0 and not watch then self:Hide() end
end)

local function watchFlips(candidates)
    say(format("asking the companion to make the flip files real; watching for %ds…", WATCH_SECONDS))
    ns.Link.Send({ t = "probe" })
    local started, nextCheck, seen = GetTime(), 0, {}
    watch = function(now)
        local waited = now - started
        if waited >= nextCheck and waited <= WATCH_SECONDS then
            nextCheck = waited + 0.5
            for _, method in ipairs(candidates) do
                if not seen[method] then
                    begin(method, "flip", function(result)
                        if result and not seen[method] then
                            seen[method] = true
                            say(format("|cff40c040%s noticed the change after %.1fs.|r", method.name, GetTime() - started))
                        end
                    end)
                end
            end
        end
        if waited > WATCH_SECONDS + CHECK_TIMEOUT then
            watch = nil
            for _, method in ipairs(candidates) do
                if not seen[method] then say(method.name .. " never noticed the change.") end
            end
            say("done. Paste these lines to Claude.")
        end
    end
end

local function runProbe()
    if #pending > 0 or watch then return say("already running.") end
    say("checking blank vs real files…")
    local candidates, left = {}, #METHODS
    for _, method in ipairs(METHODS) do
        local results, waiting = {}, 2 + #method.reals
        local function finished()
            waiting = waiting - 1
            if waiting > 0 then return end
            local reals = {}
            local anyReal = false
            for _, name in ipairs(method.reals) do
                local known = name == KNOWN_FONT
                reals[#reals + 1] = (known and "Details font" or name) .. " " .. yesNo(results[name])
                if not known then anyReal = anyReal or results[name] end
            end
            say(format("%s: blank %s, real (%s), flip file now %s", method.name, yesNo(results.empty),
                table.concat(reals, ", "), yesNo(results.flip)))
            if not results.empty and anyReal and not results.flip then candidates[#candidates + 1] = method end
            left = left - 1
            if left > 0 then return end
            if #candidates == 0 then
                say("no method tells a blank file from a real one (if a flip file was already real, restart WoW to reset it). Paste these lines to Claude.")
            else
                watchFlips(candidates)
            end
        end
        for _, file in ipairs({ "empty", "flip", unpack(method.reals) }) do
            begin(method, file, function(result)
                results[file] = result
                finished()
            end)
        end
    end
    probeFrame:Show()
end

-- ---------------------------------------------------------------- welcome
-- The first time the addon loads (for this account), a short how-to in chat; after an
-- update, one line saying so. T3WoWDB.seenVersion remembers which version said hello.

local function addonVersion()
    local get = (C_AddOns and C_AddOns.GetAddOnMetadata) or GetAddOnMetadata
    return get and get(addonName, "Version") or "?"
end

local function welcome()
    local version = addonVersion()
    local seen = T3WoWDB.seenVersion
    T3WoWDB.seenVersion = version
    local tag = "|cff8f8fffT3 WoW|r"
    if not seen then
        print(tag .. " " .. version .. " is installed: T3 Code inside the game.")
        print(tag .. " Type |cffffd100/t3|r or click the minimap button to open it. Right-click the button (or |cffffd100/t3 settings|r) for notifications and other settings.")
        print(tag .. " It needs the T3 WoW companion (t3wow.exe) running in the system tray on this PC. |cffffd100/t3 status|r shows the connection.")
    elseif seen ~= version then
        print(tag .. " updated to " .. version .. ". Type |cffffd100/t3|r to open it.")
    else
        print(tag .. " loaded. Type |cffffd100/t3|r to open it.")
    end
end

-- ---------------------------------------------------------------- startup

local events = CreateFrame("Frame")
events:RegisterEvent("ADDON_LOADED")
events:RegisterEvent("PLAYER_LOGIN")
events:SetScript("OnEvent", function(self, event, name)
    if event == "PLAYER_LOGIN" then
        -- Every non-load-on-demand addon (and the libraries they embed) is loaded by now.
        minimap.Init()
        welcome()
        return
    end
    if name ~= addonName then return end
    self:UnregisterEvent("ADDON_LOADED")
    ns.Link.Init()
end)

SLASH_T3WOW1 = "/t3"
SlashCmdList.T3WOW = function(message)
    if message == "probe" then return runProbe() end
    if message == "settings" then return ns.UI.ShowSettings() end
    if message == "trace" then
        local lines = ns.Link.Trace()
        for i = math.max(1, #lines - 30), #lines do print("|cff8f8fffT3|r " .. lines[i]) end
        return
    end
    if message == "minimap" then
        local shown = T3WoWDB.minimap and T3WoWDB.minimap.hide
        if minimap.Show then minimap.Show(shown and true or false) end
        print("|cff8f8fffT3|r minimap button " .. (shown and "shown" or "hidden") .. ".")
        return
    end
    if message == "status" then
        local s = ns.Link.Status()
        print(string.format("|cff8f8fffT3|r signals %s, watching signal %d, next slot %d, queued %d, last update %s",
            s.signals == "on" and (s.nextSignal and "on" or "on (waiting for the first update)")
                or (s.signals == "checking" and "checking" or "off (polling)"),
            s.nextSignal or 0, s.nextSlot, s.queue,
            s.inboxAge and string.format("%ds ago", s.inboxAge) or "never"))
        print(string.format("|cff8f8fffT3|r signals noticed this session: %d%s (%d checks so far)", s.signalsSeen,
            s.signalAge and string.format(", last %ds ago", s.signalAge) or "", s.fontStrings or 0))
        if s.signalCheck then print("|cff8f8fffT3|r signals off: " .. s.signalCheck .. "; checking for updates on a timer instead") end
        if s.loadError then print("|cff8f8fffT3|r slot load error: " .. tostring(s.loadError)) end
        if #s.recentAlerts == 0 then
            print("|cff8f8fffT3|r no thread has finished since this UI session started")
        end
        for _, alert in ipairs(s.recentAlerts) do
            print(string.format("|cff8f8fffT3|r %s %s (%s): arrived %ds after, via %s; %s",
                date("%H:%M:%S", alert.happened), alert.title ~= "" and alert.title or "Thread", alert.status,
                math.max(0, alert.arrived - alert.happened), alert.via, alert.result))
        end
        return
    end
    ns.UI.Toggle()
end
