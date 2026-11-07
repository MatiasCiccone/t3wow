import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { hostname } from "node:os";
import { decodeCells } from "./codec.js";
import { toLua, wowText } from "./lua.js";
import { SIGNAL_COUNT, SLOT_COUNT } from "./slots.js";
import { AuthError } from "./t3.js";

const WORKING = new Set(["preparing", "queued", "starting", "running"]);
const MIN_PUBLISH_GAP_MS = 1500;
// Routine changes while the window is open: one update at most this often (200 slots
// then last about 35 minutes of watching a busy thread, not 15).
const ROUTINE_GAP_MS = 10_000;
// While waiting, the game moves to the next signal file every SIGNAL_PERIOD seconds (it
// stops noticing a file it has checked for too long). Same clock, same arithmetic: keep
// in step with addon/T3WoW/Link.lua.
export const SIGNAL_PERIOD = 25;
const VIEW_MESSAGES = 24;
const VIEW_CHARS = 40_000;
const THREAD_LIST = { allProjects: true, includeSubagents: false };
// T3's sidebar files subagent threads, and threads an agent launched, under the thread
// they came from. Linked machines don't always apply includeSubagents, so check here too.
const onSidebar = (t) => !t.parentThreadId && t.createdBy !== "agent";

const seconds = (iso) => (iso ? Math.floor(Date.parse(iso) / 1000) : 0);
const plain = (text, max = 200) => String(text ?? "").replace(/\|/g, "||").replace(/\s+/g, " ").trim().slice(0, max);

export class Bridge {
  constructor({ client, slots, stateFile, log = console.log, now = () => Date.now() }) {
    this.client = client;
    this.slots = slots;
    this.stateFile = stateFile;
    this.log = log;
    this.now = now;
    // signal / signalTime: where the game's current wait started (file number, epoch s).
    this.link = { wowKey: null, uiSession: null, slot: 1, signal: 1, signalTime: 0, acked: {}, clientKey: null, ...this.loadState() };
    this.windowOpen = false;
    this.view = null; // { env, id, depth }
    this.envs = [{ id: "", name: hostname(), ok: true }];
    this.localEnvId = null;
    this.threads = new Map(); // env id -> thread list
    this.projects = new Map(); // env id -> project list
    this.settledTotals = new Map(); // env id -> settled thread count, listed or not
    this.viewData = null;
    this.viewKey = null;
    this.notes = [];
    this.events = [];
    this.eventSeq = 0;
    this.lastStatus = new Map();
    this.queue = Promise.resolve();
    this.lastPublishAt = 0;
    this.lastPublishedHash = null;
    this.pollAt = 0;
    this.slowAt = 0;
    this.authError = null;
  }

  loadState() {
    try {
      return JSON.parse(readFileSync(this.stateFile, "utf8"));
    } catch {
      return {};
    }
  }

  nowSeconds() {
    return Math.floor(this.now() / 1000);
  }

  /** The signal file the game watches at nowS, by the same arithmetic as Link.lua. */
  watchedSignal(nowS = this.nowSeconds()) {
    const since = nowS - (this.link.signalTime || nowS);
    return this.link.signal + Math.max(0, Math.floor(since / SIGNAL_PERIOD));
  }

  saveState() {
    try {
      writeFileSync(this.stateFile, JSON.stringify(this.link));
    } catch {}
  }

  note(text) {
    this.notes = [...this.notes, { at: Math.floor(this.now() / 1000), x: plain(text, 300) }].slice(-4);
  }

  // ---- input from the capture script ----

  onCapture(message) {
    if (message.type === "wow") {
      if (message.key !== this.link.wowKey) {
        this.log(message.key ? `WoW running (${message.key})` : "WoW not running");
        // A new client process hasn't loaded any signal font: all start blank again.
        this.slots.clearSignals();
        this.slots.flipProbe?.(false);
        this.link.signal = 1;
        this.link.signalTime = this.nowSeconds();
        this.link.wowKey = message.key;
        this.link.uiSession = null;
        this.saveState();
      }
    } else if (message.type === "strip" && message.cells) {
      const frame = decodeCells(message.cells);
      if (!frame) return;
      let payload;
      try {
        payload = JSON.parse(frame.payload);
      } catch {
        return;
      }
      this.onFrame(payload);
    } else if (message.type === "error") {
      this.log(`capture: ${message.message}`);
    }
  }

