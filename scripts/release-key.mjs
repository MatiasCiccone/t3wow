// The Ed25519 release key, shared by gen-signing-key.mjs and sign-release.mjs. The
// companion's updater (companion/src/updater.rs) embeds the public half as
// RELEASE_PUBLIC_KEY and installs only a release whose t3wow.exe.sha256 carries a valid
// signature (t3wow.exe.sha256.sig: the 64-byte signature in base64).
import { createPrivateKey, createPublicKey, sign, verify } from "node:crypto";
import { readFileSync } from "node:fs";

export const UPDATER = new URL("../companion/src/updater.rs", import.meta.url);

/** The raw 32-byte public key of an Ed25519 key object (SPKI DER ends with it). */
export function rawPublicKey(key) {
  return createPublicKey(key).export({ format: "der", type: "spki" }).subarray(-32);
}

/** The public key as the Rust literal for RELEASE_PUBLIC_KEY. */
export function rustKeyLiteral(raw) {
  const bytes = [...raw].map((b) => `0x${b.toString(16).padStart(2, "0")}`);
  const rows = [];
  for (let i = 0; i < bytes.length; i += 8) rows.push(`    ${bytes.slice(i, i + 8).join(", ")},`);
  return `[\n${rows.join("\n")}\n]`;
}

/** The RELEASE_PUBLIC_KEY bytes in updater.rs' source, or null while it's the placeholder. */
export function embeddedPublicKey(source) {
  const match = /const RELEASE_PUBLIC_KEY: \[u8; 32\] = \[([^\]]*)\];/.exec(source);
  if (!match) throw new Error("RELEASE_PUBLIC_KEY not found in companion/src/updater.rs.");
  if (match[1].includes(";")) return null; // [0; 32], the placeholder
  const bytes = match[1]
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean)
    .map((s) => Number(s));
  if (bytes.length !== 32 || bytes.some((b) => !Number.isInteger(b) || b < 0 || b > 255)) {
    throw new Error("RELEASE_PUBLIC_KEY in companion/src/updater.rs isn't 32 bytes.");
  }
  return Buffer.from(bytes);
}

/**
 * The release private key from its PKCS#8 PEM, checked against the public key the
 * companion embeds: a release signed with any other key would be refused by every
 * companion, so this throws instead.
 */
export function releaseKey(pem, updaterSource = readFileSync(UPDATER, "utf8")) {
  if (!pem || !pem.trim()) {
    throw new Error("T3WOW_SIGNING_KEY is not set. Companions refuse unsigned releases: add the secret (see scripts/gen-signing-key.mjs).");
  }
  let key;
  try {
    key = createPrivateKey(pem);
  } catch (error) {
    throw new Error(`T3WOW_SIGNING_KEY isn't a private key in PEM form: ${error.message}`);
  }
  if (key.asymmetricKeyType !== "ed25519") throw new Error(`T3WOW_SIGNING_KEY is a ${key.asymmetricKeyType} key, not Ed25519.`);
  const embedded = embeddedPublicKey(updaterSource);
  if (!embedded) throw new Error("companion/src/updater.rs still has the placeholder RELEASE_PUBLIC_KEY: put in the real public key first.");
  if (!embedded.equals(rawPublicKey(key))) {
    throw new Error("T3WOW_SIGNING_KEY doesn't match RELEASE_PUBLIC_KEY in companion/src/updater.rs: companions would refuse this release.");
  }
  return key;
}

/** The signature file's contents for `data` (the checksum file's exact bytes). */
export function signatureFile(key, data) {
  const signature = sign(null, data, key);
  if (!verify(null, data, createPublicKey(key), signature)) throw new Error("The signature doesn't verify.");
  return `${signature.toString("base64")}\n`;
}
