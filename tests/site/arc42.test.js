// Tests for the arc42 Architecture page: the table reader on a fixture and
// on the real export, box linking on a fixture SVG, the link map, ADR links,
// and the page in a real site build.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  DIAGRAMS,
  DIAGRAM_SCALE,
  adrIndex,
  arc42Page,
  bbox,
  cellHtml,
  linkAdrs,
  linkBoxes,
  prepareSvg,
  readExport,
  readLinkMap,
  sectionId,
  splitRow,
  topLevelNodes,
} from "../../scripts/site/arc42.js";
import {
  buildSite,
  checkLinks,
  DEFAULT_ARC42,
  extractLinks,
  stripTags,
} from "../../scripts/build-site.js";

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const ARC42 = path.join(REPO_ROOT, "docs/arc42");
const REPO_URL = "https://github.com/paxel/sanshain-service";

const FIXTURE_MD = `### Whitebox Thing

![Whitebox Thing](<context - Thing.svg>)

#### Motivation

Two lines of
motivation text.

#### Contained building blocks

| Name   | Responsibility                       |
|--------|--------------------------------------|
| Alpha  | Does \`a\` and \`b\` (ADR-0003).         |
| Beta   | Nothing – yet.                       |

#### External interfaces

| Partner outside | Attached to | Direction | Text |
|:---|:---:|---:|---|
| Outer › Alpha, Beta | Alpha | in | calls |
`;

// Two boxes drawn the way whiteboxed exports them: shape, icon, name, kind,
// glyph; the second is a database (polygon); a background rect, a container
// and an edge label follow the export's structure too.
const FIXTURE_SVG = `<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300" viewBox="-12.0 -12.0 400.0 300.0">
<rect x="-12.0" y="-12.0" width="400.0" height="300.0" fill="#ffffff"/>
<rect x="4.0" y="4.0" width="370.0" height="280.0" rx="4.0" fill="none" stroke="#8a939e" stroke-width="1.5" stroke-dasharray="6 4"/>
<text x="16.0" y="25.2" font-size="15" text-anchor="start" fill="#8a939e">Thing (whitebox)</text>
<rect x="20.0" y="50.0" width="150.0" height="80.0" rx="4.0" fill="#a8d5f7" stroke="#222222" stroke-width="1.5"/>
<svg x="30.0" y="55.0" width="20.0" height="12.0" viewBox="0 0 20 12"><circle cx="5" cy="6" r="4" fill="none" stroke="currentColor"></circle></svg>
<text x="95.0" y="88.2" font-size="15" text-anchor="middle" fill="#222222">Alpha &amp; Co</text>
<text x="95.0" y="105.9" font-size="11" text-anchor="middle" fill="#666666">«component»</text>
<rect x="148.0" y="112.0" width="14.0" height="10.0" rx="1.0" fill="none" stroke="#666666" stroke-width="1"/>
<rect x="151.0" y="115.0" width="4.0" height="4.0" fill="#666666"/>
<polygon points="200.0,60.0 200.0,120.0 210.0,130.0 350.0,130.0 360.0,120.0 360.0,60.0" fill="#ffd58f" stroke="#222222" stroke-width="1.5"/>
<polyline points="200.0,60.0 280.0,66.0 360.0,60.0" fill="none" stroke="#222222" stroke-width="1.5"/>
<text x="280.0" y="98.2" font-size="15" text-anchor="middle" fill="#222222">Beta</text>
<text x="280.0" y="115.9" font-size="11" text-anchor="middle" fill="#666666">«database»</text>
<polygon points="170.0,90.0 180.0,85.0 180.0,95.0" fill="#444b55"/>
<polyline points="170.0,90.0 200.0,90.0" fill="none" stroke="#444b55" stroke-width="2"/>
<rect x="180.0" y="70.0" width="30.0" height="16.0" fill="#ffffff"/>
<text x="183.0" y="82.0" font-size="12" text-anchor="start" fill="#222222">calls</text>
<text x="18.0" y="200.0" font-size="12" text-anchor="start" fill="#666666">Legend</text>
</svg>
`;

