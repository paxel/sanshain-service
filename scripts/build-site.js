#!/usr/bin/env node
// Builds the documentation site: site/**/*.html (fragments with a front-matter
// comment) -> site-dist/ (full pages, assets, search-index.json, 404.html).
// Zero dependencies; Node 22+. Usage:
//   node scripts/build-site.js [--src site] [--out site-dist] [--base /] [--serve [port]]

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import http from "node:http";
import { fileURLToPath } from "node:url";
import { escapeHtml } from "./site/html.js";
import { contractPages } from "./site/reference.js";
import { arc42Page } from "./site/arc42.js";

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(SCRIPT_DIR, "..");
const REPO_URL = "https://github.com/paxel/sanshain-service";

export const TRACKS = [
  { id: "start", label: "Start" },
  { id: "developer", label: "Developer" },
  { id: "operator", label: "Operator" },
  { id: "reference", label: "Reference" },
];

const FRONT_MATTER_KEYS = ["title", "track", "order", "summary"];

// The REST reference pages generated from the contracts (issue #54). They are
// written to the output only; the framing prose is the authored
// site/reference/api-usage.html.
export const DEFAULT_CONTRACTS = [
  {
    file: path.join(REPO_ROOT, "api.yaml"),
    url: "reference/api.html",
    title: "REST API",
    order: 3,
    summary:
      "Every Producer and Consumer endpoint of api.yaml: parameters, bodies, responses, authentication.",
  },
  {
    file: path.join(REPO_ROOT, "maintenance.yaml"),
    url: "reference/maintenance.html",
    title: "Maintenance API",
    order: 4,
    summary: "Every /admin/* route of maintenance.yaml with the permission and scope it requires.",
  },
];

// The arc42 Architecture page (issue #55), built from the whiteboxed export
// and the link map; written to the output only.
export const DEFAULT_ARC42 = {
  dir: path.join(REPO_ROOT, "docs/arc42"),
  linksFile: path.join(REPO_ROOT, "site/reference/architecture.links.json"),
  adrDir: path.join(REPO_ROOT, "docs/adr"),
  url: "reference/architecture.html",
  order: 5,
};

const FONTS_HREF =
  "https://fonts.googleapis.com/css2?family=Newsreader:opsz,wght@6..72,400;6..72,600&family=Public+Sans:wght@400;600&family=JetBrains+Mono:wght@400&display=swap";

const SUN_MARK =
  '<svg class="mark" viewBox="0 0 24 24" width="28" height="28" aria-hidden="true" focusable="false">' +
  '<circle cx="12" cy="12" r="5" fill="currentColor"/>' +
  '<g stroke="currentColor" stroke-width="2" stroke-linecap="round">' +
  '<path d="M12 1.5v3M12 19.5v3M1.5 12h3M19.5 12h3"/>' +
  '<path d="M4.6 4.6l1.8 1.8M17.6 17.6l1.8 1.8M4.6 19.4l1.8-1.8M17.6 6.4l1.8-1.8" stroke-width="1.6"/>' +
  "</g></svg>";

// ---------------------------------------------------------------------------
// Small helpers

export { escapeHtml };

const NAMED_ENTITIES = {
  lt: "<",
  gt: ">",
  quot: '"',
  apos: "'",
  nbsp: " ",
  mdash: "—",
  ndash: "–",
  hellip: "…",
  rarr: "→",
  larr: "←",
  copy: "©",
  times: "×",
  middot: "·",
  laquo: "«",
  raquo: "»",
};

