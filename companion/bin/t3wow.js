#!/usr/bin/env node
import { execFileSync, spawn } from "node:child_process";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { parseArgs } from "node:util";
import { addonVersion, captureScript, isExe } from "../src/assets.js";
import { Bridge } from "../src/bridge.js";
import { decodeCells, encodeFrame, magicCells } from "../src/codec.js";
import { Slots } from "../src/slots.js";
import { McpClient, discoverOrigin, login } from "../src/t3.js";
import { findWowDir, looksLikeClient, normalizeWowDir } from "../src/wow.js";

const CONFIG_DIR = process.env.APPDATA ? join(process.env.APPDATA, "t3wow") : join(homedir(), ".config", "t3wow");
const CONFIG_FILE = join(CONFIG_DIR, "config.json");
const DEFAULT_INTERFACE = "16001";

const { values: flags, positionals } = parseArgs({
  allowPositionals: true,
  options: {
    wow: { type: "string" },
    interface: { type: "string" },
    origin: { type: "string" },
    help: { type: "boolean", short: "h" },
  },
});

// Started by double-clicking t3wow.exe: a console window that closes the moment we exit,
// so errors wait for a key press.
const DOUBLE_CLICKED = isExe && process.argv.length <= 2;
if (isExe) process.title = "T3 WoW companion";

function readConfig() {
  try {
    return JSON.parse(readFileSync(CONFIG_FILE, "utf8"));
  } catch {
    return {};
  }
}

function writeConfig(config) {
  mkdirSync(CONFIG_DIR, { recursive: true });
  writeFileSync(CONFIG_FILE, JSON.stringify(config, null, 2));
}

function fail(message) {
  console.error(message);
  if (DOUBLE_CLICKED) {
    try {
      execFileSync("cmd.exe", ["/c", "pause"], { stdio: "inherit" });
    } catch {}
  }
  process.exit(1);
}

function ask(question) {
  if (!process.stdin.isTTY) return Promise.resolve("");
  const rl = createInterface({ input: process.stdin, output: process.stdout });
  return new Promise((resolve) => rl.question(question, (answer) => (rl.close(), resolve(answer))));
}

/** The _forever_ folder: --wow, then the saved one, then a search, then asking. Saved for next time. */
async function wowDir(config) {
  let dir = flags.wow ? normalizeWowDir(flags.wow) : null;
  if (!dir && looksLikeClient(config.wowDir)) dir = config.wowDir;
  if (!dir) dir = findWowDir();
  while (!dir) {
    const answer = await ask("Where is WoW: Forever installed? Paste its _forever_ folder (or press Enter to quit): ");
    if (!answer.trim()) fail('Could not find WoW: Forever. Pass --wow "C:\\...\\World of Warcraft\\_forever_".');
    dir = normalizeWowDir(answer);
    if (!looksLikeClient(dir)) {
      console.log(`  No Wow.exe or Interface folder in ${dir}.`);
      dir = null;
    }
  }
  if (dir !== config.wowDir) writeConfig({ ...readConfig(), wowDir: dir });
  return dir;
}

function origin(config) {
  const value = flags.origin ?? process.env.T3WOW_ORIGIN ?? config.origin ?? discoverOrigin();
  if (!value) fail("T3 Code is not running here. Start the T3 Code desktop app, or pass --origin http://127.0.0.1:3773.");
  return value.replace(/\/+$/, "");
}

const signedIn = (config) =>
  Boolean(process.env.T3WOW_TOKEN) || (Boolean(config.token) && !(config.tokenExpiresAt && config.tokenExpiresAt < Date.now()));

function token(config) {
  if (!signedIn(config)) fail(config.token ? "The T3 sign-in expired. Run: t3wow login" : "Not signed in. Run: t3wow login");
  return process.env.T3WOW_TOKEN ?? config.token;
}

async function slotsFor(config) {
  const dir = await wowDir(config);
  return { dir, slots: new Slots(join(dir, "Interface", "AddOns"), flags.interface ?? config.interface ?? DEFAULT_INTERFACE) };
}

