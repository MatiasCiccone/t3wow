-- The two doors out of the sandbox.
-- Out: commands drawn as a strip of colored 4px cells at the screen's top-left; the
--      companion screen-captures and decodes it.
-- In:  the companion writes state into load-on-demand slot addons and turns the blank
--      font sig/NNNN.ttf into a real one; seeing that font load, we load the next slot.
--      Fonts load a moment after SetFont, so every check is started, then read later.
--      The game seems to give up on a font file it has tried a few dozen times, so we
--      never watch one file for long: while waiting we move to the next signal file every
--      SIGNAL_PERIOD seconds, timed from the update that named the starting file. The
--      companion runs on this PC, shares the clock, and raises the files around ours.
--      Pictures come the same way: the companion draws them into blank image slot files
--      (img/NNNN.tga) and names the slot in an update; the game loads one filled while it
--      runs, as long as it never asked for it while blank (see checkImages).
local _, ns = ...
local Codec = ns.Codec
local Link = {}
ns.Link = Link

local SLOT_COUNT, SIGNAL_COUNT = 200, 4000
-- From this many updates used, the window says to Reload UI soon.
local SLOT_WARN = 190
local SIGNALS_OUT = "this WoW session used all its signal files; restart WoW for updates within a second"
local SIGNAL_PERIOD = 25 -- keep in step with companion/src/bridge.rs
local ROOT = "Interface\\AddOns\\T3WoW\\"
local ACK_PATIENCE = 20
-- Without signals the companion can't see which slot we load next unless we tell it,
-- so after each polled load the strip shows a header-only frame this long.
local SYNC_SECONDS = 1.5
-- How often to start a new check of the watched signal file: often right after any
-- activity, less once things have been quiet for QUIET_AFTER seconds (each check costs a
-- font string that is never freed).
local RECHECK_SECONDS, RECHECK_QUIET_SECONDS, QUIET_AFTER = 0.75, 2.5, 60
-- A command waiting this long also gets a slot loaded directly, in case a signal was missed.
local NUDGE_AFTER = 6
-- Seconds between loads for a waiting command while nothing answers, by how many loads
-- in a row went unanswered: as a nudge in signal mode, and on the timer. Once the
-- companion counts as not answering (ACK_PATIENCE), nudges stop and the timer slows down.
local NUDGE_BACKOFF, QUEUE_BACKOFF, STALLED_SECONDS = { 5, 10, 20, 60 }, { 3, 3, 10, 20, 60 }, 60

local floor, format = math.floor, string.format
local db
-- The client key, command count and queue: file-local while the game runs, so another
-- addon can't add commands to the queue through the saved variables table (it can still
-- draw a strip of its own: the companion's tray setting "Commands from the game" caps
-- what any command can do). They go back into T3WoWDB at logout, which /reload also
-- fires, so they keep across sessions.
local own
local state = {
    nextSlot = 1,
    signalBase = nil, -- first signal file of the current wait (inbox.ns)
    signalTime = nil, -- when that wait started, epoch seconds (inbox.nt)
    adoptedAt = 0, -- inbox.nt of the update we took signalBase from
    signals = "checking", -- "checking", "on" or "off"
    capability = nil,
    recentAlerts = {}, -- newest first, for /t3 status
    trace = {},
    inbox = nil,
    inboxAt = nil,
    loadError = nil,
    pollAt = 0,
    unanswered = 0, -- slot loads in a row that brought no update from the companion
    queueSince = nil,
    syncUntil = 0,
    signalCheck = nil,
}
Link.state = state