/** Decodes numeric and the named entities the pages use; `&amp;` last so `&amp;lt;` stays `&lt;`. */
export function decodeEntities(text) {
  return text
    .replace(/&#x([0-9a-f]+);/gi, (_, hex) => String.fromCodePoint(parseInt(hex, 16)))
    .replace(/&#(\d+);/g, (_, dec) => String.fromCodePoint(Number(dec)))
    .replace(/&([a-z]+);/gi, (whole, name) =>
      name === "amp" ? whole : (NAMED_ENTITIES[name] ?? whole),
    )
    .replace(/&amp;/g, "&");
}

const INLINE_TAG = /^<\/?(a|code|em|strong|kbd|span|b|i|abbr|sup|sub|mark|small|var)\b[^>]*>$/i;

/** Text of an HTML fragment: inline tags vanish, block tags become a space, entities are decoded. */
export function stripTags(html) {
  const text = html.replace(/<[^>]*>/g, (tag) => (INLINE_TAG.test(tag) ? "" : " "));
  return decodeEntities(text).replace(/\s+/g, " ").trim();
}

export function slugify(text) {
  const slug = stripTags(text)
    .toLowerCase()
    .replace(/[^a-z0-9\s-]/g, "")
    .trim()
    .replace(/[\s-]+/g, "-");
  return slug || "section";
}

export function readVersion(cargoToml) {
  const match = /^version\s*=\s*"([^"]+)"/m.exec(cargoToml);
  if (!match) throw new Error("Cargo.toml: no package version found");
  return match[1];
}

function collectIds(html) {
  const ids = new Set();
  const re = /\sid\s*=\s*(?:"([^"]+)"|'([^']+)'|([^\s>"']+))/g;
  let m;
  while ((m = re.exec(html)) !== null) ids.add(m[1] ?? m[2] ?? m[3]);
  return ids;
}

// ---------------------------------------------------------------------------
// Front matter: <!-- page: title="…" track="start" order=1 summary="…" -->

export function parseFrontMatter(source, file = "page") {
  const match = /^\s*<!--\s*page:([\s\S]*?)-->/.exec(source);
  if (!match) throw new Error(`${file}: missing front matter (<!-- page: … -->)`);
  const meta = {};
  const pair = /^([a-z]+)\s*=\s*(?:"([^"]*)"|([^\s"]+))(?:\s+|$)/;
  let rest = match[1].trim();
  while (rest.length) {
    const m = pair.exec(rest);
    if (!m) {
      throw new Error(
        `${file}: cannot read front matter at "${rest.slice(0, 40)}": expected key="value" ` +
          `pairs; a bare value is a single token, values with spaces must be quoted`,
      );
    }
    if (!FRONT_MATTER_KEYS.includes(m[1])) {
      throw new Error(`${file}: unknown front-matter key "${m[1]}"`);
    }
    meta[m[1]] = m[2] !== undefined ? m[2] : m[3];
    rest = rest.slice(m[0].length);
  }
  if (!meta.title) throw new Error(`${file}: front matter has no title`);
  if (meta.track && !TRACKS.some((t) => t.id === meta.track)) {
    throw new Error(`${file}: unknown track "${meta.track}"`);
  }
  const order = meta.order === undefined ? 0 : Number(meta.order);
  if (Number.isNaN(order)) throw new Error(`${file}: order "${meta.order}" is not a number`);
  return {
    meta: { title: meta.title, track: meta.track || "", order, summary: meta.summary || "" },
    body: source.slice(match[0].length).trim(),
  };
}

// ---------------------------------------------------------------------------
// Headings: ids, copy-link anchors, table of contents, section text.
// One pass over the h2/h3 matches feeds the rendered HTML, the TOC and the
// index sections, so they can never disagree about what a heading is.

