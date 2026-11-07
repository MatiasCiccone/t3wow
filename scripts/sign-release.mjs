// Signs a release's checksum file with the release key, in the Release workflow:
//
//   T3WOW_SIGNING_KEY="<PKCS#8 PEM>" node scripts/sign-release.mjs release/t3wow.exe.sha256
//
// writes release/t3wow.exe.sha256.sig, which the companion's updater requires. With
// --check it only checks the key (set, Ed25519, and the public half the companion embeds),
// so the workflow can fail before building anything.
import { readFileSync, writeFileSync } from "node:fs";
import { releaseKey, signatureFile } from "./release-key.mjs";

const args = process.argv.slice(2);
try {
  const key = releaseKey(process.env.T3WOW_SIGNING_KEY);
  if (args[0] === "--check") {
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