-- ---------------------------------------------------------------- trace
-- What the link did, newest last, for debugging: /t3 trace prints it, and it is saved
-- with the addon's saved variables (on /reload or logout) as T3WoWDB.trace.
local TRACE_MAX = 120
local function trace(text)
    local t = state.trace
    t[#t + 1] = format("%s.%d %s", date("%H:%M:%S"), floor((GetTime() % 1) * 10), text)
    if #t > TRACE_MAX then table.remove(t, 1) end
end
Link.Trace = function() return state.trace end


local LoadAddOnFn = (C_AddOns and C_AddOns.LoadAddOn) or LoadAddOn

local function randomHex(digits)
    local out = {}
    for i = 1, digits do out[i] = format("%x", math.random(0, 15)) end
    return table.concat(out)
end

local function signalPath(n) return format("%ssig\\%04d.ttf", ROOT, n) end

-- Font checks. A blank font file never loads; a real one loads shortly after SetFont,
-- and the font string's GetFont() then names it. Fonts only load for drawn text, so the
-- font strings sit in a near-invisible speck.
-- A font string that once failed to load a font never loads one later (seen in traces:
-- reused font strings missed real signal files for minutes, fresh ones saw them at
-- once), so every check gets a new font string. They can't be destroyed; finished ones
-- are hidden and emptied, and checks slow down when nothing is happening.
-- A real font can take a moment to load, so the last FONT_RECENT checks keep being read.
local FONT_RECENT = 12
-- Font strings are never freed, so a UI session makes at most this many checks; past it,
-- slots load on the timer until the next Reload UI.
local FONT_BUDGET = 20000
local FONT_BUDGET_OUT = "this UI session made its 20000 font checks; Reload UI to watch signals again"
local fontHolder = CreateFrame("Frame", nil, UIParent)
fontHolder:SetSize(2, 2)
fontHolder:SetPoint("BOTTOMRIGHT")
fontHolder:SetAlpha(0.05)
local fontPool, fontSize = {}, 10 -- fontPool: recent checks, oldest first
state.fontStrings = 0

-- index: the signal file number this check is for (nil for other checks).
local function startFontCheck(path, index)
    local text = fontHolder:CreateFontString(nil, "OVERLAY")
    state.fontStrings = state.fontStrings + 1
    text:SetPoint("BOTTOMRIGHT")
    fontSize = fontSize % 40 + 10 -- a new size each time, in case the game caches by size
    text:SetFont(STANDARD_TEXT_FONT, fontSize, "")
    text:SetText("MW")
    local check = { text = text, before = text:GetFont(), index = index, at = GetTime() }
    pcall(text.SetFont, text, path, fontSize, "")
    fontPool[#fontPool + 1] = check
    if #fontPool > FONT_RECENT then
        local old = table.remove(fontPool, 1)
        old.text:SetText("")
        old.text:Hide()
    end
    return check
end

local function fontLoaded(check)
    local after = check.text:GetFont()
    return after ~= nil and after ~= check.before
end

local function signalling() return state.signals == "on" and state.signalBase ~= nil end

-- The signal file to watch right now.
local function watchedSignal()
    if not state.signalBase then return nil end
    return state.signalBase + math.max(0, floor((time() - state.signalTime) / SIGNAL_PERIOD))
end

-- ---------------------------------------------------------------- strip

local strip = CreateFrame("Frame", nil, nil)
strip:SetFrameStrata("TOOLTIP")
strip:SetFrameLevel(10000)
strip:SetPoint("TOPLEFT", WorldFrame, "TOPLEFT", 0, 0)
strip:SetSize(Codec.CELLS_PER_ROW * Codec.CELL_PX, Codec.CELL_PX)
strip:Hide()
local cellTextures = {}

local function renderStrip(cells)
    local _, physicalHeight = GetPhysicalScreenSize()
    strip:SetScale(768 / physicalHeight)
    local px, perRow = Codec.CELL_PX, Codec.CELLS_PER_ROW
    for i, value in ipairs(cells) do
        local texture = cellTextures[i]
        if not texture then
            texture = strip:CreateTexture(nil, "OVERLAY")
            texture:SetSize(px, px)
            texture:SetPoint("TOPLEFT", strip, "TOPLEFT", ((i - 1) % perRow) * px, -floor((i - 1) / perRow) * px)
            cellTextures[i] = texture
        end
        texture:SetColorTexture(Codec.Color(value))
        texture:Show()
    end
    for i = #cells + 1, #cellTextures do cellTextures[i]:Hide() end
    strip:SetHeight(math.ceil(#cells / perRow) * px)
    strip:Show()
end

-- Whether the image slot files were there when the game started (it only finds files that
-- existed then): a blank sentinel next to them has a file id if so, none if not. Asking for
-- the sentinel is safe, since it stays blank; a slot itself is only asked for once an
-- update names it, filled.
local IMAGE_SENTINEL = ROOT .. "img\\here.tga"
local imageCheck
local function checkImages()
    if state.images then return end
    if not imageCheck then
        local holder = CreateFrame("Frame", nil, UIParent)
        holder:SetSize(1, 1)
        holder:SetPoint("BOTTOMRIGHT")
        holder:SetAlpha(0.01)
        imageCheck = holder:CreateTexture(nil, "BACKGROUND")
        imageCheck:SetAllPoints()
        imageCheck:SetTexture(IMAGE_SENTINEL)
        imageCheck.holder = holder
    end
    local known = imageCheck.GetTextureFilePath and imageCheck:GetTextureFilePath()
    if known ~= nil and known ~= "" then
        state.images = true
        imageCheck.holder:Hide()
        trace("image slots found: pictures on")
        -- Tell the companion now, even with nothing to send.
        state.syncUntil = math.max(state.syncUntil, GetTime() + SYNC_SECONDS)
        Link.Render()
    end
end
function Link.Images() return state.images == true end

local frameSeq = 0
function Link.Render()
    if not own then return strip:Hide() end
    if #own.queue == 0 then
        state.queueSince = nil
        if GetTime() >= state.syncUntil then return strip:Hide() end
    else
        state.queueSince = state.queueSince or GetTime()
    end
    local header = { k = own.client, u = state.uiSession, n = watchedSignal() or 0, l = state.nextSlot,
        p = (not signalling()) and 1 or nil, lid = db.login, im = state.images and 1 or nil, dc = 1 }
    -- An empty queue still sends a header-only sync frame.
    local commands = {}
    local payload = (#own.queue == 0) and Codec.Json(header) or nil
    for _, command in ipairs(own.queue) do
        commands[#commands + 1] = command
        header.q = commands
        local candidate = Codec.Json(header)
        if #candidate > Codec.MAX_PAYLOAD then
            commands[#commands] = nil
            break
        end
        payload = candidate
    end
    if not payload then
        -- A single command that can never fit: drop it rather than block the queue.
        table.remove(own.queue, 1)
        ns.UI.Notice("That message is too long to send from the game.")
        return Link.Render()
    end
    frameSeq = frameSeq + 1
    renderStrip(Codec.Encode(frameSeq, payload))
end

function Link.Send(command)
    own.seq = own.seq + 1
    command.s = own.seq
    trace(format("send #%d %s", command.s, tostring(command.t)))
    state.lastSentAt = GetTime()
    own.queue[#own.queue + 1] = command
    -- On the timer, the answer comes with the next load: bring it forward.
    if not signalling() and not Link.Stalled() then state.pollAt = math.min(state.pollAt, GetTime() + QUEUE_BACKOFF[1]) end
    Link.Render()
    return command.s
end

-- Whether a command fits in a frame on its own. One that doesn't can never be sent, so
-- the window checks before it clears what you typed.
function Link.Fits(command)
    local alone = {}
    for key, value in pairs(command) do alone[key] = value end
    alone.s = (own and own.seq or 0) + 1
    local header = { k = own and own.client or "", u = state.uiSession, n = watchedSignal() or 0, l = state.nextSlot, p = 1,
        lid = db and db.login, im = state.images and 1 or nil, dc = 1, q = { alone } }
    return #Codec.Json(header) <= Codec.MAX_PAYLOAD
end

function Link.Pending() return own and #own.queue or 0 end

-- Whether an update was written for this game's client key, and (Current) for this UI
-- session: slots written ahead survive a crash or /reload, so an earlier session's
-- update can still load, and its command numbers mean other commands now.
function Link.IsMine(inbox) return own ~= nil and inbox.k == own.client end
function Link.Session() return state.uiSession end
function Link.IsCurrent(inbox) return Link.IsMine(inbox) and inbox.u ~= nil and inbox.u == state.uiSession end

-- The queue itself, for tests.
function Link.Queue() return own and own.queue end

function Link.Stalled()
    return state.queueSince ~= nil and GetTime() - state.queueSince > ACK_PATIENCE
end

-- ---------------------------------------------------------------- sign-in
-- Each update carries a stamp (lid) for the T3 sign-in it comes from. The companion gives
-- it a new one when it moves to another T3 (`t3wow.exe login --origin`) or signs out, and
-- blanks the slot files that held the previous one's threads. Seeing a new stamp, the
-- game drops everything it had from the previous one: threads, the open thread,
-- notifications, and commands still waiting (meant for that T3's threads and machines).
-- The stamp is kept in the saved variables (db.login, with db.loginAt, when the newest
-- update with it was written), so this holds across /reload and restarts too.

-- Commands that mean the same on any T3; keep in step with LOGIN_FREE in companion/src/bridge.rs.
local LOGIN_FREE = { hello = true, ui = true, probe = true, refresh = true }

-- Drops what came from the previous sign-in.
function Link.ForgetLogin()
    local kept = {}
    for _, command in ipairs(own.queue) do
        if LOGIN_FREE[command.t] then kept[#kept + 1] = command end
    end
    own.queue = kept
    if #kept == 0 then state.queueSince = nil end
    state.inbox, state.recentAlerts = nil, {}
    ns.UI.ForgetLogin()
    Link.Render()
end

-- Whether to take this update, given the sign-in it comes from; a new sign-in's first
-- update drops the previous one's things first.
local function takeLogin(inbox)
    local lid = inbox.lid
    if type(lid) ~= "string" then return true end -- an older companion: no stamps
    local now = type(inbox.now) == "number" and inbox.now or 0
    if lid == db.login then
        db.loginAt = math.max(db.loginAt or 0, now)
        return true
    end
    -- Another stamp, older than an update we have from ours, and not written for this UI
    -- session: a slot left over from before the companion moved. Not shown.
    if inbox.u ~= state.uiSession and type(db.loginAt) == "number" and now < db.loginAt then
        trace(format("  update for another sign-in (%s), older than ours: ignored", lid))
        return false
    end
    local previous = db.login
    db.login, db.loginAt = lid, now
    if previous ~= nil then
        trace(format("  the companion moved to another T3 sign-in (%s): dropping the previous one's threads and commands", lid))
        Link.ForgetLogin()
    end
    return true
end

-- ---------------------------------------------------------------- inbox

local function receive(inbox)
    if not takeLogin(inbox) then return end
    -- An update written for an earlier UI session (u) carries that session's ack, which
    -- can be past this session's command numbers after a crash: leave the queue alone.
    if inbox.k == own.client and type(inbox.ack) == "number" and (inbox.u == nil or inbox.u == state.uiSession) then
        local kept = {}
        for _, command in ipairs(own.queue) do
            if command.s > inbox.ack then kept[#kept + 1] = command end
        end
        if #kept ~= #own.queue then
            trace(format("  acked to #%d, %d command(s) left", inbox.ack, #kept))
            own.queue = kept
            state.queueSince = nil
            Link.Render()
        end
    end
    -- Each update names where the next wait starts (ns) and when (nt). Slots are written a
    -- few ahead, so an older update can arrive after a newer one: only move forward. Only
    -- this UI session's (or an older companion's, which sends no u): one left in a slot by
    -- an earlier session can come from an earlier game process, whose companion counted
    -- signal files far past where it counts now. Until then, slots load on the timer.
    local adopted = type(inbox.ns) == "number" and type(inbox.nt) == "number" and inbox.nt > state.adoptedAt
        and (inbox.u == nil or inbox.u == state.uiSession)
    if adopted then
        state.signalBase, state.signalTime, state.adoptedAt = inbox.ns, inbox.nt, inbox.nt
    end
    trace(format("  update from %s: wait %s @%s (%s), %d event(s)", date("%H:%M:%S", inbox.now or 0), tostring(inbox.ns),
        inbox.nt and date("%H:%M:%S", inbox.nt) or "?",
        adopted and "taken" or (inbox.u ~= nil and inbox.u ~= state.uiSession) and "another UI session's, ignored"
            or "older, ignored", #(inbox.events or {})))
    -- The companion ran out of signal files for this game process: load slots on a timer.
    if type(inbox.sigoff) == "string" and inbox.u == state.uiSession and state.signals ~= "off" then
        state.signals, state.signalCheck = "off", inbox.sigoff
        trace("companion: signals off (" .. inbox.sigoff .. ")")
    end
    state.inbox = inbox
    state.inboxAt = GetTime()
    -- An answer from the companion: loads on the timer can go back to their quick pace.
    if inbox.u == nil or inbox.u == state.uiSession then state.unanswered = 0 end
    -- Nothing seen yet (a fresh install): only the last minute's events are news, not
    -- everything the companion still lists.
    if db.lastEvent == nil then db.lastEvent = ((inbox.now or time()) - 60) * 1000 end
    for _, event in ipairs(inbox.events or {}) do
        if event.n > (db.lastEvent or 0) then
            db.lastEvent = event.n
            local result = ns.UI.Alert(event)
            -- Kept for /t3 status: when it happened, how late and how it got here, what we did.
            table.insert(state.recentAlerts, 1, {
                title = event.t, status = event.s, happened = floor(event.n / 1000), arrived = time(),
                via = state.loadVia or "?", result = result or "?",
            })
            state.recentAlerts[6] = nil
        end
    end
    ns.UI.OnInbox(inbox)
end

-- Says once in chat when the session's updates run low, and when they run out.
local function noteBudget()
    local used = state.nextSlot - 1
    local tag = "|cff8f8fffT3 WoW|r "
    if used >= SLOT_COUNT then
        state.exhausted = true
        if not state.exhaustedSaid then
            state.exhaustedSaid = true
            print(tag .. "|cffff8000used all 200 updates of this UI session: no more notifications until you Reload UI (/reload).|r")
        end
    elseif used >= SLOT_WARN and not state.lowSaid then
        state.lowSaid = true
        print(tag .. format("|cffffd100%d of 200 updates used this UI session: Reload UI (/reload) soon to keep getting notifications.|r", used))
    end
end

-- via: how this load was triggered ("a signal", "the timer", "the fallback for a waiting command").
local function loadNextSlot(via)
    state.loadVia = via
    if state.nextSlot > SLOT_COUNT then
        state.exhausted = true
        ns.UI.Refresh()
        return
    end
    local name = format("T3WoW_S%03d", state.nextSlot)
    state.nextSlot = state.nextSlot + 1
    trace(format("load %s via %s", name, via or "?"))
    noteBudget()
    state.unanswered = state.unanswered + 1
    _G.T3WoW_Inbox = nil
    local loaded, reason = LoadAddOnFn(name)
    local inbox = _G.T3WoW_Inbox
    _G.T3WoW_Inbox = nil
    if not loaded then
        state.loadError = reason or "unknown"
        ns.UI.Refresh()
        return
    end
    state.loadError = nil
    if type(inbox) == "table" then receive(inbox) else trace("  slot was empty") end
end

function Link.Exhausted() return state.exhausted end

-- How many of the session's updates are used.
function Link.SlotsUsed() return math.min(SLOT_COUNT, state.nextSlot - 1), SLOT_COUNT end
function Link.SlotsLow() return state.nextSlot - 1 >= SLOT_WARN end

-- Decides once per UI session whether font signals work: the real test font must load
-- and the blank one must not. Right after the game starts (cold disk, every addon
-- loading) the test font can take a while, so a slow check is retried, never taken as
-- "fonts don't work"; only a blank font loading rules signals out. Until then slots
-- load on the timer.
-- A test font that never loads in this many tries means fonts can't signal here.
local CAPABILITY_TIMEOUT, CAPABILITY_RETRY, CAPABILITY_TRIES = 4, 3, 12
local function updateCapability(now)
    if state.signals ~= "checking" then return end
    local c = state.capability
    if not c then
        if now < (state.capabilityRetryAt or 0) then return end
        -- Started on the first drawn frame: fonts don't load during the loading screen.
        state.capabilityAttempts = (state.capabilityAttempts or 0) + 1
        state.capability = {
            empty = startFontCheck(ROOT .. "ctl\\probe\\empty.ttf"),
            valid = startFontCheck(ROOT .. "ctl\\probe\\valid.ttf"),
            at = now,
        }
        return
    end
    if fontLoaded(c.empty) then
        state.signals = "off"
        state.signalCheck = "this game client loads even a blank font file, so it can't tell when an update is ready"
        trace("font check: a blank font loaded, signals off")
    elseif fontLoaded(c.valid) and now - c.at > 0.5 then
        state.signals = "on"
        state.signalCheck = nil
        trace(format("font check passed (attempt %d, %.1fs)", state.capabilityAttempts, now - c.at))
    elseif now - c.at > CAPABILITY_TIMEOUT and state.capabilityAttempts >= CAPABILITY_TRIES then
        state.signals = "off"
        state.signalCheck = format("the test font never loaded in %d tries (Reinstall addon from the T3 WoW tray menu, then restart WoW)",
            state.capabilityAttempts)
        trace("font check: the test font never loaded, signals off")
    elseif now - c.at > CAPABILITY_TIMEOUT then
        trace(format("font check attempt %d: test font not loaded after %ds, retrying",
            state.capabilityAttempts, CAPABILITY_TIMEOUT))
        state.capability, state.capabilityRetryAt = nil, now + CAPABILITY_RETRY
        state.signalCheck = format("still checking that fonts load (attempt %d); until then updates come on a timer",
            state.capabilityAttempts)
    end
end

local elapsed = 0
local poller = CreateFrame("Frame")
poller:SetScript("OnUpdate", function(_, delta)
    elapsed = elapsed + delta
    if elapsed < 0.25 or not own then return end
    elapsed = 0
    local now = GetTime()
    if strip:IsShown() and #own.queue == 0 and now >= state.syncUntil then strip:Hide() end
    updateCapability(now)
    local watching = signalling()
    if watching ~= state.wasSignalling then
        -- Tell the companion right away: it raises signals only once it sees we watch them.
        state.wasSignalling = watching
        state.syncUntil = now + SYNC_SECONDS
        Link.Render()
    end
    if watching then
        -- Any recent check of a file in the current wait that has loaded means an update:
        -- the companion only makes files real when it publishes.
        for _, check in ipairs(fontPool) do
            if check.index and check.index >= state.signalBase and fontLoaded(check) then
                local index = check.index
                trace(format("signal %d noticed (that check started %.1fs ago; %d check(s) of it; base was %d)",
                    index, now - check.at, state.checksOfWatched or 0, state.signalBase))
                for _, other in ipairs(fontPool) do other.index = nil end
                state.lastCheckAt, state.lastSignalAt, state.signalsSeen = nil, now, (state.signalsSeen or 0) + 1
                -- The update we load names the next wait; until then, assume the companion
                -- moved past every file it raised for this one (ours and its neighbours).
                state.signalBase, state.signalTime = index + 2, time()
                loadNextSlot("a signal")
                if #own.queue > 0 then Link.Render() end
                return
            end
        end
        local watched = watchedSignal()
        if watched > SIGNAL_COUNT then
            -- Past the last signal file: nothing more can signal us in this game process.
            state.signals, state.signalCheck = "off", SIGNALS_OUT
            trace("signals off: past the last signal file")
            return
        end
        if watched ~= state.lastWatched then
            trace(format("watch %d (base %d @%s)%s", watched, state.signalBase, date("%H:%M:%S", state.signalTime),
                state.lastWatched and format("; %d check(s) of %d", state.checksOfWatched or 0, state.lastWatched) or ""))
            state.lastWatched, state.checksOfWatched = watched, 0
        end
        local quiet = #own.queue == 0 and now - math.max(state.inboxAt or 0, state.lastSentAt or 0) > QUIET_AFTER
        local every = quiet and RECHECK_QUIET_SECONDS or RECHECK_SECONDS
        if state.fontStrings >= FONT_BUDGET then
            state.signals, state.signalCheck = "off", FONT_BUDGET_OUT
            trace("signals off: font check budget used")
            return
        end
        if watched <= SIGNAL_COUNT and (not state.lastCheckAt or now - state.lastCheckAt >= every) then
            startFontCheck(signalPath(watched), watched)
            state.lastCheckAt = now
            state.checksOfWatched = (state.checksOfWatched or 0) + 1
        end
        -- A companion that isn't answering at all won't answer a nudge either.
        if not (state.queueSince and now - state.queueSince > NUDGE_AFTER) or Link.Stalled() then return end
    end
    -- No signal yet (or none on this client, or a command waits too long): load slots on
    -- a schedule, and tell the companion where we are after each one. Loads nothing
    -- answers come further and further apart, so a companion that isn't running doesn't
    -- use up the session's slots.
    if now < state.pollAt then return end
    local nudge = signalling()
    loadNextSlot(nudge and "the fallback for a waiting command" or "the timer")
    local function backoff(steps) return steps[math.max(1, math.min(state.unanswered, #steps))] end
    local interval
    if nudge then
        interval = backoff(NUDGE_BACKOFF)
    elseif #own.queue > 0 and not Link.Stalled() then
        interval = backoff(QUEUE_BACKOFF)
    elseif #own.queue > 0 then
        interval = STALLED_SECONDS
    else
        interval = ns.UI.IsShown() and 10 or 60
    end
    state.pollAt = GetTime() + interval
    state.syncUntil = GetTime() + SYNC_SECONDS
    Link.Render()
end)

function Link.Init()
    T3WoWDB = T3WoWDB or {}
    db = T3WoWDB
    own = {
        client = type(db.client) == "string" and db.client or randomHex(12),
        seq = type(db.seq) == "number" and db.seq or 0,
        queue = type(db.queue) == "table" and db.queue or {},
    }
    db.client, db.seq, db.queue = nil, nil, nil
    -- Keep the previous UI session's record too: problems often show up just before a /reload.
    db.previousTrace = db.trace
    db.trace = state.trace
    state.uiSession = format("%x%s", time(), randomHex(4))
    trace("UI session " .. state.uiSession .. " started")
    checkImages()
    -- The file id may come a moment later: look again.
    if not state.images then C_Timer.After(1, checkImages) end
    Link.Send({ t = "hello", o = false })
end

-- Back into the saved variables, which the game writes right after this event. A crash
-- skips it, and the file keeps what the last logout or /reload wrote (the companion
-- copes with command numbers that went back).
local saver = CreateFrame("Frame")
saver:RegisterEvent("PLAYER_LOGOUT")
saver:SetScript("OnEvent", function()
    if own and db then db.client, db.seq, db.queue = own.client, own.seq, own.queue end
end)

function Link.Status()
    return {
        signals = state.signals,
        nextSignal = watchedSignal(),
        nextSlot = state.nextSlot,
        slotsUsed = math.min(SLOT_COUNT, state.nextSlot - 1),
        queue = own and #own.queue or 0,
        loadError = state.loadError,
        signalCheck = state.signalCheck,
        inboxAge = state.inboxAt and (GetTime() - state.inboxAt) or nil,
        signalsSeen = state.signalsSeen or 0,
        fontStrings = state.fontStrings,
        signalAge = state.lastSignalAt and (GetTime() - state.lastSignalAt) or nil,
        recentAlerts = state.recentAlerts,
    }
end