  onFrame({ k, u, n, l, p, q }) {
    if (typeof k !== "string" || typeof u !== "string") return;
    // Until the game watches signals (it learns the next one from its first update, and
    // some clients can't use them at all) it loads slots on a schedule and reports where it is.
    const polling = p === 1;
    if (u !== this.link.uiSession || k !== this.link.clientKey) {
      // New UI session (login or /reload): every slot can load again. Signal files raised
      // earlier in this game process stay real, so the new wait starts past all of them.
      this.link.uiSession = u;
      this.link.clientKey = k;
      this.link.slot = Number(l) || 1;
      this.link.signal = this.watchedSignal() + 2;
      this.link.signalTime = this.nowSeconds();
      this.lastPublishedHash = null;
      this.lastUrgentHash = null;
      this.link.polling = undefined;
      this.log(`Game UI connected (session ${u}, next slot ${this.link.slot})`);
    } else if (Number(l) > 0) {
      // The game draws a frame with its current slot count, so it is the authority on
      // where it loads next (it may have loaded extra slots on its own).
      this.link.slot = Number(l);
    }
    if (polling !== this.link.polling) {
      if (!polling) this.log(`Game watching signals (file ${Number(n) || "?"}): updates arrive in about a second`);
      else if (this.link.polling === false) this.log("Game stopped watching signals: it checks for updates on a timer");
      this.link.polling = polling;
    }
    const acked = this.link.acked[k] ?? 0;
    for (const command of Array.isArray(q) ? q : []) {
      if (!(command?.s > acked)) continue;
      this.link.acked[k] = Math.max(this.link.acked[k] ?? 0, command.s);
      this.queue = this.queue.then(() => this.run(k, command));
    }
    this.saveState();
  }

  // ---- commands from the game ----

  envArg(env) {
    return env && env !== this.localEnvId ? { environmentId: env } : {};
  }

  async run(clientKey, command) {
    const env = typeof command.e === "string" ? command.e : "";
    const requestId = `t3wow:${clientKey}:${command.s}`;
    try {
      switch (command.t) {
        case "hello":
        case "ui":
          this.windowOpen = Boolean(command.o);
          break;
        case "open":
          this.view = command.id ? { env, id: command.id, depth: 1 } : null;
          this.viewKey = null;
          break;
        case "more":
          if (this.view) this.view.depth += 1;
          this.viewKey = null;
          break;
        case "send": {
          if (!command.id || !command.m) break;
          await this.client.call("t3_thread_send", {
            threadId: command.id,
            message: command.m,
            mode: "auto",
            clientRequestId: requestId,
            ...this.envArg(env),
          });
          this.note("Sent.");
          this.view = { env, id: command.id, depth: this.view?.id === command.id ? this.view.depth : 1 };
          this.viewKey = null;
          break;
        }
        case "new": {
          if (!command.m) break;
          const title = command.m.split("\n")[0].slice(0, 60);
          const launched = await this.client.call("t3_thread_launch", {
            title,
            message: command.m,
            clientRequestId: requestId,
            ...(command.p ? { projectId: command.p } : { scratch: true }),
            ...this.envArg(env),
          });
          const id = launched.threadId ?? launched.thread?.threadId;
          this.note(`Started "${title}".`);
          if (id) this.view = { env, id, depth: 1 };
          this.viewKey = null;
          break;
        }
        case "settle":
        case "unsettle":
          if (!command.id) break;
          await this.client.call("t3_thread_organize", { action: command.t, threadId: command.id, ...this.envArg(env) });
          this.note(command.t === "settle" ? "Settled." : "Moved back to open.");
          break;
        case "stop":
          if (!command.id) break;
          await this.client.call("t3_thread_interrupt", {
            threadId: command.id,
            clientRequestId: requestId,
            ...this.envArg(env),
          });
          this.note("Stopping.");
          break;
        case "probe": {
          const changed = this.slots.flipProbe(true);
          this.log(changed.length ? `probe: made ${changed.join(", ")} files real` : "probe: no probe files; run t3wow install");
          break;
        }
        case "refresh":
          this.viewKey = null;
          break;
        default:
          return;
      }
    } catch (error) {
      this.log(`${command.t} failed: ${error.message}`);
      this.note(`${command.t} failed: ${error.message}`);
    }
    this.pollAt = 0;
  }

  // ---- reading T3 ----

  async poll() {
    const t = this.now();
    if (t >= this.slowAt) {
      this.slowAt = t + 60_000;
      await this.refreshEnvironments();
      await Promise.all(this.envs.filter((e) => e.ok).map((e) => this.refreshProjects(e.id)));
    }
    await Promise.all(this.envs.filter((e) => e.ok).map((e) => this.refreshThreads(e.id)));
    await this.refreshView();
    this.authError = null;
  }

