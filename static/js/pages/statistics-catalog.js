// The Catalog tab of the statistics page (san-1027): documentation of the
// data. A list of systems with filters by owner, domain, Product and tag; a
// click opens the system's sheet with both roles on it. "Print" prints the
// open sheet, "Print all filtered" every sheet of the list, one per page. The
// URL carries the filters and the open system.

/* exported catalogFilter, catalogOpen, catalogPrintSheet, catalogPrintAll */

/**
 * @typedef {{ build_url?: string | null, change_url?: string | null, tickets?: string[], note?: string | null }} CatalogOrigin
 * @typedef {{ system: string, api_type: string, version: string, endpoints: number }} CatalogDependency
 * @typedef {{
 *   name: string, owner: string, domain: string, products: string[], tags: string[],
 *   description: string | null, source_location: string | null,
 *   links: { url: string, title: string }[], annotations: { name: string, value: string }[],
 *   external: boolean,
 *   provides: { api_type: string, versions: number, latest: string, latest_ga: string | null, origin: CatalogOrigin | null }[],
 *   consumes: CatalogDependency[], consumed_by: CatalogDependency[],
 * }} CatalogSheet
 */

/** @type {CatalogSheet[]} */
let catalogSystems = [];

/** The filters the URL names, by key. */
const CATALOG_FILTERS = /** @type {const} */ (["owner", "domain", "product", "tag"]);

/** @param {string} key */
function catalogParam(key) {
  return new URLSearchParams(window.location.search).get(key) ?? "";
}

/**
 * @param {string} key
 * @param {string} value
 */
function catalogSetParam(key, value) {
  const params = new URLSearchParams(window.location.search);
  if (value) params.set(key, value);
  else params.delete(key);
  history.replaceState(null, "", `${window.location.pathname}?${params.toString()}`);
}

/** The systems the current filters let through. */
function catalogFiltered() {
  const owner = catalogParam("owner");
  const domain = catalogParam("domain");
  const product = catalogParam("product");
  const tag = catalogParam("tag");
  return catalogSystems.filter(
    (s) =>
      (!owner || s.owner === owner) &&
      (!domain || s.domain === domain) &&
      (!product || s.products.includes(product)) &&
      (!tag || s.tags.includes(tag)),
  );
}

/**
 * A link that opens in a new tab, for http(s) URLs only.
 * @param {string} url
 * @param {string} label
 */
function catalogLink(url, label) {
  return /^https?:\/\//i.test(url)
    ? `<a href="${escapeAttr(url)}" target="_blank" rel="noopener noreferrer" class="text-indigo-700 underline">${escapeHtml(label)}</a>`
    : escapeHtml(label);
}

/**
 * @param {string[]} columns
 * @param {string[][]} rows rows of already escaped HTML cells
 */
function catalogTable(columns, rows) {
  if (rows.length === 0) return '<p class="text-sm text-slate-400">None.</p>';
  return `<table class="w-full text-sm mb-2"><thead class="text-slate-500"><tr>${columns
    .map((c) => `<th class="text-left font-semibold px-2 py-1">${escapeHtml(c)}</th>`)
    .join("")}</tr></thead><tbody>${rows
    .map(
      (r) =>
        `<tr class="border-t border-slate-100">${r.map((c) => `<td class="px-2 py-1">${c}</td>`).join("")}</tr>`,
    )
    .join("")}</tbody></table>`;
}

/**
 * The sheet of one system.
 * @param {CatalogSheet} s
 */
function catalogSheetHtml(s) {
  const facts = [
    ["Owner", escapeHtml(s.owner)],
    ["Domain", escapeHtml(s.domain)],
    ["Products", escapeHtml(s.products.join(", ") || "–")],
    ["Tags", escapeHtml(s.tags.join(", ") || "–")],
    ["Source", s.source_location ? catalogLink(s.source_location, s.source_location) : "–"],
  ];
  const origin = (/** @type {CatalogOrigin | null} */ o) =>
    o
      ? [
          o.build_url ? catalogLink(o.build_url, "build") : "",
          o.change_url ? catalogLink(o.change_url, "change") : "",
          escapeHtml((o.tickets ?? []).join(" ")),
          escapeHtml(o.note ?? ""),
        ]
          .filter(Boolean)
          .join(" · ")
      : "–";
  const deps = (/** @type {CatalogDependency[]} */ list) =>
    list.map((d) => [
      escapeHtml(d.system),
      escapeHtml(d.api_type),
      escapeHtml(d.version),
      escapeHtml(String(d.endpoints)),
    ]);
  return `
    <article class="statistics-sheet border border-slate-200 rounded-xl p-4 mb-4" data-sheet="${escapeAttr(s.name)}">
      <h4 class="text-lg font-bold text-slate-800">${escapeHtml(s.name)}${s.external ? ' <span class="text-xs text-slate-500">(external)</span>' : ""}</h4>
      ${s.description ? `<p class="text-sm text-slate-600 mb-2">${escapeHtml(s.description)}</p>` : ""}
      <dl class="grid grid-cols-[8rem_1fr] gap-x-3 gap-y-1 text-sm mb-3">${facts
        .map(
          ([k, v]) => `<dt class="text-slate-500">${escapeHtml(k ?? "")}</dt><dd>${v ?? ""}</dd>`,
        )
        .join("")}</dl>
      ${s.links.length > 0 ? `<p class="text-sm mb-3">${s.links.map((l) => catalogLink(l.url, l.title || l.url)).join(" · ")}</p>` : ""}
      ${
        s.annotations.length > 0
          ? `<p class="text-sm mb-3">${s.annotations
              .map(
                (a) =>
                  `<span class="text-slate-500">${escapeHtml(a.name)}:</span> ${catalogLink(a.value, a.value)}`,
              )
              .join(" · ")}</p>`
          : ""
      }
      <h5 class="font-semibold text-slate-700 mt-2">Provides</h5>
      ${catalogTable(
        ["API type", "Versions", "Latest", "Latest GA", "Origin of the latest"],
        s.provides.map((p) => [
          escapeHtml(p.api_type),
          escapeHtml(String(p.versions)),
          escapeHtml(p.latest),
          escapeHtml(p.latest_ga ?? "–"),
          origin(p.origin),
        ]),
      )}
      <h5 class="font-semibold text-slate-700 mt-2">Consumes</h5>
      ${catalogTable(["Producer", "API type", "Pin", "Endpoints"], deps(s.consumes))}
      <h5 class="font-semibold text-slate-700 mt-2">Consumed by</h5>
      ${catalogTable(["Consumer", "API type", "Pin", "Endpoints"], deps(s.consumed_by))}
    </article>`;
}

