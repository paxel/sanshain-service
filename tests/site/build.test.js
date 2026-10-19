// Tests for scripts/build-site.js: a fixture site in a temp dir, assertions on the output.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import http from "node:http";
import os from "node:os";
import path from "node:path";
import {
  buildSite,
  buildNav,
  buildIndex,
  changelogUrl,
  checkLinks,
  decodeEntities,
  extractLinks,
  parseFrontMatter,
  processHeadings,
  readVersion,
  renderPage,
  resolveRequest,
  serve,
  slugify,
  stripTags,
} from "../../scripts/build-site.js";

const FIXTURE = {
  "index.html": `<!-- page: title="Home" summary="Landing" -->
<p>Welcome to the fixture.</p>
<h2>Why</h2><p>Because pinning is good. See <a href="start/install.html#docker">Docker</a>.</p>
<p id="docker">An authored id that a heading must not steal.</p>
<h2>Docker</h2><p>Collides with the paragraph above.</p>
<h2>Provide &amp; require &mdash; the &#x201c;contract&#8221;</h2><p>Entities.</p>
<p><a href="./">Here</a> <a href=".">Here too</a></p>`,
  "start/install.html": `<!-- page: title="Install" track="start" order=2 summary="How to install" -->
<p>Intro text about installing.</p>
<h2>Docker</h2><p>Run the container with the allow-list set.</p>
<h3>Volumes</h3><p>Mount <code>/data</code>.</p>
<h2>Docker</h2><p>A second section with the same name.</p>
<h2 id="kept">Authored</h2><p>Keeps its id.</p>
<p><a href="../index.html#why">Back</a> <a href="../">Root</a> <a href="..">Root too</a>
<a href='../index.html'>single quoted</a> <a href=../index.html>unquoted</a>
<img src="../assets/sun.svg" alt="sun" srcset="../assets/sun.svg 1x, ../assets/sun.svg 2x"></p>
<table><tr><td>x</td></tr></table>`,
  "start/first-steps.html": `<!-- page: title="First steps" track="start" order=1 -->
<h2>Log in</h2><p>Open the admin page.</p>`,
  "operator/helm.html": `<!-- page: title="Helm chart" track="operator" order=1 -->
<h2>Values</h2><p>Set the ingress.</p>`,
  "assets/site.css": 'body{background:url("sun.svg")} .x{background:url(sun.svg)}',
  "assets/site.js": '/* href="not-a-link" */',
  "assets/sun.svg": "<svg/>",
};

let tmp;
let srcDir;
let outDir;

function writeFixture(dir, files) {
  for (const [rel, content] of Object.entries(files)) {
    const full = path.join(dir, rel);
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, content);
  }
}

function buildVariant(name, overrides, options = {}) {
  const src = path.join(tmp, `${name}-src`);
  writeFixture(src, { ...FIXTURE, ...overrides });
  return buildSite({
    srcDir: src,
    outDir: path.join(tmp, `${name}-out`),
    version: "1.0.0",
    contracts: [],
    arc42: null,
    ...options,
  });
}

before(() => {
  tmp = fs.mkdtempSync(path.join(os.tmpdir(), "sanshain-site-"));
  srcDir = path.join(tmp, "site");
  outDir = path.join(tmp, "site-dist");
  writeFixture(srcDir, FIXTURE);
  buildSite({ srcDir, outDir, version: "9.9.9", contracts: [], arc42: null });
});

after(() => {
  fs.rmSync(tmp, { recursive: true, force: true });
});

const read = (rel) => fs.readFileSync(path.join(outDir, rel), "utf8");

test("front matter parsing reads quoted and bare values and strips the block", () => {
  const { meta, body } = parseFrontMatter(
    '<!-- page: title="A title" track="operator" order=3 summary="Sum" -->\n<p>x</p>',
  );
  assert.deepEqual(meta, { title: "A title", track: "operator", order: 3, summary: "Sum" });
  assert.equal(body, "<p>x</p>");
  assert.throws(
    () => parseFrontMatter("<p>no front matter</p>", "f.html"),
    /f\.html.*front matter/,
  );
  assert.throws(
    () => parseFrontMatter('<!-- page: title="x" track="nope" -->', "g.html"),
    /unknown track "nope"/,
  );
  assert.throws(
    () => parseFrontMatter('<!-- page: title="x" colour="red" -->', "h.html"),
    /h\.html: unknown front-matter key "colour"/,
  );
  assert.throws(
    () => parseFrontMatter("<!-- page: title=Getting started -->", "i.html"),
    /i\.html: cannot read front matter at "started".*values with spaces must be quoted/,
  );
});

