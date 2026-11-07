import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { basename, dirname, join } from "node:path";

// Finds the WoW: Forever client folder (the one named _forever_, holding Wow.exe and
// Interface\AddOns) without asking: a running client first, then Battle.net's install
// record, then the usual places.

const FLAVOR = "_forever_";
const GUESSES = [
  "C:\\Program Files (x86)\\World of Warcraft",
  "C:\\Program Files\\World of Warcraft",
  "D:\\World of Warcraft",
  "D:\\Games\\World of Warcraft",
  "E:\\World of Warcraft",
  "E:\\Games\\World of Warcraft",
];

const looksLikeClient = (dir) => Boolean(dir) && (existsSync(join(dir, "Wow.exe")) || existsSync(join(dir, "Interface")));

function quiet(command, args) {
  try {
    return execFileSync(command, args, { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"], windowsHide: true, timeout: 10_000 });
  } catch {
    return "";
  }
}

/** The client folder of a running Wow.exe, if it is a _forever_ one. */
function fromRunningClient() {
  const out = quiet("powershell.exe", [
    "-NoProfile",
    "-Command",
    "Get-Process Wow -ErrorAction SilentlyContinue | ForEach-Object { $_.Path }",
  ]);
  for (const line of out.split(/\r?\n/)) {
    const dir = line.trim() && dirname(line.trim());
    if (dir && basename(dir).toLowerCase() === FLAVOR) return dir;
  }
  return null;
}

/** Battle.net records where World of Warcraft lives; _forever_ sits beside the other flavors. */
function fromRegistry() {
  for (const key of [
    "HKLM\\SOFTWARE\\WOW6432Node\\Blizzard Entertainment\\World of Warcraft",
    "HKLM\\SOFTWARE\\Blizzard Entertainment\\World of Warcraft",
  ]) {
    const match = /InstallPath\s+REG_SZ\s+(.+)/i.exec(quiet("reg.exe", ["query", key, "/v", "InstallPath"]));
    if (!match) continue;
    const path = match[1].trim().replace(/[\\/]+$/, "");
    // InstallPath points at one flavor (e.g. ...\_retail_) or at the root.
    const root = /^_.+_$/.test(basename(path)) ? dirname(path) : path;
    if (looksLikeClient(join(root, FLAVOR))) return join(root, FLAVOR);
  }
  return null;
}

export function findWowDir() {
  if (process.platform !== "win32") return null;
  return (
    fromRunningClient() ??
    fromRegistry() ??
    GUESSES.map((root) => join(root, FLAVOR)).find(looksLikeClient) ??
    null
  );
}

/** Accepts the _forever_ folder, its parent, or Wow.exe itself. */
export function normalizeWowDir(input) {
  let dir = String(input ?? "")
    .replace(/["']/g, "")
    .trim()
    .replace(/[\\/]+$/, "");
  if (!dir) return null;
  if (/\.exe$/i.test(dir)) dir = dirname(dir);
  if (basename(dir).toLowerCase() !== FLAVOR && looksLikeClient(join(dir, FLAVOR))) dir = join(dir, FLAVOR);
  return dir;
}

export { looksLikeClient };