/** Draws the filters, the list and the open sheet. */
function catalogDraw() {
  const root = document.getElementById("catalog-root");
  if (!root) return;
  /** @type {Record<string, Set<string>>} */
  const values = { owner: new Set(), domain: new Set(), product: new Set(), tag: new Set() };
  for (const s of catalogSystems) {
    values.owner?.add(s.owner);
    values.domain?.add(s.domain);
    for (const p of s.products) values.product?.add(p);
    for (const t of s.tags) values.tag?.add(t);
  }
  const selects = CATALOG_FILTERS.map((key) => {
    const current = catalogParam(key);
    const options = [...(values[key] ?? [])]
      .sort((a, b) => a.localeCompare(b))
      .map(
        (v) =>
          `<option value="${escapeAttr(v)}"${v === current ? " selected" : ""}>${escapeHtml(v)}</option>`,
      )
      .join("");
    return `<label class="text-sm text-slate-600">${escapeHtml(key)}
      <select id="catalog-filter-${key}" data-change="catalogFilter" data-change-args='${attrJson([key, "$value"])}'
        class="border border-slate-300 rounded-lg px-2 py-1 text-sm bg-slate-50"><option value="">all</option>${options}</select></label>`;
  }).join("");
  const list = catalogFiltered();
  const open = catalogParam("system");
  const sheet = catalogSystems.find((s) => s.name === open);
  root.innerHTML = `
    <div class="statistics-controls flex flex-wrap items-center gap-3 mb-3">${selects}
      <span class="flex-1"></span>
      <button type="button" id="catalog-print-all" data-click="catalogPrintAll"
        class="text-xs px-2 py-1 rounded-md border border-slate-300 hover:bg-slate-50">Print all filtered (${String(list.length)})</button>
    </div>
    <div class="statistics-controls max-h-72 overflow-auto mb-4">${catalogTable(
      ["System", "Owner", "Domain", "Provides", "Consumes"],
      list.map((s) => [
        `<button type="button" data-click="catalogOpen" data-click-args='${attrJson([s.name])}'
          class="text-indigo-700 underline">${escapeHtml(s.name)}</button>`,
        escapeHtml(s.owner),
        escapeHtml(s.domain),
        escapeHtml(s.provides.map((p) => p.api_type).join(", ") || "–"),
        escapeHtml(String(s.consumes.length)),
      ]),
    )}</div>
    <div id="catalog-sheet">${
      sheet
        ? `<div class="statistics-controls flex justify-end mb-1"><button type="button" data-click="catalogPrintSheet"
            class="text-xs px-2 py-1 rounded-md border border-slate-300 hover:bg-slate-50">Print sheet</button></div>${catalogSheetHtml(sheet)}`
        : '<p class="text-sm text-slate-400">Pick a system to open its sheet.</p>'
    }</div>
    <div id="catalog-print-sheets" class="hidden"></div>`;
}

/**
 * @param {string} key
 * @param {string} value
 */
function catalogFilter(key, value) {
  catalogSetParam(key, value);
  catalogDraw();
}

/** @param {string} name */
function catalogOpen(name) {
  catalogSetParam("system", name);
  catalogDraw();
}

function catalogPrintSheet() {
  statisticsPrintPanel("catalog-systems");
}

function catalogPrintAll() {
  const target = document.getElementById("catalog-print-sheets");
  if (!target) return;
  target.innerHTML = catalogFiltered().map(catalogSheetHtml).join("");
  document.body.classList.add("print-catalog-all");
  const done = () => {
    document.body.classList.remove("print-catalog-all");
    target.innerHTML = "";
    window.removeEventListener("afterprint", done);
  };
  window.addEventListener("afterprint", done);
  window.print();
}

STATISTICS_RENDERERS.set("catalog", (/** @type {{ systems: CatalogSheet[] }} */ data) => {
  catalogSystems = data.systems;
  // The panel's own markup is drawn once it is in the page.
  setTimeout(catalogDraw, 0);
  /** @type {StatisticsPanel[]} */
  const panels = [
    {
      id: "catalog-systems",
      title: "Catalog",
      audience: "architects",
      note: `${String(data.systems.length)} systems, both roles on one sheet`,
      wide: true,
      html: '<div id="catalog-root"></div>',
      hideTable: true,
      table: {
        columns: [
          "System",
          "Owner",
          "Domain",
          "Products",
          "Tags",
          "Provides",
          "Consumes",
          "Consumed by",
        ],
        rows: data.systems.map((s) => [
          s.name,
          s.owner,
          s.domain,
          s.products.join(" "),
          s.tags.join(" "),
          s.provides.map((p) => `${p.api_type} ${p.latest}`).join(" "),
          s.consumes.map((d) => `${d.system} ${d.version}`).join(" "),
          s.consumed_by.map((d) => d.system).join(" "),
        ]),
      },
    },
  ];
  return panels;
});
