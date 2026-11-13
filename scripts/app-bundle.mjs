// The pieces of the Mac app that scripts/build-app.mjs puts together, kept apart so tests
// can check them without building anything. The companion's updater (companion/src/updater.rs
// and updater/bundle.rs) reads the same identifier and zip name: a Rust test checks they match.

/** The app's bundle identifier: the updater only installs an app that carries it. */
export const BUNDLE_ID = "com.t3wow.companion";
/** The program inside the app, Contents/MacOS/t3wow. */
export const EXECUTABLE = "t3wow";
export const APP_NAME = "T3 WoW";
/** The zipped app in a release; its checksum is `${ZIP_NAME}.sha256`, signed as `.sha256.sig`. */
export const ZIP_NAME = "t3wow-macos-arm64.zip";
/** ScreenCaptureKit's single-window stream needs 12.3. */
export const MINIMUM_MACOS = "12.3";

const escape = (text) => String(text).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

/**
 * Info.plist for the companion's `version` (crate::VERSION, as companion/build.rs computes
 * it: "0.10.0" for a release, "0.10.0-dev+57ee2ec" otherwise). LSUIElement: a menu bar app,
 * no Dock icon. LSArchitecturePriority: run natively on Apple silicon.
 */
export function infoPlist(version) {
  const entries = [
    ["CFBundleDevelopmentRegion", "en"],
    ["CFBundleDisplayName", APP_NAME],
    ["CFBundleExecutable", EXECUTABLE],
    ["CFBundleIconFile", "AppIcon"],
    ["CFBundleIdentifier", BUNDLE_ID],
    ["CFBundleInfoDictionaryVersion", "6.0"],
    ["CFBundleName", APP_NAME],
    ["CFBundlePackageType", "APPL"],
    ["CFBundleShortVersionString", version],
    ["CFBundleVersion", version],
    ["LSApplicationCategoryType", "public.app-category.utilities"],
    ["LSArchitecturePriority", ["arm64"]],
    ["LSMinimumSystemVersion", MINIMUM_MACOS],
    ["LSUIElement", true],
    ["NSHighResolutionCapable", true],
  ];
  const value = (v) =>
    v === true ? "\t<true/>" : Array.isArray(v) ? `\t<array>\n${v.map((s) => `\t\t<string>${escape(s)}</string>`).join("\n")}\n\t</array>` : `\t<string>${escape(v)}</string>`;
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
${entries.map(([key, v]) => `\t<key>${key}</key>\n${value(v)}`).join("\n")}
</dict>
</plist>
`;
}

/** The checksum file, as for t3wow.exe: "<sha-256 hex>  <name>", no newline. */
export function checksumFile(hex, name = ZIP_NAME) {
  if (!/^[0-9a-f]{64}$/.test(hex)) throw new Error(`not a SHA-256: ${hex}`);
  return `${hex}  ${name}`;
}

/**
 * The companion's build-script output folder (where build.rs writes t3wow-version.txt), from
 * cargo's --message-format=json output.
 */
export function outDirFromMessages(stdout) {
  let found = null;
  for (const line of stdout.split("\n")) {
    if (!line.startsWith("{")) continue;
    const message = JSON.parse(line);
    // "path+file:///.../companion#t3wow@0.10.0"; older cargo: "t3wow 0.10.0 (path+file://...)".
    if (message.reason === "build-script-executed" && /(#t3wow@|^t3wow )/.test(message.package_id)) found = message.out_dir;
  }
  if (!found) throw new Error("cargo didn't say where the companion's build script ran");
  return found;
}

/**
 * codesign's arguments. `identity` "-" is ad hoc (no certificate). The hardened runtime is on:
 * the companion loads only system frameworks, so library validation costs nothing, and a
 * Developer ID build needs it for notarization. No secure timestamp unless asked for (it
 * needs Apple's timestamp server and a real certificate).
 */
export function codesignArgs(identity, app, { timestamp = false } = {}) {
  return ["--force", "--options", "runtime", timestamp ? "--timestamp" : "--timestamp=none", "--sign", identity, app];
}

/** The iconset iconutil wants, from one square picture: [file name, pixels]. */
export const ICONSET = [
  ["icon_16x16.png", 16],
  ["icon_16x16@2x.png", 32],
  ["icon_32x32.png", 32],
  ["icon_32x32@2x.png", 64],
  ["icon_128x128.png", 128],
  ["icon_128x128@2x.png", 256],
  ["icon_256x256.png", 256],
  ["icon_256x256@2x.png", 512],
  ["icon_512x512.png", 512],
];
