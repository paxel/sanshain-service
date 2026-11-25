// The statistics page (san-1021): one tab per topic, one range for the page,
// and every panel with its own exports (CSV for the data, SVG and PNG for
// slides, Print). The URL carries the tab and the range, so a copied link
// shows the same numbers. The data is aggregated on the server; no answer
// carries a person's name.

/* exported statisticsSortTable, statisticsFilterTable */
/* exported STATISTICS_RENDERERS, statisticsSelectTab, statisticsSelectPreset, statisticsSelectCustom, statisticsCopyLink, statisticsExportCsv, statisticsExportSvg, statisticsExportPng, statisticsPrintPanel */

/**
 * @typedef {{ id: string, label: string }} StatisticsTabInfo
 * @typedef {{ from: string, to: string }} StatisticsRangeValue
 * @typedef {{ columns: string[], rows: (string | number)[][] }} StatisticsTable
 * @typedef {{
 *   id: string,
 *   title: string,
 *   audience: string,
 *   note?: string,
 *   chart?: Record<string, unknown>,
 *   chartHeight?: number,
 *   table?: StatisticsTable,
 *   html?: string,
 *   wide?: boolean,
 *   hideTable?: boolean,
 *   sortable?: boolean,
 *   filterable?: boolean,
 * }} StatisticsPanel
 * @typedef {(data: any, range: StatisticsRangeValue) => StatisticsPanel[]} StatisticsRenderer
 */

/** @type {StatisticsTabInfo[]} */
const STATISTICS_TABS = [
  { id: "landscape", label: "Landscape" },
  { id: "activity", label: "Activity" },
  { id: "versions", label: "Versions and releases" },
  { id: "branches", label: "Tags and branches" },
  { id: "access", label: "Users and access" },
  { id: "catalog", label: "Catalog" },
];

/**
 * The renderer of each tab, filled by the tab's own script. A Map, so a tab
 * name from the URL can only ever find a renderer that was registered.
 * @type {Map<string, StatisticsRenderer>}
 */
const STATISTICS_RENDERERS = new Map();

/** The range presets, in days, today included. */
const STATISTICS_PRESETS = [30, 90, 365];

/** @type {Map<string, echarts.ECharts>} */
const statisticsCharts = new Map();
/** @type {Map<string, StatisticsPanel>} */
const statisticsPanels = new Map();
/**
 * How each sortable or filterable table is shown: sort column, direction,
 * filter text.
 * @type {Map<string, { column: number, descending: boolean, filter: string }>}
 */
const statisticsTableViews = new Map();
/** @type {StatisticsRangeValue} */
let statisticsRange = { from: "", to: "" };
let statisticsTab = "landscape";

/**
 * A UTC day as `YYYY-MM-DD`.
 * @param {Date} date
 */
function statisticsDay(date) {
  return date.toISOString().slice(0, 10);
}

/**
 * The range of a preset: `days` days up to today.
 * @param {number} days
 * @returns {StatisticsRangeValue}
 */
function statisticsPresetRange(days) {
  const to = new Date();
  const from = new Date(to.getTime() - (days - 1) * 86400000);
  return { from: statisticsDay(from), to: statisticsDay(to) };
}

/** Reads the tab and the range from the URL, defaulting to 90 days. */
function statisticsReadUrl() {
  const params = new URLSearchParams(window.location.search);
  const tab = params.get("tab") ?? "landscape";
  statisticsTab = STATISTICS_TABS.some((t) => t.id === tab) ? tab : "landscape";
  const from = params.get("from");
  const to = params.get("to");
  statisticsRange = from && to ? { from, to } : statisticsPresetRange(90);
}

/** Writes the tab and the range into the URL, without a new history entry. */
function statisticsWriteUrl() {
  const params = new URLSearchParams(window.location.search);
  params.set("tab", statisticsTab);
  params.set("from", statisticsRange.from);
  params.set("to", statisticsRange.to);
  history.replaceState(null, "", `${window.location.pathname}?${params.toString()}`);
}