export function processHeadings(body) {
  const used = collectIds(body); // authored ids on any element are never rewritten
  const emitted = new Set();
  const headings = [];
  const matches = [];
  const re = /<(h[23])(\s[^>]*)?>([\s\S]*?)<\/\1>/g;
  let m;
  while ((m = re.exec(body)) !== null) {
    const [whole, tag, attrs = "", inner] = m;
    const text = stripTags(inner);
    const authored = /\sid\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>"']+))/.exec(attrs);
    const base = authored ? (authored[1] ?? authored[2] ?? authored[3]) : slugify(text);
    let id = base;
    let n = 1;
    // An authored id keeps its name unless an earlier heading already emitted
    // it; a generated id must not collide with any id in the page.
    while (authored ? emitted.has(id) : used.has(id)) id = `${base}-${++n}`;
    emitted.add(id);
    used.add(id);
    headings.push({ level: Number(tag[1]), id, text });
    const rest = attrs.replace(/\sid\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>"']+)/, "");
    const anchor =
      `<a class="anchor" href="#${id}" aria-label="Copy link to ${escapeHtml(text)}" ` +
      `title="Copy link to this section">#</a>`;
    matches.push({
      start: m.index,
      end: m.index + whole.length,
      html: `<${tag} id="${id}"${rest}>${inner}${anchor}</${tag}>`,
    });
  }
  let html = "";
  let cursor = 0;
  const sections = [];
  for (let i = 0; i < matches.length; i++) {
    html += body.slice(cursor, matches[i].start) + matches[i].html;
    cursor = matches[i].end;
    const next = i + 1 < matches.length ? matches[i + 1].start : body.length;
    sections.push(stripTags(body.slice(cursor, next)));
  }
  html += body.slice(cursor);
  const intro = stripTags(body.slice(0, matches.length ? matches[0].start : body.length));
  return { html, headings, intro, sections };
}

// ---------------------------------------------------------------------------
// Evidence: recorded walkthrough files (issue #57). A page writes
// `<pre data-evidence="developer/03-provide.http"></pre>` and the build fills
// the block with the file's text, escaped, from site/evidence/. A missing file
// fails the build, so a story can never show an empty block.

const EVIDENCE_BLOCK = /<pre(\s[^>]*?)?\sdata-evidence\s*=\s*"([^"]+)"([^>]*)>([\s\S]*?)<\/pre>/g;

/** Fills every evidence block of a page body from evidenceDir; throws, naming page and file, when one is missing. */
export function inlineEvidence(body, evidenceDir, pageUrl = "page") {
  return body.replace(EVIDENCE_BLOCK, (whole, before = "", name, after) => {
    if (name.split("/").some((part) => part === "" || part === "." || part === "..")) {
      throw new Error(`${pageUrl}: evidence name "${name}" must be a plain relative path`);
    }
    const file = path.join(evidenceDir, ...name.split("/"));
    if (!fs.existsSync(file) || !fs.statSync(file).isFile()) {
      throw new Error(
        `${pageUrl}: evidence file "${name}" is missing (expected ${file}); run ` +
          `scripts/record-walkthroughs.sh to record it`,
      );
    }
    const text = fs.readFileSync(file, "utf8").replace(/\s+$/, "");
    if (!text) throw new Error(`${pageUrl}: evidence file "${name}" is empty (${file})`);
    return `<pre${before} data-evidence="${name}"${after}><code>${escapeHtml(text)}</code></pre>`;
  });
}

// ---------------------------------------------------------------------------
// Pages, nav and search index

export function loadPages(srcDir) {
  const pages = [];
  const walk = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        if (entry.name !== "assets" && entry.name !== "evidence") walk(full);
      } else if (entry.name.endsWith(".html")) {
        const url = path.relative(srcDir, full).split(path.sep).join("/");
        const { meta, body } = parseFrontMatter(fs.readFileSync(full, "utf8"), url);
        pages.push({ ...meta, url, body });
      }
    }
  };
  walk(srcDir);
  return sortPages(pages);
}

/** Landing page first, then tracks in order, then page order: the search index inherits this order and uses it to break ranking ties. */
export function sortPages(pages) {
  const trackNo = (id) => (id ? TRACKS.findIndex((t) => t.id === id) : -1);
  return pages.sort(
    (a, b) =>
      trackNo(a.track) - trackNo(b.track) || a.order - b.order || a.url.localeCompare(b.url),
  );
}

export function buildNav(pages) {
  return TRACKS.map((track) => ({
    ...track,
    pages: pages
      .filter((p) => p.track === track.id)
      .sort((a, b) => a.order - b.order || a.title.localeCompare(b.title))
      .map(({ title, url, order, summary }) => ({ title, url, order, summary })),
  }));
}

/** Generated example blocks (`<pre class="example">`) are placeholders, not text worth finding. */
export const GENERATED_EXAMPLE = /<pre class="example">[\s\S]*?<\/pre>/g;
/** Inline diagrams carry box names and legend labels, which the tables below them repeat. */
export const GENERATED_DIAGRAM = /<figure class="arc42-diagram"[^>]*>[\s\S]*?<\/figure>/g;
/** Recorded evidence is an HTTP dump or a transcript: the step's prose says what it shows. */
export const GENERATED_EVIDENCE = /<pre[^>]*\sdata-evidence=[^>]*>[\s\S]*?<\/pre>/g;

