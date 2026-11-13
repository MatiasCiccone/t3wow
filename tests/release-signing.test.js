// Release signing: scripts/sign-release.mjs signs t3wow.exe.sha256 in the Release workflow,
// and the companion's updater (companion/src/updater.rs) checks the signature with the
// public key it embeds (companion/src/updater/release_key.rs). The fixture is checked from
// both sides.
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash, createPrivateKey, createPublicKey, generateKeyPairSync, verify } from "node:crypto";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { insideRepo } from "../scripts/gen-signing-key.mjs";
import { embeddedPublicKey, KEY_FILE, rawPublicKey, releaseKey, rustKeyLiteral, signatureFile, signingKey, withPublicKey } from "../scripts/release-key.mjs";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const fixture = JSON.parse(readFileSync(new URL("../companion/tests/fixtures/signed-release.json", import.meta.url), "utf8"));
const keyFileWith = (raw) => `pub(super) const RELEASE_PUBLIC_KEY: [u8; 32] = ${rustKeyLiteral(raw)};\n`;
const repoKey = () => embeddedPublicKey(readFileSync(KEY_FILE, "utf8"));
// A repository file with LF line ends, whichever ones the checkout gave it.
const readText = (path) => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");
const publicKeyFromRaw = (raw) => createPublicKey({ key: { kty: "OKP", crv: "Ed25519", x: raw.toString("base64url") }, format: "jwk" });

test("the fixture's signature and checksum match its exe", () => {
  const exe = Buffer.alloc(70000);
  exe.write("MZ");
  assert.equal(fixture.checksum, `${createHash("sha256").update(exe).digest("hex")}  t3wow.exe`);
  const key = publicKeyFromRaw(Buffer.from(fixture.publicKey, "hex"));
  assert.ok(verify(null, Buffer.from(fixture.checksum), key, Buffer.from(fixture.signature.trim(), "base64")));
});