/** The preset the current range matches, or "custom". */
function statisticsCurrentPreset() {
  for (const days of STATISTICS_PRESETS) {
    const r = statisticsPresetRange(days);
    if (r.from === statisticsRange.from && r.to === statisticsRange.to) return String(days);
  }
  return "custom";
}

/**
 * Draws the page frame: the range picker, the share button and the tab bar.
 * @param {HTMLElement} root
 */
function statisticsRenderFrame(root) {
  const preset = statisticsCurrentPreset();
  const options = [
    ...STATISTICS_PRESETS.map(
      (d) =>
        `<option value="${String(d)}"${preset === String(d) ? " selected" : ""}>Last ${d === 365 ? "12 months" : `${String(d)} days`}</option>`,
    ),
    `<option value="custom"${preset === "custom" ? " selected" : ""}>Custom</option>`,
  ].join("");
  const tabs = STATISTICS_TABS.map(
    (t) =>
      `<button type="button" role="tab" data-click="statisticsSelectTab" data-click-args='${attrJson([t.id])}'
        id="statistics-tab-${t.id}" aria-selected="${String(t.id === statisticsTab)}"
        class="px-3 py-2 text-sm font-medium rounded-t-lg border-b-2 ${
          t.id === statisticsTab
            ? "border-indigo-600 text-indigo-700"
            : "border-transparent text-slate-500 hover:text-slate-800"
        }">${escapeHtml(t.label)}</button>`,
  ).join("");
  root.innerHTML = `
    <div class="statistics-controls bg-white p-4 rounded-2xl border border-slate-200 shadow-xs flex flex-wrap items-center gap-3 mb-4">
      <label for="statistics-preset" class="text-sm font-semibold text-slate-600">Range</label>
      <select id="statistics-preset" data-change="statisticsSelectPreset" data-change-args='["$value"]'
        class="border border-slate-300 rounded-lg px-2 py-1.5 text-sm bg-slate-50">${options}</select>
      <input id="statistics-from" type="date" value="${escapeAttr(statisticsRange.from)}"
        data-change="statisticsSelectCustom" class="border border-slate-300 rounded-lg px-2 py-1 text-sm" aria-label="From" />
      <span class="text-slate-400">–</span>
      <input id="statistics-to" type="date" value="${escapeAttr(statisticsRange.to)}"
        data-change="statisticsSelectCustom" class="border border-slate-300 rounded-lg px-2 py-1 text-sm" aria-label="To" />
      <span class="flex-1"></span>
      <button type="button" id="statistics-share" data-click="statisticsCopyLink"
        class="text-sm px-3 py-1.5 rounded-lg border border-slate-300 hover:bg-slate-50">Copy link</button>
    </div>
    <div role="tablist" class="statistics-controls flex flex-wrap gap-1 border-b border-slate-200 mb-4">${tabs}</div>
    <div id="statistics-body" class="grid grid-cols-1 lg:grid-cols-2 gap-4"></div>`;
}

/**
 * The card of one panel, its chart container and its export buttons.
 * @param {StatisticsPanel} panel
 */
