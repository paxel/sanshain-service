// The arc42 building-block view (issue #55): one page built from the
// whiteboxed export in docs/arc42. Each of the five diagrams (the context and
// four whiteboxes) is inlined as SVG with its boxes wrapped in links, followed
// by the motivation text and the partner, building-block and interface tables
// of the Markdown file next to it. The Markdown tables are the one place the
// site build reads Markdown; everything else in those files is found by
// structure (the image line, the paragraph after it, the `####` table titles).

import fs from "node:fs";
import path from "node:path";
import { escapeHtml } from "./html.js";

/** @typedef {import("../build-site.js").Page} Page */

/** @typedef {{ stem: string, name: string, path: string[] }} Diagram */
/** @typedef {{ title: string, header: string[], rows: string[][] }} Table */
/** @typedef {{ doc: string, whitebox?: string | undefined }} LinkEntry */
/** @typedef {{ links: Record<string, LinkEntry>, unlinked: string[] }} LinkMap */
/** @typedef {{ tag: string, open: string, start: number, end: number, text: string }} SvgNode */
/** @typedef {[number, number, number, number]} Box */

/**
 * The diagrams in reading order: file stem, box name and the level path above it.
 * @type {Diagram[]}
 */
export const DIAGRAMS = [
  { stem: "context", name: "Context", path: [] },
  { stem: "context - Sanshain Service", name: "Sanshain Service", path: ["Context"] },
  {
    stem: "context - Sanshain Service - HTTP API",
    name: "HTTP API",
    path: ["Context", "Sanshain Service"],
  },
  {
    stem: "context - Sanshain Service - Application Services",
    name: "Application Services",
    path: ["Context", "Sanshain Service"],
  },
  {
    stem: "context - Sanshain Service - Persistence",
    name: "Persistence",
    path: ["Context", "Sanshain Service"],
  },
];

/**
 * @param {string} name
 * @returns {string}
 */
export function sectionId(name) {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9\s-]/g, "")
    .trim()
    .replace(/[\s-]+/g, "-");
}

// ---------------------------------------------------------------------------
// Markdown export: title, image, motivation, pipe tables

/**
 * Splits a table row on the pipes outside backticks; `\|` is a literal pipe.
 * @param {string} line
 * @returns {string[]}
 */
export function splitRow(line) {
  /** @type {string[]} */
  const cells = [];
  let cell = "";
  let inCode = false;
  const trimmed = line.trim().replace(/^\|/, "").replace(/\|$/, "");
  for (let i = 0; i < trimmed.length; i++) {
    const ch = /** @type {string} */ (trimmed[i]);
    if (ch === "\\" && trimmed[i + 1] === "|") {
      cell += "|";
      i++;
    } else if (ch === "`") {
      inCode = !inCode;
      cell += ch;
    } else if (ch === "|" && !inCode) {
      cells.push(cell.trim());
      cell = "";
    } else cell += ch;
  }
  cells.push(cell.trim());
  return cells;
}

const SEPARATOR = /^\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?$/;

/**
 * Reads one exported Markdown file: `{ title, image, motivation, tables }`
 * where each table is `{ title, header, rows }` of plain cell strings. The
 * motivation is the first paragraph after the image line; a `####` heading
 * other than Motivation introduces the table that follows it. Throws, naming
 * the file and line, when a table does not parse.
 * @param {string} source
 * @param {string} [file]
 * @returns {{ title: string, image: string, motivation: string, tables: Table[] }}
 */
