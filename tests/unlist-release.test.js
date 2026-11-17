// Unlisting a release (scripts/unlist-release.mjs) against a pretend MatiasCiccone/t3wow (or,
// with --repo, another repository): GitHub's "latest" pointer moves only when a release is
// marked latest.
import assert from "node:assert/strict";
import { test } from "node:test";
import { apply, check, compareVersions, describe, newestListed, plan, releases, repoOption } from "../scripts/unlist-release.mjs";

function fakeGitHub(tags, latest = tags.at(-1), repo = "MatiasCiccone/t3wow") {
  const state = tags.map((tagName) => ({ tagName, isDraft: false, isPrerelease: false }));
  const calls = [];
  const gh = (args) => {
    calls.push(args.join(" "));
    const at = args.indexOf("--repo");
    if (at !== -1) assert.equal(args[at + 1], repo);
    if (args[0] === "release" && args[1] === "list") return JSON.stringify(state.map((r) => ({ ...r, isLatest: r.tagName === latest })));
    if (args[0] === "api") {
      assert.equal(args[1], `repos/${repo}/releases/latest`);
      return latest;
    }
    if (args[0] === "release" && args[1] === "edit") {
      const release = state.find((r) => r.tagName === args[2]);
      for (const flag of args.slice(3)) {
        if (flag === "--latest") {
          assert.ok(!release.isPrerelease, "GitHub refuses to mark a prerelease latest");
          latest = release.tagName;
        } else if (flag.startsWith("--prerelease=")) {
          release.isPrerelease = flag.endsWith("true");
          if (release.isPrerelease && latest === release.tagName) latest = "a release GitHub picks";
        }
      }
      return "";
    }
    throw new Error(`unexpected gh ${args.join(" ")}`);
  };
  return { gh, state, calls, latest: () => latest };
}

test("versions compare as the companion compares them", () => {
  assert.equal(compareVersions("v0.10.0", "v0.9.1"), 1);
  assert.equal(compareVersions("0.9", "v0.9.0"), 0);
  assert.equal(compareVersions("v0.9.0", "v0.9.1"), -1);
  assert.equal(newestListed([{ tagName: "v0.10.0", isPrerelease: true }, { tagName: "v0.9.0" }, { tagName: "v0.8.5" }]), "v0.9.0");
  assert.equal(newestListed([{ tagName: "v1.0.0", isDraft: true }]), null);
});

test("unlisting the newest release points latest back at the newest listed one", () => {
  const github = fakeGitHub(["v0.8.0", "v0.9.0", "v0.9.1"]);
  const steps = plan(releases(github.gh), "v0.9.1", false);
  assert.deepEqual(steps, { tagName: "v0.9.1", edit: { tagName: "v0.9.1", prerelease: true }, latest: "v0.9.0" });
  assert.match(describe(steps, false), /^Unlist v0\.9\.1 \(prerelease: true\)\.\nMark v0\.9\.0 "latest"/);
  apply(steps, github.gh);
  assert.equal(github.latest(), "v0.9.0");
  assert.equal(github.state.find((r) => r.tagName === "v0.9.1").isPrerelease, true);
  assert.match(check(github.gh), /^Latest: v0\.9\.0\. Unlisted: v0\.9\.1\.$/);
});

test("a patch released after an unlisted one is the latest, and relisting doesn't take that from it", () => {
  const github = fakeGitHub(["v0.9.0", "v0.9.1"]);
  apply(plan(releases(github.gh), "0.9.1", false), github.gh);
  // The Release workflow publishes the patch as latest.
  github.state.push({ tagName: "v0.9.2", isDraft: false, isPrerelease: false });
  github.gh(["release", "edit", "v0.9.2", "--latest"]);
  assert.match(check(github.gh), /^Latest: v0\.9\.2\. Unlisted: v0\.9\.1\.$/);
  const relist = plan(releases(github.gh), "v0.9.1", true);
  assert.deepEqual(relist, { tagName: "v0.9.1", edit: { tagName: "v0.9.1", prerelease: false }, latest: "v0.9.2" });
  apply(relist, github.gh);
  assert.equal(github.latest(), "v0.9.2");
});