function statisticsPanelHtml(panel) {
  const button = (/** @type {string} */ action, /** @type {string} */ label) =>
    `<button type="button" data-click="${action}" data-click-args='${attrJson([panel.id])}'
      class="text-xs px-2 py-1 rounded-md border border-slate-300 hover:bg-slate-50">${label}</button>`;
  const exports = [
    panel.table ? button("statisticsExportCsv", "CSV") : "",
    panel.chart ? button("statisticsExportSvg", "SVG") : "",
    panel.chart ? button("statisticsExportPng", "PNG") : "",
    button("statisticsPrintPanel", "Print"),
  ].join("");
  const filter = panel.filterable
    ? `<input type="search" id="statistics-filter-${escapeAttr(panel.id)}" placeholder="Filter…" aria-label="Filter"
        data-input="statisticsFilterTable" data-input-args='${attrJson([panel.id, "$value"])}'
        class="statistics-controls mt-2 w-full border border-slate-300 rounded-lg px-2 py-1 text-sm" />`
    : "";
  const table =
    panel.table && !panel.hideTable
      ? `${filter}<div id="statistics-table-${escapeAttr(panel.id)}">${statisticsTableHtml(statisticsViewTable(panel), panel.sortable ? panel.id : null)}</div>`
      : "";
  const chart = panel.chart
    ? `<div id="statistics-chart-${escapeAttr(panel.id)}" class="w-full" style="height:${String(panel.chartHeight ?? 320)}px"></div>`
    : "";
  return `
    <section id="statistics-panel-${escapeAttr(panel.id)}" data-panel="${escapeAttr(panel.id)}"
      class="statistics-panel bg-white p-4 rounded-2xl border border-slate-200 shadow-xs ${panel.wide ? "lg:col-span-2" : ""}">
      <div class="flex items-start justify-between gap-2 mb-2">
        <div>
          <h3 class="text-base font-bold text-slate-800">${escapeHtml(panel.title)}</h3>
          <p class="text-xs text-slate-500">For ${escapeHtml(panel.audience)}${panel.note ? ` · ${escapeHtml(panel.note)}` : ""}</p>
        </div>
        <div class="statistics-controls flex gap-1 shrink-0">${exports}</div>
      </div>
      ${chart}
      ${panel.html ?? ""}
      ${table}
    </section>`;
}

/**
 * The panel's table as it is shown: filtered by the filter text, sorted by
 * the chosen column.
 * @param {StatisticsPanel} panel
 * @returns {StatisticsTable}
 */
function statisticsViewTable(panel) {
  const table = panel.table ?? { columns: [], rows: [] };
  const view = statisticsTableViews.get(panel.id);
  if (!view) return table;
  const needle = view.filter.trim().toLowerCase();
  let rows = needle
    ? table.rows.filter((r) => r.some((v) => String(v).toLowerCase().includes(needle)))
    : [...table.rows];
  if (view.column >= 0) {
    const c = view.column;
    rows = rows.sort((a, b) => {
      const x = a[c] ?? "";
      const y = b[c] ?? "";
      const order =
        typeof x === "number" && typeof y === "number" ? x - y : String(x).localeCompare(String(y));
      return view.descending ? -order : order;
    });
  }
  return { columns: table.columns, rows };
}

/**
 * @param {string} id
 * @param {number} column
 */
function statisticsSortTable(id, column) {
  const view = statisticsTableViews.get(id) ?? { column: -1, descending: false, filter: "" };
  view.descending = view.column === column ? !view.descending : false;
  view.column = column;
  statisticsTableViews.set(id, view);
  statisticsRedrawTable(id);
}

/**
 * @param {string} id
 * @param {string} text
 */
function statisticsFilterTable(id, text) {
  const view = statisticsTableViews.get(id) ?? { column: -1, descending: false, filter: "" };
  view.filter = text;
  statisticsTableViews.set(id, view);
  statisticsRedrawTable(id);
}

/** @param {string} id */
function statisticsRedrawTable(id) {
  const panel = statisticsPanels.get(id);
  const target = document.getElementById(`statistics-table-${id}`);
  if (!panel || !target) return;
  target.innerHTML = statisticsTableHtml(statisticsViewTable(panel), panel.sortable ? id : null);
}

/**
 * A plain table of a panel's data, numbers right-aligned; with `sortId`, the
 * column heads sort it.
 * @param {StatisticsTable} table
 * @param {string | null} [sortId]
 */
