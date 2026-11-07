// Builds dist/t3wow.exe: the native companion (companion/, Rust), with the addon embedded.
//
//   npm run build:exe
//
// It signs the exe when a code-signing certificate is configured (see signArgs), using
// signtool.exe from the Windows SDK and an RFC 3161 timestamp.
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const DIST = process.env.T3WOW_DIST ?? join(ROOT, "dist");
const TIMESTAMP_URL = process.env.T3WOW_TIMESTAMP_URL ?? "http://timestamp.digicert.com";

if (process.platform !== "win32") {
  console.error("t3wow.exe is a Windows program: build it on Windows.");
  process.exit(1);
}

execFileSync("cargo", ["build", "--release", "--manifest-path", join(ROOT, "companion", "Cargo.toml")], { stdio: "inherit" });
mkdirSync(DIST, { recursive: true });
const exe = join(DIST, "t3wow.exe");
try {
  copyFileSync(join(ROOT, "companion", "target", "release", "t3wow.exe"), exe);
} catch (error) {
  if (error.code !== "EBUSY" && error.code !== "EPERM") throw error;
  console.error(`${exe} is in use: quit T3 WoW from its tray icon, or build elsewhere with T3WOW_DIST.`);
  process.exit(1);
}

const sign = signArgs();
if (sign) {
  const signtool = findSigntool();
  if (!signtool) throw new Error("Signing is configured but signtool.exe was not found: install the Windows SDK, or set T3WOW_SIGNTOOL.");
  execFileSync(signtool, ["sign", "/fd", "SHA256", "/tr", TIMESTAMP_URL, "/td", "SHA256", "/d", "T3 WoW companion", ...sign, exe], { stdio: "inherit" });
}

const size = (statSync(exe).size / 1024 / 1024).toFixed(1);
const version = /^## Version:\s*(\S+)/m.exec(readFileSync(join(ROOT, "addon", "T3WoW", "T3WoW.toc"), "utf8"))?.[1];
console.log(`Built ${relative(ROOT, exe)} (${size} MB, addon ${version}).`);
if (!sign) {
  console.log("Unsigned: Windows SmartScreen may warn the first time it runs (More info > Run anyway).");
  console.log("To sign, set T3WOW_SIGN_THUMBPRINT, T3WOW_SIGN_PFX or T3WOW_SIGN_DLIB (see scripts/build-exe.mjs).");
}

function findSigntool() {
  if (process.env.T3WOW_SIGNTOOL) return process.env.T3WOW_SIGNTOOL;
  const kits = "C:\\Program Files (x86)\\Windows Kits\\10\\bin";
  if (!existsSync(kits)) return null;
  for (const version of readdirSync(kits).filter((name) => /^10\./.test(name)).sort().reverse()) {
    const candidate = join(kits, version, "x64", "signtool.exe");
    if (existsSync(candidate)) return candidate;
  }
  return null;
}

/**
 * signtool arguments for the configured certificate, or null to leave the exe unsigned.
 * Set one of:
 *   T3WOW_SIGN_THUMBPRINT  a certificate in your Windows certificate store, including one
 *                          on a hardware token (how OV/EV certificates are delivered now)
 *   T3WOW_SIGN_PFX         a .pfx file, with its password in T3WOW_SIGN_PASSWORD
 *   T3WOW_SIGN_DLIB        Azure Trusted Signing: the Azure.CodeSigning.Dlib.dll path, with
 *                          T3WOW_SIGN_METADATA pointing at its metadata.json
 */
function signArgs() {
  const env = process.env;
  if (env.T3WOW_SIGN_THUMBPRINT) return ["/sha1", env.T3WOW_SIGN_THUMBPRINT];
  if (env.T3WOW_SIGN_PFX) return ["/f", env.T3WOW_SIGN_PFX, ...(env.T3WOW_SIGN_PASSWORD ? ["/p", env.T3WOW_SIGN_PASSWORD] : [])];
  if (env.T3WOW_SIGN_DLIB) {
    if (!env.T3WOW_SIGN_METADATA) throw new Error("T3WOW_SIGN_DLIB needs T3WOW_SIGN_METADATA (the Trusted Signing metadata.json).");
    return ["/dlib", env.T3WOW_SIGN_DLIB, "/dmdf", env.T3WOW_SIGN_METADATA];
  }
  return null;
}
