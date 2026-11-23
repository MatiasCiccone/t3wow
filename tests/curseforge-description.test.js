// A release with features updates the CurseForge description (scripts/curseforge-description.mjs):
// on pretend repositories, on the real v0.12.1..v0.13.0 that shipped without it, and on this
// checkout, which is the gate. CI and the Release workflow check out the whole history so it
// runs there; a shallow checkout fails it under CI.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { pathToFileURL } from "node:url";
import { DESCRIPTION, OPT_OUT, explain, notices, releaseCheck } from "../scripts/curseforge-description.mjs";

// A repository at v0.1.0 (tagged), with a package.json and a description.
function pretendRepo(t) {
  const dir = mkdtempSync(join(tmpdir(), "t3wow-cf-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const git = (...args) => execFileSync("git", args, { cwd: dir, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
  git("init", "-q", "-b", "main");
  for (const [key, value] of [["user.name", "Test"], ["user.email", "test@example.com"], ["commit.gpgsign", "false"], ["tag.gpgsign", "false"], ["core.hooksPath", join(dir, "no-hooks")]]) git("config", key, value);
  const write = (path, text) => {
    mkdirSync(dirname(join(dir, path)), { recursive: true });
    writeFileSync(join(dir, path), text);
  };
  const commit = (message, files = {}) => {
    for (const [path, text] of Object.entries(files)) write(path, text);
    git("add", "-A");
    git("commit", "-q", "--allow-empty", "-m", message);
  };
  const version = (v, t3Code = "0.1.10") => ({ "package.json": JSON.stringify({ name: "t3wow", version: v, t3Code }) });
  commit("chore: release 0.1.0", { ...version("0.1.0"), [DESCRIPTION]: "# T3 WoW. Made for T3 Code 0.1.10\n" });
  git("tag", "-a", "-m", "T3 WoW v0.1.0", "v0.1.0"); // annotated, as release tags are
  return { dir, git, commit, version, check: (to) => releaseCheck({ cwd: dir, to }) };
}

test("a release with a feature and no change to the description fails, naming the feature and the way out", (t) => {
  const repo = pretendRepo(t);
  repo.commit("feat: pictures in the game");
  repo.commit("fix: a crash");
  repo.commit("chore: release 0.2.0", repo.version("0.2.0"));
  const result = repo.check();
  assert.equal(result.ok, false);
  assert.deepEqual(result.feats, ["feat: pictures in the game"]);
  assert.match(explain(result), /^0\.2\.0 \(v0\.1\.0\.\.HEAD\) has 1 feat commit but doesn't change assets\/curseforge-description\.md \(beyond the T3 Code version\):\n  feat: pictures in the game\n/);
  assert.match(explain(result), /CurseForge-Description: unchanged/);
});

test("a release that changes the description passes, wherever in the range the change is", (t) => {
  const repo = pretendRepo(t);
  repo.commit("feat: pictures in the game");
  repo.commit("docs: the CurseForge page tells of pictures", { [DESCRIPTION]: "# T3 WoW\n\nPictures.\n" });
  repo.commit("chore: release 0.2.0", repo.version("0.2.0"));
  const result = repo.check();
  assert.equal(result.ok, true);
  assert.equal(result.changed, true);
});

test("the opt-out line in the release commit's message passes a release that leaves the description alone", (t) => {
  const repo = pretendRepo(t);
  repo.commit("feat(ui): a wider scroll bar");
  repo.commit(`chore: release 0.2.0\n\n${OPT_OUT} (a scroll bar isn't news)`, repo.version("0.2.0"));
  const result = repo.check();
  assert.equal(result.ok, true);
  assert.equal(result.optOut, true);
  assert.match(explain(result), /as the release commit's "CurseForge-Description: unchanged" says/);
});

test("a T3 Code move that only changes the description's version numbers isn't a change to it", (t) => {
  const repo = pretendRepo(t);
  repo.commit("feat: pictures in the game");
  repo.commit("chore: release 0.2.0, made for T3 Code 0.1.11", { ...repo.version("0.2.0", "0.1.11"), [DESCRIPTION]: "# T3 WoW. Made for T3 Code 0.1.11\n" });
  const result = repo.check();
  assert.equal(result.changed, false);
  assert.equal(result.ok, false);
});

test("any other version number the description changes is a change to it", (t) => {
  const repo = pretendRepo(t);
  repo.commit("feat: the Mac app runs on macOS 26.7.2");
  repo.commit("docs: the CurseForge page names macOS 26.7.2", { [DESCRIPTION]: "# T3 WoW. Made for T3 Code 0.1.10\nTested on macOS 26.7.2.\n" });
  repo.commit("chore: release 0.2.0", repo.version("0.2.0"));
  repo.git("tag", "v0.2.0");
  repo.commit("feat: the Mac app runs on macOS 26.8.0");
  repo.commit("docs: the CurseForge page names macOS 26.8.0", { [DESCRIPTION]: "# T3 WoW. Made for T3 Code 0.1.10\nTested on macOS 26.8.0.\n" });
  repo.commit("chore: release 0.3.0", repo.version("0.3.0"));
  assert.equal(repo.check().changed, true);
});

test("the opt-out counts only as a trailer of the commit that sets the version, and only as the whole word", (t) => {
  const repo = pretendRepo(t);
  repo.commit("feat: pictures in the game");
  repo.commit(`ci: the release check\n\nA release passes when its commit says\n${OPT_OUT}\nin its message.`, { "scripts/check.mjs": "" });
  repo.commit(`chore: T3 Code 0.1.11\n\n${OPT_OUT}`, repo.version("0.1.0", "0.1.11"));
  repo.commit("chore: release 0.2.0", repo.version("0.2.0", "0.1.11"));
  assert.equal(repo.check().optOut, false, "neither a commit that talks about it nor one that changes package.json but not its version");
  repo.git("commit", "-q", "--amend", "-m", "chore: release 0.2.0\n\nCurseForge-Description: unchanged-ish");
  assert.equal(repo.check().optOut, false);
  repo.git("commit", "-q", "--amend", "-m", `chore: release 0.2.0\n\nTo skip it, say\n\n~~~text\n${OPT_OUT}\n~~~`);
  assert.equal(repo.check().optOut, false, "an example in a code block isn't a trailer");
  repo.git("commit", "-q", "--amend", "-m", "chore: release 0.2.0\n\nCurseForge-Description: updated\n  unchanged parts listed below");
  assert.equal(repo.check().optOut, false, "a folded value's second line isn't a value of its own");
  repo.git("commit", "-q", "--amend", "-m", `chore: release 0.2.0\n\n${OPT_OUT}`);
  assert.equal(repo.check().ok, true);
});

test("an opt-out from the release before doesn't carry over", (t) => {
  const repo = pretendRepo(t);
  repo.commit(`chore: release 0.1.1\n\n${OPT_OUT}`, repo.version("0.1.1"));
  repo.git("tag", "v0.1.1");
  repo.commit("feat: pictures in the game");
  repo.commit("chore: release 0.2.0", repo.version("0.2.0"));
  assert.equal(repo.check().ok, false);
});

test("a release with only fixes, and any commit that isn't a release, pass without a change", (t) => {
  const repo = pretendRepo(t);
  repo.commit("feat: pictures in the game");
  const feature = repo.check();
  assert.equal(feature.applies, false, "same version as v0.1.0: a feature pull request, not a release");
  assert.equal(feature.ok, true);
  repo.commit("chore: release 0.1.1", repo.version("0.1.1"));
  repo.git("tag", "v0.1.1");
  repo.commit("fix: a crash");
  repo.commit("chore: release 0.1.2", repo.version("0.1.2"));
  const fixes = repo.check();
  assert.equal(fixes.applies, true);
  assert.deepEqual(fixes.feats, []);
  assert.equal(fixes.ok, true);
});

test("the release tag checks the range it releases, not an empty one", (t) => {
  const repo = pretendRepo(t);
  repo.commit("feat: pictures in the game");
  repo.commit("chore: release 0.2.0", repo.version("0.2.0"));
  repo.git("tag", "v0.2.0");
  const result = repo.check("v0.2.0");
  assert.equal(result.previous, "v0.1.0");
  assert.equal(result.ok, false);
});

test("a pull request's merge commit, as CI checks it out, counts the pull request's commits", (t) => {
  const repo = pretendRepo(t);
  repo.git("checkout", "-q", "-b", "release");
  repo.commit("feat: pictures in the game");
  repo.commit("chore: release 0.2.0", repo.version("0.2.0"));
  repo.git("checkout", "-q", "main");
  repo.commit("docs: a typo");
  repo.git("merge", "-q", "--no-ff", "-m", "Merge release into main", "release");
  const result = repo.check();
  assert.equal(result.previous, "v0.1.0");
  assert.equal(result.ok, false);
});

test("a shallow checkout says there's no history to check", (t) => {
  const repo = pretendRepo(t);
  repo.commit("feat: pictures in the game");
  const shallow = mkdtempSync(join(tmpdir(), "t3wow-cf-shallow-"));
  t.after(() => rmSync(shallow, { recursive: true, force: true }));
  execFileSync("git", ["clone", "-q", "--depth", "1", pathToFileURL(repo.dir).href, shallow], { stdio: "ignore" });
  const result = releaseCheck({ cwd: shallow });
  assert.equal(result.shallow, true);
  assert.equal(result.ok, false);
  assert.match(explain(result), /fetch-depth: 0/);
});

test("the release job's notice names what to copy into CurseForge, and only what changed", (t) => {
  const repo = pretendRepo(t);
  repo.commit("chore: release 0.1.1", repo.version("0.1.1"));
  repo.git("tag", "v0.1.1");
  assert.deepEqual(notices({ cwd: repo.dir, to: "v0.1.1" }), []);
  repo.commit("docs: the CurseForge page tells of pictures", { [DESCRIPTION]: "# T3 WoW\n\nPictures.\n" });
  repo.commit("chore: release 0.2.0", repo.version("0.2.0"));
  repo.git("tag", "v0.2.0");
  const [line, ...rest] = notices({ cwd: repo.dir, to: "v0.2.0" });
  assert.deepEqual(rest, []);
  assert.match(line, /^::notice title=Copy into CurseForge by hand::assets\/curseforge-description\.md changed since v0\.1\.1: paste it into the CurseForge project's description/);
});

test("a ref that isn't a commit is an error, not a pass", (t) => {
  const repo = pretendRepo(t);
  assert.throws(() => repo.check("v9.9.9"), /"v9\.9\.9" isn't a commit/);
  assert.throws(() => notices({ cwd: repo.dir, to: "" }), /"" isn't a commit/);
});

// Whether git succeeds in this checkout.
const git = (...args) => {
  try {
    execFileSync("git", args, { cwd: new URL("..", import.meta.url), stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
};

test("0.13.0 shipped with four features and the 0.12.1 description: the check fails it", (t) => {
  const tagged = (tag) => git("rev-parse", "--verify", "-q", `refs/tags/${tag}`);
  if ((!tagged("v0.12.1") || !tagged("v0.13.0")) && !process.env.CI) return t.skip("no v0.12.1 and v0.13.0 tags in this checkout");
  const result = releaseCheck({ to: "v0.13.0" });
  if (result.shallow && !process.env.CI) return t.skip(explain(result));
  assert.ok(!result.shallow, explain(result));
  assert.equal(result.previous, "v0.12.1");
  assert.equal(result.feats.length, 4);
  assert.equal(result.changed, false);
  assert.equal(result.ok, false);
});

test("this checkout passes the check (a release with features updates the CurseForge description)", (t) => {
  if (!git("rev-parse", "--git-dir") && !process.env.CI) return t.skip("not a git checkout (a source zip, say)");
  const result = releaseCheck();
  if (result.shallow && !process.env.CI) return t.skip(explain(result));
  assert.ok(!result.shallow, explain(result));
  // This repository has release tags, so under CI a checkout without them is set up wrong.
  if (process.env.CI) assert.ok(result.previous, `${explain(result)} Check out the tags too (fetch-depth: 0).`);
  assert.ok(result.ok, explain(result));
});
