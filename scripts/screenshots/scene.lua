-- What the screenshots show: made-up threads, fed to the real UI as the companion would.
local compact = SCENE == "compact-list"
local themeKey = ({ ["theme-flat"] = "flat", ["theme-classic"] = "classic" })[SCENE] or "t3"
T3WoWDB.settings.theme = themeKey
T3WoWDB.settings.compact = compact
T3WoWDB.settings.maxToasts = 3
-- The compact shot is the window alone: no notifications for the threads waiting on you.
T3WoWDB.settings.toasts = not compact
fire("PLAYER_LOGIN")

local now = time()
local ENVS = { { id = "local", name = "gaming-pc", ok = true, dm = "full-access" }, { id = "vps", name = "vps-1", ok = true, dm = "full-access" } }
local function ago(minutes) return now - minutes * 60 end
local function T(e, id, title, s, minutes, project, extra)
  local t = { e = e, id = id, t = title, s = s, set = false, at = ago(minutes), p = project }
  for k, v in pairs(extra or {}) do t[k] = v end
  return t
end

local THREADS = {
  T(1, "t1", "Add a pricing page", "completed", 1, "landing", { pr = "#128", ps = "open" }),
  T(1, "t2", "Pick a theme for the landing page", "waiting", 0, "landing", { pr = "#131", ps = "open" }),
  T(1, "t3", "Set up landing page assets", "approval", 1, "landing", { par = "t2" }),
  T(2, "t4", "Fix flaky Windows CI run", "completed", 23, "api", { pr = "#311", ps = "merged" }),
  T(1, "t5", "Show more paging for groups", "completed", 180, "dashboard", { prs = { { n = "#92", s = "open", u = "x" }, { n = "#94", s = "open", u = "x" } } }),
  T(2, "t6", "Sign release checksums", "failed", 360, "api"),
  T(1, "t7", "Trim the bundle size", "completed", 420, "dashboard"),
  T(1, "t8", "Keyboard shortcuts for the editor", "completed", 2880, "editor", { pr = "#57", ps = "closed" }),
  T(2, "t9", "Retry the billing webhook on 502", "completed", 4320, "api"),
  T(1, "t10", "Dark mode for the docs site", "completed", 5760, "docs"),
  T(1, "t11", "Refactor the polling loop", "running", 0, "dashboard", { st = ago(6) }),
  T(1, "t13", "Upgrade the test runner", "idle", 1440, "editor", { sn = true }),
  T(2, "t14", "Rotate the staging keys", "idle", 2880, "api", { sn = true }),
  T(1, "t15", "Write the launch post", "completed", 8640, "docs", { set = true }),
}

local ask = { k = "abcd1234", qs = {
  { h = "Colors", q = "Which color scheme should the landing page use?", m = false, o = {
    { l = "Dark", d = "Near-black surfaces with one bright accent" },
    { l = "Light", d = "White surfaces, for a brighter first look" },
  } },
} }

local ITEMS = {
  { r = "u", x = "Give the landing page a theme. Ask me about the color scheme first; don't build anything yet." },
  { r = "a", x = "Read 6 files", w = { p = 1, l = 6, k = "r" } },
  { r = "a", x = "Searched the codebase", w = { p = 7, l = 7, k = "s" } },
  { r = "a", x = "The page already has tokens for surfaces and text, so a theme is one file. One question before I start:" },
}

local CATALOG = {
  { e = 1, id = "claudeAgent", t = "Claude", m = { { id = "opus", t = "Claude Opus 5.5", o = "effort", ef = { "low", "high", "xhigh" } } } },
  { e = 2, id = "claudeAgent", t = "Claude", m = { { id = "opus", t = "Claude Opus 5.5", o = "effort", ef = { "low", "high", "xhigh" } } } },
}

local view = { e = 1, id = "t2", t = "Pick a theme for the landing page", s = "waiting", br = "feature/landing-theme", prs = { { n = "#131", s = "open", u = "x" } }, items = ITEMS, ask = ask,
  cfg = { pv = "claudeAgent", m = "opus", mt = "Claude Opus 5.5", ef = "high", rm = "full-access" } }

local function inbox(extra)
  local data = { k = "c1", ack = 0, envs = ENVS, threads = THREADS, projects = {}, notes = {}, events = {}, now = time(), u = Link.Session(),
    sel = true, models = CATALOG, more = { open = true }, settledTotal = 42, cv = "0.0.0" }
  for k, v in pairs(extra or {}) do data[k] = v end
  local q = Link.Queue()
  for i = #q, 1, -1 do q[i] = nil end
  UI.OnInbox(data)
end

Link.Init()
_G.T3WoWFrame:ClearAllPoints()
_G.T3WoWFrame:SetPoint("TOPLEFT", UIParent, "TOPLEFT", 24, -24)
UI.Toggle()
inbox()
UI.Select(THREADS[2])
inbox({ view = view })
settle(1)
inbox({ view = view })
settle(1)

if not compact then
  UI.ShowSettings()
  settle(0.25)
  _G.T3WoWSettings:ClearAllPoints()
  _G.T3WoWSettings:SetPoint("TOPLEFT", _G.T3WoWFrame, "TOPRIGHT", 16, 0)
  -- The notifications go just under Settings, which starts level with the window's top.
  T3WoWDB.settings.toastPoint = { "TOPLEFT", "TOPLEFT", 980, -(24 + math.floor(_G.T3WoWSettings:GetHeight()) + 10) }
  UI.Alert({ e = "vps", id = "t6", t = "Sign release checksums", s = "failed" })
  UI.Alert({ e = "local", id = "t1", t = "Add a pricing page", s = "completed" })
  UI.Alert({ e = "local", id = "t3", t = "Set up landing page assets", s = "approval" })
  settle(1)
  for _, name in ipairs({ "T3WoWThemeMenu", "T3WoWTheme", "T3WoWThemeDropdown" }) do
    if _G[name] and _G[name].scripts.OnClick then _G[name].scripts.OnClick(_G[name], "LeftButton"); break end
  end
  settle(0.25)
end

-- The strip Link.lua draws for the companion, the faint text it checks fonts with, and the
-- "Sending…" the commands above left.
for _, f in ipairs(frames) do
  if f.level == 10000 or (f.kind == "FontString" and f.text == "MW") then f:Hide() end
end
local q = Link.Queue()
for i = #q, 1, -1 do q[i] = nil end
UI.Refresh()
settle(0.25)
