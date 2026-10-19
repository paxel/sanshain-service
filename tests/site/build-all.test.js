// Tests for scripts/build-site-all.js: a throwaway git repository with two
// release tags and an unreleased working tree, built into one folder.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import {
  buildAll,
  compareVersions,
  extractTag,
  listReleaseTags,
  planVersions,
} from "../../scripts/build-site-all.js";

const SITE_V1 = {
  "site/index.html": `<!-- page: title="Home" summary="Landing" -->
<p>Version one. See the <a href="https://github.com/paxel/sanshain-service/blob/summer/CHANGELOG.md">changelog</a>.</p>`,
  "site/start/install.html": `<!-- page: title="Install" track="start" order=1 -->
<h2>Docker</h2><p>Run it.</p>`,
  "site/operator/legacy.html": `<!-- page: title="Legacy" track="operator" order=1 -->
<h2>Old</h2><p>Only version one has this chapter.</p>`,
  "site/assets/site.css": "body{color:black}",
  "site/assets/site.js": "/* v1 */",
  "site/assets/sun.svg": "<svg/>",
  "api.yaml": "openapi: 3.0.3\n",
  "maintenance.yaml": "openapi: 3.0.3\n",
  "Cargo.toml": '[package]\nname = "x"\nversion = "1.0.0"\n',
  "docs/arc42/.keep": "",
  "docs/adr/.keep": "",
};

let tmp;
let repo;
let outDir;

function git(args, options = {}) {
  return execFileSync("git", args, { cwd: repo, encoding: "utf8", stdio: "pipe", ...options });
}

function write(files) {
  for (const [rel, content] of Object.entries(files)) {
    const full = path.join(repo, rel);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, content);
  }
}

function commitAll(message) {
  git(["add", "-A"]);
  git(["commit", "-q", "-m", message]);
}

before(() => {
  tmp = fs.mkdtempSync(path.join(os.tmpdir(), "sanshain-site-all-"));
  repo = path.join(tmp, "repo");
  outDir = path.join(tmp, "dist");
  fs.mkdirSync(repo);
  git(["init", "-q", "-b", "summer"]);
  git(["config", "user.email", "test@example.invalid"]);
  git(["config", "user.name", "Test"]);
  git(["config", "commit.gpgsign", "false"]);
  // v0.9.0: a release before the site existed; it must not be listed.
  write({ "Cargo.toml": '[package]\nname = "x"\nversion = "0.9.0"\n', "README.md": "old" });
  commitAll("v0.9.0 without a site");
  git(["tag", "v0.9.0"]);
  write(SITE_V1);
  commitAll("v1.0.0 with the site");
  git(["tag", "v1.0.0"]);
  git(["tag", "v1.0.0-rc1"]); // not a release
  git(["tag", "nightly"]);
  // v1.1.0 drops the legacy chapter and gains a new one; today's assets differ.
  fs.rmSync(path.join(repo, "site/operator/legacy.html"));
  write({
    "site/operator/helm.html": `<!-- page: title="Helm chart" track="operator" order=1 -->
<h2>Values</h2><p>New in 1.1.</p>`,
    "site/assets/site.css": "body{color:navy}",
    "Cargo.toml": '[package]\nname = "x"\nversion = "1.1.0"\n',
  });
  commitAll("v1.1.0");
  git(["tag", "v1.1.0"]);
  // The working tree is the unreleased 1.2.0 with one more chapter.
  write({
    "site/developer/mcp.html": `<!-- page: title="MCP" track="developer" order=1 -->
<h2>Tools</h2><p>Unreleased.</p>`,
    "Cargo.toml": '[package]\nname = "x"\nversion = "1.2.0"\n',
  });
  buildAll({
    repoRoot: repo,
    outDir,
    base: "/docs/",
    contractsFor: () => [],
    arc42For: () => null,
  });
});

after(() => {
  fs.rmSync(tmp, { recursive: true, force: true });
});

const read = (rel) => fs.readFileSync(path.join(outDir, rel), "utf8");
const exists = (rel) => fs.existsSync(path.join(outDir, rel));

test("compareVersions sorts newest first, numerically per part", () => {
  assert.deepEqual(["2.9.0", "2.10.0", "10.0.0", "2.9.1"].sort(compareVersions), [
    "10.0.0",
    "2.10.0",
    "2.9.1",
    "2.9.0",
  ]);
});

test("release tags are the vX.Y.Z tags whose tree carries the site, newest first", () => {
  assert.deepEqual(listReleaseTags(repo), ["v1.1.0", "v1.0.0"]);
});

test("the plan puts the working tree in place of its tag and marks the newest as latest", () => {
  assert.deepEqual(planVersions({ tags: ["v1.1.0", "v1.0.0"], workingVersion: "1.2.0" }), [
    { version: "1.2.0", tag: null, source: "worktree", latest: true },
    { version: "1.1.0", tag: "v1.1.0", source: "tag", latest: false },
    { version: "1.0.0", tag: "v1.0.0", source: "tag", latest: false },
  ]);
  // On a release tag the working tree is that tag: one entry, carrying the tag.
  assert.deepEqual(planVersions({ tags: ["v1.1.0", "v1.0.0"], workingVersion: "1.1.0" }), [
    { version: "1.1.0", tag: "v1.1.0", source: "worktree", latest: true },
    { version: "1.0.0", tag: "v1.0.0", source: "tag", latest: false },
  ]);
  // A maintenance branch below the newest tag is not the latest.
  const plan = planVersions({ tags: ["v1.1.0", "v1.0.0"], workingVersion: "1.0.1" });
  assert.equal(plan[0].version, "1.1.0");
  assert.equal(plan[1].version, "1.0.1");
});