// No background, frame or title: the first element is a box shape, and the
// box draws two nested rects the way the export draws a cache.
const FIXTURE_SVG_BARE = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 100">
<rect x="10.0" y="10.0" width="152.0" height="80.0" rx="4.0" fill="#ffd58f" stroke="#222222" stroke-width="1.5"/>
<rect x="14.0" y="14.0" width="144.0" height="72.0" rx="2.0" fill="none" stroke="#222222" stroke-width="1"/>
<text x="86.0" y="48.2" font-size="15" text-anchor="middle" fill="#222222">Cache</text>
<text x="86.0" y="65.9" font-size="11" text-anchor="middle" fill="#666666">«cache»</text>
<text x="170.0" y="50.0" font-size="12" text-anchor="start" fill="#222222">label</text>
</svg>`;

const resolver = (links) => (name) => {
  if (name in links) return links[name];
  throw new Error(`unmapped ${name}`);
};

let tmp;
let realOut;

before(() => {
  tmp = fs.mkdtempSync(path.join(os.tmpdir(), "sanshain-arc42-"));
  realOut = path.join(tmp, "real-out");
  buildSite({ srcDir: path.join(REPO_ROOT, "site"), outDir: realOut, version: "2.6.0" });
});

after(() => {
  fs.rmSync(tmp, { recursive: true, force: true });
});

// ---------------------------------------------------------------------------
// Table reader

test("readExport finds the title, image, motivation and tables by structure", () => {
  const doc = readExport(FIXTURE_MD, "thing.md");
  assert.equal(doc.title, "Whitebox Thing");
  assert.equal(doc.image, "context - Thing.svg");
  assert.equal(doc.motivation, "Two lines of motivation text.");
  assert.equal(doc.tables.length, 2);
  assert.deepEqual(doc.tables[0], {
    title: "Contained building blocks",
    header: ["Name", "Responsibility"],
    rows: [
      ["Alpha", "Does `a` and `b` (ADR-0003)."],
      ["Beta", "Nothing – yet."],
    ],
  });
  assert.equal(doc.tables[1].title, "External interfaces");
  assert.deepEqual(doc.tables[1].header, ["Partner outside", "Attached to", "Direction", "Text"]);
  assert.deepEqual(doc.tables[1].rows, [["Outer › Alpha, Beta", "Alpha", "in", "calls"]]);
});

test("the context export's paragraph follows the image directly, without a Motivation heading", () => {
  const doc = readExport(
    "### Context\n\n![Context view](<context.svg>)\n\nThe paragraph.\n\n#### Partners\n\n| A | B |\n|---|---|\n| 1 | 2 |\n",
  );
  assert.equal(doc.motivation, "The paragraph.");
  assert.deepEqual(doc.tables[0].rows, [["1", "2"]]);
});

test("a table that does not parse fails naming the file and line", () => {
  const head = "### T\n\n![T](<t.svg>)\n\nText.\n\n#### Blocks\n\n";
  assert.throws(
    () => readExport(`${head}| A | B |\n|---|---|\n| only one |\n`, "t.md"),
    /t\.md:11: table "Blocks": row has 1 cells, the header has 2/,
  );
  assert.throws(
    () => readExport(`${head}| A | B |\n| 1 | 2 |\n`, "t.md"),
    /t\.md:10: table "Blocks": the header is not followed by a \|---\| separator/,
  );
  assert.throws(
    () => readExport(`${head}| A | B |\n|---|\n| 1 | 2 |\n`, "t.md"),
    /t\.md:10: table "Blocks": separator and header column counts differ/,
  );
  assert.throws(
    () => readExport(`${head}No table here.\n`, "t.md"),
    /"Blocks" is not followed by a table/,
  );
  assert.throws(() => readExport("### T\n\nno image\n", "t.md"), /t\.md:1: no image line/);
  assert.throws(() => readExport("![T](<t.svg>)\n", "t.md"), /t\.md:1: no ### title/);
});

test("splitRow keeps pipes inside backticks and unescapes \\|; empty and dash cells survive", () => {
  assert.deepEqual(splitRow("| a | `x|y` | c \\| d |  | – |"), ["a", "`x|y`", "c | d", "", "–"]);
  assert.deepEqual(splitRow("a|b"), ["a", "b"]);
  const doc = readExport(
    "### T\n\n![T](<t.svg>)\n\nText.\n\n#### Blocks\n\n| A | B | C |\n|---|---|---|\n| `p|q` | | – |\n",
  );
  assert.deepEqual(doc.tables[0].rows, [["`p|q`", "", "–"]]);
});

test("cellHtml turns backticks into code and escapes the rest", () => {
  assert.equal(cellHtml("Does `a<b` & `c`"), "Does <code>a&lt;b</code> &amp; <code>c</code>");
  assert.equal(cellHtml("–"), "–");
});

test("the real export files read with the expected table and row counts", () => {
  const counts = {};
  const firstWords = {
    Context: "Sanshain sits between the build pipelines",
    "Sanshain Service": "Hexagonal layering: the HTTP API and the MCP Server",
    "HTTP API": "The endpoint groups follow the public REST surface",
    "Application Services": "Split by use case rather than by entity",
    Persistence: "The cache is a decorator over the same port",
  };
  for (const d of DIAGRAMS) {
    const file = path.join(ARC42, `${d.stem}.md`);
    const doc = readExport(fs.readFileSync(file, "utf8"), `${d.stem}.md`);
    assert.ok(doc.motivation.startsWith(firstWords[d.name]), `${d.stem}: ${doc.motivation}`);
    assert.ok(fs.existsSync(path.join(ARC42, doc.image)), `${d.stem}: ${doc.image} exists`);
    counts[d.name] = doc.tables.map((t) => `${t.title}:${t.rows.length}`);
  }
  assert.deepEqual(counts, {
    Context: ["Communication partners:8", "Building blocks:2"],
    "Sanshain Service": [
      "Contained building blocks:9",
      "External interfaces:9",
      "Internal relations:11",
    ],
    "HTTP API": ["Contained building blocks:7", "External interfaces:16"],
    "Application Services": [
      "Contained building blocks:8",
      "External interfaces:17",
      "Internal relations:6",
    ],
    Persistence: ["Contained building blocks:3", "External interfaces:4", "Internal relations:1"],
  });
});

// ---------------------------------------------------------------------------
// Link map and ADR links

test("readLinkMap accepts URL and { doc, whitebox } entries and rejects the rest", () => {
  const map = readLinkMap(
    '{"links":{"A":"a.html","B":{"doc":"b.html","whitebox":"#b"}},"unlinked":["C"]}',
  );
  assert.deepEqual(map, {
    links: { A: { doc: "a.html" }, B: { doc: "b.html", whitebox: "#b" } },
    unlinked: ["C"],
  });
  assert.throws(() => readLinkMap("{", "m.json"), /m\.json: /);
  assert.throws(() => readLinkMap('{"unlinked":[]}', "m.json"), /"links" object/);
  assert.throws(() => readLinkMap('{"links":{"A":""}}', "m.json"), /"A" must map to a URL/);
  assert.throws(() => readLinkMap('{"links":{"A":{"whitebox":"#a"}}}', "m.json"), /"A" must map/);
  assert.throws(
    () => readLinkMap('{"links":{"A":"a"},"unlinked":["A"]}'),
    /both linked and unlinked/,
  );
  assert.throws(() => readLinkMap('{"links":{},"unlinked":"A"}'), /"unlinked" must be an array/);
});

test("ADR references become links to the files in docs/adr; an unknown number fails", () => {
  const adrs = adrIndex(path.join(REPO_ROOT, "docs/adr"));
  assert.equal(adrs["0003"], "0003-versions-replace-branches.md");
  assert.equal(adrs["0009"], "0009-webhooks-outbox-and-egress-allowlist.md");
  const base = `${REPO_URL}/blob/summer/docs/adr`;
  assert.equal(
    linkAdrs("see ADR-0003.", adrs, base),
    `see <a href="${base}/0003-versions-replace-branches.md">ADR-0003</a>.`,
  );
  assert.equal(
    linkAdrs("(x, ADR-0004/0005):", adrs, base),
    `(x, <a href="${base}/0004-trunk-flag-and-graph-views.md">ADR-0004</a>/` +
      `<a href="${base}/0005-sanshain-branches-and-timeline.md">0005</a>):`,
  );
  assert.throws(
    () => linkAdrs("ADR-0042", adrs, base),
    /ADR-0042 is referenced but docs\/adr has no such file/,
  );
  assert.throws(() => linkAdrs("ADR-0004/0042", adrs, base), /ADR-0042/);
});

// ---------------------------------------------------------------------------
// SVG

test("prepareSvg drops the prolog, width, height and background rect and keeps the viewBox", () => {
  const { open, inner, viewBox, width } = prepareSvg(FIXTURE_SVG, "f.svg");
  assert.equal(viewBox, "-12.0 -12.0 400.0 300.0");
  assert.equal(width, 400);
  assert.equal(
    open,
    '<svg class="arc42-svg" xmlns="http://www.w3.org/2000/svg" viewBox="-12.0 -12.0 400.0 300.0">',
  );
  assert.ok(inner.startsWith('<rect x="4.0"'), "the background rect is gone, the container stays");
  assert.ok(!inner.includes("<?xml"));
  assert.throws(() => prepareSvg("<html></html>", "f.svg"), /f\.svg: not an SVG document/);
  assert.throws(() => prepareSvg("<svg></svg>", "f.svg"), /f\.svg: the root <svg> has no viewBox/);
});

test("topLevelNodes and bbox read the export's element runs", () => {
  const { inner } = prepareSvg(FIXTURE_SVG);
  const nodes = topLevelNodes(inner);
  assert.deepEqual(
    nodes.map((n) => n.tag),
    "rect text rect svg text text rect rect polygon polyline text text polygon polyline rect text text".split(
      " ",
    ),
  );
  assert.equal(nodes[4].text, "Alpha &amp; Co");
  assert.deepEqual(bbox(nodes[2]), [20, 50, 170, 130]);
  assert.deepEqual(bbox(nodes[8]), [200, 60, 360, 130]);
  assert.deepEqual(bbox(nodes[4]), [95, 88.2, 95, 88.2]);
  assert.equal(bbox({ tag: "path", open: '<path d="M0 0"/>' }), null);
  assert.throws(() => topLevelNodes("<g><rect/>"), /unbalanced/);
});

test("linkBoxes wraps each box run, shape to glyph, in an SVG <a> with the resolved href", () => {
  const { inner } = prepareSvg(FIXTURE_SVG);
  const { svg, boxes } = linkBoxes(inner, resolver({ "Alpha & Co": "#alpha", Beta: "../b.html" }));
  assert.deepEqual(boxes, [
    { name: "Alpha & Co", href: "#alpha" },
    { name: "Beta", href: "../b.html" },
  ]);
  // The first run: the box rect, its icon, both texts and the component glyph.
  assert.match(
    svg,
    /<a href="#alpha" class="arc42-box" aria-label="Alpha &amp; Co"><rect x="20\.0"[^>]*\/>\n<svg x="30\.0"[\s\S]*?Alpha &amp; Co<\/text>\n<text[^>]*>«component»<\/text>\n<rect x="148\.0"[^>]*\/>\n<rect x="151\.0"[^>]*\/><\/a>\n<a [^>]*><polygon/,
  );
  // The database: polygon, lid polyline, texts; the edge arrow after it stays outside.
  assert.match(
    svg,
    /<a href="\.\.\/b\.html" class="arc42-box" aria-label="Beta"><polygon points="200\.0[\s\S]*?«database»<\/text><\/a>\n<polygon points="170\.0/,
  );
  // Container, title and edge labels are untouched.
  assert.equal(svg.match(/<a /g).length, 2);
  assert.ok(svg.includes('fill="#8a939e">Thing (whitebox)</text>'));
  assert.ok(svg.endsWith('fill="#666666">Legend</text>'));
});

test("an unlinked box stays plain; a linked box without a containing shape fails", () => {
  const { inner } = prepareSvg(FIXTURE_SVG);
  const { svg, boxes } = linkBoxes(inner, (name) => (name === "Beta" ? "#beta" : null));
  assert.deepEqual(boxes, [
    { name: "Alpha & Co", href: null },
    { name: "Beta", href: "#beta" },
  ]);
  assert.equal(svg.match(/<a /g).length, 1);
  assert.throws(
    () =>
      linkBoxes(
        '<rect x="50" y="50" width="9" height="9"/>\n<text x="1" y="2">Lone</text>\n<text x="1" y="3">«component»</text>',
        resolver({ Lone: "x.html" }),
        "lone.svg",
      ),
    /lone\.svg: no shape contains the name of box "Lone"; cannot link it/,
  );
});

test("without a frame title the search still finds the box shape, widened to a nested rect", () => {
  const { inner } = prepareSvg(FIXTURE_SVG_BARE);
  const { svg, boxes } = linkBoxes(inner, resolver({ Cache: "c.html" }));
  assert.deepEqual(boxes, [{ name: "Cache", href: "c.html" }]);
  assert.equal(
    svg,
    '<a href="c.html" class="arc42-box" aria-label="Cache"><rect x="10.0" y="10.0" width="152.0" height="80.0" rx="4.0" fill="#ffd58f" stroke="#222222" stroke-width="1.5"/>\n' +
      '<rect x="14.0" y="14.0" width="144.0" height="72.0" rx="2.0" fill="none" stroke="#222222" stroke-width="1"/>\n' +
      '<text x="86.0" y="48.2" font-size="15" text-anchor="middle" fill="#222222">Cache</text>\n' +
      '<text x="86.0" y="65.9" font-size="11" text-anchor="middle" fill="#666666">«cache»</text></a>\n' +
      '<text x="170.0" y="50.0" font-size="12" text-anchor="start" fill="#222222">label</text>',
  );
});

test("an unmapped box fails with its name and file", () => {
  const { inner } = prepareSvg(FIXTURE_SVG);
  assert.throws(() => linkBoxes(inner, resolver({ Beta: "#b" })), /unmapped Alpha & Co/);
  const dir = path.join(tmp, "unmapped");
  fs.mkdirSync(dir, { recursive: true });
  for (const d of DIAGRAMS) {
    for (const ext of [".md", ".svg"])
      fs.copyFileSync(path.join(ARC42, d.stem + ext), path.join(dir, d.stem + ext));
  }
  const links = path.join(dir, "links.json");
  const real = JSON.parse(fs.readFileSync(DEFAULT_ARC42.linksFile, "utf8"));
  delete real.links["Spec Parsers"];
  fs.writeFileSync(links, JSON.stringify(real));
  const page = (linksFile) => arc42Page({ ...DEFAULT_ARC42, dir, linksFile, repoUrl: REPO_URL });
  assert.throws(
    () => page(links),
    /context - Sanshain Service\.svg: box "Spec Parsers" has no entry in the link map; add it to "links" or "unlinked"/,
  );
  real.unlinked.push("Spec Parsers");
  fs.writeFileSync(links, JSON.stringify(real));
  assert.ok(page(links).body.includes("Spec Parsers"));
  // The motivation goes through the same backtick rendering as the cells.
  const md = path.join(dir, "context - Sanshain Service - Persistence.md");
  fs.writeFileSync(md, fs.readFileSync(md, "utf8").replace("The cache is", "The `cache` is"));
  assert.ok(page(links).body.includes("<p>The <code>cache</code> is a decorator"));
  fs.rmSync(path.join(dir, "context - Sanshain Service - Persistence.svg"));
  assert.throws(
    () => page(links),
    /arc42 diagram context - Sanshain Service - Persistence\.svg: cannot read/,
  );
  fs.rmSync(path.join(dir, "context.md"));
  assert.throws(() => page(links), /arc42 export context\.md: cannot read/);
});

test("sectionId matches the ids the headings get", () => {
  assert.equal(sectionId("HTTP API"), "http-api");
  assert.equal(sectionId("Branches & Reports"), "branches-reports");
});

// ---------------------------------------------------------------------------
// The real page and build

test("the real site build renders the Architecture page with all five diagrams inline", () => {
  assert.ok(
    !fs.existsSync(path.join(REPO_ROOT, "site/reference/architecture.html")),
    "generated only into the output",
  );
  const html = fs.readFileSync(path.join(realOut, "reference/architecture.html"), "utf8");
  assert.match(html, /<title>Architecture — Sanshain<\/title>/);
  assert.match(html, /<p class="eyebrow track-eyebrow">Reference<\/p>/);
  assert.equal(
    html.match(
      /<figure class="arc42-diagram" style="--diagram-min-width: \d+px"><svg class="arc42-svg" xmlns="http:\/\/www\.w3\.org\/2000\/svg" viewBox="/g,
    ).length,
    5,
  );
  assert.ok(!html.includes("<?xml"));
  assert.ok(!/<svg class="arc42-svg"[^>]*\swidth=/.test(html), "width/height left to the class");
  assert.ok(!html.includes('role="img"'));
  // Each figure: min-width from the export's viewBox, the svg labelled by the
  // section heading with the diagram name as its <title>.
  for (const d of DIAGRAMS) {
    const id = d.name.toLowerCase().replace(/ /g, "-");
    const source = fs.readFileSync(path.join(ARC42, `${d.stem}.svg`), "utf8");
    const width = Number(/viewBox="[^ ]+ [^ ]+ ([^ ]+) /.exec(source)[1]);
    const title = d.name === "Context" ? "Context" : `Whitebox ${d.name}`;
    assert.ok(
      html.includes(
        `<figure class="arc42-diagram" style="--diagram-min-width: ${Math.round(width * DIAGRAM_SCALE)}px"><svg class="arc42-svg" xmlns="http://www.w3.org/2000/svg" viewBox="-12.0 -12.0 ${width.toFixed(1)} `,
      ),
      `${d.stem}: figure min-width`,
    );
    assert.match(html, new RegExp(`aria-labelledby="${id}"><title>${title}</title>`));
  }
  assert.ok(
    Math.round(1272.4 * DIAGRAM_SCALE) >= 960,
    "the context diagram keeps 12 px labels above 9 px",
  );
  // Every box link holds exactly its run: shapes, icon, the name and the «kind».
  const runs = [
    ...html.matchAll(/<a href="[^"]*" class="arc42-box" aria-label="([^"]*)">([\s\S]*?)<\/a>/g),
  ];
  assert.equal(runs.length, 36);
  for (const [, label, run] of runs) {
    const nodes = topLevelNodes(run);
    const texts = nodes.filter((n) => n.tag === "text").map((n) => n.text);
    assert.equal(texts.length, 2, `${label}: two texts`);
    assert.equal(texts[0], label, `${label}: the label is the name`);
    assert.match(texts[1], /^«[^»]+»$/);
    for (const n of nodes) {
      assert.ok(
        ["rect", "polygon", "polyline", "circle", "svg", "text"].includes(n.tag),
        `${label}: ${n.tag}`,
      );
    }
  }
  // One section per diagram in order, each with breadcrumb, diagram, motivation, tables.
  const order = [...html.matchAll(/<h2 id="([^"]+)">/g)].map((m) => m[1]);
  assert.deepEqual(order, [
    "reading-the-diagrams",
    "context",
    "sanshain-service",
    "http-api",
    "application-services",
    "persistence",
  ]);
  assert.match(
    html,
    /<nav class="breadcrumb" aria-label="Level"><a href="#context">Context<\/a> › <a href="#sanshain-service">Sanshain Service<\/a> › <span aria-current="true">HTTP API<\/span><\/nav>/,
  );
  assert.match(html, /<\/figure>\n<p>Hexagonal layering: the HTTP API and the MCP Server/);
  assert.match(html, /<h3 id="context-communication-partners">Communication partners/);
  assert.match(
    html,
    /<div class="table-wrap"><table><thead><tr><th>Partner<\/th><th>Description<\/th>/,
  );
  // Line kinds explained once, next to the legend the SVG carries.
  for (const kind of ["business", "admin", "telemetry", "async-outbound"]) {
    assert.match(html, new RegExp(`<dt><code>${kind}</code></dt>`));
  }
  // Boxes with a whitebox link down a level in the diagram and to the docs in the table.
  assert.match(html, /<a href="#http-api" class="arc42-box" aria-label="HTTP API"><rect /);
  assert.match(html, /<td><a href="api.html">HTTP API<\/a><\/td>/);
  assert.match(html, /<td><a href="api.html">HTTP API<\/a> › Provide Endpoints, /);
  assert.ok(!/<a [^>]*>User<\/a>/.test(html), "User is unlinked");
  assert.match(
    html,
    /<a href="[^"]*\/docs\/adr\/0009-webhooks-outbox-and-egress-allowlist\.md">ADR-0009<\/a>/,
  );
  // The nav lists it after the API pages; the TOC is the six h2s.
  const nav = /<section class="nav-track" aria-label="Reference">[\s\S]*?<\/section>/.exec(html)[0];
  assert.match(nav, /Using the API.*REST API.*Maintenance API.*Architecture/);
  assert.match(
    html,
    /<aside class="toc"[\s\S]*?#reading-the-diagrams[\s\S]*?#persistence"[\s\S]*?<\/aside>/,
  );
});