export function buildIndex(pages) {
  const trackLabel = (id) => TRACKS.find((t) => t.id === id)?.label || "";
  return pages.map((page) => {
    const { headings, intro, sections } = processHeadings(
      page.body
        .replace(GENERATED_EXAMPLE, "")
        .replace(GENERATED_DIAGRAM, "")
        .replace(GENERATED_EVIDENCE, ""),
    );
    return {
      title: page.title,
      track: trackLabel(page.track),
      url: page.url,
      summary: page.summary,
      intro,
      headings: headings.map((h, i) => ({ id: h.id, text: h.text, body: sections[i] || "" })),
    };
  });
}

// ---------------------------------------------------------------------------
// Rendering

function relPrefix(url) {
  const depth = url.split("/").length - 1;
  return depth === 0 ? "" : "../".repeat(depth);
}

function renderNav(nav, currentUrl, rel) {
  const link = (p) =>
    `<li><a href="${rel}${p.url}"${p.url === currentUrl ? ' aria-current="page"' : ""}>${escapeHtml(p.title)}</a></li>`;
  const home = `<a class="nav-home" href="${rel}index.html"${currentUrl === "index.html" ? ' aria-current="page"' : ""}>Overview</a>`;
  const tracks = nav
    .map(
      (t) =>
        `<section class="nav-track" aria-label="${escapeHtml(t.label)}"><p class="eyebrow">${escapeHtml(t.label)}</p>` +
        (t.pages.length
          ? `<ul>${t.pages.map(link).join("")}</ul>`
          : `<p class="nav-empty">No chapters yet</p>`) +
        `</section>`,
    )
    .join("");
  return home + tracks;
}

function renderToc(headings) {
  if (!headings.length) return "";
  const items = headings
    .map((h) => `<li class="toc-l${h.level}"><a href="#${h.id}">${escapeHtml(h.text)}</a></li>`)
    .join("");
  return `<aside class="toc" aria-label="Table of contents">\n<p class="eyebrow">On this page</p><ol>${items}</ol>\n</aside>\n`;
}

function shell({ title, summary, rel, root, nav, currentUrl, eyebrow, main, toc, version }) {
  const versionChip = `<span class="chip" title="Sanshain Service version">v${escapeHtml(version)}</span>`;
  return `<!doctype html>
<html lang="en" data-root="${root}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(title)} — Sanshain</title>
${summary ? `<meta name="description" content="${escapeHtml(summary)}">\n` : ""}<script>try{var t=localStorage.getItem("sanshain-theme");if(t==="light"||t==="dark"){document.documentElement.setAttribute("data-theme",t)}}catch(e){}</script>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link rel="stylesheet" href="${FONTS_HREF}">
<link rel="stylesheet" href="${rel}assets/site.css">
<link rel="icon" href="${rel}assets/sun.svg" type="image/svg+xml">
</head>
<body>
<a class="skip" href="#content">Skip to content</a>
<div class="sr-only" id="live" aria-live="polite" aria-atomic="true"></div>
<header class="site-header">
<button class="nav-toggle" type="button" aria-controls="sidebar" aria-expanded="false" aria-label="Open navigation"><span class="nav-toggle-bars" aria-hidden="true"></span></button>
<a class="brand" href="${rel}index.html">${SUN_MARK}<span class="brand-name">Sanshain</span></a>
${versionChip}
<div class="search" role="search">
<input class="search-input" id="search" type="search" placeholder="Search docs" aria-label="Search documentation" role="combobox" aria-autocomplete="list" aria-controls="search-results" aria-expanded="false" autocomplete="off" spellcheck="false">
<kbd class="search-kbd" aria-hidden="true">/</kbd>
<ul class="search-results" id="search-results" role="listbox" aria-label="Search results" hidden></ul>
</div>
<button class="theme-toggle" type="button" aria-label="Switch to dark theme" title="Switch theme">
<svg class="icon-sun" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><circle cx="12" cy="12" r="4" fill="currentColor"/><g stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M12 2v2M12 20v2M2 12h2M20 12h2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/></g></svg>
<svg class="icon-moon" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true"><path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z" fill="currentColor"/></svg>
</button>
</header>
<div class="layout">
<nav class="sidebar" id="sidebar" aria-label="Documentation">
${renderNav(nav, currentUrl, rel)}
</nav>
<main class="content" id="content" tabindex="-1">
${eyebrow ? `<p class="eyebrow track-eyebrow">${escapeHtml(eyebrow)}</p>\n` : ""}${main}
</main>
${toc}</div>
<footer class="site-footer">
<span>Sanshain Service ${versionChip}</span>
<nav aria-label="Project links"><a href="${REPO_URL}">GitHub</a><a href="${REPO_URL}/blob/summer/CHANGELOG.md">Changelog</a></nav>
</footer>
<script src="${rel}assets/site.js" defer></script>
</body>
</html>
`;
}