test("slugify, stripTags, decodeEntities and readVersion", () => {
  assert.equal(slugify("Docker &amp; Compose: the basics!"), "docker-compose-the-basics");
  assert.equal(slugify("<code>sanshain.yaml</code> file"), "sanshainyaml-file");
  assert.equal(
    stripTags("Mount <code>/data</code>. See <a href='x'>Docker</a>."),
    "Mount /data. See Docker.",
  );
  assert.equal(stripTags("<p>one</p><p>two</p>"), "one two");
  assert.equal(decodeEntities("a &amp;lt; b &ndash; &#169; &#x2192; &hellip;"), "a &lt; b – © → …");
  assert.equal(decodeEntities("&unknown; stays"), "&unknown; stays");
  assert.equal(readVersion('[package]\nname = "x"\nversion = "2.6.0"\n'), "2.6.0");
});

test("nav lists the four tracks in order with pages sorted by order", () => {
  const nav = buildNav([
    { title: "B", track: "start", order: 2, url: "b.html" },
    { title: "A", track: "start", order: 1, url: "a.html" },
    { title: "H", track: "operator", order: 1, url: "h.html" },
    { title: "Home", track: "", order: 0, url: "index.html" },
  ]);
  assert.deepEqual(
    nav.map((t) => t.label),
    ["Start", "Developer", "Operator", "Reference"],
  );
  assert.deepEqual(
    nav[0].pages.map((p) => p.title),
    ["A", "B"],
  );
  assert.equal(nav[1].pages.length, 0);
  assert.equal(nav[2].pages[0].url, "h.html");

  const html = read("start/install.html");
  const sidebar = html.slice(html.indexOf('id="sidebar"'), html.indexOf("</nav>"));
  const order = [
    "Start",
    "First steps",
    "Install",
    "Developer",
    "Operator",
    "Helm chart",
    "Reference",
  ].map((label) => sidebar.indexOf(label));
  assert.ok(
    order.every((i) => i >= 0),
    "every label present",
  );
  assert.deepEqual(
    order,
    [...order].sort((a, b) => a - b),
    "labels appear in track order",
  );
  assert.match(sidebar, /<a href="\.\.\/start\/install\.html" aria-current="page">Install<\/a>/);
  assert.match(sidebar, /<p class="eyebrow">Start<\/p>/, "track labels are not headings");
  assert.doesNotMatch(sidebar, /<h[1-6]/);
  assert.equal(html.search(/<h[1-6]/), html.indexOf("<h1>"), "the page h1 is the first heading");
});

