// Replays into the real addon (in the simulated game) every update the live tests
// (companion/src/bridge/tests/live.rs) wrote for the game, in the order they were written:
// none may raise a Lua error, and at each checkpoint the window shows the thread with the
// texts and buttons it names. Not part of `npm test`: `npm run test:live` runs the live
// tests first, then this.
import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { runGame } from "./game.js";

const root = new URL("../companion/target/live/", import.meta.url);
const runs = existsSync(root) ? readdirSync(root).filter((d) => existsSync(new URL(`${d}/checkpoints.json`, root))).sort() : [];

// Text as a Lua long string, whatever it holds.
const longString = (text) => {
  let level = "";
  while (text.includes(`]${level}]`)) level += "=";
  return `[${level}[\n${text}]${level}]`;
};

// A JSON value as a Lua literal.
const luaValue = (v) =>
  typeof v === "string"
    ? longString(v)
    : Array.isArray(v)
      ? `{${v.map(luaValue).join(",")}}`
      : `{${Object.entries(v).map(([k, x]) => `[ ${longString(k)} ]=${luaValue(x)}`).join(",")}}`;

const REPLAY = `
-- Whether one of the agent's messages in the conversation holds \`text\`, in any case (the
-- agent picks the case it answers in). A conversation line has a bubble, shown only for
-- your own messages; your words, the title and buttons are left out, so a reply the window
-- doesn't show can't be found in the prompt that asked for it.
local function agentSays(text)
  text = text:lower()
  for _, fs in ipairs(fontStrings) do
    local bubble = rawget(fs, "bubble")
    if bubble and not bubble.shown and not fs.hidden and type(fs.text) == "string" and fs.text:lower():find(text, 1, true) then
      return true
    end
  end
  return false
end
-- Whether a string on the question's card (it has no bubble) shows \`text\` as given.
local function cardShows(text)
  for _, fs in ipairs(fontStrings) do
    if rawget(fs, "bubble") == nil and not fs.hidden and fs.text == text then return true end
  end
  return false
end
local function button(text)
  for _, f in ipairs(frames) do
    if f.kind == "Button" and f.shown and (f.text == text or rawget(f, "tip") == text) then return f end
  end
end
-- An update as the game loads it from a slot, for this game's client and UI session.
local function inbox(source, name)
  local chunk, err = load(source, name)
  assert(chunk, name .. " doesn't load: " .. tostring(err))
  chunk()
  local data = T3WoW_Inbox
  T3WoW_Inbox = nil
  data.k, data.u = "c1", Link.Session()
  return data
end
Link.Init()
UI.Toggle()
for i, name in ipairs(FILES) do
  local ok, err = pcall(UI.OnInbox, inbox(INBOXES[i], name))
  if not ok then error(name .. ": the window failed on this update: " .. tostring(err)) end
  local check = CHECKS[name]
  if check then
    local data = inbox(INBOXES[i], name)
    local row
    for _, t in ipairs(data.threads or {}) do
      if t.id == check.id then row = t end
    end
    assert(row, check.label .. " (" .. name .. "): the thread isn't in the game's list")
    UI.Select(row)
    UI.OnInbox(data)
    for _, text in ipairs(check.texts) do
      if not agentSays(text) then
        local shown = {}
        for _, fs in ipairs(fontStrings) do
          if not fs.hidden and type(fs.text) == "string" and fs.text ~= "" then shown[#shown + 1] = fs.text:sub(1, 80) end
        end
        error(check.label .. " (" .. name .. "): no message from the agent says " .. text .. "; the window shows:\\n  " .. table.concat(shown, "\\n  "))
      end
    end
    for _, text in ipairs(check.previews) do
      assert(cardShows(text), check.label .. " (" .. name .. "): no option's preview shows " .. text)
    end
    for _, text in ipairs(check.buttons) do
      assert(button(text), check.label .. " (" .. name .. "): no " .. text .. " button")
    end
  end
end
return "ok"
`;

test("the live tests left updates to replay", () => {
  assert.ok(runs.length > 0, "no live test output in companion/target/live: run npm run test:live");
});

for (const run of runs) {
  test(`the addon shows what the live test "${run}" sent the game`, () => {
    const dir = new URL(`${run}/`, root);
    const files = readdirSync(new URL("inboxes/", dir)).filter((f) => f.endsWith(".lua")).sort();
    const sources = files.map((f) => readFileSync(new URL(`inboxes/${f}`, dir), "utf8"));
    const checks = {};
    for (const c of JSON.parse(readFileSync(new URL("checkpoints.json", dir), "utf8"))) checks[c.file] = c;
    for (const file of Object.keys(checks)) assert.ok(files.includes(file), `checkpoint on ${file}, which wasn't saved`);
    const script = `FILES = ${luaValue(files)}\nINBOXES = ${luaValue(sources)}\nCHECKS = ${luaValue(checks)}\n${REPLAY}`;
    assert.equal(runGame(script, { ui: true }), "ok");
  });
}
