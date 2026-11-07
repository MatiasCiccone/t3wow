// Release signing: scripts/sign-release.mjs signs t3wow.exe.sha256 in the Release workflow,
// and the companion's updater (companion/src/updater.rs) checks the signature with the
// public key it embeds. The fixture is checked from both sides.
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash, createPrivateKey, createPublicKey, generateKeyPairSync, verify } from "node:crypto";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { insideRepo } from "../scripts/gen-signing-key.mjs";
import { embeddedPublicKey, rawPublicKey, releaseKey, rustKeyLiteral, signatureFile } from "../scripts/release-key.mjs";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const fixture = JSON.parse(readFileSync(new URL("../companion/tests/fixtures/signed-release.json", import.meta.url), "utf8"));
const updaterWith = (raw) => `const RELEASE_PUBLIC_KEY: [u8; 32] = ${rustKeyLiteral(raw)};\n`;
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
  const key = releaseKey(pem, updaterWith(raw));
  const data = Buffer.from("02aebc3eb5dc2ccf20933eb608352fb8ecdf86342e308e3cec4d53c1dd9e46bc  t3wow.exe");
  const sig = signatureFile(key, data);
  assert.match(sig, /^[A-Za-z0-9+/]{86}==\n$/);
  assert.ok(verify(null, data, publicKeyFromRaw(raw), Buffer.from(sig, "base64")));

  const other = rawPublicKey(generateKeyPairSync("ed25519").privateKey);
  assert.throws(() => releaseKey(pem, updaterWith(other)), /doesn't match RELEASE_PUBLIC_KEY/);
  assert.throws(() => releaseKey(pem, "const RELEASE_PUBLIC_KEY: [u8; 32] = [0; 32];"), /placeholder/);
  assert.throws(() => releaseKey("", updaterWith(raw)), /T3WOW_SIGNING_KEY is not set/);
  assert.throws(() => releaseKey("not a pem", updaterWith(raw)), /isn't a private key/);
  const rsa = generateKeyPairSync("rsa", { modulusLength: 1024 }).privateKey.export({ format: "pem", type: "pkcs8" });
  assert.throws(() => releaseKey(rsa, updaterWith(raw)), /not Ed25519/);
});

test("updater.rs holds the placeholder or a 32-byte key", () => {
  const embedded = embeddedPublicKey(readFileSync(new URL("../companion/src/updater.rs", import.meta.url), "utf8"));
  assert.ok(embedded === null || embedded.length === 32);
  assert.throws(() => embeddedPublicKey("const RELEASE_PUBLIC_KEY: [u8; 32] = [1, 2, 3];"), /isn't 32 bytes/);
});

test("signing without the secret fails clearly", () => {
  const env = { ...process.env, GITHUB_ACTIONS: "true" };
  delete env.T3WOW_SIGNING_KEY;
  const run = spawnSync(process.execPath, [join(ROOT, "scripts", "sign-release.mjs"), "--check"], { env, encoding: "utf8" });
  assert.equal(run.status, 1);
  assert.match(run.stderr, /^::error::T3WOW_SIGNING_KEY is not set/);
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
    const printed = execFileSync(process.execPath, [join(ROOT, "scripts", "gen-signing-key.mjs"), out], { encoding: "utf8" });
    const literal = /const RELEASE_PUBLIC_KEY: \[u8; 32\] = \[[^\]]*\];/.exec(printed)[0];
    assert.deepEqual(embeddedPublicKey(literal), rawPublicKey(createPrivateKey(readFileSync(out))));
    const again = spawnSync(process.execPath, [join(ROOT, "scripts", "gen-signing-key.mjs"), out], { encoding: "utf8" });
    assert.equal(again.status, 1);
    assert.match(again.stderr, /already exists/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
