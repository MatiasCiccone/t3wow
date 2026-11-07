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
  // The game watches file 3 (past anything raised before this UI session); we raise it
  // and its later neighbour.
  assert.deepEqual(slots.raised, [3, 4]);
  const inbox = runLua(`${slots.written.get(1).replace("T3WoW_Inbox =", "local T3WoW_Inbox =")}
return T3WoW_Inbox.ack .. "|" .. T3WoW_Inbox.envs[2].name .. "|" .. T3WoW_Inbox.view.items[2].x .. "|" .. T3WoW_Inbox.slot .. "|" .. T3WoW_Inbox.ns`);
  assert.equal(inbox, "2|Mac|done|1|5");

  // Nothing changed: no slot is spent.
  now += 5000;
  await bridge.tick();
  assert.deepEqual(slots.raised, [3, 4]);

  // The thread finishes: the next slot carries the change and an alert.
  t3.threads[0].status = "completed";
  t3.threads[0].updatedAt = "2026-11-06T10:05:00Z";
  now += 5000;
  await bridge.tick();
  assert.deepEqual(slots.raised, [3, 4, 5, 6]);
  assert.match(slots.written.get(2), /\{n=\d+,e="mac",id="t1",t="Fix tests",s="completed"\}/);
});

test("without sound signals the game's own slot count drives where updates go", async () => {
  const calls = [];
  let now = 2_000_000;
  const t3 = fakeT3(calls);
  const slots = fakeSlots();
  const bridge = new Bridge({ client: t3, slots, stateFile: join(tmpdir(), `t3wow-test-poll-${process.pid}.json`), log: () => {}, now: () => now });
  bridge.link = { wowKey: null, uiSession: null, slot: 1, signal: 1, acked: {}, clientKey: null };
  bridge.onCapture({ type: "wow", key: "123@1" });
  const frame = (l, q = []) => ({ type: "strip", cells: encodeFrame(l, JSON.stringify({ k: "c1", u: "ui1", n: 1, l, p: 1, q })).join("") });

  bridge.onCapture(frame(1, [{ s: 1, t: "ui", o: true }]));
  await bridge.queue;
  await bridge.tick();
  assert.deepEqual(slots.raised, []);
  assert.deepEqual([...slots.written.keys()], [1, 2, 3]);

  // The game polled ahead on its own (slots 1..6) and reports slot 7 in a sync frame.
  bridge.onCapture(frame(7));
  t3.threads[0].status = "completed";
  t3.threads[0].updatedAt = "2026-11-06T10:05:00Z";
  now += 5000;
  await bridge.tick();
  assert.deepEqual(slots.raised, []);
  assert.match(slots.written.get(7), /s="completed"/);
  assert.equal(bridge.link.slot, 7);
});

test("the list follows T3's sidebar: no subagent or agent-launched threads, snoozed kept apart", async () => {
  const calls = [];
  const t3 = fakeT3(calls);
  t3.threads.push(
    { threadId: "child", projectId: "p1", title: "Review", status: "completed", parentThreadId: "t1", updatedAt: "2026-11-06T10:00:00Z" },
    { threadId: "launched", projectId: "p1", title: "Test it", status: "completed", createdBy: "agent", updatedAt: "2026-11-06T10:00:00Z" },
    { threadId: "nap", projectId: "p1", title: "Later", status: "completed", snoozed: true, updatedAt: "2026-11-06T10:00:00Z" },
  );
  const bridge = new Bridge({ client: t3, slots: fakeSlots(), stateFile: join(tmpdir(), `t3wow-test-list-${process.pid}.json`), log: () => {} });
  await bridge.refreshThreads("local");
  assert.deepEqual(bridge.threads.get("local").map((t) => t.threadId), ["t1", "nap"]);
  const snapshot = bridge.snapshot();
  assert.deepEqual(snapshot.threads.map((t) => [t.id, t.sn ?? false]), [["t1", false], ["nap", true]]);
  for (const [name, args] of calls) if (name === "t3_thread_list") assert.equal(args.includeSubagents, false);
});