test("a release is signed only with the key whose public half the companion embeds", () => {
  const { privateKey } = generateKeyPairSync("ed25519");
  const pem = privateKey.export({ format: "pem", type: "pkcs8" });
  const raw = rawPublicKey(privateKey);
  const key = releaseKey(pem, keyFileWith(raw));
  const data = Buffer.from("02aebc3eb5dc2ccf20933eb608352fb8ecdf86342e308e3cec4d53c1dd9e46bc  t3wow.exe");
  const sig = signatureFile(key, data);
  assert.match(sig, /^[A-Za-z0-9+/]{86}==\n$/);
  assert.ok(verify(null, data, publicKeyFromRaw(raw), Buffer.from(sig, "base64")));

  const other = rawPublicKey(generateKeyPairSync("ed25519").privateKey);
  assert.throws(() => releaseKey(pem, keyFileWith(other)), /doesn't match RELEASE_PUBLIC_KEY/);
  assert.throws(() => releaseKey(pem, "const RELEASE_PUBLIC_KEY: [u8; 32] = [0; 32];"), /placeholder/);
  assert.throws(() => releaseKey("", keyFileWith(raw)), /T3WOW_SIGNING_KEY is not set/);
  assert.throws(() => releaseKey("not a pem", keyFileWith(raw)), /isn't a private key/);
  const rsa = generateKeyPairSync("rsa", { modulusLength: 1024 }).privateKey.export({ format: "pem", type: "pkcs8" });
  assert.throws(() => releaseKey(rsa, keyFileWith(raw)), /not Ed25519/);
});

test("release_key.rs holds the placeholder or a 32-byte key, and updater.rs uses it", () => {
  const embedded = repoKey();
  assert.ok(embedded === null || embedded.length === 32);
  assert.throws(() => embeddedPublicKey("const RELEASE_PUBLIC_KEY: [u8; 32] = [1, 2, 3];"), /isn't 32 bytes/);
  assert.throws(() => embeddedPublicKey("const OTHER: [u8; 32] = [0; 32];"), /not found/);
  // The key stays out of updater.rs, so setting it never conflicts with updater changes.
  const updater = readText("../companion/src/updater.rs");
  assert.match(updater, /^mod release_key;$/m);
  assert.doesNotMatch(updater, /const RELEASE_PUBLIC_KEY/);
});

test("the generator's edit puts the key in place of the placeholder, and never replaces a real key", () => {
  const placeholder = readFileSync(KEY_FILE, "utf8").replace(/= \[[^\]]*\];/, "= [0; 32];");
  const raw = rawPublicKey(generateKeyPairSync("ed25519").privateKey);
  const written = withPublicKey(placeholder, raw);
  assert.deepEqual(embeddedPublicKey(written), raw);
  // Only the key changes: the doc comment and the visibility stay.
  assert.equal(written.replace(/= \[[^\]]*\];/, "= [0; 32];"), placeholder);
  assert.match(written, /^pub\(super\) const RELEASE_PUBLIC_KEY: \[u8; 32\] = \[\r?\n {4}0x/m);
  const other = rawPublicKey(generateKeyPairSync("ed25519").privateKey);
  assert.throws(() => withPublicKey(written, other), /already holds a release key/);
});

test("a release goes out unsigned only while signing isn't set up at all", () => {
  const placeholder = "const RELEASE_PUBLIC_KEY: [u8; 32] = [0; 32];";
  const { privateKey } = generateKeyPairSync("ed25519");
  const pem = privateKey.export({ format: "pem", type: "pkcs8" });
  const raw = rawPublicKey(privateKey);
  assert.equal(signingKey("", placeholder), null);
  assert.equal(signingKey(undefined, placeholder), null);
  // Half set up: the secret without the key, or the key without the secret.
  assert.throws(() => signingKey(pem, placeholder), /placeholder/);
  assert.throws(() => signingKey("", keyFileWith(raw)), /T3WOW_SIGNING_KEY is not set/);
  assert.ok(signingKey(pem, keyFileWith(raw)));
});

test("the workflow's key check without the secret: a warning before the key is set up, an error after", () => {
  const env = { ...process.env, GITHUB_ACTIONS: "true" };
  delete env.T3WOW_SIGNING_KEY;
  const dir = mkdtempSync(join(tmpdir(), "t3wow-output-"));
  env.GITHUB_OUTPUT = join(dir, "output");
  const run = spawnSync(process.execPath, [join(ROOT, "scripts", "sign-release.mjs"), "--check"], { env, encoding: "utf8" });
  const output = existsSync(env.GITHUB_OUTPUT) ? readFileSync(env.GITHUB_OUTPUT, "utf8") : "";
  rmSync(dir, { recursive: true, force: true });
  if (repoKey() === null) {
    assert.equal(run.status, 0, run.stderr);
    assert.match(run.stdout, /^::warning::Release signing isn't set up/);
    // The step output the workflow's later steps read.
    assert.equal(output, "signed=false\n");
  } else {
    assert.equal(run.status, 1);
    assert.match(run.stderr, /^::error::T3WOW_SIGNING_KEY is not set/);
  }
});

test("the key generator keeps the private key out of the repository", () => {
  assert.equal(insideRepo(join(ROOT, "release-key.pem")), true);
  assert.equal(insideRepo(join(ROOT, "scripts", "release-key.pem")), true);
  const refused = spawnSync(process.execPath, [join(ROOT, "scripts", "gen-signing-key.mjs"), join(ROOT, "release-key.pem")], { encoding: "utf8" });
  assert.equal(refused.status, 1);
  assert.match(refused.stderr, /inside the repository/);
  assert.equal(existsSync(join(ROOT, "release-key.pem")), false);

  const dir = mkdtempSync(join(tmpdir(), "t3wow-key-"));
  try {
    const out = join(dir, "throwaway.pem");
    assert.equal(insideRepo(out), false);
    // Always --print here: without it the throwaway key would go into the repository's
    // release_key.rs.
    const before = readFileSync(KEY_FILE, "utf8");
    const printed = execFileSync(process.execPath, [join(ROOT, "scripts", "gen-signing-key.mjs"), "--print", out], { encoding: "utf8" });
    assert.equal(readFileSync(KEY_FILE, "utf8"), before);
    const literal = /const RELEASE_PUBLIC_KEY: \[u8; 32\] = \[[^\]]*\];/.exec(printed)[0];
    assert.deepEqual(embeddedPublicKey(literal), rawPublicKey(createPrivateKey(readFileSync(out))));
    const again = spawnSync(process.execPath, [join(ROOT, "scripts", "gen-signing-key.mjs"), "--print", out], { encoding: "utf8" });
    assert.equal(again.status, 1);
    assert.match(again.stderr, /already exists/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test("the release workflow signs the exe before hashing it, and the checksum after", () => {
  const workflow = readText("../.github/workflows/release.yml");
  const step = (name) => {
    const at = workflow.indexOf(`- name: ${name}\n`);
    assert.notEqual(at, -1, `no "${name}" step`);
    return at;
  };
  // Authenticode changes the exe's bytes: the SHA-256 and its signature must cover the result.
  assert.ok(step("Stage t3wow.exe") < step("Authenticode (SSL.com eSigner)"));
  assert.ok(step("Authenticode (SSL.com eSigner)") < step("Check the Authenticode signature"));
  assert.ok(step("Check the Authenticode signature") < step("Package"));
  assert.ok(step("Package") < step("Sign the checksum"));
  // The release key is checked, and an unsigned release after a signed one refused, before building.
  assert.ok(step("Release key") < step("Signed releases stay signed"));
  assert.ok(step("Signed releases stay signed") < workflow.indexOf("cargo build --release"));
  assert.match(workflow, /- name: Signed releases stay signed\n\s+if: steps\.key\.outputs\.signed != 'true'/);
  // Authenticode is optional: gated on its secret, so a release without the certificate still runs.
  assert.match(workflow, /AUTHENTICODE: \$\{\{ secrets\.ESIGNER_TOTP_SECRET != '' \}\}/);
  for (const name of ["Authenticode (SSL.com eSigner)", "Check the Authenticode signature"]) {
    assert.match(workflow.slice(step(name)), /^- name: .+\n\s+if: env\.AUTHENTICODE == 'true'\n/);
  }
});

test("the Mac app is built on the Mac runner, and its checksum is signed and published with the exe's", () => {
  const workflow = readText("../.github/workflows/release.yml");
  const job = (name) => {
    const at = workflow.indexOf(`\n  ${name}:\n`);
    assert.notEqual(at, -1, `no ${name} job`);
    const next = workflow.slice(at + 1).search(/\n  [a-z-]+:\n/);
    return workflow.slice(at, next === -1 ? undefined : at + 1 + next);
  };
  // Not the PC's [self-hosted, t3wow] runners, which can't build for macOS.
  assert.match(job("build-macos"), /runs-on: \[self-hosted, t3wow-mac\]/);
  assert.match(job("build-macos"), /npm run build:app/);
  assert.match(job("build-windows"), /runs-on: windows-latest/);
  const publish = job("publish");
  assert.match(publish, /needs: \[build-windows, build-macos\]/);
  for (const sha of ["t3wow.exe.sha256", "t3wow-macos-arm64.zip.sha256"]) {
    assert.ok(publish.includes(`node scripts/sign-release.mjs release/${sha}\n`), sha);
  }
  const files = publish.slice(publish.indexOf("files: |")).split("\n").map((line) => line.trim());
  for (const file of ["t3wow.exe", "t3wow.exe.sha256", "t3wow.exe.sha256.sig", "t3wow-macos-arm64.zip", "t3wow-macos-arm64.zip.sha256", "t3wow-macos-arm64.zip.sha256.sig"]) {
    assert.ok(files.includes(`release/${file}`), file);
  }
  // Signing happens once both builds are in, before the release is published.
  assert.ok(publish.indexOf("- name: Sign the checksum") < publish.indexOf("softprops/action-gh-release"));
});