export function readExport(source, file = "export") {
  const lines = source.split(/\r?\n/);
  /** @type {(lineNo: number, why: string) => never} */
  const fail = (lineNo, why) => {
    throw new Error(`${file}:${String(lineNo + 1)}: ${why}`);
  };
  let title = "";
  let image = "";
  let imageAt = -1;
  for (let i = 0; i < lines.length; i++) {
    const line = /** @type {string} */ (lines[i]);
    const heading = /^###\s+(\S.*)$/.exec(line.trimEnd());
    if (heading && !title) title = /** @type {string} */ (heading[1]);
    const img = /^!\[[^\]]*\]\(<?([^>)]+?)>?\)\s*$/.exec(line);
    if (img && !image) {
      image = /** @type {string} */ (img[1]);
      imageAt = i;
    }
  }
  if (!title) fail(0, "no ### title");
  if (!image) fail(0, "no image line (![…](<file.svg>))");

  /** @type {string[]} */
  const paragraph = [];
  for (let i = imageAt + 1; i < lines.length; i++) {
    const line = /** @type {string} */ (lines[i]).trim();
    if (!line || line.startsWith("#")) {
      if (paragraph.length) break;
      continue;
    }
    if (line.startsWith("|")) fail(i, "a table where the motivation paragraph was expected");
    paragraph.push(line);
  }
  const motivation = paragraph.join(" ");

  /** @type {Table[]} */
  const tables = [];
  // The first line after the last table read; the lines before it are that table's.
  let next = 0;
  for (let i = imageAt + 1; i < lines.length; i++) {
    if (i < next) continue;
    const heading = /^####\s+(\S.*)$/.exec(/** @type {string} */ (lines[i]).trimEnd());
    if (!heading || heading[1] === "Motivation") continue;
    const tableTitle = /** @type {string} */ (heading[1]);
    let at = i + 1;
    while (at < lines.length && /** @type {string} */ (lines[at]).trim() === "") at++;
    if (at >= lines.length || !(/** @type {string} */ (lines[at]).trim().startsWith("|"))) {
      fail(i, `"${tableTitle}" is not followed by a table`);
    }
    const header = splitRow(/** @type {string} */ (lines[at]));
    if (at + 1 >= lines.length || !SEPARATOR.test(/** @type {string} */ (lines[at + 1]).trim())) {
      fail(at + 1, `table "${tableTitle}": the header is not followed by a |---| separator`);
    }
    if (splitRow(/** @type {string} */ (lines[at + 1])).length !== header.length) {
      fail(at + 1, `table "${tableTitle}": separator and header column counts differ`);
    }
    /** @type {string[][]} */
    const rows = [];
    for (
      at += 2;
      at < lines.length && /** @type {string} */ (lines[at]).trim().startsWith("|");
      at++
    ) {
      const cells = splitRow(/** @type {string} */ (lines[at]));
      if (cells.length !== header.length) {
        fail(
          at,
          `table "${tableTitle}": row has ${String(cells.length)} cells, the header has ${String(header.length)}`,
        );
      }
      rows.push(cells);
    }
    tables.push({ title: tableTitle, header, rows });
    next = at;
  }
  return { title, image, motivation, tables };
}

/**
 * A cell's text: backticks become <code>, everything else is escaped.
 * @param {string} text
 * @returns {string}
 */