test("every box in the real diagrams links and every map entry resolves in the output", () => {
  const map = readLinkMap(fs.readFileSync(DEFAULT_ARC42.linksFile, "utf8"));
  const seen = new Set();
  for (const d of DIAGRAMS) {
    const { inner } = prepareSvg(fs.readFileSync(path.join(ARC42, `${d.stem}.svg`), "utf8"));
    // linkBoxes throws for a linked box without a shape, so every link is a whole box.
    const { boxes } = linkBoxes(
      inner,
      (name) => {
        seen.add(name);
        return map.unlinked.includes(name) ? null : map.links[name].whitebox || map.links[name].doc;
      },
      `${d.stem}.svg`,
    );
    assert.ok(boxes.length >= 3, `${d.stem}: boxes found`);
  }
  for (const name of [...Object.keys(map.links), ...map.unlinked]) {
    assert.ok(seen.has(name), `"${name}" in the link map is a box in some diagram`);
  }
  // Every doc and whitebox URL of the map resolves against the built site.
  const files = new Map();
  const walk = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(full);
      else
        files.set(
          path.relative(realOut, full).split(path.sep).join("/"),
          fs.readFileSync(full, "utf8"),
        );
    }
  };
  walk(realOut);
  const urls = Object.values(map.links).flatMap((e) =>
    [e.doc, e.whitebox && `architecture.html${e.whitebox}`].filter(Boolean),
  );
  files.set("reference/links-probe.html", urls.map((u) => `<a href="${u}">x</a>`).join(""));
  checkLinks(files);
  const unlinkedTarget = { ...map, links: { ...map.links, Probe: "nowhere.html" } };
  files.set("reference/links-probe.html", `<a href="${unlinkedTarget.links.Probe}">x</a>`);
  assert.throws(() => checkLinks(files), /nowhere\.html/);
});

test("the link check reads href and xlink:href inside inline SVG", () => {
  assert.deepEqual(
    extractLinks(
      "p.html",
      '<svg><a href="#a"><text>1</text></a><a xlink:href="b.html"><rect/></a></svg><a href=c>3</a>',
    ),
    ["#a", "b.html", "c"],
  );
});

test("the search index carries the page's tables but not the diagram labels", () => {
  const index = JSON.parse(fs.readFileSync(path.join(realOut, "search-index.json"), "utf8"));
  const page = index.find((p) => p.url === "reference/architecture.html");
  assert.equal(page.title, "Architecture");
  const text = page.headings.map((h) => h.body).join(" ");
  assert.ok(text.includes("Moka-based caching decorator"), "table text is indexed");
  assert.ok(text.includes("The cache is a decorator over the same port"), "motivation is indexed");
  assert.ok(!text.includes("(whitebox)"), "diagram titles are not");
  assert.ok(!stripTags(page.intro).includes("Legend"));
});