function statisticsTableHtml(table, sortId = null) {
  if (table.rows.length === 0) {
    return '<p class="text-sm text-slate-400 py-2">Nothing in this range.</p>';
  }
  const head = table.columns
    .map((c, i) =>
      sortId
        ? `<th class="text-left font-semibold px-2 py-1"><button type="button" data-click="statisticsSortTable"
            data-click-args='${attrJson([sortId, i])}' class="hover:text-slate-800">${escapeHtml(c)} ↕</button></th>`
        : `<th class="text-left font-semibold px-2 py-1">${escapeHtml(c)}</th>`,
    )
    .join("");
  const body = table.rows
    .map(
      (r) =>
        `<tr class="border-t border-slate-100">${r
          .map(
            (v) =>
              `<td class="px-2 py-1 ${typeof v === "number" ? "text-right tabular-nums" : ""}">${escapeHtml(String(v))}</td>`,
          )
          .join("")}</tr>`,
    )
    .join("");
  return `<div class="max-h-80 overflow-auto mt-2"><table class="w-full text-sm"><thead class="text-slate-500">${head}</thead><tbody>${body}</tbody></table></div>`;
}

/** Loads the current tab's data and draws its panels. */
async function statisticsLoadTab() {
  const body = document.getElementById("statistics-body");
  if (!body) return;
  for (const chart of statisticsCharts.values()) chart.dispose();
  statisticsCharts.clear();
  statisticsPanels.clear();
  statisticsTableViews.clear();
  const renderer = STATISTICS_RENDERERS.get(statisticsTab);
  if (typeof renderer !== "function") {
    body.innerHTML = '<p class="text-slate-500 col-span-2">This tab has no data yet.</p>';
    return;
  }
  body.innerHTML = '<p class="text-slate-500 col-span-2">Loading…</p>';
  const q = new URLSearchParams(statisticsRange).toString();
  const res = await apiCall(`/admin/statistics/${statisticsTab}?${q}`);
  if (!res.ok) {
    body.innerHTML = `<p class="text-red-600 col-span-2">${escapeHtml(await errorMessage(res))}</p>`;
    return;
  }
  const answer = /** @type {{ data: unknown }} */ (await res.json());
  const panels = renderer(answer.data, statisticsRange);
  body.innerHTML = panels.map(statisticsPanelHtml).join("");
  for (const panel of panels) {
    statisticsPanels.set(panel.id, panel);
    const el = document.getElementById(`statistics-chart-${panel.id}`);
    if (panel.chart && el) {
      const chart = echarts.init(el, null, { renderer: "svg" });
      chart.setOption({ animation: false, ...panel.chart });
      statisticsCharts.set(panel.id, chart);
    }
  }
}

/** @param {string} tab */
function statisticsSelectTab(tab) {
  if (!STATISTICS_TABS.some((t) => t.id === tab)) return;
  statisticsTab = tab;
  statisticsWriteUrl();
  statisticsRedraw();
}

/** @param {string} value */
function statisticsSelectPreset(value) {
  if (value !== "custom") statisticsRange = statisticsPresetRange(Number(value));
  statisticsWriteUrl();
  statisticsRedraw();
}

function statisticsSelectCustom() {
  const from = /** @type {HTMLInputElement | null} */ (document.getElementById("statistics-from"));
  const to = /** @type {HTMLInputElement | null} */ (document.getElementById("statistics-to"));
  if (!from?.value || !to?.value) return;
  statisticsRange = { from: from.value, to: to.value };
  statisticsWriteUrl();
  statisticsRedraw();
}

function statisticsRedraw() {
  const root = document.getElementById("statistics-root");
  if (!root) return;
  statisticsRenderFrame(root);
  void statisticsLoadTab();
}

async function statisticsCopyLink() {
  const button = document.getElementById("statistics-share");
  try {
    await navigator.clipboard.writeText(window.location.href);
    if (button) button.textContent = "Link copied";
  } catch {
    if (button) button.textContent = "Copy the address bar";
  }
}

/**
 * The file name of an export: panel, range, extension.
 * @param {string} id
 * @param {string} extension
 */