export function cellHtml(text) {
  return text
    .split(/(`[^`]*`)/)
    .map((part, i) => (i % 2 ? `<code>${escapeHtml(part.slice(1, -1))}</code>` : escapeHtml(part)))
    .join("");
}

// ---------------------------------------------------------------------------
// ADR references: `ADR-0003`, `ADR-0004/0005` → links to the files in docs/adr

/**
 * `{ "0003": "0003-versions-replace-branches.md", … }` from the ADR folder.
 * @param {string} adrDir
 * @returns {Record<string, string>}
 */
export function adrIndex(adrDir) {
  /** @type {Record<string, string>} */
  const index = {};
  for (const name of fs.readdirSync(adrDir)) {
    const m = /^(\d{4})-.*\.md$/.exec(name);
    if (m) index[/** @type {string} */ (m[1])] = name;
  }
  return index;
}

/**
 * Wraps every ADR reference in already-escaped HTML in a link; an unknown number fails.
 * @param {string} html
 * @param {Record<string, string>} adrs
 * @param {string} adrBaseUrl
 * @returns {string}
 */
export function linkAdrs(html, adrs, adrBaseUrl) {
  /** @type {(number: string, label: string) => string} */
  const link = (number, label) => {
    const file = adrs[number];
    if (!file) throw new Error(`ADR-${number} is referenced but docs/adr has no such file`);
    return `<a href="${adrBaseUrl}/${file}">${label}</a>`;
  };
  /** @type {(whole: string, first: string, more: string) => string} */
  const linked = (_, first, more) =>
    [link(first, `ADR-${first}`)]
      .concat(
        more
          ? more
              .slice(1)
              .split("/")
              .map((n) => link(n, n))
          : [],
      )
      .join("/");
  return html.replace(/ADR-(\d{4})((?:\/\d{4})*)/g, linked);
}

// ---------------------------------------------------------------------------
// Link map: site/reference/architecture.links.json

/**
 * `{ links: { name: url | { doc, whitebox } }, unlinked: [name] }`. The SVG
 * box links to `whitebox` when there is one (the section of the level below),
 * table cells always to `doc`.
 * @param {string} source
 * @param {string} [file]
 * @returns {LinkMap}
 */
export function readLinkMap(source, file = "architecture.links.json") {
  /**
   * The file is read as what the checks below ask of it; anything else fails them.
   * @typedef {string | number | boolean | null | { doc?: unknown, whitebox?: unknown }} RawEntry
   * @type {{ links?: Record<string, RawEntry> | null, unlinked?: unknown[] | string | number | boolean | null } | string | number | boolean | null}
   */
  let data;
  try {
    data = /** @type {typeof data} */ (JSON.parse(source));
  } catch (err) {
    const why = err instanceof Error ? err.message : String(err);
    throw new Error(`${file}: ${why}`, { cause: err });
  }
  if (!data || typeof data !== "object" || !data.links || typeof data.links !== "object") {
    throw new Error(`${file}: expected an object with a "links" object`);
  }
  const unlinked = data.unlinked ?? [];
  if (!Array.isArray(unlinked) || unlinked.some((n) => typeof n !== "string")) {
    throw new Error(`${file}: "unlinked" must be an array of box names`);
  }
  /** @type {Record<string, LinkEntry>} */
  const links = {};
  for (const [name, entry] of Object.entries(data.links)) {
    if (unlinked.includes(name)) throw new Error(`${file}: "${name}" is both linked and unlinked`);
    if (typeof entry === "string" && entry) links[name] = { doc: entry };
    else if (entry && typeof entry === "object" && typeof entry.doc === "string" && entry.doc) {
      if (entry.whitebox !== undefined && typeof entry.whitebox !== "string") {
        throw new Error(`${file}: "${name}": "whitebox" must be a string`);
      }
      links[name] = { doc: entry.doc, whitebox: entry.whitebox };
    } else throw new Error(`${file}: "${name}" must map to a URL or to { doc, whitebox }`);
  }
  return { links, unlinked: /** @type {string[]} */ (unlinked) };
}

/**
 * The target of a box in the diagram: the level below first, else its documentation.
 * @param {LinkMap} map
 * @param {string} name
 * @param {string} file
 * @returns {string | null}
 */
function boxHref(map, name, file) {
  if (map.unlinked.includes(name)) return null;
  const entry = map.links[name];
  if (!entry) {
    throw new Error(
      `${file}: box "${name}" has no entry in the link map; add it to "links" or "unlinked"`,
    );
  }
  return (entry.whitebox ?? "") || entry.doc;
}

/**
 * A table cell naming a box ("Webhooks", or "HTTP API › Admin Endpoints") links to its documentation.
 * @param {LinkMap} map
 * @param {string} text
 * @returns {string}
 */
function linkCell(map, text) {
  const sep = text.indexOf(" › ");
  const name = sep >= 0 ? text.slice(0, sep) : text;
  const entry = map.links[name];
  if (!entry) return cellHtml(text);
  const rest = sep >= 0 ? cellHtml(text.slice(sep)) : "";
  return `<a href="${escapeHtml(entry.doc)}">${cellHtml(name)}</a>${rest}`;
}

// ---------------------------------------------------------------------------
// SVG: top-level elements, geometry, wrapping boxes in <a>

const TAG = /<(\/?)([a-zA-Z][\w:-]*)((?:\s(?:"[^"]*"|'[^']*'|[^>"'/]|\/(?!>))*)?)(\/?)>/g;

/** @type {Record<string, string>} */
const XML_ENTITIES = { lt: "<", gt: ">", quot: '"', apos: "'" };

/**
 * @param {string} text
 * @returns {string}
 */
function decodeXml(text) {
  return text
    .replace(/&#x([0-9a-f]+);/gi, (_, /** @type {string} */ hex) =>
      String.fromCodePoint(Number.parseInt(hex, 16)),
    )
    .replace(/&#(\d+);/g, (_, /** @type {string} */ dec) => String.fromCodePoint(Number(dec)))
    .replace(
      /&(lt|gt|quot|apos);/g,
      (_, /** @type {string} */ n) => /** @type {string} */ (XML_ENTITIES[n]),
    )
    .replace(/&amp;/g, "&");
}

/**
 * @param {string} open
 * @param {string} name
 * @returns {string | undefined}
 */
function attr(open, name) {
  const m = new RegExp(`\\s${name}\\s*=\\s*"([^"]*)"`).exec(open);
  return m ? m[1] : undefined;
}

