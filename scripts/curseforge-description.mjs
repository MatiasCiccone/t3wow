// The CurseForge project page's text is assets/curseforge-description.md, pasted in by hand:
// CurseForge's upload API takes the addon zip and its changelog, not the page. So nothing but
// this check makes a release update it.
//
//   node scripts/curseforge-description.mjs check [ref]    # exits 1 when ref fails the check
//   node scripts/curseforge-description.mjs notice [ref]   # says what to paste into CurseForge
//
// ref defaults to HEAD. Its release is the version in its package.json, and the previous release
// is the newest v* tag before it (the tag on ref itself doesn't count, so the release tag checks
// the range it releases). When those versions differ, ref is a release, and any feat commit in
// that range needs a change to the description beyond the T3 Code version it names (moving to a
// new T3 Code changes only that), or this trailer, in the last paragraph of the release commit's
// message (the commit that set the version), in a paragraph of its own:
//
//   CurseForge-Description: unchanged
//
// for a release whose features don't belong on the page (a reason may follow the word). npm test
// runs the check on the checkout (tests/curseforge-description.test.js), so the release pull
// request's CI and the Release workflow's builds stop on it. Both need the history and tags
// (fetch-depth: 0).
import { execFileSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

export const DESCRIPTION = "assets/curseforge-description.md";
export const OPT_OUT = "CurseForge-Description: unchanged";
const OPT_OUT_VALUE = /^unchanged(?:\s|$)/i;
// The subjects the release notes call features (release.yml's Release notes step).
const FEAT = /^feat(\([^)]*\))?!?:/;
// The phrases that name the T3 Code a release is made for (tests/made-for.test.js checks them).
export const MADE_FOR = /(?:Made for T3 Code|stable release of T3 Code, version|T3 Code desktop|developed and tested against) (\d+\.\d+\.\d+)/g;

// What's copied into CurseForge by hand, and how. The notice names the ones a release changed.
export const BY_HAND = [
  { path: DESCRIPTION, how: "paste it into the CurseForge project's description (Edit Project, Description)" },
  { path: "assets/curseforge-avatar.png", how: "upload it as the CurseForge project's avatar" },
];

function gitIn(cwd, to) {
  const git = (...args) => execFileSync("git", args, { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
  const tryGit = (...args) => {
    try {
      return git(...args);
    } catch {
      return null;
    }
  };
  if (!to || !tryGit("rev-parse", "--verify", "--quiet", `${to}^{commit}`)) throw new Error(`"${to}" isn't a commit in ${cwd}.`);
  return { git, tryGit };
}

/// The newest v* tag before ref (not ref's own), or null when there's none.
function previousTag(tryGit, to) {
  return tryGit("describe", "--tags", "--abbrev=0", "--match", "v*", `${to}^`);
}

/// Checks ref. `shallow` means the history isn't there to check (and fails); `applies` is false
/// when ref isn't a release (same version as the previous tag) or there's no previous tag.
export function releaseCheck({ cwd = ROOT, to = "HEAD" } = {}) {
  const { git, tryGit } = gitIn(cwd, to);
  if (git("rev-parse", "--is-shallow-repository") === "true") return { to, shallow: true, applies: false, ok: false };
  const previous = previousTag(tryGit, to);
  if (!previous) return { to, previous: null, applies: false, ok: true };
  const show = (ref, path) => tryGit("show", `${ref}:${path}`);
  const version = (ref) => JSON.parse(show(ref, "package.json") ?? "{}").version;
  const [from, release] = [version(previous), version(to)];
  const result = { to, previous, from, release, applies: from !== release };
  if (!result.applies) return { ...result, ok: true };
  const range = `${previous}..${to}`;
  const feats = git("log", "--no-merges", "--format=%s", range).split("\n").filter((s) => FEAT.test(s));
  const words = (ref) => (show(ref, DESCRIPTION) ?? "").replace(MADE_FOR, (phrase, v) => phrase.replace(v, "x.y.z"));
  const changed = words(previous) !== words(to);
  // The release commit is the newest one in the range that changed package.json's version.
  const releaseCommit = git("log", "--no-merges", "--format=%H", range, "--", "package.json")
    .split("\n")
    .find((sha) => sha && version(sha) !== version(`${sha}^`));
  const trailers = releaseCommit ? git("show", "-s", "--format=%(trailers:key=CurseForge-Description,valueonly,unfold)", releaseCommit) : "";
  const optOut = trailers.split("\n").some((value) => OPT_OUT_VALUE.test(value.trim()));
  return { ...result, feats, changed, optOut, ok: feats.length === 0 || changed || optOut };
}

/// One line on how the check went, or, when it failed, what to do.
export function explain(r) {
  if (r.shallow) return "This checkout is shallow, so there's no history to check: fetch it with fetch-depth: 0 (or git fetch --unshallow --tags).";
  if (!r.previous) return `No v* tag before ${r.to}: nothing to compare with.`;
  if (!r.applies) return `${r.to} is ${r.release}, as ${r.previous} is: not a release, so the CurseForge description isn't checked.`;
  const subject = `${r.release} (${r.previous}..${r.to})`;
  if (r.feats.length === 0) return `${subject} has no feat commits.`;
  if (r.changed) return `${subject} changes ${DESCRIPTION}.`;
  if (r.optOut) return `${subject} leaves ${DESCRIPTION} as it is, as the release commit's "${OPT_OUT}" says.`;
  return [
    `${subject} has ${r.feats.length} feat commit${r.feats.length === 1 ? "" : "s"} but doesn't change ${DESCRIPTION} (beyond the T3 Code version):`,
    ...r.feats.map((s) => `  ${s}`),
    `Describe them there (the CurseForge page is pasted from it by hand), and commit it: the check reads commits, not the working tree. If none of them belongs on the page, end the release commit's message with "${OPT_OUT}" as a paragraph of its own (a reason can follow on the same line).`,
  ].join("\n");
}

/// The files copied into CurseForge by hand that changed since the tag before ref, as GitHub
/// Actions notices.
export function notices({ cwd = ROOT, to = "HEAD" } = {}) {
  const { git, tryGit } = gitIn(cwd, to);
  const previous = previousTag(tryGit, to);
  if (!previous) return [];
  const changed = new Set(git("diff", "--name-only", previous, to, "--", ...BY_HAND.map((f) => f.path)).split("\n"));
  return BY_HAND.filter((f) => changed.has(f.path)).map(
    (f) => `::notice title=Copy into CurseForge by hand::${f.path} changed since ${previous}: ${f.how}. CurseForge's upload doesn't.`,
  );
}

function main([command, to = "HEAD"]) {
  try {
    if (command === "check") {
      const result = releaseCheck({ to });
      console.log(explain(result));
      return result.ok ? 0 : 1;
    }
    if (command === "notice") {
      const lines = notices({ to });
      console.log(lines.length ? lines.join("\n") : "Nothing copied into CurseForge by hand changed in this release.");
      return 0;
    }
  } catch (error) {
    console.error(error.message);
    return 1;
  }
  console.error("Usage: node scripts/curseforge-description.mjs check|notice [ref]");
  return 2;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) process.exitCode = main(process.argv.slice(2));
