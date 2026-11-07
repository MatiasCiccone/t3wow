local addonName, ns = ...

BINDING_HEADER_T3WOW = "T3 Code"
BINDING_NAME_T3WOW_TOGGLE = "Toggle T3 Code"
function T3WoW_Toggle() ns.UI.Toggle() end

local events = CreateFrame("Frame")
events:RegisterEvent("ADDON_LOADED")
events:SetScript("OnEvent", function(self, _, name)
    if name ~= addonName then return end
    self:UnregisterEvent("ADDON_LOADED")
    ns.Link.Init()
end)

SLASH_T3WOW1 = "/t3"
SlashCmdList.T3WOW = function(message)
    if message == "status" then
        local s = ns.Link.Status()
        print(string.format("|cff8f8fffT3|r signals %s, next signal %d, next slot %d, queued %d, last update %s",
            s.signalsOk and "on" or "off (polling)", s.nextSignal or 0, s.nextSlot, s.queue,
            s.inboxAge and string.format("%ds ago", s.inboxAge) or "never"))
        if s.loadError then print("|cff8f8fffT3|r slot load error: " .. tostring(s.loadError)) end
        return
    end
    ns.UI.Toggle()
end