/** @type {(open: string, name: string) => number} */
const num = (open, name) => Number(attr(open, name) ?? Number.NaN);

/**
 * The direct children of the root element, each `{ tag, open, start, end, text }` (text: inner content of <text>).
 * @param {string} inner
 * @returns {SvgNode[]}
 */
export function topLevelNodes(inner) {
  /** @type {SvgNode[]} */
  const nodes = [];
  TAG.lastIndex = 0;
  let depth = 0;
  /** @type {SvgNode | null} */
  let current = null;
  let m;
  while ((m = TAG.exec(inner)) !== null) {
    const [whole, closing, name, , selfClosing] = m;
    const tag = /** @type {string} */ (name);
    if (closing) {
      depth--;
      if (depth === 0 && current) {
        current.end = m.index + whole.length;
        if (current.tag === "text")
          current.text = inner.slice(current.open.length + current.start, m.index);
        nodes.push(current);
        current = null;
      }
      continue;
    }
    if (depth === 0) current = { tag, open: whole, start: m.index, end: 0, text: "" };
    if (selfClosing) {
      if (depth === 0) {
        const node = /** @type {SvgNode} */ (current);
        node.end = m.index + whole.length;
        nodes.push(node);
        current = null;
      }
    } else depth++;
  }
  if (depth !== 0) throw new Error("SVG: unbalanced elements");
  return nodes;
}

/**
 * `[x1, y1, x2, y2]` of a node, or null when its shape is not read (paths, groups).
 * @param {Pick<SvgNode, "tag" | "open">} node
 * @returns {Box | null}
 */
export function bbox(node) {
  const o = node.open;
  switch (node.tag) {
    case "rect":
    case "svg": {
      const [x, y, w, h] = /** @type {Box} */ (["x", "y", "width", "height"].map((n) => num(o, n)));
      return [x, y, w, h].some(Number.isNaN) ? null : [x, y, x + w, y + h];
    }
    case "circle": {
      const [cx, cy, r] = /** @type {[number, number, number]} */ (
        ["cx", "cy", "r"].map((n) => num(o, n))
      );
      return [cx, cy, r].some(Number.isNaN) ? null : [cx - r, cy - r, cx + r, cy + r];
    }
    case "ellipse": {
      const [cx, cy, rx, ry] = /** @type {Box} */ (["cx", "cy", "rx", "ry"].map((n) => num(o, n)));
      return [cx, cy, rx, ry].some(Number.isNaN) ? null : [cx - rx, cy - ry, cx + rx, cy + ry];
    }
    case "polygon":
    case "polyline": {
      const points = (attr(o, "points") ?? "")
        .trim()
        .split(/[\s,]+/)
        .map(Number);
      if (points.length < 4 || points.some(Number.isNaN)) return null;
      const xs = points.filter((_, i) => i % 2 === 0);
      const ys = points.filter((_, i) => i % 2 === 1);
      return [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)];
    }
    case "text": {
      const [x, y] = /** @type {[number, number]} */ (["x", "y"].map((n) => num(o, n)));
      return [x, y].some(Number.isNaN) ? null : [x, y, x, y];
    }
    default:
      return null;
  }
}

