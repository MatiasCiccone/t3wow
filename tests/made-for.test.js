// package.json's t3Code is the one place the T3 Code this release is made for is set: the
// companion is built with it (build.rs), and the docs that name that version must agree.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");
const { t3Code } = JSON.parse(read("package.json"));

// The phrases that name the version a release is made for (not "as in T3 Code 0.19.0", which
// says when a behavior came in), and how many each file has.
const MENTIONS = /(?:Made for T3 Code|stable release of T3 Code, version|T3 Code desktop|tested against) (\d+\.\d+\.\d+)/g;
const FILES = { "README.md": 2, "assets/curseforge-description.md": 4 };

test("package.json names the T3 Code this release is made for", () => {
  assert.match(t3Code ?? "", /^\d+\.\d+\.\d+$/);
});

for (const [file, count] of Object.entries(FILES)) {
  test(`${file} says it's made for T3 Code ${t3Code}`, () => {
    const named = [...read(file).matchAll(MENTIONS)].map((m) => m[1]);
    assert.equal(named.length, count, `${file} names the version ${named.length} times: ${named}`);
    for (const version of named) assert.equal(version, t3Code, `${file} names T3 Code ${version}; package.json's t3Code is ${t3Code}`);
  });
}
