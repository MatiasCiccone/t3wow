// Unlists a published release on t3wow-releases, so no companion is offered it, or lists it
// again. See docs/unlisting-a-release.md.
//
//   node scripts/unlist-release.mjs v0.9.1            # shows what it would do
//   node scripts/unlist-release.mjs v0.9.1 --apply    # does it
//   node scripts/unlist-release.mjs v0.9.1 --relist [--apply]
//   node scripts/unlist-release.mjs --check           # is "latest" the newest listed release?
//
// A listed release is a published release that isn't a prerelease; unlisting marks it a
// prerelease. Every companion asks GitHub for the repository's latest release, which is never
// a prerelease, and also ignores a prerelease itself. That pointer isn't moved reliably on its
// own (the releases share one commit, so GitHub can't order them by date), so after each
// change the script marks the newest listed release "latest" and reads the pointer back. The
// release and its signed files stay as they were: nothing is deleted or re-signed.
//
// Needs the GitHub CLI (gh), signed in to an account that can edit t3wow-releases.
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

export const RELEASES_REPO = "MatiasCiccone/t3wow-releases";

/// Compares "v1.2.3" and "1.10" part by part, as the companion does (slots.rs compare_versions).
export function compareVersions(a, b) {
  const parse = (v) => v.replace(/^v/, "").split(".").map((p) => (/^\d+$/.test(p) ? Number(p) : 0));
  const [pa, pb] = [parse(a), parse(b)];
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const order = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (order !== 0) return Math.sign(order);
  }
  return 0;
}

export const listed = (release) => !release.isDraft && !release.isPrerelease;

/// The newest listed release's tag, or null when there's none.
export function newestListed(releases) {
  return releases.filter(listed).reduce((newest, r) => (newest && compareVersions(newest, r.tagName) >= 0 ? newest : r.tagName), null);
}

/// What to do, from `gh release list --json tagName,isDraft,isPrerelease,isLatest`: the release
/// to edit (or null when it's already that way), and the tag that must end up "latest".
export function plan(releases, tag, relist) {
  const tagName = tag.startsWith("v") ? tag : `v${tag}`;
  const release = releases.find((r) => r.tagName === tagName);
  if (!release) throw new Error(`There's no release ${tagName} on ${RELEASES_REPO}.`);
  if (release.isDraft) throw new Error(`${tagName} is a draft: it was never listed, so there's nothing to change.`);
  const edit = release.isPrerelease === relist ? { tagName, prerelease: !relist } : null;
  const after = releases.map((r) => (r.tagName === tagName ? { ...r, isPrerelease: !relist } : r));
  return { tagName, edit, latest: newestListed(after) };
}

export function describe({ tagName, edit, latest }, relist) {
  const lines = [edit ? `${relist ? "List" : "Unlist"} ${tagName} (prerelease: ${edit.prerelease}).` : `${tagName} is already ${relist ? "listed" : "unlisted"}.`];
  lines.push(
    latest
      ? `Mark ${latest} "latest": companions older than ${latest} are offered ${latest}, and no companion is offered an unlisted release.`
      : `No listed release is left: companions are offered nothing until the next release.`,
  );
  return lines.join("\n");
}

const ghCli = (args) => execFileSync("gh", args, { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] }).trim();

export function releases(gh = ghCli) {
  return JSON.parse(gh(["release", "list", "--repo", RELEASES_REPO, "--limit", "1000", "--json", "tagName,isDraft,isPrerelease,isLatest"]));
}

/// The tag GitHub answers for the latest release (what companions ask), or null when there's none.
export function latestTag(gh = ghCli) {
  const all = releases(gh);
  if (!all.some(listed)) return null;
  return gh(["api", `repos/${RELEASES_REPO}/releases/latest`, "--jq", ".tag_name"]);
}

/// Makes the change, marks the newest listed release "latest" and checks GitHub now answers it.
export function apply({ edit, latest }, gh = ghCli) {
  if (edit) gh(["release", "edit", edit.tagName, "--repo", RELEASES_REPO, `--prerelease=${edit.prerelease}`]);
  if (latest) gh(["release", "edit", latest, "--repo", RELEASES_REPO, "--latest"]);
  const now = latestTag(gh);
  if (now !== latest) throw new Error(`GitHub's latest release is ${now ?? "none"}, not ${latest ?? "none"}: check ${RELEASES_REPO} by hand.`);
}

/// --check: fails when GitHub's latest release isn't the newest listed one.
export function check(gh = ghCli) {
  const all = releases(gh);
  const [want, now] = [newestListed(all), latestTag(gh)];
  const unlisted = all.filter((r) => !r.isDraft && r.isPrerelease).map((r) => r.tagName);
  const summary = `Latest: ${now ?? "none"}. Unlisted: ${unlisted.join(", ") || "none"}.`;
  if (now !== want) throw new Error(`${summary} The newest listed release is ${want}: run node scripts/unlist-release.mjs ${want} --relist --apply.`);
  return summary;
}

function main(args) {
  if (args[0] === "--check" && args.length === 1) return console.log(check());
  const tag = args.find((a) => !a.startsWith("--"));
  const unknown = args.filter((a) => a.startsWith("--") && a !== "--relist" && a !== "--apply");
  if (!tag || unknown.length) throw new Error("Usage: node scripts/unlist-release.mjs <tag> [--relist] [--apply] | --check");
  const relist = args.includes("--relist");
  const steps = plan(releases(), tag, relist);
  console.log(describe(steps, relist));
  if (!args.includes("--apply")) return console.log("Nothing changed: run it again with --apply to do it.");
  apply(steps);
  console.log(`Done. GitHub's latest release is ${steps.latest ?? "none"}.`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
