import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

// The game reads an addon's files only when it loads that addon, and finds only files
// that existed when the client started. So replies travel through a pool of
// load-on-demand "slot" addons (each loads once per UI session), and "a reply is
// ready" travels through pre-made .wav files: an empty one won't play, a valid one will.
export const SLOT_COUNT = 200;
export const SIGNAL_COUNT = 2000;
export const ADDON = "T3WoW";
export const slotName = (n) => `${ADDON}_S${String(n).padStart(3, "0")}`;
const signalFile = (n) => `${String(n).padStart(4, "0")}.wav`;

const ADDON_SOURCE = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "addon", ADDON);

/** 50 ms of 16-bit mono silence. */
export function silentWav() {
  const samples = 2205;
  const buffer = Buffer.alloc(44 + samples * 2);
  buffer.write("RIFF", 0);
  buffer.writeUInt32LE(36 + samples * 2, 4);
  buffer.write("WAVEfmt ", 8);
  buffer.writeUInt32LE(16, 16);
  buffer.writeUInt16LE(1, 20);
  buffer.writeUInt16LE(1, 22);
  buffer.writeUInt32LE(44100, 24);
  buffer.writeUInt32LE(88200, 28);
  buffer.writeUInt16LE(2, 32);
  buffer.writeUInt16LE(16, 34);
  buffer.write("data", 36);
  buffer.writeUInt32LE(samples * 2, 40);
  return buffer;
}

export class Slots {
  constructor(addonsDir, interfaceVersion) {
    this.addonsDir = addonsDir;
    this.interfaceVersion = interfaceVersion;
    this.addonDir = join(addonsDir, ADDON);
    this.wav = silentWav();
  }

  installed() {
    return existsSync(join(this.addonDir, `${ADDON}.toc`)) && existsSync(join(this.addonsDir, slotName(SLOT_COUNT)));
  }

  install() {
    mkdirSync(this.addonsDir, { recursive: true });
    cpSync(ADDON_SOURCE, this.addonDir, { recursive: true });
    const toc = join(this.addonDir, `${ADDON}.toc`);
    writeFileSync(toc, readFileSync(toc, "utf8").replace(/^## Interface:.*$/m, `## Interface: ${this.interfaceVersion}`));
    for (const dir of ["sig", "ctl"]) mkdirSync(join(this.addonDir, dir), { recursive: true });
    writeFileSync(join(this.addonDir, "ctl", "empty.wav"), "");
    writeFileSync(join(this.addonDir, "ctl", "valid.wav"), this.wav);
    for (let n = 1; n <= SIGNAL_COUNT; n++) writeFileSync(join(this.addonDir, "sig", signalFile(n)), "");
    for (let n = 1; n <= SLOT_COUNT; n++) {
      const dir = join(this.addonsDir, slotName(n));
      mkdirSync(dir, { recursive: true });
      writeFileSync(
        join(dir, `${slotName(n)}.toc`),
        [
          `## Interface: ${this.interfaceVersion}`,
          `## Title: |cff808080T3 WoW slot ${String(n).padStart(3, "0")}|r`,
          "## Notes: Carries T3 Code updates into the game. Keep enabled.",
          "## LoadOnDemand: 1",
          `## Dependencies: ${ADDON}`,
          "Inbox.lua",
          "",
        ].join("\n"),
      );
      writeFileSync(join(dir, "Inbox.lua"), "");
    }
  }

  uninstall() {
    rmSync(this.addonDir, { recursive: true, force: true });
    for (const name of readdirSync(this.addonsDir)) {
      if (name.startsWith(`${ADDON}_S`)) rmSync(join(this.addonsDir, name), { recursive: true, force: true });
    }
  }

  writeSlot(n, lua) {
    if (n < 1 || n > SLOT_COUNT) return;
    const file = join(this.addonsDir, slotName(n), "Inbox.lua");
    writeFileSync(file + ".tmp", lua);
    renameSync(file + ".tmp", file);
  }

  raiseSignal(n) {
    if (n < 1 || n > SIGNAL_COUNT) return false;
    writeFileSync(join(this.addonDir, "sig", signalFile(n)), this.wav);
    return true;
  }

  /** Safe while the game runs: a file the client already played stays cached as valid. */
  clearSignals() {
    const dir = join(this.addonDir, "sig");
    for (let n = 1; n <= SIGNAL_COUNT; n++) {
      const file = join(dir, signalFile(n));
      if (existsSync(file) && statSync(file).size > 0) writeFileSync(file, "");
    }
  }
}

