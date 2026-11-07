-- The two doors out of the sandbox.
-- Out: commands drawn as a strip of colored 4px cells at the screen's top-left; the
--      companion screen-captures and decodes it.
-- In:  the companion writes state into load-on-demand slot addons and flips
--      sig/NNNN.wav from empty to playable; seeing it playable, we load the next slot.
local _, ns = ...
local Codec = ns.Codec
local Link = {}
ns.Link = Link

local SLOT_COUNT, SIGNAL_COUNT = 200, 2000
local ROOT = "Interface\\AddOns\\T3WoW\\"
local ACK_PATIENCE = 20

local floor, format = math.floor, string.format
local db
local state = {
    nextSlot = 1,
    nextSignal = nil,
    signalsOk = false,
    inbox = nil,
    inboxAt = nil,
    loadError = nil,
    pollAt = 0,
    queueSince = nil,
}
Link.state = state

local LoadAddOnFn = (C_AddOns and C_AddOns.LoadAddOn) or LoadAddOn

local function randomHex(digits)
    local out = {}
    for i = 1, digits do out[i] = format("%x", math.random(0, 15)) end
    return table.concat(out)
end

local function playable(path)
    local ok, willPlay, handle = pcall(PlaySoundFile, path, "Master")
    if ok and willPlay and handle then pcall(StopSound, handle) end
    return ok and willPlay and true or false
end

local function signalPath(n) return format("%ssig\\%04d.wav", ROOT, n) end

-- Signals the companion raised in this client process stay playable, and it raises
-- them in order, so the playable ones form a prefix we can binary-search.
local function findNextSignal()
    local lo, hi = 0, SIGNAL_COUNT
    while lo < hi do
        local mid = floor((lo + hi + 1) / 2)
        if playable(signalPath(mid)) then lo = mid else hi = mid - 1 end
    end
    return lo + 1
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

local frameSeq = 0
function Link.Render()
    if not db or #db.queue == 0 then
        strip:Hide()
        state.queueSince = nil
        return
    end
    state.queueSince = state.queueSince or GetTime()
    local header = { k = db.client, u = state.uiSession, n = state.nextSignal or 0, l = state.nextSlot }
    local commands, payload = {}, nil
    for _, command in ipairs(db.queue) do
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
        table.remove(db.queue, 1)
        ns.UI.Notice("That message is too long to send from the game.")
        return Link.Render()
    end
    frameSeq = frameSeq + 1
    renderStrip(Codec.Encode(frameSeq, payload))
end

function Link.Send(command)
    db.seq = db.seq + 1
    command.s = db.seq
    db.queue[#db.queue + 1] = command
    Link.Render()
end

function Link.Pending() return db and #db.queue or 0 end

function Link.Stalled()
    return state.queueSince ~= nil and GetTime() - state.queueSince > ACK_PATIENCE
end

-- ---------------------------------------------------------------- inbox

local function receive(inbox)
    if inbox.k == db.client and type(inbox.ack) == "number" then
        local kept = {}
        for _, command in ipairs(db.queue) do
            if command.s > inbox.ack then kept[#kept + 1] = command end
        end
        if #kept ~= #db.queue then
            db.queue = kept
            state.queueSince = nil
            Link.Render()
        end
    end
    state.inbox = inbox
    state.inboxAt = GetTime()
    for _, event in ipairs(inbox.events or {}) do
        if event.n > (db.lastEvent or 0) then
            db.lastEvent = event.n
            ns.UI.Alert(event)
        end
    end
    ns.UI.OnInbox(inbox)
end

local function loadNextSlot()
    if state.nextSlot > SLOT_COUNT then
        state.exhausted = true
        ns.UI.Refresh()
        return
    end
    local name = format("T3WoW_S%03d", state.nextSlot)
    state.nextSlot = state.nextSlot + 1
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
    if type(inbox) == "table" then receive(inbox) end
end

function Link.Exhausted() return state.exhausted end

local elapsed = 0
local poller = CreateFrame("Frame")
poller:SetScript("OnUpdate", function(_, delta)
    elapsed = elapsed + delta
    if elapsed < 0.25 or not db then return end
    elapsed = 0
    if state.signalsOk then
        if state.nextSignal <= SIGNAL_COUNT and playable(signalPath(state.nextSignal)) then
            state.nextSignal = state.nextSignal + 1
            loadNextSlot()
            if #db.queue > 0 then Link.Render() end
        end
        return
    end
    -- No sound signal on this client: poll slots on a schedule instead.
    local now = GetTime()
    if now < state.pollAt then return end
    local interval = (#db.queue > 0 and 3) or (ns.UI.IsShown() and 10) or 60
    state.pollAt = now + interval
    loadNextSlot()
end)

function Link.Init()
    T3WoWDB = T3WoWDB or {}
    db = T3WoWDB
    db.client = db.client or randomHex(12)
    db.seq = db.seq or 0
    db.queue = db.queue or {}
    state.uiSession = format("%x%s", time(), randomHex(4))
    state.signalsOk = type(PlaySoundFile) == "function"
        and not playable(ROOT .. "ctl\\empty.wav")
        and playable(ROOT .. "ctl\\valid.wav")
    state.nextSignal = state.signalsOk and findNextSignal() or 1
    Link.Send({ t = "hello", o = false })
end

function Link.Status()
    return {
        signalsOk = state.signalsOk,
        nextSignal = state.nextSignal,
        nextSlot = state.nextSlot,
        queue = db and #db.queue or 0,
        loadError = state.loadError,
        inboxAge = state.inboxAt and (GetTime() - state.inboxAt) or nil,
    }
end