  async refreshEnvironments() {
    const result = await this.client.call("t3_environment_links");
    this.localEnvId = result.environmentId ?? null;
    this.envs = [
      { id: this.localEnvId ?? "", name: hostname(), ok: true },
      ...(result.links ?? []).map((link) => ({
        id: link.environmentId,
        name: link.label ?? link.environmentId.slice(0, 8),
        ok: link.status === "reachable",
      })),
    ];
  }

  async refreshProjects(env) {
    try {
      const page = await this.client.call("t3_project_list", { limit: 100, ...this.envArg(env) });
      this.projects.set(env, page.projects ?? []);
    } catch (error) {
      this.log(`projects (${env || "local"}): ${error.message}`);
    }
  }

  async refreshThreads(env) {
    try {
      const list = (args) => this.client.call("t3_thread_list", { ...THREAD_LIST, ...args, ...this.envArg(env) });
      const [open, snoozed, settled] = await Promise.all([
        list({ settled: false, snoozed: false, limit: 60 }),
        list({ settled: false, snoozed: true, limit: 15 }),
        list({ settled: true, limit: 15 }),
      ]);
      const threads = [
        ...(open.threads ?? []).filter((t) => t.snoozed !== true),
        ...(snoozed.threads ?? []).filter((t) => t.snoozed === true),
        ...(settled.threads ?? []),
      ].filter(onSidebar);
      for (const thread of threads) this.trackStatus(env, thread);
      this.threads.set(env, threads);
      this.settledTotals.set(env, Math.max(settled.total ?? 0, (settled.threads ?? []).filter(onSidebar).length));
      const target = this.envs.find((e) => e.id === env);
      if (target) target.ok = true;
    } catch (error) {
      if (error instanceof AuthError) throw error;
      const target = this.envs.find((e) => e.id === env);
      if (target && target.id !== this.localEnvId) target.ok = false;
      this.log(`threads (${env || "local"}): ${error.message}`);
    }
  }

  trackStatus(env, thread) {
    const key = `${env}:${thread.threadId}`;
    const before = this.lastStatus.get(key);
    this.lastStatus.set(key, thread.status);
    if (before && WORKING.has(before) && !WORKING.has(thread.status)) {
      this.events = [
        ...this.events,
        { n: (this.eventSeq = Math.max(this.eventSeq + 1, this.now())), e: env, id: thread.threadId, t: plain(thread.title, 80), s: thread.status },
      ].slice(-10);
    }
  }

  async refreshView() {
    if (!this.view) {
      this.viewData = null;
      return;
    }
    const { env, id, depth } = this.view;
    const listed = (this.threads.get(env) ?? []).find((t) => t.threadId === id);
    const key = `${env}:${id}:${depth}:${listed?.updatedAt ?? ""}:${listed?.status ?? ""}`;
    if (key === this.viewKey && this.viewData) return;
    const head = await this.client.call("t3_thread_read", { threadId: id, limit: 1, maxCharsPerItem: 1, ...this.envArg(env) });
    const thread = head.thread;
    const want = VIEW_MESSAGES * depth;
    let messages = [];
    let window = 80 * depth;
    for (let attempt = 0; attempt < 4; attempt++) {
      const start = Math.max(0, (thread.itemCount ?? 0) - window);
      messages = [];
      let after = start > 0 ? start : undefined;
      for (let page = 0; page < 10; page++) {
        const result = await this.client.call("t3_thread_read", {
          threadId: id,
          view: "messages",
          limit: 100,
          maxCharsPerItem: 6000,
          ...(after === undefined ? {} : { afterPosition: after }),
          ...this.envArg(env),
        });
        messages.push(...(result.items ?? []));
        if (!result.hasMore || result.nextPosition == null) break;
        after = result.nextPosition;
      }
      if (messages.length >= want || start === 0) break;
      window *= 4;
    }
    const items = [];
    let chars = 0;
    for (const item of messages.slice(-want).reverse()) {
      if (item.type !== "user_message" && item.type !== "assistant_message") continue;
      const text = wowText(item.text ?? "") + (item.textTruncated ? "\n…" : "");
      if (chars + text.length > VIEW_CHARS && items.length) break;
      chars += text.length;
      items.unshift({ r: item.type === "user_message" ? "u" : "a", x: text });
    }
    this.viewData = {
      e: env,
      id,
      t: plain(thread.title, 120),
      s: thread.status,
      set: thread.settled === true,
      at: seconds(thread.updatedAt),
      more: messages.length > items.length || (thread.itemCount ?? 0) > window,
      items,
    };
    this.viewKey = key;
  }

  // ---- writing into the game ----

