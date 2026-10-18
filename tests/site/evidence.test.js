// Tests for the evidence include of scripts/build-site.js (issue #57): a page
// block `<pre data-evidence="story/file">` is filled from site/evidence at
// build time, and a missing file fails the build by name.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { buildIndex, buildSite, inlineEvidence } from "../../scripts/build-site.js";

let tmp;

before(() => {
  tmp = fs.mkdtempSync(path.join(os.tmpdir(), "sanshain-evidence-"));
});

after(() => {
  fs.rmSync(tmp, { recursive: true, force: true });
});

function writeFiles(dir, files) {
  for (const [rel, content] of Object.entries(files)) {
    const full = path.join(dir, rel);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, content);
  }
}

const EXCHANGE = 'POST /provide HTTP/1.1\nContent-Type: application/json\n\n{"a":"<b>"}\n\n';

test("inlineEvidence fills the block with the escaped file text and keeps the attribute", () => {
  const dir = path.join(tmp, "ev1");
  writeFiles(dir, { "developer/03-provide.http": EXCHANGE });
  const out = inlineEvidence(
    '<p>x</p><pre class="evidence" data-evidence="developer/03-provide.http"></pre>',
    dir,
    "developer/story.html",
  );
  assert.equal(
    out,
    '<p>x</p><pre class="evidence" data-evidence="developer/03-provide.http"><code>' +
      "POST /provide HTTP/1.1\nContent-Type: application/json\n\n{&quot;a&quot;:&quot;&lt;b&gt;&quot;}" +
      "</code></pre>",
  );
});

test("inlineEvidence fails by page and file name when the file is missing", () => {
  const dir = path.join(tmp, "ev2");
  fs.mkdirSync(dir, { recursive: true });
  assert.throws(
    () =>
      inlineEvidence('<pre data-evidence="operator/09-audit.http"></pre>', dir, "operator/x.html"),
    /operator\/x\.html: evidence file "operator\/09-audit\.http" is missing/,
  );
});

test("inlineEvidence fails on an empty evidence file", () => {
  const dir = path.join(tmp, "ev-empty");
  writeFiles(dir, { "operator/10-metrics.txt": "   \n" });
  assert.throws(
    () =>
      inlineEvidence('<pre data-evidence="operator/10-metrics.txt"></pre>', dir, "operator/s.html"),
    /operator\/s\.html: evidence file "operator\/10-metrics\.txt" is empty/,
  );
});

test("inlineEvidence refuses a name that leaves the evidence folder", () => {
  const dir = path.join(tmp, "ev3");
  fs.mkdirSync(dir, { recursive: true });
  assert.throws(
    () => inlineEvidence('<pre data-evidence="../Cargo.toml"></pre>', dir),
    /must be a plain relative path/,
  );
});

test("the search index does not carry evidence text", () => {
  const body =
    '<h2>Step</h2><pre data-evidence="d/a.http"><code>SECRET-DUMP</code></pre><p>prose</p>';
  const index = buildIndex([{ title: "S", track: "developer", url: "s.html", summary: "", body }]);
  assert.equal(index[0].headings[0].body, "prose");
});

test("a site build inlines evidence, copies the folder, and fails on a missing file", () => {
  const src = path.join(tmp, "site");
  writeFiles(src, {
    "index.html": '<!-- page: title="Home" -->\n<p>Home</p>',
    "developer/story.html":
      '<!-- page: title="Story" track="developer" order=8 -->\n' +
      '<pre data-evidence="developer/01-provide.http"></pre>\n' +
      '<img src="../evidence/developer/01-provide.png" alt="shot">',
    "evidence/developer/01-provide.http": EXCHANGE,
    "evidence/developer/01-provide.png": "not really a png",
    "assets/site.css": "body{}",
    "assets/site.js": "",
    "assets/sun.svg": "<svg/>",
  });
  const outDir = path.join(tmp, "site-dist");
  buildSite({ srcDir: src, outDir, version: "1.0.0", contracts: [], arc42: null });
  const page = fs.readFileSync(path.join(outDir, "developer/story.html"), "utf8");
  assert.match(page, /data-evidence="developer\/01-provide.http"><code>POST \/provide HTTP\/1.1/);
  assert.ok(fs.existsSync(path.join(outDir, "evidence/developer/01-provide.png")));
  assert.ok(fs.existsSync(path.join(outDir, "evidence/developer/01-provide.http")));

  fs.rmSync(path.join(src, "evidence/developer/01-provide.http"));
  assert.throws(
    () =>
      buildSite({
        srcDir: src,
        outDir: path.join(tmp, "site-dist-2"),
        version: "1.0.0",
        contracts: [],
        arc42: null,
      }),
    /developer\/story\.html: evidence file "developer\/01-provide\.http" is missing/,
  );
});
