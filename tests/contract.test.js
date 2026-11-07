// The contract between the addon (Lua, run here in fengari) and the companion (Rust):
// the same fixture files are checked from both sides (see companion/tests/fixtures and
// the Rust tests in codec.rs and lua.rs), so neither can drift without a test failing.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import fengari from "fengari";

const { lua, lauxlib, lualib, to_luastring } = fengari;
const fixture = (name) => readFileSync(new URL(`../companion/tests/fixtures/${name}`, import.meta.url), "utf8");
const codecSource = readFileSync(new URL("../addon/T3WoW/Codec.lua", import.meta.url), "utf8");

function runLua(body) {
  const L = lauxlib.luaL_newstate();
  lualib.luaL_openlibs(L);
  const chunk = `local Codec = (function(...)\n${codecSource}\nend)("T3WoW", {})\n${body}`;
  if (lauxlib.luaL_loadstring(L, to_luastring(chunk)) !== lua.LUA_OK || lua.lua_pcall(L, 0, 1, 0) !== lua.LUA_OK) {
    throw new Error(lua.lua_tojsstring(L, -1));
  }
  return lua.lua_tojsstring(L, -1);
}

test("the addon draws exactly the frames the companion decodes", () => {
  const { frames } = JSON.parse(fixture("frames.json"));
  for (const { seq, payload, cells } of frames) {
    const drawn = runLua(`local out = {}
for i, v in ipairs(Codec.Encode(${seq}, [==[${payload}]==])) do out[i] = string.format("%d", v) end
return table.concat(out)`);
    assert.equal(drawn, cells);
  }
});

test("the inbox the companion writes loads in the game with every field intact", () => {
  const inbox = JSON.parse(fixture("inbox.json"));
  const source = fixture("inbox.lua").replace("T3WoW_Inbox =", "local T3WoW_Inbox =");
  const back = runLua(`${source}
local t = T3WoW_Inbox
return table.concat({ t.k, t.ack, t.envs[1].name, tostring(t.envs[2].ok), t.threads[1].t, tostring(t.threads[1].sn),
  t.view.items[1].x, #t.view.items, t.events[1].s, t.settledTotal, t["end"], t["with space"], t.ns, t.nt }, "|")`);
  assert.equal(
    back,
    [inbox.k, inbox.ack, inbox.envs[0].name, "false", inbox.threads[0].t, "true", inbox.view.items[0].x, 2, "completed",
      inbox.settledTotal, inbox.end, inbox["with space"], inbox.ns, inbox.nt].join("|"),
  );
});

test("an answer from the window encodes the way the companion reads it", () => {
  // Codec.Json sorts keys; unanswered questions are false so the list stays in step.
  const json = runLua(`return Codec.Json({ t = "answer", id = "t1", k = "abcd1234", a = { 2, { 1, 2 }, false, "my own words" } })`);
  assert.deepEqual(JSON.parse(json), { a: [2, [1, 2], false, "my own words"], id: "t1", k: "abcd1234", t: "answer" });
});
