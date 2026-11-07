// Signs a release's checksum file with the release key, in the Release workflow:
//
//   T3WOW_SIGNING_KEY="<PKCS#8 PEM>" node scripts/sign-release.mjs release/t3wow.exe.sha256
//
// writes release/t3wow.exe.sha256.sig, which the companion's updater requires once it
// embeds the real public key. With --check it only checks the key (set, Ed25519, and the
// public half the companion embeds), so the workflow can fail before building anything.
// While signing isn't set up at all (no secret, placeholder public key), both say so as a
// warning and succeed: the release goes out with its SHA-256 only. In a workflow, --check
// also sets the step output `signed` (true or false), which later steps read.
import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { signatureFile, signingKey } from "./release-key.mjs";

const args = process.argv.slice(2);
try {
  const key = signingKey(process.env.T3WOW_SIGNING_KEY);
  if (args[0] === "--check" && process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `signed=${key !== null}\n`);
  if (!key && (args[0] === "--check" || args.length === 1)) {
    const message =
      "Release signing isn't set up (no T3WOW_SIGNING_KEY, placeholder RELEASE_PUBLIC_KEY): this release goes out with its SHA-256 only. See docs/release-signing.md.";
    console.log(process.env.GITHUB_ACTIONS ? `::warning::${message}` : message);
  } else if (args[0] === "--check") {
    console.log("The release key is set and matches the companion's public key.");
  } else if (args.length === 1) {
    writeFileSync(`${args[0]}.sig`, signatureFile(key, readFileSync(args[0])));
    console.log(`Signed ${args[0]} as ${args[0]}.sig.`);
  } else {
    throw new Error("Usage: node scripts/sign-release.mjs <checksum file> | --check");
  }
} catch (error) {
  // An annotation on the workflow run, and plain text elsewhere.
  console.error(process.env.GITHUB_ACTIONS ? `::error::${error.message}` : error.message);
  process.exit(1);
}