  snapshot() {
    const envIndex = new Map(this.envs.map((e, i) => [e.id, i + 1]));
    const projectTitle = new Map();
    for (const [env, list] of this.projects) for (const p of list) projectTitle.set(`${env}:${p.id}`, p.title);
    const threads = [];
    for (const [env, list] of this.threads) {
      for (const t of list) {
        threads.push({
          e: envIndex.get(env) ?? 1,
          id: t.threadId,
          t: plain(t.title, 90),
          s: t.status,
          set: t.settled === true,
          sn: t.snoozed === true || undefined,
          at: seconds(t.updatedAt),
          p: plain(projectTitle.get(`${env}:${t.projectId}`) ?? "", 40),
        });
      }
    }
    threads.sort((a, b) => b.at - a.at);
    const projects = [];
    for (const [env, list] of this.projects) {
      for (const p of list) if (!p.archivedAt && !p.deletedAt) projects.push({ e: envIndex.get(env) ?? 1, id: p.id, t: plain(p.title, 40) });
    }
    const view = this.viewData ? { ...this.viewData, e: envIndex.get(this.viewData.e) ?? 1 } : undefined;
    return {
      v: 1,
      k: this.link.clientKey,
      ack: this.link.acked[this.link.clientKey] ?? 0,
      envs: this.envs.map((e) => ({ id: e.id, name: plain(e.name, 40), ok: e.ok })),
      threads,
      settledTotal: [...this.settledTotals.values()].reduce((a, b) => a + b, 0),
      projects,
      view,
      notes: this.notes,
      events: this.events,
      auth: this.authError ?? undefined,
    };
  }

  publish() {
    if (!this.link.wowKey || !this.link.uiSession) return;
    const t = this.now();
    if (t - this.lastPublishAt < MIN_PUBLISH_GAP_MS) return;
    const data = this.snapshot();
    // Each update spends one of the UI session's 200 slots. Urgent: what answers you
    // (acks, notes, a thread you just opened), alerts and errors; with the window closed
    // nothing else is worth a slot. Everything else (list times, a running agent's new
    // messages) waits until ROUTINE_GAP_MS after the last update.
    const urgent = {
      ack: data.ack,
      events: data.events,
      notes: data.notes,
      auth: data.auth,
      open: this.windowOpen,
      view: data.view ? [data.view.id, data.view.more] : null,
    };
    const hash = createHash("sha1").update(JSON.stringify(this.windowOpen ? data : urgent)).digest("hex");
    if (hash === this.lastPublishedHash) return;
    const urgentHash = createHash("sha1").update(JSON.stringify(urgent)).digest("hex");
    if (urgentHash === this.lastUrgentHash && t - this.lastPublishAt < ROUTINE_GAP_MS) return;
    const polling = this.link.polling === true;
    if (this.link.slot > SLOT_COUNT) return;
    const nowS = Math.floor(t / 1000);
    // ns/nt: where and when the game's next wait starts. Polling doesn't move it.
    let wait = { ns: this.link.signal, nt: this.link.signalTime };
    const raise = [];
    if (!polling) {
      // The game may be a moment either side of moving to its next file, so raise the
      // file it should be watching now and both neighbours.
      const watched = this.watchedSignal(nowS);
      if (watched + 1 > SIGNAL_COUNT) return;
      for (let n = Math.max(this.link.signal, watched - 1); n <= watched + 1; n++) raise.push(n);
      wait = { ns: watched + 2, nt: nowS };
    }
    const lua = (slot) => `T3WoW_Inbox = ${toLua({ ...data, now: nowS, slot, ns: wait.ns, nt: wait.nt })}\n`;
    // Write a few slots ahead so a missed count on either side still finds fresh data,
    // and only then raise the signals that send the game to read them.
    for (let slot = this.link.slot; slot < this.link.slot + 3 && slot <= SLOT_COUNT; slot++) {
      this.slots.writeSlot(slot, lua(slot));
    }
    if (!polling) {
      for (const n of raise) this.slots.raiseSignal(n);
      this.link.slot += 1;
      this.link.signal = wait.ns;
      this.link.signalTime = wait.nt;
    }
    this.lastPublishAt = t;
    this.lastPublishedHash = hash;
    this.lastUrgentHash = urgentHash;
    this.saveState();
  }

  async tick() {
    const t = this.now();
    if (t >= this.pollAt && !this.polling) {
      this.polling = true;
      this.pollAt = t + (this.windowOpen ? 4000 : 12_000);
      try {
        await this.poll();
      } catch (error) {
        if (error instanceof AuthError) {
          if (this.authError !== error.message) this.log(error.message);
          this.authError = error.message;
          this.pollAt = t + 30_000;
        } else {
          this.log(`poll: ${error.message}`);
        }
      } finally {
        this.polling = false;
      }
    }
    this.publish();
  }
}