test("headings get ids, deduplicated, with copy-link anchors and a table of contents", () => {
  const html = read("start/install.html");
  assert.match(html, /<h2 id="docker">Docker<a class="anchor" href="#docker"/);
  assert.match(html, /<h3 id="volumes">Volumes<a class="anchor" href="#volumes"/);
  assert.match(html, /<h2 id="docker-2">Docker<a class="anchor" href="#docker-2"/);
  assert.match(html, /<h2 id="kept">Authored<a class="anchor" href="#kept"/);
  const toc = html.slice(html.indexOf('class="toc"'), html.indexOf("</aside>"));
  assert.match(toc, /<li class="toc-l2"><a href="#docker">Docker<\/a><\/li>/);
  assert.match(toc, /<li class="toc-l3"><a href="#volumes">Volumes<\/a><\/li>/);
  assert.match(toc, /#docker-2/);
  assert.match(html, /<div class="table-wrap"><table>/);
  assert.match(html, /<title>Install — Sanshain<\/title>/);
  assert.match(html, /<p class="eyebrow track-eyebrow">Start<\/p>/);
  assert.match(html, /<h1>Install<\/h1>/);
  assert.match(html, /href="\.\.\/assets\/site\.css"/);
});

test("a heading never takes an authored id elsewhere in the page; entities decode in ids and TOC", () => {
  const html = read("index.html");
  assert.match(html, /<p id="docker">/);
  assert.match(html, /<h2 id="docker-2">Docker<a class="anchor" href="#docker-2"/);
  assert.equal((html.match(/id="docker"/g) || []).length, 1, "no duplicate id");
  assert.match(html, /<h2 id="provide-require-the-contract">/);
  const toc = html.slice(html.indexOf('class="toc"'), html.indexOf("</aside>"));
  assert.match(toc, /Provide &amp; require — the “contract”/);

  const { headings } = processHeadings(
    '<h2 id="keep">Keep</h2><h2 id="keep">Keep</h2><h2>Keep</h2>',
  );
  assert.deepEqual(
    headings.map((h) => h.id),
    ["keep", "keep-2", "keep-3"],
  );
});

test("shell carries the version chip, footer links, combobox search and live region", () => {
  const html = read("index.html");
  assert.equal((html.match(/v9\.9\.9/g) || []).length, 2, "version chip in header and footer");
  assert.match(html, /Sanshain Service <span class="chip"/);
  assert.match(html, /href="https:\/\/github\.com\/paxel\/sanshain-service">GitHub<\/a>/);
  assert.match(html, /CHANGELOG\.md">Changelog<\/a>/);
  assert.match(html, /fonts\.googleapis\.com\/css2\?family=Newsreader/);
  assert.match(html, /class="theme-toggle"/);
  assert.match(
    html,
    /<input class="search-input" id="search" type="search"[^>]*role="combobox"[^>]*aria-autocomplete="list"[^>]*aria-controls="search-results"[^>]*aria-expanded="false"/,
  );
  assert.match(html, /<div class="sr-only" id="live" aria-live="polite"/);
  assert.match(html, /<html lang="en" data-root="\.\/">/);
  assert.doesNotMatch(html, /<script src="http/, "no third-party script");
});

test("search index holds pages in nav order, headings and stripped section text", () => {
  const index = JSON.parse(read("search-index.json"));
  assert.deepEqual(
    index.map((p) => p.url),
    ["index.html", "start/first-steps.html", "start/install.html", "operator/helm.html"],
    "landing page first, then tracks in order, then page order",
  );
  const install = index.find((p) => p.url === "start/install.html");
  assert.equal(install.title, "Install");
  assert.equal(install.track, "Start");
  assert.equal(install.intro, "Intro text about installing.");
  assert.deepEqual(
    install.headings.map((h) => h.id),
    ["docker", "volumes", "docker-2", "kept"],
  );
  assert.equal(install.headings[0].text, "Docker");
  assert.equal(install.headings[0].body, "Run the container with the allow-list set.");
  assert.equal(install.headings[1].body, "Mount /data.");
  const home = index.find((p) => p.url === "index.html");
  assert.equal(home.track, "");
  assert.equal(
    home.headings[0].body,
    "Because pinning is good. See Docker. An authored id that a heading must not steal.",
  );
  assert.equal(home.headings[2].text, "Provide & require — the “contract”");

  const direct = buildIndex([
    {
      title: "T",
      track: "operator",
      order: 1,
      url: "t.html",
      summary: "",
      body: "<h2>A &amp; B</h2><p>c</p>",
    },
  ]);
  assert.deepEqual(direct[0].headings, [{ id: "a-b", text: "A & B", body: "c" }]);
});

test("404 page and assets are written in the site look; no empty TOC aside", () => {
  const notFound = read("404.html");
  assert.match(notFound, /<h1>Page not found<\/h1>/);
  assert.match(notFound, /id="search"/);
  assert.match(notFound, /href="\/assets\/site\.css"/);
  assert.doesNotMatch(notFound, /<aside/, "no TOC without headings");
  assert.match(read("index.html"), /<aside class="toc"/);
  assert.ok(fs.existsSync(path.join(outDir, "assets/site.css")));
  assert.ok(fs.existsSync(path.join(outDir, "assets/sun.svg")));
  assert.ok(fs.existsSync(path.join(outDir, ".nojekyll")));
});

test("a base path makes the 404 page absolute while nested pages stay relative", () => {
  const out = path.join(tmp, "based");
  buildSite({
    srcDir,
    outDir: out,
    version: "1.0.0",
    base: "/sanshain-service/",
    contracts: [],
    arc42: null,
  });
  const notFound = fs.readFileSync(path.join(out, "404.html"), "utf8");
  assert.match(notFound, /href="\/sanshain-service\/assets\/site\.css"/);
  assert.match(notFound, /data-root="\/sanshain-service\/"/);
  assert.match(notFound, /href="\/sanshain-service\/index\.html">overview/);
  const nested = fs.readFileSync(path.join(out, "start/install.html"), "utf8");
  assert.match(
    nested,
    /<html lang="en" data-root="\.\.\/">/,
    "search fetches ../search-index.json",
  );
  assert.match(nested, /href="\.\.\/assets\/site\.css"/);
  assert.match(nested, /<script src="\.\.\/assets\/site\.js" defer>/);
  assert.match(nested, /<a class="nav-home" href="\.\.\/index\.html">Overview<\/a>/);
  assert.match(nested, /<a href="\.\.\/operator\/helm\.html">Helm chart<\/a>/);
  assert.match(fs.readFileSync(path.join(out, "index.html"), "utf8"), /href="assets\/site\.css"/);
});

test("a release tag points the changelog links, footer and prose, at that tag", () => {
  const out = path.join(tmp, "tagged");
  buildSite({
    srcDir,
    outDir: out,
    version: "2.7.0",
    tag: "v2.7.0",
    contracts: [],
    arc42: null,
  });
  const html = fs.readFileSync(path.join(out, "index.html"), "utf8");
  assert.match(
    html,
    /href="https:\/\/github\.com\/paxel\/sanshain-service\/blob\/v2\.7\.0\/CHANGELOG\.md">Changelog<\/a>/,
  );
  assert.doesNotMatch(html, /blob\/summer\/CHANGELOG\.md/);
  assert.equal(
    changelogUrl(null),
    "https://github.com/paxel/sanshain-service/blob/summer/CHANGELOG.md",
  );
  const prose = renderPage(
    {
      title: "Deploy",
      url: "operator/deploy.html",
      body: `<p>See the <a href="${changelogUrl(null)}">changelog</a>.</p>`,
    },
    { nav: buildNav([]), version: "2.7.0", tag: "v2.7.0" },
  );
  assert.match(
    prose,
    /<a href="https:\/\/github\.com\/paxel\/sanshain-service\/blob\/v2\.7\.0\/CHANGELOG\.md">changelog<\/a>/,
  );
});

test("the version menu links the same chapter in other versions, the start page where it is missing", () => {
  const versions = [
    { version: "2.0.0", base: "/docs/", pages: ["index.html", "start/install.html"], latest: true },
    {
      version: "1.0.0",
      base: "/docs/v1.0.0/",
      pages: ["index.html", "start/install.html", "operator/helm.html"],
      latest: false,
    },
    { version: "0.9.0", base: "/docs/v0.9.0/", pages: ["index.html"], latest: false },
  ];
  // What build-site-all.js passes: every page of every version, root-relative.
  const targetsOf = (entries) =>
    new Set(entries.flatMap((entry) => entry.pages.map((page) => `${entry.base}${page}`)));
  const out = path.join(tmp, "versioned");
  buildSite({
    srcDir,
    outDir: out,
    version: "1.0.0",
    tag: "v1.0.0",
    base: "/docs/v1.0.0/",
    versions,
    knownTargets: targetsOf(versions),
    contracts: [],
    arc42: null,
  });
  const helm = fs.readFileSync(path.join(out, "operator/helm.html"), "utf8");
  assert.match(
    helm,
    /<details class="version-menu"><summary class="chip"[^>]*>v1\.0\.0<\/summary>/,
  );
  assert.match(
    helm,
    /<li><a href="\/docs\/index\.html">v2\.0\.0 \(latest\)<\/a><\/li><li><span aria-current="true">v1\.0\.0<\/span><\/li><li><a href="\/docs\/v0\.9\.0\/index\.html">v0\.9\.0<\/a><\/li>/,
    "latest lacks the chapter: its start page; the current version is text; 0.9.0 lacks it too",
  );
  assert.match(
    helm,
    /<p class="version-notice">Documentation for v1\.0\.0\. <a href="\/docs\/index\.html">Read the latest, v2\.0\.0<\/a>\.<\/p>/,
  );
  const install = fs.readFileSync(path.join(out, "start/install.html"), "utf8");
  assert.match(
    install,
    /<a href="\/docs\/start\/install\.html">v2\.0\.0 \(latest\)<\/a>/,
    "same chapter",
  );
  assert.match(install, /Read the latest, v2\.0\.0<\/a>/);
  assert.match(
    fs.readFileSync(path.join(out, "404.html"), "utf8"),
    /<a href="\/docs\/index\.html">v2\.0\.0/,
  );
  assert.equal((install.match(/class="chip"/g) || []).length, 2, "menu summary and footer chip");

  // The latest version itself carries the menu but no notice.
  const latestOut = path.join(tmp, "versioned-latest");
  buildSite({
    srcDir,
    outDir: latestOut,
    version: "2.0.0",
    base: "/docs/",
    versions,
    knownTargets: targetsOf(versions),
    contracts: [],
    arc42: null,
  });
  const latestIndex = fs.readFileSync(path.join(latestOut, "index.html"), "utf8");
  assert.doesNotMatch(latestIndex, /version-notice/);
  assert.match(latestIndex, /<li><span aria-current="true">v2\.0\.0 \(latest\)<\/span><\/li>/);

  // A menu link to a version that is not in the published set is a broken link.
  assert.throws(
    () =>
      buildSite({
        srcDir,
        outDir: path.join(tmp, "versioned-broken"),
        version: "1.0.0",
        base: "/docs/v1.0.0/",
        versions,
        contracts: [],
        arc42: null,
      }),
    /Broken link in index\.html: "\/docs\/index\.html" is outside the site base/,
  );
});

test("a broken link fails the build naming the file and the link", () => {
  const page = (body) => `<!-- page: title="Helm" track="operator" order=1 -->\n${body}`;
  assert.throws(
    () =>
      buildVariant("broken", {
        "operator/helm.html": page('<a href="../start/missing.html">gone</a>'),
      }),
    (err) =>
      err.message.includes("operator/helm.html") && err.message.includes('"../start/missing.html"'),
  );
  assert.throws(
    () =>
      buildVariant("anchor", {
        "operator/helm.html": page('<a href="../index.html#nope">gone</a>'),
      }),
    /operator\/helm\.html: "\.\.\/index\.html#nope" points to a missing anchor #nope/,
  );
  assert.throws(
    () =>
      buildVariant("image", {
        "index.html": '<!-- page: title="Home" -->\n<img src="assets/missing.png" alt="">',
      }),
    /index\.html: "assets\/missing\.png"/,
  );
  assert.throws(
    () =>
      buildVariant("srcset", {
        "operator/helm.html": page(
          '<img src="../assets/sun.svg" srcset="../assets/nope.png 2x" alt="">',
        ),
      }),
    /operator\/helm\.html: "\.\.\/assets\/nope\.png"/,
  );
  assert.throws(
    () => buildVariant("css", { "assets/site.css": "body{background:url(missing.png)}" }),
    /assets\/site\.css: "missing\.png"/,
  );
  assert.throws(
    () =>
      buildVariant("above-root", {
        "operator/helm.html": page('<a href="../../index.html">too far</a>'),
      }),
    /operator\/helm\.html: "\.\.\/\.\.\/index\.html"/,
  );
  // Root-relative links fail identically with the local base and the Pages base.
  for (const base of ["/", "/sanshain-service/"]) {
    assert.throws(
      () =>
        buildVariant(
          `root-${base.length}`,
          { "operator/helm.html": page('<a href="/start/install.html">abs</a>') },
          { base },
        ),
      /operator\/helm\.html: "\/start\/install\.html" is root-relative/,
    );
  }
});

test("checkLinks accepts external links, same-page anchors, root and directory links", () => {
  const files = new Map([
    [
      "index.html",
      '<a href="https://x.example/">x</a> <a href="mailto:a@b">m</a> <a href="#top">t</a> <a href="start/">d</a> <a href="start/index.html?v=1#s">q</a> <a href="./">dot</a> <p id="top"></p>',
    ],
    [
      "start/index.html",
      '<h2 id="s">s</h2><a href="../index.html">up</a><a href="../">r</a><a href="..">rr</a><a href=".">me</a>',
    ],
    [
      "404.html",
      '<a href="/index.html">home</a><link href="/assets/site.css"><a href="/">root</a>',
    ],
    ["assets/site.css", "a{}"],
  ]);
  assert.doesNotThrow(() => checkLinks(files));
  files.set("start/index.html", '<h2 id="s">s</h2><a href="#zzz">bad</a>');
  assert.throws(() => checkLinks(files), /start\/index\.html: "#zzz"/);
  files.set("start/index.html", '<h2 id="s">s</h2>');
  assert.throws(
    () => checkLinks(files, { base: "/docs/" }),
    /404\.html: "\/index\.html" is outside the site base \/docs\//,
  );
  assert.deepEqual(
    extractLinks("a.html", "<a href='x'>1</a><a href=y>2</a><img srcset=\"p 1x, q 2x\">"),
    ["x", "y", "p", "q"],
  );
});

test("the output folder guard refuses dangerous targets", () => {
  const opts = { srcDir, version: "1", contracts: [], arc42: null };
  assert.throws(() => buildSite({ ...opts, outDir: "/" }), /Refusing to wipe/);
  assert.throws(() => buildSite({ ...opts, outDir: os.homedir() }), /Refusing to wipe/);
  assert.throws(() => buildSite({ ...opts, outDir: tmp }), /contains the source folder/);
  assert.throws(() => buildSite({ ...opts, outDir: srcDir }), /contains the source folder/);
});

test("renderPage escapes the title and renders without a track", () => {
  const nav = buildNav([]);
  const html = renderPage(
    { title: "A <b>", track: "", order: 0, url: "index.html", summary: "", body: "<p>hi</p>" },
    { nav, version: "0.0.1" },
  );
  assert.match(html, /<title>A &lt;b&gt; — Sanshain<\/title>/);
  assert.doesNotMatch(html, /track-eyebrow/);
  assert.doesNotMatch(html, /<aside/);
  assert.match(html, /No chapters yet/);
});

test("the preview server stays inside the output folder and binds to loopback", async () => {
  assert.equal(resolveRequest(outDir, "/"), path.join(outDir, "index.html"));
  assert.equal(
    resolveRequest(outDir, "/start/install.html"),
    path.join(outDir, "start", "install.html"),
  );
  assert.equal(resolveRequest(outDir, "/start/"), null, "no index.html in that folder");
  assert.equal(resolveRequest(outDir, "/../site/index.html"), null);
  assert.equal(resolveRequest(outDir, "/..%2F..%2Fetc%2Fpasswd"), null);
  assert.equal(resolveRequest(outDir, "/%ZZ"), null);

  const server = serve(outDir, 0);
  await new Promise((resolve) => server.once("listening", resolve));
  try {
    const { address, port } = server.address();
    assert.equal(address, "127.0.0.1");
    const get = (p) =>
      new Promise((resolve, reject) =>
        http
          .get({ host: "127.0.0.1", port, path: p }, (res) => {
            let body = "";
            res.on("data", (c) => (body += c));
            res.on("end", () => resolve({ status: res.statusCode, body }));
          })
          .on("error", reject),
      );
    assert.equal((await get("/")).status, 200);
    assert.equal((await get("/start/install.html")).status, 200);
    const escape = await get("/..%2F..%2Fetc%2Fpasswd");
    assert.equal(escape.status, 404);
    assert.match(escape.body, /Page not found/);
    assert.equal((await get("/nope/deep")).status, 404);
  } finally {
    await new Promise((resolve) => server.close(resolve));
  }
});