const TOLERANCE = 0.5;

/**
 * @param {Box} outer
 * @param {Box} inner
 * @returns {boolean}
 */
function within(outer, inner) {
  return (
    inner[0] >= outer[0] - TOLERANCE &&
    inner[1] >= outer[1] - TOLERANCE &&
    inner[2] <= outer[2] + TOLERANCE &&
    inner[3] <= outer[3] + TOLERANCE
  );
}

/**
 * Wraps every box of a whiteboxed SVG in `<a href>`. A box is a text element
 * followed by its «kind» text. The export draws each box as a run of top-level
 * elements (shape, icon, name, kind, glyph), so the link covers the run from
 * the box's shape to the last element inside it: searching backwards from the
 * name, the nearest shape containing the name's anchor point, widened to any
 * directly preceding shape that contains the point too (a cache draws two
 * nested rects). A linked box without such a shape fails the build.
 * `resolve(name)` returns the href, or null for a box left unlinked.
 * Returns `{ svg, boxes: [{ name, href }] }`.
 * @param {string} svgInner
 * @param {(name: string) => string | null} resolve
 * @param {string} [file]
 * @returns {{ svg: string, boxes: { name: string, href: string | null }[] }}
 */
export function linkBoxes(svgInner, resolve, file = "diagram.svg") {
  const nodes = topLevelNodes(svgInner);
  /** @type {{ name: string, href: string | null }[]} */
  const boxes = [];
  /** @type {{ from: number, to: number, href: string, name: string }[]} */
  const wraps = [];
  let lastEnd = -1;
  /** @type {(k: number, point: Box | null) => boolean} */
  const contains = (k, point) => {
    const node = /** @type {SvgNode} */ (nodes[k]);
    const b = node.tag === "text" ? null : bbox(node);
    return Boolean(b && point && within(b, point));
  };
  for (let i = 0; i + 1 < nodes.length; i++) {
    // Nodes up to lastEnd belong to the box just wrapped.
    if (i <= lastEnd) continue;
    const label = /** @type {SvgNode} */ (nodes[i]);
    const kind = /** @type {SvgNode} */ (nodes[i + 1]);
    if (label.tag !== "text" || kind.tag !== "text") continue;
    if (!decodeXml(kind.text).trim().startsWith("«")) continue;
    const name = decodeXml(label.text).trim();
    const href = resolve(name);
    if (href === null) {
      boxes.push({ name, href: null });
      continue;
    }
    const anchor = bbox(label);
    let start = i - 1;
    while (start > lastEnd && !contains(start, anchor)) start--;
    if (start <= lastEnd) {
      throw new Error(`${file}: no shape contains the name of box "${name}"; cannot link it`);
    }
    while (start - 1 > lastEnd && contains(start - 1, anchor)) start--;
    const shape = /** @type {Box} */ (bbox(/** @type {SvgNode} */ (nodes[start])));
    let end = i + 1;
    while (end + 1 < nodes.length) {
      const b = bbox(/** @type {SvgNode} */ (nodes[end + 1]));
      if (!b || !within(shape, b)) break;
      end++;
    }
    wraps.push({
      from: /** @type {SvgNode} */ (nodes[start]).start,
      to: /** @type {SvgNode} */ (nodes[end]).end,
      href,
      name,
    });
    boxes.push({ name, href });
    lastEnd = end;
  }
  let out = "";
  let cursor = 0;
  for (const w of wraps) {
    out +=
      svgInner.slice(cursor, w.from) +
      `<a href="${escapeHtml(w.href)}" class="arc42-box" aria-label="${escapeHtml(w.name)}">` +
      svgInner.slice(w.from, w.to) +
      `</a>`;
    cursor = w.to;
  }
  return { svg: out + svgInner.slice(cursor), boxes };
}