test("the game polls until its first update names a signal file, then follows the clock between updates", async () => {
  const calls = [];
  let now = 3_000_000; // 3000 s
  const t3 = fakeT3(calls);
  const slots = fakeSlots();
  const bridge = new Bridge({ client: t3, slots, stateFile: join(tmpdir(), `t3wow-test-sig-${process.pid}.json`), log: () => {}, now: () => now });
  bridge.link = { wowKey: null, uiSession: null, slot: 1, signal: 1, signalTime: 0, acked: {}, clientKey: null };
  bridge.onCapture({ type: "wow", key: "123@1" });
  const frame = (u, l, p, q = []) => ({ type: "strip", cells: encodeFrame(l, JSON.stringify({ k: "c1", u, n: 0, l, ...(p ? { p: 1 } : {}), q })).join("") });
  const waitIn = (slot) => {
    const lua = slots.written.get(slot);
    return [Number(/ns=([0-9]+)/.exec(lua)[1]), Number(/nt=([0-9]+)/.exec(lua)[1])];
  };
  // Link.lua's arithmetic: the file the game watches at time t for a wait (ns, nt).
  const gameWatches = ([ns, nt], t) => ns + Math.max(0, Math.floor((t - nt) / 25));
  const change = (status) => {
    t3.threads[0].status = status;
    t3.threads[0].updatedAt = new Date(now).toISOString();
  };

  // Fresh UI, still polling: nothing raised, the update names the wait.
  bridge.onCapture(frame("ui1", 1, true, [{ s: 1, t: "hello", o: false }]));
  await bridge.queue;
  await bridge.tick();
  assert.deepEqual(slots.raised, []);
  const firstWait = waitIn(1);

  // The game took that wait and watches signals. 15 s later a thread finishes.
  bridge.onCapture(frame("ui1", 2, false));
  change("completed");
  now += 15_000;
  await bridge.tick();
  assert.ok(slots.raised.includes(gameWatches(firstWait, now / 1000)));

  // Five quiet minutes: the game has moved on 12 files; the next update raises the right one.
  const secondWait = waitIn(2);
  slots.raised = [];
  change("running"); // seen on the next poll, but with the window closed not worth a slot
  now += 15_000;
  await bridge.tick();
  assert.deepEqual(slots.raised, []);
  change("completed");
  now += 285_000;
  await bridge.tick();
  const watched = gameWatches(secondWait, now / 1000);
  assert.equal(watched, secondWait[0] + 12);
  assert.deepEqual(slots.raised, [watched - 1, watched, watched + 1]);

  // /reload: the new wait starts past every file raised in this game process.
  bridge.onCapture(frame("ui2", 1, true, [{ s: 2, t: "hello", o: false }]));
  await bridge.queue;
  now += 5000;
  await bridge.tick();
  assert.ok(waitIn(1)[0] > watched + 1);
});

test("with the window open, routine changes wait 10 s but answers to you go out at once", async () => {
  const calls = [];
  let now = 4_000_000;
  const t3 = fakeT3(calls);
  const slots = fakeSlots();
  const bridge = new Bridge({ client: t3, slots, stateFile: join(tmpdir(), `t3wow-test-gap-${process.pid}.json`), log: () => {}, now: () => now });
  bridge.link = { wowKey: null, uiSession: null, slot: 1, signal: 1, signalTime: 0, acked: {}, clientKey: null };
  bridge.onCapture({ type: "wow", key: "123@1" });
  // l: the slot the game loads next (it has read one slot per update by then).
  const frame = (q, l = 1) => ({ type: "strip", cells: encodeFrame(l, JSON.stringify({ k: "c1", u: "ui1", n: 0, l, q })).join("") });
  const publishes = () => slots.written.size === 0 ? 0 : new Set([...slots.written.values()].map((lua) => /now=(\d+)/.exec(lua)[1])).size;

  bridge.onCapture(frame([{ s: 1, t: "ui", o: true }]));
  await bridge.queue;
  await bridge.tick();
  assert.equal(publishes(), 1);

  // A running agent touches the thread: routine, held back.
  t3.threads[0].updatedAt = new Date(now + 1000).toISOString();
  now += 4_000;
  await bridge.tick();
  assert.equal(publishes(), 1);

  // You send a message: its ack goes out right away.
  bridge.onCapture(frame([{ s: 2, t: "send", id: "t1", m: "hi" }], 2));
  await bridge.queue;
  now += 2_000;
  await bridge.tick();
  assert.equal(publishes(), 2);

  // Another routine change, then the 10 s gap passes.
  t3.threads[0].updatedAt = new Date(now + 1000).toISOString();
  now += 4_000;
  await bridge.tick();
  assert.equal(publishes(), 2);
  now += 7_000;
  await bridge.tick();
  assert.equal(publishes(), 3);
});

test("repair puts back missing files and updates an older addon, leaving the rest alone", async () => {
  const { mkdtempSync, rmSync: rm, existsSync, writeFileSync: write, readFileSync: read } = await import("node:fs");
  const { Slots, slotName } = await import("../src/slots.js");
  const dir = mkdtempSync(join(tmpdir(), "t3wow-repair-"));
  try {
    const slots = new Slots(dir, "16001");
    const first = slots.repair();
    assert.match(first.addon, /^installed /);
    assert.ok(first.created > 4000);
    assert.ok(slots.installed());

    // An addon manager removed a slot and a signal file; nothing else changed.
    rm(join(dir, slotName(7)), { recursive: true });
    rm(join(dir, "T3WoW", "sig", "0042.ttf"));
    const second = slots.repair();
    assert.equal(second.addon, null);
    assert.equal(second.created, 3); // the slot's toc and Inbox.lua, and the signal file
    assert.ok(existsSync(join(dir, slotName(7), "Inbox.lua")));

    // An older addon gets the shipped one.
    const toc = join(dir, "T3WoW", "T3WoW.toc");
    write(toc, read(toc, "utf8").replace(/^## Version:.*$/m, "## Version: 0.0.1"));
    assert.match(slots.repair().addon, /^updated 0\.0\.1 to /);
    assert.match(read(toc, "utf8"), /^## Interface: 16001$/m);
  } finally {
    rm(dir, { recursive: true, force: true });
  }
});