test("an older release can be unlisted without moving latest, and doing it twice changes nothing more", () => {
  const github = fakeGitHub(["v0.8.0", "v0.9.0"]);
  apply(plan(releases(github.gh), "v0.8.0", false), github.gh);
  assert.equal(github.latest(), "v0.9.0");
  const again = plan(releases(github.gh), "v0.8.0", false);
  assert.equal(again.edit, null);
  assert.match(describe(again, false), /^v0\.8\.0 is already unlisted\./);
  const before = github.calls.length;
  apply(again, github.gh);
  assert.ok(!github.calls.slice(before).some((c) => c.includes("--prerelease")));
});

test("unlisting the only listed release leaves companions nothing to be offered", () => {
  const github = fakeGitHub(["v0.9.0"]);
  const steps = plan(releases(github.gh), "v0.9.0", false);
  assert.equal(steps.latest, null);
  assert.match(describe(steps, false), /No listed release is left/);
  apply(steps, github.gh);
  assert.match(check(github.gh), /^Latest: none\. Unlisted: v0\.9\.0\.$/);
});

test("a missing release or a draft is refused, and a wrong latest is caught", () => {
  const github = fakeGitHub(["v0.9.0"]);
  assert.throws(() => plan(releases(github.gh), "v0.7.0", false), /There's no release v0\.7\.0/);
  assert.throws(() => plan([{ tagName: "v1.0.0", isDraft: true, isPrerelease: false }], "v1.0.0", false), /is a draft/);
  // GitHub left latest on an older release: --check says so, and how to fix it.
  const stale = fakeGitHub(["v0.8.0", "v0.9.0"], "v0.8.0");
  assert.throws(() => check(stale.gh), /The newest listed release is v0\.9\.0: run node scripts\/unlist-release\.mjs v0\.9\.0 --relist --apply/);
  // A pointer that doesn't move after the edit fails apply.
  const stuck = (args) => (args[1] === "edit" ? "" : stale.gh(args));
  assert.throws(() => apply({ edit: null, latest: "v0.9.0" }, stuck), /GitHub's latest release is v0\.8\.0, not v0\.9\.0/);
});

test("--repo works on another repository instead of this one", () => {
  assert.deepEqual(repoOption(["v0.9.1", "--apply"]), { repo: "MatiasCiccone/t3wow", rest: ["v0.9.1", "--apply"] });
  assert.deepEqual(repoOption(["v0.9.1", "--repo", "someone/t3wow-fork", "--apply"]), { repo: "someone/t3wow-fork", rest: ["v0.9.1", "--apply"] });
  assert.throws(() => repoOption(["v0.9.1", "--repo"]), /--repo takes owner\/name/);
  assert.throws(() => repoOption(["v0.9.1", "--repo", "--apply"]), /--repo takes owner\/name/);
  const other = "someone/t3wow-fork";
  const github = fakeGitHub(["v0.9.0", "v0.9.1"], "v0.9.1", other);
  const steps = plan(releases(github.gh, other), "v0.9.1", false, other);
  apply(steps, github.gh, other);
  assert.equal(github.latest(), "v0.9.0");
  assert.ok(github.calls.every((c) => !c.includes("--repo") || c.includes(`--repo ${other}`)));
  assert.throws(() => plan(releases(github.gh, other), "v0.7.0", false, other), /There's no release v0\.7\.0 on someone\/t3wow-fork/);
  const stale = fakeGitHub(["v0.8.0", "v0.9.0"], "v0.8.0", other);
  assert.throws(() => check(stale.gh, other), /--relist --apply --repo someone\/t3wow-fork\./);
});