export function renderPage(page, { nav, version }) {
  const rel = relPrefix(page.url);
  const { html, headings } = processHeadings(page.body);
  const body = html.replace(
    /<table(\s[^>]*)?>[\s\S]*?<\/table>/g,
    (t) => `<div class="table-wrap">${t}</div>`,
  );
  const track = TRACKS.find((t) => t.id === page.track);
  const main = `<h1>${escapeHtml(page.title)}</h1>\n${body}`;
  return shell({
    title: page.title,
    summary: page.summary,
    rel,
    root: rel || "./",
    nav,
    currentUrl: page.url,
    eyebrow: track ? track.label : "",
    main,
    toc: page.toc !== undefined ? page.toc : renderToc(headings),
    version,
  });
}

export function render404({ nav, version, base = "/" }) {
  const main = `<h1>Page not found</h1>
<p class="lede">There is nothing at this address. The chapter may have moved, or the link was older than the site.</p>
<p>Try the search box above (press <kbd>/</kbd>) or start from the <a href="${base}index.html">overview</a>.</p>`;
  return shell({
    title: "Page not found",
    summary: "",
    rel: base,
    root: base,
    nav,
    currentUrl: "",
    eyebrow: "404",
    main,
    toc: "",
    version,
  });
}

// ---------------------------------------------------------------------------
// Link check

const ATTR_LINK = /\s(?:xlink:)?(?:href|src)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>"']+))/g;
const SRCSET = /\ssrcset\s*=\s*(?:"([^"]*)"|'([^']*)')/g;
const CSS_URL = /url\(\s*(?:"([^"]*)"|'([^']*)'|([^)"']*))\s*\)/g;

/** Every link (href or xlink:href, also inside inline SVG, src, srcset candidates, CSS url()) in one output file. */
export function extractLinks(file, content) {
  const links = [];
  if (file.endsWith(".css")) {
    for (const m of content.matchAll(CSS_URL)) links.push((m[1] ?? m[2] ?? m[3]).trim());
    return links;
  }
  if (!file.endsWith(".html")) return links;
  for (const m of content.matchAll(ATTR_LINK)) links.push(m[1] ?? m[2] ?? m[3]);
  for (const m of content.matchAll(SRCSET)) {
    for (const candidate of (m[1] ?? m[2]).split(",")) {
      const url = candidate.trim().split(/\s+/)[0];
      if (url) links.push(url);
    }
  }
  return links;
}

const EXTERNAL = /^(https?:|mailto:|data:|tel:)/i;

/**
 * files: Map<relativePath, content> of every output file. HTML and CSS files
 * are scanned; every file is a valid target. Root-relative links must start
 * with the base. Throws on the first broken link or anchor, naming the file
 * and the link.
 */
