import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import fengari from "fengari";
import { Bridge } from "../src/bridge.js";
import { decodeCells, encodeFrame, magicCells } from "../src/codec.js";
import { toLua, wowText } from "../src/lua.js";

const { lua, lauxlib, lualib, to_luastring } = fengari;
const codecSource = readFileSync(new URL("../../addon/T3WoW/Codec.lua", import.meta.url), "utf8");

/** Runs Lua (5.3 here, but the addon code avoids anything 5.1 lacks) and returns its string result. */
function runLua(body) {
  const L = lauxlib.luaL_newstate();
  lualib.luaL_openlibs(L);
  const chunk = `local Codec = (function(...)\n${codecSource}\nend)("T3WoW", {})\n${body}`;
  if (lauxlib.luaL_loadstring(L, to_luastring(chunk)) !== lua.LUA_OK || lua.lua_pcall(L, 0, 1, 0) !== lua.LUA_OK) {
    throw new Error(lua.lua_tojsstring(L, -1));
  }
  return lua.lua_tojsstring(L, -1);
}

const luaCells = (seq, payloadExpr) =>
  runLua(`local out = {}
for i, v in ipairs(Codec.Encode(${seq}, ${payloadExpr})) do out[i] = string.format("%d", v) end
return table.concat(out)`);

test("a frame drawn by the addon decodes in the companion", () => {
  const message = { k: "abc", u: "s1", n: 3, l: 2, q: [{ s: 7, t: "send", m: 'línea 1\n"quoted" | pipe ✓' }] };
  const payload = runLua(`return Codec.Json({ k = "abc", u = "s1", n = 3, l = 2, q = { { s = 7, t = "send", m = "línea 1\\n\\"quoted\\" | pipe ✓" } } })`);
  assert.deepEqual(JSON.parse(payload), message);
  const cells = luaCells(513, `[==[${payload}]==]`);
  assert.deepEqual(decodeCells(cells), { seq: 513, payload });
  assert.equal(cells.slice(0, 8), magicCells().join(""));
  assert.equal(cells, encodeFrame(513, payload).join(""));
});

test("a damaged frame is rejected", () => {
  const cells = encodeFrame(1, '{"k":"a"}').join("");
  const flipped = cells.slice(0, 30) + ((Number(cells[30]) + 1) % 8) + cells.slice(31);
  assert.equal(decodeCells(flipped), null);
  assert.equal(decodeCells(cells.slice(0, cells.length - 3)), null);
});

test("the snapshot the companion writes is valid Lua with the same data", () => {
  const data = { v: 1, envs: [{ id: "x", name: 'Mac "home"', ok: true }], view: { items: [{ r: "a", x: "a\\b\n\0end ✓" }] } };
  const back = runLua(`local t = ${toLua(data)}
return t.envs[1].name .. "|" .. tostring(t.envs[1].ok) .. "|" .. t.view.items[1].x .. "|" .. #t.view.items`);
  assert.equal(back, 'Mac "home"|true|a\\b\n\0end ✓|1');
});

test("markdown becomes WoW text with literal pipes escaped", () => {
  assert.equal(wowText("**Bold** a|b `code`\n```js\nx()\n```\n## Head"), "Bold a||b |cffc8c8a0code|r\n|cffc8c8a0x()|r\n|cffffd100Head|r");
});

function fakeT3(calls) {
  const threads = [{ threadId: "t1", projectId: "p1", title: "Fix tests", status: "running", settled: false, updatedAt: "2026-11-06T10:00:00Z", itemCount: 3 }];
  return {
    threads,
    async call(name, args) {
      calls.push([name, args]);
      if (name === "t3_environment_links")
        return { environmentId: "local", links: [{ environmentId: "mac", label: "Mac", status: "reachable" }] };
      if (name === "t3_project_list") return { projects: [{ id: "p1", title: "app" }] };
      if (name === "t3_thread_list") return { threads: args.settled ? [] : threads.map((t) => ({ ...t })) };
      if (name === "t3_thread_read")
        return {
          thread: threads[0],
          items: args.view ? [{ type: "user_message", text: "hi" }, { type: "assistant_message", text: "**done**" }] : [],
          hasMore: false,
        };
      return {};
    },
  };
}

function fakeSlots() {
  return { written: new Map(), raised: [], cleared: 0, writeSlot(n, lua) { this.written.set(n, lua); }, raiseSignal(n) { this.raised.push(n); }, clearSignals() { this.cleared++; } };
}

test("commands from a frame run once, reach the linked machine, and are acked through the next slot", async () => {
  const calls = [];
  let now = 1_000_000;
  const t3 = fakeT3(calls);
  const slots = fakeSlots();
  const bridge = new Bridge({ client: t3, slots, stateFile: join(tmpdir(), `t3wow-test-${process.pid}.json`), log: () => {}, now: () => now });
  bridge.link = { wowKey: null, uiSession: null, slot: 1, signal: 1, acked: {}, clientKey: null };
  bridge.onCapture({ type: "wow", key: "123@1" });
  const frame = (q) => ({ type: "strip", cells: encodeFrame(1, JSON.stringify({ k: "c1", u: "ui1", n: 5, l: 1, q })).join("") });

  bridge.onCapture(frame([{ s: 1, t: "ui", o: true }, { s: 2, t: "send", e: "mac", id: "t1", m: "go" }]));
  bridge.onCapture(frame([{ s: 1, t: "ui", o: true }, { s: 2, t: "send", e: "mac", id: "t1", m: "go" }]));
  await bridge.queue;
  await bridge.tick();

  const sends = calls.filter(([name]) => name === "t3_thread_send");
  assert.equal(sends.length, 1);
  assert.deepEqual(sends[0][1], { threadId: "t1", message: "go", mode: "auto", clientRequestId: "t3wow:c1:2", environmentId: "mac" });
  assert.deepEqual(slots.raised, [5]);
  const inbox = runLua(`${slots.written.get(1).replace("T3WoW_Inbox =", "local T3WoW_Inbox =")}
return T3WoW_Inbox.ack .. "|" .. T3WoW_Inbox.envs[2].name .. "|" .. T3WoW_Inbox.view.items[2].x .. "|" .. T3WoW_Inbox.slot .. "|" .. T3WoW_Inbox.sig`);
  assert.equal(inbox, "2|Mac|done|1|5");

  // Nothing changed: no slot is spent.
  now += 5000;
  await bridge.tick();
  assert.deepEqual(slots.raised, [5]);

  // The thread finishes: the next slot carries the change and an alert.
  t3.threads[0].status = "completed";
  t3.threads[0].updatedAt = "2026-11-06T10:05:00Z";
  now += 5000;
  await bridge.tick();
  assert.deepEqual(slots.raised, [5, 6]);
  assert.match(slots.written.get(2), /\{n=\d+,e="mac",id="t1",t="Fix tests",s="completed"\}/);
});
