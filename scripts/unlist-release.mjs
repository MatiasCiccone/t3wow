// Unlists a published release on MatiasCiccone/t3wow, so no companion is offered it, or lists
// it again. See docs/unlisting-a-release.md.
//
//   node scripts/unlist-release.mjs v0.9.1            # shows what it would do
//   node scripts/unlist-release.mjs v0.9.1 --apply    # does it
//   node scripts/unlist-release.mjs v0.9.1 --relist [--apply]
//   node scripts/unlist-release.mjs --check           # is "latest" the newest listed release?
//
// --repo <owner/name> works on another repository instead: while the Release workflow also
// publishes to MatiasCiccone/t3wow-releases (for companions up to 0.10.0), run it again with
// --repo MatiasCiccone/t3wow-releases.
//
// A listed release is a published release that isn't a prerelease; unlisting marks it a
// prerelease. Every companion asks GitHub for the repository's latest release, which is never
// a prerelease, and also ignores a prerelease itself. That pointer isn't moved reliably on its
// own (on t3wow-releases the releases share one commit, so GitHub can't order them by date),
// so after each change the script marks the newest listed release "latest" and reads the
// pointer back. The release and its signed files stay as they were: nothing is deleted or
// re-signed.
//
// Needs the GitHub CLI (gh), signed in to an account that can edit the repository's releases.
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

export const REPO = "MatiasCiccone/t3wow";

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
export function plan(releases, tag, relist, repo = REPO) {
  const tagName = tag.startsWith("v") ? tag : `v${tag}`;
  const release = releases.find((r) => r.tagName === tagName);
  if (!release) throw new Error(`There's no release ${tagName} on ${repo}.`);
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

export function releases(gh = ghCli, repo = REPO) {
  return JSON.parse(gh(["release", "list", "--repo", repo, "--limit", "1000", "--json", "tagName,isDraft,isPrerelease,isLatest"]));
}

/// The tag GitHub answers for the latest release (what companions ask), or null when there's none.
export function latestTag(gh = ghCli, repo = REPO) {
  const all = releases(gh, repo);
  if (!all.some(listed)) return null;
  return gh(["api", `repos/${repo}/releases/latest`, "--jq", ".tag_name"]);
}

/// Makes the change, marks the newest listed release "latest" and checks GitHub now answers it.
export function apply({ edit, latest }, gh = ghCli, repo = REPO) {
  if (edit) gh(["release", "edit", edit.tagName, "--repo", repo, `--prerelease=${edit.prerelease}`]);
  if (latest) gh(["release", "edit", latest, "--repo", repo, "--latest"]);
  const now = latestTag(gh, repo);
  if (now !== latest) throw new Error(`GitHub's latest release is ${now ?? "none"}, not ${latest ?? "none"}: check ${repo} by hand.`);
}

/// --check: fails when GitHub's latest release isn't the newest listed one.
export function check(gh = ghCli, repo = REPO) {
  const all = releases(gh, repo);
  const [want, now] = [newestListed(all), latestTag(gh, repo)];
  const unlisted = all.filter((r) => !r.isDraft && r.isPrerelease).map((r) => r.tagName);
  const summary = `Latest: ${now ?? "none"}. Unlisted: ${unlisted.join(", ") || "none"}.`;
  const again = repo === REPO ? "" : ` --repo ${repo}`;
  if (now !== want) throw new Error(`${summary} The newest listed release is ${want}: run node scripts/unlist-release.mjs ${want} --relist --apply${again}.`);
  return summary;
}

const USAGE = "Usage: node scripts/unlist-release.mjs <tag> [--relist] [--apply] [--repo <owner/name>] | --check [--repo <owner/name>]";

/// The arguments, with --repo <owner/name> taken out: { repo, rest }.
export function repoOption(args) {
  const at = args.indexOf("--repo");
  if (at === -1) return { repo: REPO, rest: args };
  const repo = args[at + 1];
  if (!repo || !/^[\w.-]+\/[\w.-]+$/.test(repo)) throw new Error(`--repo takes owner/name. ${USAGE}`);
  return { repo, rest: [...args.slice(0, at), ...args.slice(at + 2)] };
}

function main(argv) {
  const { repo, rest: args } = repoOption(argv);
  if (args[0] === "--check" && args.length === 1) return console.log(check(ghCli, repo));
  const tag = args.find((a) => !a.startsWith("--"));
  const unknown = args.filter((a) => a.startsWith("--") && a !== "--relist" && a !== "--apply");
  if (!tag || unknown.length) throw new Error(USAGE);
  const relist = args.includes("--relist");
  const steps = plan(releases(ghCli, repo), tag, relist, repo);
  console.log(`On ${repo}:`);
  console.log(describe(steps, relist));
  if (!args.includes("--apply")) return console.log("Nothing changed: run it again with --apply to do it.");
  apply(steps, ghCli, repo);
  console.log(`Done. GitHub's latest release on ${repo} is ${steps.latest ?? "none"}.`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
