// The Mac app's pieces (scripts/app-bundle.mjs), which scripts/build-app.mjs puts together.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { BUNDLE_ID, checksumFile, CODESIGN_TIMEOUT_MS, codesignArgs, codesignTimeoutMessage, ICONSET, infoPlist, outDirFromMessages, ZIP_NAME } from "../scripts/app-bundle.mjs";

const pairs = (plist) => Object.fromEntries([...plist.matchAll(/<key>([^<]+)<\/key>\s*<(string|true|array)\/?>([^<]*)/g)].map((m) => [m[1], m[2] === "true" ? true : m[3]]));

test("Info.plist names the app, its version and how it runs", () => {
  const plist = infoPlist("0.10.0-dev+4c1db55");
  const keys = pairs(plist);
  assert.equal(keys.CFBundleIdentifier, "com.t3wow.companion");
  assert.equal(keys.CFBundleName, "T3 WoW");
  assert.equal(keys.CFBundleDisplayName, "T3 WoW");
  assert.equal(keys.CFBundleExecutable, "t3wow");
  assert.equal(keys.CFBundleShortVersionString, "0.10.0-dev+4c1db55");
  assert.equal(keys.CFBundleVersion, "0.10.0-dev+4c1db55");
  assert.equal(keys.CFBundleIconFile, "AppIcon");
  assert.equal(keys.LSMinimumSystemVersion, "12.3");
  assert.equal(keys.LSUIElement, true);
  assert.equal(keys.NSHighResolutionCapable, true);
  assert.match(plist, /<key>LSArchitecturePriority<\/key>\n\t<array>\n\t\t<string>arm64<\/string>\n\t<\/array>/);
  assert.equal(pairs(infoPlist("0.10.0")).CFBundleShortVersionString, "0.10.0");
  assert.match(infoPlist("1 & <2>"), /<string>1 &amp; &lt;2&gt;<\/string>/);
});

test("Info.plist is a valid property list", { skip: process.platform !== "darwin" && "plutil is on a Mac" }, () => {
  const dir = mkdtempSync(join(tmpdir(), "t3wow-plist-"));
  try {
    const file = join(dir, "Info.plist");
    writeFileSync(file, infoPlist("0.10.0"));
    execFileSync("/usr/bin/plutil", ["-lint", "-s", file]);
    const json = JSON.parse(execFileSync("/usr/bin/plutil", ["-convert", "json", "-o", "-", file], { encoding: "utf8" }));
    assert.deepEqual(json.LSArchitecturePriority, ["arm64"]);
    assert.equal(json.CFBundleIdentifier, BUNDLE_ID);
    assert.equal(json.LSUIElement, true);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("the checksum file reads like t3wow.exe.sha256, for the zip", () => {
  const hex = "ab".repeat(32);
  assert.equal(ZIP_NAME, "t3wow-macos-arm64.zip");
  assert.equal(checksumFile(hex), `${hex}  t3wow-macos-arm64.zip`);
  assert.throws(() => checksumFile("ABC"));
});

test("the version comes from the companion's build script folder", () => {
  const stdout = [
    "not json",
    JSON.stringify({ reason: "build-script-executed", package_id: "registry+https://github.com/rust-lang/crates.io-index#ring@0.17.14", out_dir: "/x/ring" }),
    JSON.stringify({ reason: "compiler-artifact", package_id: "path+file:///r/companion#t3wow@0.10.0" }),
    JSON.stringify({ reason: "build-script-executed", package_id: "path+file:///r/companion#t3wow@0.10.0", out_dir: "/r/companion/target/release/build/t3wow-1/out" }),
    "",
  ].join("\n");
  assert.equal(outDirFromMessages(stdout), "/r/companion/target/release/build/t3wow-1/out");
  assert.equal(outDirFromMessages(JSON.stringify({ reason: "build-script-executed", package_id: "t3wow 0.10.0 (path+file:///r)", out_dir: "/o" })), "/o");
  assert.throws(() => outDirFromMessages(stdout.split("\n").slice(0, 2).join("\n")), /build script/);
});

test("codesign runs with the hardened runtime, ad hoc unless an identity is given", () => {
  assert.deepEqual(codesignArgs("-", "T3 WoW.app"), ["--force", "--options", "runtime", "--timestamp=none", "--sign", "-", "T3 WoW.app"]);
  assert.deepEqual(codesignArgs("T3 WoW Dev", "a.app", { timestamp: true }).slice(3, 6), ["--timestamp", "--sign", "T3 WoW Dev"]);
});

test("codesign looks only in the given keychain, and a dialog it waits on fails the build", () => {
  assert.deepEqual(codesignArgs("ABCD", "a.app", { keychain: "/tmp/ci.keychain-db" }).slice(3, 8), ["--timestamp=none", "--keychain", "/tmp/ci.keychain-db", "--sign", "ABCD"]);
  assert.ok(!codesignArgs("-", "a.app", { keychain: "/tmp/ci.keychain-db" }).includes("--keychain"), "ad hoc uses no keychain");
  assert.ok(CODESIGN_TIMEOUT_MS > 0 && CODESIGN_TIMEOUT_MS <= 300_000, "well inside the job's own timeout");
  const message = codesignTimeoutMessage("ABCD", "/tmp/ci.keychain-db");
  for (const words of ["ABCD", "/tmp/ci.keychain-db", "keychain dialog", "set-key-partition-list", "unlock-keychain"]) {
    assert.ok(message.includes(words), words);
  }
});

test("build:app gives codesign a timeout and passes it the keychain", () => {
  const script = readFileSync(new URL("../scripts/build-app.mjs", import.meta.url), "utf8");
  assert.match(script, /timeout: CODESIGN_TIMEOUT_MS/);
  assert.match(script, /keychain: KEYCHAIN/);
  assert.match(script, /codesignTimeoutMessage\(IDENTITY, KEYCHAIN\)/);
});

test("the iconset has every size iconutil reads, from 16 to 512 pixels", () => {
  for (const [name, size] of ICONSET) {
    const [, base, double] = /^icon_(\d+)x\1(@2x)?\.png$/.exec(name);
    assert.equal(size, Number(base) * (double ? 2 : 1), name);
  }
  assert.equal(ICONSET.length, 9);
});

test("npm run build:app runs the bundle script", () => {
  const scripts = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8")).scripts;
  assert.equal(scripts["build:app"], "node scripts/build-app.mjs");
});