test("extractTag unpacks the release paths of a tag and fails on a tag that lacks one", () => {
  const dest = path.join(tmp, "extract");
  extractTag(repo, "v1.0.0", dest);
  assert.ok(fs.existsSync(path.join(dest, "site/operator/legacy.html")));
  assert.match(fs.readFileSync(path.join(dest, "Cargo.toml"), "utf8"), /1\.0\.0/);
  assert.throws(() => extractTag(repo, "v0.9.0", path.join(tmp, "extract-old")), /site|pathspec/i);
});

test("the root is the latest version, every version lives under /vX.Y.Z/, versions.json lists them", () => {
  assert.match(read("index.html"), /<summary class="chip"[^>]*>v1\.2\.0<\/summary>/);
  assert.ok(exists("developer/mcp.html"), "the unreleased chapter is in the root");
  assert.ok(exists("v1.2.0/developer/mcp.html"));
  assert.ok(exists("v1.1.0/operator/helm.html"));
  assert.ok(!exists("v1.1.0/operator/legacy.html"));
  assert.ok(exists("v1.0.0/operator/legacy.html"));
  assert.ok(!exists("v0.9.0"), "a release without a site is not built");
  assert.ok(exists("404.html") && exists("v1.0.0/404.html"));
  assert.ok(exists(".nojekyll"));
  assert.deepEqual(JSON.parse(read("versions.json")), [
    { version: "1.2.0", tag: null, path: "v1.2.0/", latest: true },
    { version: "1.1.0", tag: "v1.1.0", path: "v1.1.0/", latest: false },
    { version: "1.0.0", tag: "v1.0.0", path: "v1.0.0/", latest: false },
  ]);
});

test("the menu of an old version links the same chapter where it exists and the start page elsewhere", () => {
  const legacy = read("v1.0.0/operator/legacy.html");
  assert.match(
    legacy,
    /<li><a href="\/docs\/index\.html">v1\.2\.0 \(latest\)<\/a><\/li><li><a href="\/docs\/v1\.1\.0\/index\.html">v1\.1\.0<\/a><\/li><li><span aria-current="true">v1\.0\.0<\/span><\/li>/,
  );
  assert.match(
    legacy,
    /<p class="version-notice">Documentation for v1\.0\.0\. <a href="\/docs\/index\.html">Read the latest, v1\.2\.0<\/a>\./,
  );
  const install = read("v1.0.0/start/install.html");
  assert.match(install, /<a href="\/docs\/start\/install\.html">v1\.2\.0 \(latest\)<\/a>/);
  assert.match(install, /<a href="\/docs\/v1\.1\.0\/start\/install\.html">v1\.1\.0<\/a>/);
  const latest = read("start/install.html");
  assert.doesNotMatch(latest, /version-notice/);
  assert.match(latest, /<a href="\/docs\/v1\.0\.0\/start\/install\.html">v1\.0\.0<\/a>/);
  assert.match(
    read("v1.2.0/start/install.html"),
    /<span aria-current="true">v1\.2\.0 \(latest\)<\/span>/,
  );
});

test("changelog links follow the tag; the working tree without a tag keeps summer", () => {
  assert.match(read("v1.0.0/index.html"), /blob\/v1\.0\.0\/CHANGELOG\.md">changelog<\/a>/);
  assert.match(read("v1.0.0/index.html"), /blob\/v1\.0\.0\/CHANGELOG\.md">Changelog<\/a>/);
  assert.match(read("v1.1.0/index.html"), /blob\/v1\.1\.0\/CHANGELOG\.md">Changelog<\/a>/);
  assert.match(read("index.html"), /blob\/summer\/CHANGELOG\.md">Changelog<\/a>/);
});

test("every version is served with today's assets, its pages from its own tree", () => {
  assert.equal(read("v1.0.0/assets/site.css"), "body{color:navy}");
  assert.equal(read("v1.0.0/assets/site.js"), "/* v1 */");
  assert.match(read("v1.0.0/index.html"), /Version one\./);
  assert.match(read("v1.0.0/index.html"), /href="assets\/site\.css"/, "assets stay relative");
  assert.match(read("v1.0.0/404.html"), /href="\/docs\/v1\.0\.0\/assets\/site\.css"/);
});

test("a release that no longer builds fails the whole build naming the tag", () => {
  const broken = path.join(tmp, "broken");
  fs.cpSync(repo, broken, { recursive: true });
  // Today's build requires an index page; v1.0.0 in the copy loses its front matter.
  const b = (args) => execFileSync("git", args, { cwd: broken, encoding: "utf8", stdio: "pipe" });
  b(["stash", "-q"]); // the copied working tree carries the unreleased 1.2.0
  b(["checkout", "-q", "v1.0.0"]);
  fs.writeFileSync(path.join(broken, "site/index.html"), "<p>no front matter</p>");
  b(["commit", "-q", "-a", "-m", "break v1.0.0"]);
  b(["tag", "-f", "v1.0.0"]);
  b(["checkout", "-q", "summer"]);
  b(["stash", "pop", "-q"]);
  assert.throws(
    () =>
      buildAll({
        repoRoot: broken,
        outDir: path.join(tmp, "broken-dist"),
        contractsFor: () => [],
        arc42For: () => null,
      }),
    /^Error: v1\.0\.0: index\.html: missing front matter/,
  );
});