async function install() {
  const config = readConfig();
  const { dir, slots } = await slotsFor(config);
  slots.install();
  writeConfig({ ...readConfig(), wowDir: dir, interface: slots.interfaceVersion });
  console.log(`Installed the T3WoW addon and its reply slots into ${slots.addonsDir}.`);
  console.log("Restart WoW (a full restart, not /reload) so the game sees the new files.");
}

async function uninstall() {
  const config = readConfig();
  (await slotsFor(config)).slots.uninstall();
  console.log("Removed the T3WoW addon and its slots. Your sign-in stays in " + CONFIG_FILE);
}

async function signIn() {
  const config = readConfig();
  const at = origin(config);
  const result = await login(at);
  writeConfig({ ...readConfig(), origin: flags.origin ?? config.origin, token: result.token, tokenExpiresAt: result.expiresAt });
  console.log(`Signed in to ${at} (${result.scope || "no scope reported"}). Expires ${new Date(result.expiresAt).toLocaleString()}.`);
}

async function status() {
  const config = readConfig();
  const { dir, slots } = await slotsFor(config);
  console.log(`WoW: ${dir} (addon ${slots.installed() ? `installed, ${slots.installedVersion()}` : "NOT installed: run t3wow install"})`);
  if (process.platform === "win32") console.log(`Capture: ${await checkCapture()}`);
  const at = origin(config);
  const client = new McpClient(at, token(config));
  const bridge = new Bridge({ client, slots, stateFile: join(tmpdir(), "t3wow-status.json"), log: (m) => console.log(`  ${m}`) });
  await bridge.poll();
  console.log(`T3: ${at}`);
  for (const env of bridge.envs) {
    const threads = bridge.threads.get(env.id) ?? [];
    const projects = bridge.projects.get(env.id) ?? [];
    console.log(`  ${env.name}${env.id === bridge.localEnvId ? " (this PC)" : ""}: ${env.ok ? "reachable" : "unreachable"}, ${threads.length} threads, ${projects.length} projects`);
  }
}

/** Draws a frame into a bitmap, runs the capture script on it, and decodes the result. */
async function checkCapture() {
  const payload = JSON.stringify({ check: "T3 WoW ✓", at: Date.now() });
  const file = join(tmpdir(), `t3wow-capture-${process.pid}.bmp`);
  writeFileSync(file, bitmap(encodeFrame(42, payload), 3, 2));
  const output = await new Promise((resolve) => {
    const child = spawn("powershell.exe", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", captureScript(CONFIG_DIR),
      "-MagicCells", magicCells().join(""), "-ImageFile", file], { stdio: ["ignore", "pipe", "pipe"], windowsHide: true });
    let text = "";
    child.stdout.on("data", (chunk) => (text += chunk));
    child.stderr.on("data", (chunk) => (text += chunk));
    child.on("exit", () => resolve(text));
  });
  rmSync(file, { force: true });
  try {
    const decoded = decodeCells(JSON.parse(output.trim().split("\n").pop()).cells);
    return decoded?.payload === payload ? "ok (test frame decoded)" : `FAILED: ${output.trim().slice(0, 300)}`;
  } catch {
    return `FAILED: ${output.trim().slice(0, 300)}`;
  }
}

/** 24-bit BMP of the strip, offset by (dx, dy) pixels on a grey background. */
function bitmap(cells, dx, dy) {
  const px = 4, perRow = 200;
  const width = perRow * px + 20, height = Math.ceil(cells.length / perRow) * px + 20;
  const stride = Math.ceil((width * 3) / 4) * 4;
  const out = Buffer.alloc(54 + stride * height, 0x60);
  out.write("BM", 0);
  out.writeUInt32LE(out.length, 2);
  out.writeUInt32LE(0, 6);
  out.writeUInt32LE(54, 10);
  out.writeUInt32LE(40, 14);
  out.writeInt32LE(width, 18);
  out.writeInt32LE(height, 22);
  out.writeUInt16LE(1, 26);
  out.writeUInt16LE(24, 28);
  out.writeUInt32LE(0, 30);
  out.writeUInt32LE(stride * height, 34);
  out.fill(0, 38, 54);
  cells.forEach((value, i) => {
    const x0 = dx + (i % perRow) * px, y0 = dy + Math.floor(i / perRow) * px;
    for (let y = y0; y < y0 + px; y++) {
      for (let x = x0; x < x0 + px; x++) {
        const at = 54 + (height - 1 - y) * stride + x * 3;
        out[at] = value & 1 ? 255 : 0;
        out[at + 1] = value & 2 ? 255 : 0;
        out[at + 2] = value & 4 ? 255 : 0;
      }
    }
  });
  return out;
}

