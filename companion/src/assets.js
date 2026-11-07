import { mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

// The files the companion ships: the addon and the screen-capture script. From a source
// checkout they are read from the repo; inside t3wow.exe they are embedded assets
// (see companion/build/build-exe.mjs), named "addon/<file>" and "capture.ps1".

function seaModule() {
  try {
    // Inside the bundled exe `require` is Node's built-in-only require; from source it isn't defined.
    const load = typeof require === "function" ? require : createRequire(import.meta.url);
    const sea = load("node:sea");
    return sea.isSea() ? sea : null;
  } catch {
    return null;
  }
}

const sea = seaModule();
export const isExe = sea !== null;

const REPO = isExe ? null : join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const ADDON_DIR = REPO && join(REPO, "addon", "T3WoW");

/** Addon file names, as embedded by the build. */
export const ADDON_FILES = ["T3WoW.toc", "Bindings.xml", "Codec.lua", "UI.lua", "Link.lua", "T3WoW.lua"];

/** The addon's files as { name: Buffer }. */
export function addonFiles() {
  const out = {};
  if (isExe) {
    for (const name of ADDON_FILES) out[name] = Buffer.from(sea.getRawAsset(`addon/${name}`));
  } else {
    for (const name of readdirSync(ADDON_DIR)) out[name] = readFileSync(join(ADDON_DIR, name));
  }
  return out;
}

/** The `## Version:` of the shipped addon. */
export function addonVersion() {
  return parseVersion(addonFiles()["T3WoW.toc"]?.toString("utf8"));
}

export function parseVersion(toc) {
  return /^## Version:\s*(\S+)/m.exec(toc ?? "")?.[1] ?? null;
}

/** -1, 0 or 1, comparing dotted version numbers. */
export function compareVersions(a, b) {
  const pa = String(a ?? "0").split(".").map(Number);
  const pb = String(b ?? "0").split(".").map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] || 0) - (pb[i] || 0);
    if (d) return Math.sign(d);
  }
  return 0;
}

/** A path to capture.ps1 that PowerShell can run (extracted next to the config for the exe). */
export function captureScript(configDir) {
  if (!isExe) return join(REPO, "companion", "src", "capture.ps1");
  mkdirSync(configDir, { recursive: true });
  const file = join(configDir, "capture.ps1");
  writeFileSync(file, Buffer.from(sea.getRawAsset("capture.ps1")));
  return file;
}