export function checkLinks(files, { base = "/" } = {}) {
  const ids = new Map();
  for (const [file, html] of files) if (file.endsWith(".html")) ids.set(file, collectIds(html));
  const asIndex = (target) => `${target.replace(/\/$/, "")}/index.html`;
  const exists = (target) => files.has(target) || files.has(asIndex(target));
  for (const [file, content] of files) {
    for (const link of extractLinks(file, content)) {
      if (EXTERNAL.test(link)) continue;
      const hashAt = link.indexOf("#");
      const fragment = hashAt >= 0 ? link.slice(hashAt + 1) : "";
      let target = (hashAt >= 0 ? link.slice(0, hashAt) : link).replace(/\?.*$/, "");
      if (target.startsWith("/")) {
        if (!target.startsWith(base)) {
          throw new Error(`Broken link in ${file}: "${link}" is outside the site base ${base}`);
        }
        target = target.slice(base.length) || "index.html";
      } else if (target) {
        target = path.posix.normalize(path.posix.join(path.posix.dirname(file), target));
        if (target === "." || target === "./") target = "index.html";
      }
      let resolved = target || file;
      if (target) {
        if (!exists(resolved)) {
          throw new Error(
            `Broken link in ${file}: "${link}" does not resolve to a file in the output`,
          );
        }
        if (!files.has(resolved)) resolved = asIndex(resolved);
      }
      if (fragment && !ids.get(resolved)?.has(fragment)) {
        throw new Error(
          `Broken link in ${file}: "${link}" points to a missing anchor #${fragment}`,
        );
      }
    }
  }
}

// Authored pages must not use root-relative links: on GitHub Pages the site
// lives under /<repo>/, so "/start/x.html" breaks there however it looks locally.
function rejectRootRelativeLinks(page) {
  for (const link of extractLinks(page.url, page.body)) {
    if (link.startsWith("/")) {
      throw new Error(
        `Broken link in ${page.url}: "${link}" is root-relative; the site is served under a ` +
          `path on GitHub Pages, use a relative link`,
      );
    }
  }
}

// ---------------------------------------------------------------------------
// Build

function copyDir(from, to, files) {
  fs.mkdirSync(to, { recursive: true });
  for (const entry of fs.readdirSync(from, { withFileTypes: true })) {
    const src = path.join(from, entry.name);
    const dst = path.join(to, entry.name);
    if (entry.isDirectory()) copyDir(src, dst, files);
    else {
      fs.copyFileSync(src, dst);
      files.push(dst);
    }
  }
}

const isInside = (dir, parent) => {
  const rel = path.relative(parent, dir);
  return rel !== "" && !rel.startsWith("..") && !path.isAbsolute(rel);
};

/** The output folder is wiped; refuse anything that is not clearly a build output. */
function guardOutDir(outDir, srcDir) {
  const tmp = fs.realpathSync(os.tmpdir());
  const out = path.resolve(outDir);
  if (!isInside(out, REPO_ROOT) && !isInside(out, tmp)) {
    throw new Error(
      `Refusing to wipe ${out}: the output folder must be inside the repository or the temp dir`,
    );
  }
  const src = path.resolve(srcDir);
  if (out === src || isInside(src, out)) {
    throw new Error(`Refusing to wipe ${out}: it contains the source folder ${src}`);
  }
}