/** Everything a first start needs: find WoW, put the addon files in place, sign in, run. */
async function run() {
  const log = (message) => console.log(`[${new Date().toLocaleTimeString()}] ${message}`);
  let config = readConfig();
  const { dir, slots } = await slotsFor(config);
  if (!config.interface) writeConfig({ ...readConfig(), interface: slots.interfaceVersion });
  // Addon managers replace addon folders on update and cleanups delete "unknown" ones;
  // put back whatever is missing, and install or update the addon this companion ships.
  const repaired = slots.repair();
  if (repaired.addon) log(`Addon ${repaired.addon} in ${slots.addonDir}`);
  if (repaired.created > 0) log(`Created ${repaired.created} missing addon files`);
  if (repaired.addon || repaired.created > 0) {
    log("If WoW is running, restart it completely (not /reload) so it sees the new files.");
  }
  const at = origin(config);
  if (!signedIn(config)) {
    log(config.token ? "The T3 sign-in expired; signing in again." : "Not signed in to T3 yet; opening the sign-in page.");
    await signIn();
    config = readConfig();
  }
  const client = new McpClient(at, token(config));
  const bridge = new Bridge({ client, slots, stateFile: join(CONFIG_DIR, "state.json"), log });
  log(`T3 WoW companion ${addonVersion() ?? ""}: T3 at ${at}, WoW at ${dir}`);
  log("Leave this window open while you play. In the game: /t3");
  if (process.platform !== "win32") log("Screen capture runs only on Windows; T3 polling still runs.");
  else startCapture(dir, (message) => bridge.onCapture(message), log);

  let busy = false;
  setInterval(async () => {
    if (busy) return;
    busy = true;
    try {
      await bridge.tick();
    } finally {
      busy = false;
    }
  }, 250);
}

function startCapture(dir, onMessage, log) {
  const script = captureScript(CONFIG_DIR);
  let delay = 1000;
  let child = null;
  process.once("exit", () => child?.kill());
  const start = () => {
    child = spawn(
      "powershell.exe",
      ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script, "-MagicCells", magicCells().join(""), "-ClientFolder", dir],
      { stdio: ["ignore", "pipe", "pipe"], windowsHide: true },
    );
    createInterface({ input: child.stdout }).on("line", (line) => {
      try {
        onMessage(JSON.parse(line));
      } catch {}
    });
    createInterface({ input: child.stderr }).on("line", (line) => log(`capture: ${line}`));
    const startedAt = Date.now();
    child.on("exit", (code) => {
      // Back off while it keeps failing fast; a long healthy run resets the delay.
      delay = Date.now() - startedAt > 60_000 ? 2000 : Math.min(delay * 2, 60_000);
      log(`capture exited (${code}); restarting in ${delay / 1000}s`);
      setTimeout(start, delay);
    });
  };
  start();
}

const commands = { install, uninstall, login: signIn, status, run };
const command = positionals[0] ?? "run";
if (flags.help || !commands[command]) {
  console.log(`t3wow: T3 Code inside World of Warcraft: Forever

  t3wow run     start the companion (default; double-clicking t3wow.exe does this).
                Finds WoW, puts the addon in place, signs in to T3 if needed.
  t3wow install [--wow <_forever_ folder>] [--interface ${DEFAULT_INTERFACE}]   reinstall from scratch
  t3wow login   [--origin http://127.0.0.1:3773]
  t3wow status  check the T3 connection and the addon install
  t3wow uninstall`);
  process.exit(flags.help ? 0 : 1);
}
commands[command]().catch((error) => fail(error.message));