/**
 * Prepares an exported SVG for inlining: the XML prolog goes, the root keeps
 * its viewBox but loses width/height in favour of the `arc42-svg` class, and
 * the full-size white background rect goes so the page's diagram surface shows.
 * Returns `{ open, inner, viewBox, width }` (width: the viewBox width in px).
 * @param {string} source
 * @param {string} [file]
 * @returns {{ open: string, inner: string, viewBox: string, width: number }}
 */
export function prepareSvg(source, file = "diagram.svg") {
  const text = source.replace(/^\s*<\?xml[^>]*\?>\s*/, "").replace(/<!DOCTYPE[^>]*>\s*/i, "");
  const root = /^\s*<svg(\s[^>]*)?>/.exec(text);
  const close = text.lastIndexOf("</svg>");
  if (!root || close < 0) throw new Error(`${file}: not an SVG document`);
  const viewBox = attr(root[0], "viewBox");
  if (!viewBox) throw new Error(`${file}: the root <svg> has no viewBox`);
  const attrs = (root[1] ?? "").replace(/\s(?:width|height|class)\s*=\s*"[^"]*"/g, "").trimEnd();
  let inner = text.slice(root[0].length, close);
  const [vx, vy, vw, vh] = /** @type {Box} */ (
    viewBox
      .trim()
      .split(/[\s,]+/)
      .map(Number)
  );
  inner = inner.replace(/^\s*<rect\s[^>]*\/>/, (rect) => {
    const [x, y, w, h] = /** @type {Box} */ (
      ["x", "y", "width", "height"].map((n) => num(rect, n))
    );
    /** @type {(a: number, b: number) => boolean} */
    const same = (a, b) => Math.abs(a - b) < 0.05;
    return same(x, vx) && same(y, vy) && same(w, vw) && same(h, vh) ? "" : rect;
  });
  return { open: `<svg class="arc42-svg"${attrs}>`, inner: inner.trim(), viewBox, width: vw };
}

// ---------------------------------------------------------------------------
// The page

/** Minimum rendered width of a diagram as a fraction of its export width. */
export const DIAGRAM_SCALE = 0.8;

/** @type {[string, string][]} */
const LINE_KINDS = [
  [
    "business",
    "Solid, the strongest line, with a plain arrowhead: the catalog flows. Provide, require, reports, catalog pushes and the queries that answer them.",
  ],
  [
    "admin",
    "Thin and solid with a key as its head: operation and login. Administration, authentication, settings, the backup download and the directory lookups behind a login.",
  ],
  [
    "telemetry",
    "Dotted with a gauge as its head: traces and metrics. They leave every request through middleware, so no box calls Telemetry on purpose.",
  ],
  [
    "async-outbound",
    "Dashed with an envelope as its head: pushes that leave the request. Webhook deliveries from the outbox and the live updates streamed to the dashboard.",
  ],
];

/**
 * @param {Diagram} diagram
 * @returns {string}
 */
function breadcrumb(diagram) {
  const parts = diagram.path.map((name) => `<a href="#${sectionId(name)}">${escapeHtml(name)}</a>`);
  parts.push(`<span aria-current="true">${escapeHtml(diagram.name)}</span>`);
  return `<nav class="breadcrumb" aria-label="Level">${parts.join(" › ")}</nav>`;
}

/**
 * @param {Table} table
 * @param {string} id
 * @param {LinkMap} map
 * @param {(html: string) => string} prose
 * @returns {string}
 */
function renderTable(table, id, map, prose) {
  const head = table.header.map((h) => `<th>${escapeHtml(h)}</th>`).join("");
  const rows = table.rows
    .map((cells) => `<tr>${cells.map((c) => `<td>${prose(linkCell(map, c))}</td>`).join("")}</tr>`)
    .join("\n");
  return (
    `<h3 id="${id}">${escapeHtml(table.title)}</h3>\n` +
    `<table><thead><tr>${head}</tr></thead><tbody>\n${rows}\n</tbody></table>`
  );
}

function renderToc() {
  const items = [`<li class="toc-l2"><a href="#reading-the-diagrams">Reading the diagrams</a></li>`]
    .concat(
      DIAGRAMS.map(
        (d) => `<li class="toc-l2"><a href="#${sectionId(d.name)}">${escapeHtml(d.name)}</a></li>`,
      ),
    )
    .join("");
  return `<aside class="toc" aria-label="Table of contents">\n<p class="eyebrow">On this page</p><ol>${items}</ol>\n</aside>\n`;
}

