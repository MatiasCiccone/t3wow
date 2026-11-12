// The game's AddOns list shows T3 WoW as one row: the 200 slot addons come out of the
// list's rows (Link.lua), and stay in the game, loading as before.
import assert from "node:assert/strict";
import { test } from "node:test";
import { runGame } from "./game.js";

// The parts of the game's AddOns list the addon touches, after Blizzard_AddOnList and
// TreeListDataProvider.lua (WoW: Forever 1.60.1): a tree of { addonIndex } rows the list
// builds from scratch (AddonList_Update), slots grouped under T3WoW as the game groups them,
// handed to its scroll box with SetDataProvider.
const LIST = `
addons = { "Details", "T3WoW" }
for n = 1, 200 do addons[#addons + 1] = string.format("T3WoW_S%03d", n) end
addons[#addons + 1] = "T3WoW_Skins" -- someone else's addon, named like ours
C_AddOns.GetAddOnName = function(i) return addons[i] end
redraws = 0

local function node(provider, parent, data)
  local n = { nodes = {}, parent = parent, data = data, provider = provider }
  function n:GetData() return self.data end
  function n:Insert(data) local child = node(self.provider, self, data); table.insert(self.nodes, child); return child end
  function n:Remove(child, skipInvalidation)
    for i, c in ipairs(self.nodes) do
      if c == child then table.remove(self.nodes, i); if not skipInvalidation then self.provider:Invalidate() end; return c end
    end
  end
  return n
end
function CreateTreeDataProvider()
  local p = {}
  p.root = node(p, nil, nil)
  function p:Insert(data) return self.root:Insert(data) end
  function p:Invalidate() redraws = redraws + 1 end
  function p:EnumerateEntireRange()
    local all = {}
    local function walk(n) for _, c in ipairs(n.nodes) do all[#all + 1] = c; walk(c) end end
    walk(self.root)
    local i = 0
    return function() i = i + 1; if all[i] then return i, all[i] end end
  end
  return p
end

function buildList()
  local p = CreateTreeDataProvider()
  local parent
  for i, name in ipairs(addons) do
    if name:find("^T3WoW_S%d") then parent:Insert({ addonIndex = i })
    else local n = p:Insert({ addonIndex = i }); if name == "T3WoW" then parent = n end end
  end
  AddonList.ScrollBox:SetDataProvider(p)
end

function rows()
  local names = {}
  for _, n in AddonList.ScrollBox.provider:EnumerateEntireRange() do names[#names + 1] = addons[n:GetData().addonIndex] end
  return table.concat(names, ",")
end

function hooksecurefunc(t, method, post)
  local original = t[method]
  t[method] = function(...) local r = original(...); post(...); return r end
end

function makeList()
  AddonList = { ScrollBox = {} }
  function AddonList.ScrollBox:SetDataProvider(p) self.provider = p end
  function AddonList.ScrollBox:GetDataProvider() return self.provider end
end
`;

const SHOWN = "Details,T3WoW,T3WoW_Skins";

test("the AddOns list shows T3 WoW as one row, however often it rebuilds", () => {
  runGame(
    `
makeList()
buildList() -- already open when the addon loads
Link.Init()
assert(rows() == "${SHOWN}", rows())
redraws = 0
buildList() -- a key typed in its search box
assert(rows() == "${SHOWN}", rows())
assert(redraws == 1, "redraws " .. redraws)
buildList()
assert(rows() == "${SHOWN}", rows())
return "ok"
`,
    { before: LIST },
  );
});

test("an AddOns list that loads after the addon hides the slots too", () => {
  runGame(
    `
Link.Init()
makeList()
fire("ADDON_LOADED", "Blizzard_AddOnList")
buildList()
assert(rows() == "${SHOWN}", rows())
return "ok"
`,
    { before: LIST },
  );
});

test("hidden from the list, the slots still load", () => {
  runGame(
    `
makeList()
Link.Init()
buildList()
inboxes.T3WoW_S001 = { k = "c1", ack = 1, now = time() }
tick(1)
assert(loads[1] == "T3WoW_S001", tostring(loads[1]))
assert(Link.SlotsUsed() == 1, "used " .. Link.SlotsUsed())
return "ok"
`,
    { before: LIST },
  );
});