export function buildSite({
  srcDir = path.join(REPO_ROOT, "site"),
  outDir = path.join(REPO_ROOT, "site-dist"),
  version = readVersion(fs.readFileSync(path.join(REPO_ROOT, "Cargo.toml"), "utf8")),
  base = "/",
  contracts = DEFAULT_CONTRACTS,
  arc42 = DEFAULT_ARC42,
  evidenceDir = path.join(srcDir, "evidence"),
} = {}) {
  if (!base.startsWith("/") || !base.endsWith("/")) {
    throw new Error(`--base must start and end with "/": got "${base}"`);
  }
  guardOutDir(outDir, srcDir);
  const pages = loadPages(srcDir);
  if (!pages.some((p) => p.url === "index.html")) throw new Error(`${srcDir}: no index.html`);
  for (const page of pages) page.body = inlineEvidence(page.body, evidenceDir, page.url);
  for (const generated of contractPages(contracts, {
    repoUrl: REPO_URL,
    usageUrl: "api-usage.html",
  })) {
    if (pages.some((p) => p.url === generated.url)) {
      throw new Error(`${generated.url}: an authored page shadows the generated reference page`);
    }
    pages.push(generated);
  }
  if (arc42) {
    const page = arc42Page({ ...arc42, repoUrl: REPO_URL });
    if (pages.some((p) => p.url === page.url)) {
      throw new Error(`${page.url}: an authored page shadows the generated architecture page`);
    }
    pages.push(page);
  }
  sortPages(pages);
  for (const page of pages) rejectRootRelativeLinks(page);
  const nav = buildNav(pages);
  const ctx = { nav, version, base };

  fs.rmSync(outDir, { recursive: true, force: true });
  fs.mkdirSync(outDir, { recursive: true });

  const files = new Map();
  for (const page of pages) files.set(page.url, renderPage(page, ctx));
  files.set("404.html", render404(ctx));

  const copied = [];
  const assets = path.join(srcDir, "assets");
  if (fs.existsSync(assets)) copyDir(assets, path.join(outDir, "assets"), copied);
  for (const file of copied) {
    const rel = path.relative(outDir, file).split(path.sep).join("/");
    if (!files.has(rel)) files.set(rel, fs.readFileSync(file, "utf8"));
  }
  // The screenshots of the recorded walkthroughs are linked as images; the
  // text files are inlined above and copied too, so a reader can open them raw.
  const evidenceOut = [];
  if (fs.existsSync(evidenceDir)) copyDir(evidenceDir, path.join(outDir, "evidence"), evidenceOut);
  for (const file of evidenceOut) {
    const rel = path.relative(outDir, file).split(path.sep).join("/");
    if (!files.has(rel)) files.set(rel, "");
  }

  checkLinks(files, { base });

  for (const [rel, html] of files) {
    if (!rel.endsWith(".html")) continue;
    const dest = path.join(outDir, rel);
    fs.mkdirSync(path.dirname(dest), { recursive: true });
    fs.writeFileSync(dest, html);
  }
  fs.writeFileSync(
    path.join(outDir, "search-index.json"),
    JSON.stringify(buildIndex(pages), null, 0),
  );
  fs.writeFileSync(path.join(outDir, ".nojekyll"), "");
  return { pages: pages.length, outDir, version };
}

// ---------------------------------------------------------------------------
// Preview server

const MIME = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".webp": "image/webp",
  ".ico": "image/x-icon",
  ".txt": "text/plain; charset=utf-8",
};

/** Resolves a request path to a file under root, or null when it escapes or is missing. */
export function resolveRequest(root, urlPath) {
  let decoded;
  try {
    decoded = decodeURIComponent(urlPath);
  } catch {
    return null;
  }
  let file = path.resolve(root, `.${decoded}`);
  const rel = path.relative(root, file);
  if (rel.startsWith("..") || path.isAbsolute(rel)) return null;
  if (fs.existsSync(file) && fs.statSync(file).isDirectory()) file = path.join(file, "index.html");
  return fs.existsSync(file) ? file : null;
}

export function serve(outDir, port, host = "127.0.0.1") {
  const root = path.resolve(outDir);
  const server = http.createServer((req, res) => {
    const urlPath = new URL(req.url, "http://localhost").pathname;
    let file = resolveRequest(root, urlPath);
    let status = 200;
    if (!file) {
      file = path.join(root, "404.html");
      status = 404;
    }
    res.writeHead(status, {
      "content-type": MIME[path.extname(file)] || "application/octet-stream",
    });
    fs.createReadStream(file).pipe(res);
  });
  server.listen(port, host, () => {
    const { port: bound } = server.address();
    console.log(`Serving ${root} at http://${host}:${bound}/ (Ctrl+C to stop)`);
  });
  return server;
}

// ---------------------------------------------------------------------------
// CLI

function parseArgs(argv) {
  const opts = { serve: false, port: 8080 };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--serve") {
      opts.serve = true;
      if (/^\d+$/.test(argv[i + 1] || "")) opts.port = Number(argv[++i]);
    } else if (arg === "--src") opts.srcDir = path.resolve(argv[++i]);
    else if (arg === "--out") opts.outDir = path.resolve(argv[++i]);
    else if (arg === "--base") opts.base = argv[++i];
    else throw new Error(`Unknown argument: ${arg}`);
  }
  return opts;
}

const isMain = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  try {
    const opts = parseArgs(process.argv.slice(2));
    const result = buildSite(opts);
    console.log(`Built ${result.pages} pages (v${result.version}) into ${result.outDir}`);
    if (opts.serve) serve(result.outDir, opts.port);
  } catch (err) {
    console.error(`build-site: ${err.message}`);
    process.exit(1);
  }
}