/**
 * @param {string} file
 * @param {string} what
 * @returns {string}
 */
function readFile(file, what) {
  try {
    return fs.readFileSync(file, "utf8");
  } catch (err) {
    const why = err instanceof Error ? err.message : String(err);
    throw new Error(`${what} ${path.basename(file)}: cannot read: ${why}`, { cause: err });
  }
}

/**
 * Builds the Architecture page from `dir` (the whiteboxed export), the link
 * map and the ADR folder. Returns a page object ({ title, track, order,
 * summary, url, body, toc }) the site build treats like an authored page.
 * @param {{ dir: string, linksFile: string, adrDir: string, repoUrl: string, url: string, order: number, title?: string }} options
 * @returns {Page}
 */
export function arc42Page({ dir, linksFile, adrDir, repoUrl, url, order, title = "Architecture" }) {
  const map = readLinkMap(readFile(linksFile, "link map"), path.basename(linksFile));
  const adrs = adrIndex(adrDir);
  const adrBase = `${repoUrl}/blob/summer/docs/adr`;
  const prose = (/** @type {string} */ html) => linkAdrs(html, adrs, adrBase);
  const sections = DIAGRAMS.map((diagram) => {
    const mdFile = path.join(dir, `${diagram.stem}.md`);
    const doc = readExport(readFile(mdFile, "arc42 export"), path.basename(mdFile));
    const svgFile = path.join(dir, doc.image);
    const { open, inner, width } = prepareSvg(readFile(svgFile, "arc42 diagram"), doc.image);
    const { svg } = linkBoxes(inner, (name) => boxHref(map, name, doc.image), doc.image);
    const id = sectionId(diagram.name);
    const tables = doc.tables.map((t) => renderTable(t, `${id}-${sectionId(t.title)}`, map, prose));
    // The diagram never shrinks below DIAGRAM_SCALE of its export size, so its
    // 12 px labels stay legible; a narrower page scrolls the figure sideways.
    const minWidth = Math.round(width * DIAGRAM_SCALE);
    const labelled = open.replace(/>$/, ` aria-labelledby="${id}">`);
    return [
      `<h2 id="${id}">${escapeHtml(doc.title)}</h2>`,
      breadcrumb(diagram),
      `<figure class="arc42-diagram" style="--diagram-min-width: ${String(minWidth)}px">${labelled}<title>${escapeHtml(doc.title)}</title>${svg}</svg></figure>`,
      `<p>${prose(cellHtml(doc.motivation))}</p>`,
      ...tables,
    ].join("\n");
  });
  const kinds = LINE_KINDS.map(
    ([kind, text]) => `<dt><code>${kind}</code></dt><dd>${escapeHtml(text)}</dd>`,
  ).join("");
  const body = [
    `<p class="lede">The building-block view of arc42: the context first, then one whitebox per box that opens, down to the modules of the code. Every box but the User is a link: a box with a whitebox leads to the level below, every other box to the chapter that documents it.</p>`,
    `<h2 id="reading-the-diagrams">Reading the diagrams</h2>`,
    `<p>A building-block view shows a system as nested boxes: the context view has the system as one black box among its communication partners, and each whitebox opens one box to show the blocks inside it and the lines between them. The tables under each diagram carry the responsibility of every box and the text of every line. The diagrams are exported from the model in <a href="${repoUrl}/blob/summer/docs/architecture.yaml">docs/architecture.yaml</a> and rebuilt with the site.</p>`,
    `<p>Each diagram's legend shows a sample of the four line kinds:</p>`,
    `<dl class="line-kinds">${kinds}</dl>`,
    ...sections,
  ].join("\n");
  return {
    title,
    track: "reference",
    order,
    summary:
      "The arc42 building-block view: the context and four whiteboxes as clickable diagrams with the responsibilities, interfaces and motivation of the model.",
    url,
    body,
    toc: renderToc(),
  };
}
