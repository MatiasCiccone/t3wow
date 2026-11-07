import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { addonFiles, addonVersion, compareVersions, parseVersion } from "./assets.js";

// The game reads an addon's files only when it loads that addon, and finds only files
// that existed when the client started. So replies travel through a pool of
// load-on-demand "slot" addons (each loads once per UI session), and "a reply is
// ready" travels through pre-made font files: a blank one never loads, a real one does.
// (Sound files were the first idea, but some clients call even a blank .wav playable.)
export const SLOT_COUNT = 200;
export const SIGNAL_COUNT = 4000; // keep in step with addon/T3WoW/Link.lua
export const ADDON = "T3WoW";
export const slotName = (n) => `${ADDON}_S${String(n).padStart(3, "0")}`;
const signalFile = (n) => `${String(n).padStart(4, "0")}.ttf`;

/** A 2x2 white 32-bit TGA, the smallest texture the game is sure to accept. */
export function tinyTga() {
  const header = Buffer.from([0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, 32, 0x28]);
  return Buffer.concat([header, Buffer.alloc(16, 0xff)]);
}

/** A Windows text font, or null. Only copied on this PC, never shipped. Symbol fonts
 * (Marlett and the like) don't load in the game, so these are plain text fonts. */
function windowsFont(...names) {
  const dir = join(process.env.WINDIR ?? "C:\\Windows", "Fonts");
  for (const name of names) {
    const file = join(dir, name);
    if (existsSync(file)) return readFileSync(file);
  }
  return null;
}
// Every signal is a copy of this font (Lucida Console is about 110 KB), so small wins.
const smallFont = () => windowsFont("lucon.ttf", "verdana.ttf", "arial.ttf");

// /t3 probe looks for a file type whose change the game notices while running: it compares
// a blank and a real file of each type, then asks us to turn the "flip" files real.
// The addon also uses probe/empty.ttf and probe/valid.ttf to check fonts work at all.
const PROBE_FILES = { ttf: smallFont, tga: tinyTga };

export class Slots {
  constructor(addonsDir, interfaceVersion) {
    this.addonsDir = addonsDir;
    this.interfaceVersion = interfaceVersion;
    this.addonDir = join(addonsDir, ADDON);
    this.font = smallFont();
  }

  installed() {
    return existsSync(join(this.addonDir, `${ADDON}.toc`)) && existsSync(join(this.addonsDir, slotName(SLOT_COUNT)));
  }

  installedVersion() {
    try {
      return parseVersion(readFileSync(join(this.addonDir, `${ADDON}.toc`), "utf8"));
    } catch {
      return null;
    }
  }

  /** Writes the shipped addon files; the toc gets this game's interface version. */
  writeAddon() {
    mkdirSync(this.addonDir, { recursive: true });
    for (const [name, data] of Object.entries(addonFiles())) {
      const out =
        name === `${ADDON}.toc`
          ? data.toString("utf8").replace(/^## Interface:.*$/m, `## Interface: ${this.interfaceVersion}`)
          : data;
      writeFileSync(join(this.addonDir, name), out);
    }
  }

  /**
   * Creates the slot addons, blank signal files and probe files. With `reset`, existing
   * ones are blanked too (a fresh install); without, only missing ones are created.
   * Returns how many files did not exist before.
   */
  writeSupport(reset) {
    let created = 0;
    const put = (file, data) => {
      const exists = existsSync(file);
      if (!exists) created++;
      if (reset || !exists) writeFileSync(file, data);
    };
    if (reset) {
      // Older installs used .wav signals; start those folders clean.
      rmSync(join(this.addonDir, "sig"), { recursive: true, force: true });
      for (const old of ["empty.wav", "valid.wav"]) rmSync(join(this.addonDir, "ctl", old), { force: true });
    }
    const probe = join(this.addonDir, "ctl", "probe");
    for (const dir of [join(this.addonDir, "sig"), probe]) mkdirSync(dir, { recursive: true });
    for (let n = 1; n <= SIGNAL_COUNT; n++) put(join(this.addonDir, "sig", signalFile(n)), "");
    for (const [ext, real] of Object.entries(PROBE_FILES)) {
      put(join(probe, `empty.${ext}`), "");
      put(join(probe, `valid.${ext}`), real() ?? "");
      put(join(probe, `flip.${ext}`), "");
    }
    put(join(probe, "valid2.ttf"), windowsFont("arial.ttf", "segoeui.ttf") ?? "");
    for (let n = 1; n <= SLOT_COUNT; n++) {
      const dir = join(this.addonsDir, slotName(n));
      mkdirSync(dir, { recursive: true });
      put(
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
      put(join(dir, "Inbox.lua"), "");
    }
    return created;
  }

  install() {
    if (!this.font) throw new Error("No usable font in the Windows Fonts folder (looked for lucon, verdana, arial).");
    mkdirSync(this.addonsDir, { recursive: true });
    this.writeAddon();
    this.writeSupport(true);
  }

  /**
   * For every companion start: puts back whatever an addon manager or a cleanup removed,
   * and installs or updates the addon when this companion ships a newer one. Files the
   * game didn't have when it started only show up after a full restart of WoW.
   */
  repair() {
    if (!this.font) throw new Error("No usable font in the Windows Fonts folder (looked for lucon, verdana, arial).");
    mkdirSync(this.addonsDir, { recursive: true });
    const shipped = addonVersion();
    const installed = this.installedVersion();
    let addon = null;
    if (!installed) {
      this.writeAddon();
      addon = `installed ${shipped}`;
    } else if (compareVersions(shipped, installed) > 0) {
      this.writeAddon();
      addon = `updated ${installed} to ${shipped}`;
    }
    return { addon, created: this.writeSupport(false) };
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
    if (n < 1 || n > SIGNAL_COUNT || !this.font) return false;
    writeFileSync(join(this.addonDir, "sig", signalFile(n)), this.font);
    return true;
  }

  /** Makes the probe's flip files real (`real`) or blank again. Returns the types changed. */
  flipProbe(real) {
    const changed = [];
    for (const [ext, make] of Object.entries(PROBE_FILES)) {
      const file = join(this.addonDir, "ctl", "probe", `flip.${ext}`);
      if (!existsSync(file)) continue;
      writeFileSync(file, real ? (make() ?? "") : "");
      changed.push(ext);
    }
    return changed;
  }

  /** For a new game process, which hasn't loaded any signal font yet: all start blank. */
  clearSignals() {
    const dir = join(this.addonDir, "sig");
    for (let n = 1; n <= SIGNAL_COUNT; n++) {
      const file = join(dir, signalFile(n));
      if (existsSync(file) && statSync(file).size > 0) writeFileSync(file, "");
    }
  }
}
