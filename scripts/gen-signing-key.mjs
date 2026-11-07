// Makes the Ed25519 key pair releases are signed with. Run it once, on your own PC (or let
// scripts/release-signing-wizard.sh walk you through it):
//
//   node scripts/gen-signing-key.mjs C:\Users\<you>\keys\t3wow-release-key.pem
//
// It writes the private key (PKCS#8 PEM) to that path, which must be outside this
// repository, and the public key into companion/src/updater/release_key.rs, replacing the
// placeholder (it refuses to replace a real key). With --print it leaves release_key.rs
// alone and prints the public key's Rust literal instead. Then commit release_key.rs, add
// the private key as the T3WOW_SIGNING_KEY secret (gh secret set T3WOW_SIGNING_KEY < <path>)
// and keep a backup somewhere safe: without it, no new release can update existing
// companions. See docs/release-signing.md.
import { generateKeyPairSync } from "node:crypto";
import { existsSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { KEY_FILE, rawPublicKey, rustKeyLiteral, withPublicKey } from "./release-key.mjs";

const ROOT = realpathSync(join(dirname(fileURLToPath(import.meta.url)), ".."));

/** True when `path` (resolved through its existing folder) is inside the repository. */
export function insideRepo(path, root = ROOT) {
  // Caseless where file systems usually are; a false "inside" only refuses a path.
  const fold = (p) => (process.platform === "win32" || process.platform === "darwin" ? p.toLowerCase() : p);
  const target = join(realpathSync(dirname(resolve(path))), basename(path));
  const rel = relative(fold(root), fold(target));
  // Another drive gives an absolute path.
  return !(rel === ".." || rel.startsWith(`..${sep}`) || isAbsolute(rel));
}

function main() {
  const args = process.argv.slice(2);
  const print = args.includes("--print");
  const paths = args.filter((a) => a !== "--print");
  if (paths.length !== 1) throw new Error("Usage: node scripts/gen-signing-key.mjs [--print] <path outside this repository>");
  const out = paths[0];
  if (!existsSync(dirname(resolve(out)))) throw new Error(`${dirname(resolve(out))} doesn't exist.`);
  if (insideRepo(out)) throw new Error("That path is inside the repository: keep the private key outside it, so it can never be committed.");
  if (existsSync(out)) throw new Error(`${out} already exists; not replacing a key.`);
  // Refuse before making anything when the companion already has a real key.
  const keySource = print ? null : readFileSync(KEY_FILE, "utf8");
  if (keySource !== null) withPublicKey(keySource, Buffer.alloc(32));

  const { privateKey } = generateKeyPairSync("ed25519");
  const raw = rawPublicKey(privateKey);
  writeFileSync(out, privateKey.export({ format: "pem", type: "pkcs8" }), { flag: "wx", mode: 0o600 });
  console.log(`Wrote the private key to ${resolve(out)}. Keep it secret and back it up.`);
  if (keySource === null) {
    console.log("\nPublic key, for RELEASE_PUBLIC_KEY in companion/src/updater/release_key.rs:\n");
    console.log(`const RELEASE_PUBLIC_KEY: [u8; 32] = ${rustKeyLiteral(raw)};`);
  } else {
    writeFileSync(KEY_FILE, withPublicKey(keySource, raw));
    console.log("Wrote the public key to companion/src/updater/release_key.rs: commit it.");
  }
  console.log("\nThen: gh secret set T3WOW_SIGNING_KEY < <that path>");
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