function statisticsFileName(id, extension) {
  return `sanshain-${id}-${statisticsRange.from}-${statisticsRange.to}.${extension}`;
}

/**
 * Offers a blob as a download.
 * @param {Blob} blob
 * @param {string} name
 */
function statisticsDownload(blob, name) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => {
    URL.revokeObjectURL(url);
  }, 1000);
}

/**
 * A table as CSV text (RFC 4180 quoting).
 * @param {StatisticsTable} table
 */
function statisticsCsv(table) {
  const cell = (/** @type {string | number} */ v) => {
    const s = String(v);
    return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
  };
  return [table.columns, ...table.rows].map((r) => r.map(cell).join(",")).join("\r\n") + "\r\n";
}

/** @param {string} id */
function statisticsExportCsv(id) {
  const panel = statisticsPanels.get(id);
  if (!panel?.table) return;
  // What is shown: filtered and sorted.
  statisticsDownload(
    new Blob([statisticsCsv(statisticsViewTable(panel))], { type: "text/csv;charset=utf-8" }),
    statisticsFileName(id, "csv"),
  );
}

/** @param {string} id */
function statisticsExportSvg(id) {
  const chart = statisticsCharts.get(id);
  if (!chart) return;
  statisticsDownload(
    new Blob([chart.renderToSVGString()], { type: "image/svg+xml" }),
    statisticsFileName(id, "svg"),
  );
}

/**
 * The chart as a PNG at twice its size on white, drawn from its SVG: the SVG
 * renderer has no bitmap of its own.
 * @param {string} id
 */
function statisticsExportPng(id) {
  const chart = statisticsCharts.get(id);
  if (!chart) return;
  const width = chart.getWidth();
  const height = chart.getHeight();
  const image = new Image();
  image.onload = () => {
    const canvas = document.createElement("canvas");
    canvas.width = width * 2;
    canvas.height = height * 2;
    const context = canvas.getContext("2d");
    if (!context) return;
    context.fillStyle = "#ffffff";
    context.fillRect(0, 0, canvas.width, canvas.height);
    context.drawImage(image, 0, 0, canvas.width, canvas.height);
    canvas.toBlob((blob) => {
      if (blob) statisticsDownload(blob, statisticsFileName(id, "png"));
    }, "image/png");
  };
  image.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(chart.renderToSVGString())}`;
}

/**
 * Prints one panel alone: the print stylesheet hides everything else.
 * @param {string} id
 */
function statisticsPrintPanel(id) {
  const section = document.getElementById(`statistics-panel-${id}`);
  if (!section) return;
  document.body.classList.add("print-one-panel");
  section.classList.add("print-this");
  const done = () => {
    document.body.classList.remove("print-one-panel");
    section.classList.remove("print-this");
    window.removeEventListener("afterprint", done);
  };
  window.addEventListener("afterprint", done);
  window.print();
}

window.addEventListener("resize", () => {
  for (const chart of statisticsCharts.values()) chart.resize();
});

/**
 * Starts the page for `user`; without `view_statistics` it says what is
 * missing instead.
 * @param {SanshainUser | null} user
 */
function initStatistics(user) {
  const root = document.getElementById("statistics-root");
  if (!root) return false;
  if (!hasPermission(user, "view_statistics")) {
    root.innerHTML = `<div class="bg-white p-6 rounded-2xl border border-slate-200 text-slate-600">
      The statistics need the <code>view_statistics</code> permission (role <code>report_viewer</code>).
      Ask an administrator for it.</div>`;
    return false;
  }
  statisticsReadUrl();
  statisticsWriteUrl();
  statisticsRedraw();
  return true;
}

function initStatisticsPage() {
  void checkDiscoveryAuth((user) => {
    initStatistics(user);
    hideLoader();
  });
}
if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", initStatisticsPage);
} else {
  initStatisticsPage();
}
