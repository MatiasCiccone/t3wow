// Builds dist/T3 WoW.app, the companion as a Mac menu bar app, and its zip for a release:
//
//   npm run build:app
//
// makes dist/T3 WoW.app, dist/t3wow-macos-arm64.zip and dist/t3wow-macos-arm64.zip.sha256
// ("<sha-256>  t3wow-macos-arm64.zip", no newline, as for t3wow.exe). The release workflow
// signs that checksum file with scripts/sign-release.mjs, as for Windows.
//
// The app is code signed with T3WOW_CODESIGN_IDENTITY (a certificate's name in the keychain),
// or ad hoc ("-", the default) without one. T3WOW_CODESIGN_TIMESTAMP=1 adds a secure
// timestamp (needs the network and a real certificate). T3WOW_RELEASE=1, as for the exe, gives
// the plain version ("0.10.0"); any other build names its commit (companion/build.rs).
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { APP_NAME, checksumFile, codesignArgs, EXECUTABLE, ICONSET, infoPlist, outDirFromMessages, ZIP_NAME } from "./app-bundle.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const DIST = process.env.T3WOW_DIST ?? join(ROOT, "dist");
const IDENTITY = process.env.T3WOW_CODESIGN_IDENTITY || "-";
// One picture for every size: the addon's logo, the same artwork as t3wow.ico, at 512 pixels.
const ICON_SOURCE = join(ROOT, "assets", "logo.png");

if (process.platform !== "darwin") {
  console.error("T3 WoW.app is a Mac app: build it on a Mac.");
  process.exit(1);
}

const run = (command, args, options = {}) => execFileSync(command, args, { stdio: "inherit", ...options });

// The build script's folder, from cargo's messages, holds the version it computed.
const messages = execFileSync(
  "cargo",
  ["build", "--release", "--manifest-path", join(ROOT, "companion", "Cargo.toml"), "--message-format=json-render-diagnostics"],
  { stdio: ["ignore", "pipe", "inherit"], maxBuffer: 256 * 1024 * 1024, encoding: "utf8" },
);
const version = readFileSync(join(outDirFromMessages(messages), "t3wow-version.txt"), "utf8").trim();
if (!version) throw new Error("the companion's build script wrote no version");

const app = join(DIST, `${APP_NAME}.app`);
const contents = join(app, "Contents");
rmSync(app, { recursive: true, force: true });
mkdirSync(join(contents, "MacOS"), { recursive: true });
mkdirSync(join(contents, "Resources"), { recursive: true });
const program = join(contents, "MacOS", EXECUTABLE);
copyFileSync(join(ROOT, "companion", "target", "release", "t3wow"), program);
chmodSync(program, 0o755);
writeFileSync(join(contents, "Info.plist"), infoPlist(version));
writeFileSync(join(contents, "PkgInfo"), "APPL????");
run("/usr/bin/plutil", ["-lint", "-s", join(contents, "Info.plist")]);

// AppIcon.icns, made here from the logo rather than kept as a second copy of the artwork.
const work = mkdtempSync(join(tmpdir(), "t3wow-icon-"));
try {
  const iconset = join(work, "AppIcon.iconset");
  mkdirSync(iconset);
  for (const [name, size] of ICONSET) {
    run("/usr/bin/sips", ["-s", "format", "png", "-z", String(size), String(size), ICON_SOURCE, "--out", join(iconset, name)], { stdio: "ignore" });
  }
  run("/usr/bin/iconutil", ["-c", "icns", iconset, "-o", join(contents, "Resources", "AppIcon.icns")]);
} finally {
  rmSync(work, { recursive: true, force: true });
}

// Extended attributes (Finder info, quarantine) would fail codesign's strict check.
run("/usr/bin/xattr", ["-cr", app]);
run("/usr/bin/codesign", codesignArgs(IDENTITY, app, { timestamp: process.env.T3WOW_CODESIGN_TIMESTAMP === "1" }));
run("/usr/bin/codesign", ["--verify", "--deep", "--strict", "--verbose=2", app]);

// The release's zip: the app as Finder would zip it, without extended attributes.
const zip = join(DIST, ZIP_NAME);
rmSync(zip, { force: true });
run("/usr/bin/ditto", ["-c", "-k", "--norsrc", "--keepParent", app, zip]);
const hex = createHash("sha256").update(readFileSync(zip)).digest("hex");
writeFileSync(`${zip}.sha256`, checksumFile(hex));

const mb = (file) => (statSync(file).size / 1024 / 1024).toFixed(1);
console.log(`Built ${relative(ROOT, app)} (${version}, program ${mb(program)} MB) and ${relative(ROOT, zip)} (${mb(zip)} MB, sha256 ${hex}).`);
if (IDENTITY === "-") {
  console.log("Signed ad hoc: macOS asks for the Keychain and Screen Recording again after each update, and Gatekeeper blocks it once when downloaded");
  console.log("(System Settings > Privacy & Security > Open Anyway). Set T3WOW_CODESIGN_IDENTITY to sign with a certificate.");
}
