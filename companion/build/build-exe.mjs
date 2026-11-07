// Builds dist/t3wow.exe: the companion as one Windows executable (Node's single
// executable application support), with the addon and the capture script embedded.
//
//   npm run build:exe
//
// Steps: bundle the companion into one CommonJS file (esbuild), turn it and the assets
// into a blob (node --experimental-sea-config), copy this node.exe and inject the blob
// (postject). The result needs nothing installed: no Node, no repo.
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";
import { inject } from "postject";
import { ADDON_FILES } from "../src/assets.js";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const DIST = process.env.T3WOW_DIST ?? join(ROOT, "dist");
const at = (...parts) => join(ROOT, ...parts);
// RFC 3161 timestamp: keeps the signature valid after the certificate expires.
const TIMESTAMP_URL = process.env.T3WOW_TIMESTAMP_URL ?? "http://timestamp.digicert.com";

if (process.platform !== "win32") {
  console.error("t3wow.exe is built on Windows (it embeds this machine's node.exe).");
  process.exit(1);
}

try {
  rmSync(DIST, { recursive: true, force: true });
} catch (error) {
  if (error.code !== "EPERM" && error.code !== "EBUSY") throw error;
  console.error(`${DIST} is in use: is t3wow.exe running from there? Close it, or build elsewhere with T3WOW_DIST.`);
  process.exit(1);
}
mkdirSync(DIST, { recursive: true });

// 1. One CommonJS file. import.meta is only read when running from source (assets.js).
await build({
  entryPoints: [at("companion", "bin", "t3wow.js")],
  outfile: join(DIST, "t3wow.cjs"),
  bundle: true,
  platform: "node",
  format: "cjs",
  target: `node${process.versions.node.split(".")[0]}`,
  logOverride: { "empty-import-meta": "silent" },
});

// 2. The blob: the bundle plus the files the companion writes or runs.
const assets = { "capture.ps1": at("companion", "src", "capture.ps1") };
for (const name of ADDON_FILES) assets[`addon/${name}`] = at("addon", "T3WoW", name);
const config = {
  main: join(DIST, "t3wow.cjs"),
  output: join(DIST, "sea-prep.blob"),
  disableExperimentalSEAWarning: true,
  useCodeCache: true,
  assets,
};
writeFileSync(join(DIST, "sea-config.json"), JSON.stringify(config, null, 2));
execFileSync(process.execPath, ["--experimental-sea-config", join(DIST, "sea-config.json")], { stdio: "inherit" });

// 3. A copy of this node.exe with the blob injected. Node's own signature would only be
// broken by the injection, so it comes off first (when signtool is around).
const exe = join(DIST, "t3wow.exe");
copyFileSync(process.execPath, exe);
const signtool = findSigntool();
if (signtool) execFileSync(signtool, ["remove", "/s", exe], { stdio: "ignore" });
await inject(exe, "NODE_SEA_BLOB", readFileSync(config.output), {
  sentinelFuse: "NODE_SEA_FUSE_fce680ab2cc467b6e072b8b5df1996b2",
});

// 4. Signing, when a certificate is configured (see signArgs).
const sign = signArgs();
if (sign) {
  if (!signtool) throw new Error("Signing is configured but signtool.exe was not found: install the Windows SDK, or set T3WOW_SIGNTOOL.");
  execFileSync(signtool, ["sign", "/fd", "SHA256", "/tr", TIMESTAMP_URL, "/td", "SHA256", "/d", "T3 WoW companion", ...sign, exe], {
    stdio: "inherit",
  });
}

for (const name of ["t3wow.cjs", "sea-prep.blob", "sea-config.json"]) rmSync(join(DIST, name), { force: true });
const size = (statSync(exe).size / 1024 / 1024).toFixed(0);
const version = /^## Version:\s*(\S+)/m.exec(readFileSync(at("addon", "T3WoW", "T3WoW.toc"), "utf8"))?.[1];
console.log(`Built ${relative(ROOT, exe)} (${size} MB, addon ${version}, Node ${process.version}).`);
if (!sign) {
  console.log("Unsigned: Windows SmartScreen may warn the first time it runs (More info > Run anyway).");
  console.log("To sign, set T3WOW_SIGN_THUMBPRINT, T3WOW_SIGN_PFX or T3WOW_SIGN_DLIB (see companion/build/build-exe.mjs).");
}

function findSigntool() {
  if (process.env.T3WOW_SIGNTOOL) return process.env.T3WOW_SIGNTOOL;
  const kits = "C:\\Program Files (x86)\\Windows Kits\\10\\bin";
  if (!existsSync(kits)) return null;
  const versions = readdirSync(kits).filter((name) => /^10\./.test(name)).sort().reverse();
  for (const version of versions) {
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
