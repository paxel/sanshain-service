// The releases of a Product (san-1033): one snapshot per umbrella chart
// version, its release sheet (members with their chart versions, the API
// versions they provide in the snapshot's scope, their require-set version and
// Pins, mismatches marked), CSV and Print, the sales name and scope set on the
// page, and the comparison of two snapshots.

/* exported openProductSnapshot, saveProductSnapshot, compareProductSnapshots */
/* exported downloadProductSheet, printProductSheet */

/**
 * `ProductSnapshot` in `src/domain/models.rs`.
 * @typedef {{
 *   chart_version: string, sales_name: string | null, scope: string, pushed_at: string,
 *   pushed_by: string | null, members: { system: string, chart_version: string | null }[],
 * }} ProductSnapshotView
 * @typedef {{ api_type: string, version: string, stability: string | null }} SheetProvided
 * @typedef {{
 *   producer: string, api_type: string, version: string, shipped: string | null,
 *   mismatch: boolean, proposal: string | null,
 * }} SheetPinView
 * @typedef {{
 *   system: string, chart_version: string | null, provides: SheetProvided[],
 *   require_set: RequireSet | null, pins: SheetPinView[],
 * }} SheetMemberView
 * @typedef {{
 *   product: string, chart_version: string, sales_name: string | null, scope: string,
 *   pushed_at: string, members: SheetMemberView[],
 * }} ReleaseSheetView
 * @typedef {{
 *   system: string, from_chart: string | null, to_chart: string | null, only_in: string | null,
 *   moved: { api_type: string, from: string | null, to: string | null, added: EndpointRef[],
 *     changed: EndpointRef[], removed: EndpointRef[] }[],
 * }} MemberComparisonView
 */

/** @type {string} */
let snapshotProduct = "";
/** @type {ReleaseSheetView | null} */
let openSheet = null;

/** @param {string} id */
function snapshotEl(id) {
  return /** @type {HTMLElement} */ (document.getElementById(id));
}

/** @param {string} name */
async function loadProductSnapshots(name) {
  snapshotProduct = name;
  /** @type {ProductSnapshotView[]} */
  let snapshots;
  try {
    snapshots = /** @type {ProductSnapshotView[]} */ (
      await fetchJSON(`/admin/products/${encodeURIComponent(name)}/snapshots`)
    );
  } catch {
    return;
  }
  const list = snapshotEl("product-snapshot-list");
  if (snapshots.length === 0) {
    list.innerHTML =
      '<p class="text-sm text-slate-400" data-no-snapshots>No release yet: push or paste a Chart.yaml with a version.</p>';
    snapshotEl("product-snapshot-compare").innerHTML = "";
    return;
  }
  list.innerHTML = `<table class="w-full text-sm"><thead class="text-slate-500"><tr>
      <th class="text-left font-semibold px-2 py-1">Chart version</th>
      <th class="text-left font-semibold px-2 py-1">Sales name</th>
      <th class="text-left font-semibold px-2 py-1">Scope</th>
      <th class="text-left font-semibold px-2 py-1">Pushed</th></tr></thead><tbody>${snapshots
        .map(
          (
            s,
          ) => `<tr class="border-t border-slate-100" data-snapshot="${escapeAttr(s.chart_version)}">
            <td class="px-2 py-1"><button type="button" data-click="openProductSnapshot" data-click-args='${attrJson([s.chart_version])}' class="font-mono text-indigo-700 underline">${escapeHtml(s.chart_version)}</button></td>
            <td class="px-2 py-1">${escapeHtml(s.sales_name ?? "–")}</td>
            <td class="px-2 py-1 font-mono text-xs">${escapeHtml(s.scope)}</td>
            <td class="px-2 py-1 text-xs text-slate-500">${escapeHtml(s.pushed_at.slice(0, 16).replace("T", " "))}${s.pushed_by ? ` by ${escapeHtml(s.pushed_by)}` : ""}</td>
          </tr>`,
        )
        .join("")}</tbody></table>`;
  const options = snapshots
    .map(
      (s) =>
        `<option value="${escapeAttr(s.chart_version)}">${escapeHtml(s.chart_version)}</option>`,
    )
    .join("");
  snapshotEl("product-snapshot-compare").innerHTML =
    snapshots.length < 2
      ? ""
      : `<div class="flex flex-wrap items-center gap-2 text-sm">
          <span class="text-slate-600">Compare</span>
          <select id="snapshot-compare-from" class="border border-slate-300 rounded-md px-2 py-1">${options}</select>
          <span>→</span>
          <select id="snapshot-compare-to" class="border border-slate-300 rounded-md px-2 py-1">${options}</select>
          <button type="button" data-click="compareProductSnapshots" class="px-3 py-1 bg-indigo-600 text-white rounded-lg text-sm">Compare</button>
        </div>
        <div id="snapshot-comparison" class="mt-2"></div>`;
  const from = /** @type {HTMLSelectElement | null} */ (
    document.getElementById("snapshot-compare-from")
  );
  if (from && snapshots[1]) from.value = snapshots[1].chart_version;
}

/** @param {string} version */
async function openProductSnapshot(version) {
  const sheetEl = snapshotEl("product-snapshot-sheet");
  try {
    openSheet = /** @type {ReleaseSheetView} */ (
      await fetchJSON(
        `/admin/products/${encodeURIComponent(snapshotProduct)}/snapshots/${encodeURIComponent(version)}`,
      )
    );
  } catch (err) {
    sheetEl.innerHTML = `<p class="text-red-600 text-sm">${escapeHtml(/** @type {Error} */ (err).message)}</p>`;
    return;
  }
  const s = openSheet;
  const rows = s.members
    .map((m) => {
      const provides = m.provides
        .map(
          (p) =>
            `<span class="px-2 py-0.5 rounded-full text-[11px] font-mono bg-slate-50 border border-slate-200">${escapeHtml(p.api_type)} ${escapeHtml(p.version)}${p.stability === "ga" ? " GA" : ""}</span>`,
        )
        .join(" ");
      const pins = m.pins
        .map(
          (p) =>
            `<div class="text-xs${p.mismatch ? " text-amber-700" : ""}" ${p.mismatch ? "data-pin-mismatch" : ""}>${escapeHtml(p.producer)} ${escapeHtml(p.version)}${
              p.mismatch
                ? ` · the Product ships ${escapeHtml(p.shipped ?? "")}${p.proposal ? ` · free move to ${escapeHtml(p.proposal)}` : ""}`
                : ""
            }</div>`,
        )
        .join("");
      return `<tr class="border-t border-slate-100 align-top" data-sheet-member="${escapeAttr(m.system)}">
        <td class="px-2 py-1 font-semibold">${escapeHtml(m.system)}</td>
        <td class="px-2 py-1 font-mono text-xs">${escapeHtml(m.chart_version ?? "–")}</td>
        <td class="px-2 py-1">${provides || "–"}</td>
        <td class="px-2 py-1 text-xs">${m.require_set ? `#${String(m.require_set.number)}` : "–"}</td>
        <td class="px-2 py-1">${pins || "–"}</td>
      </tr>`;
    })
    .join("");
  sheetEl.innerHTML = `<article class="border border-slate-200 rounded-xl p-4" data-release-sheet="${escapeAttr(s.chart_version)}">
      <div class="flex flex-wrap items-center justify-between gap-2 mb-2">
        <h4 class="text-lg font-bold text-slate-800">${escapeHtml(s.product)} ${escapeHtml(s.chart_version)}${s.sales_name ? ` · <span data-sheet-sales-name>${escapeHtml(s.sales_name)}</span>` : ""}</h4>
        <span class="flex gap-2 print:hidden">
          <button type="button" data-click="downloadProductSheet" class="text-xs px-2 py-1 rounded-md border border-slate-300">CSV</button>
          <button type="button" data-click="printProductSheet" class="text-xs px-2 py-1 rounded-md border border-slate-300">Print</button>
        </span>
      </div>
      <p class="text-xs text-slate-500 mb-3">Read from <span class="font-mono">${escapeHtml(s.scope)}</span>, pushed ${escapeHtml(s.pushed_at.slice(0, 16).replace("T", " "))}</p>
      <div class="flex flex-wrap items-center gap-2 mb-3 text-sm print:hidden">
        <input id="snapshot-sales-name" data-needs="manage_producers" data-needs-product="${escapeAttr(s.product)}" value="${escapeAttr(s.sales_name ?? "")}" placeholder="Sales name" class="px-2 py-1 border border-slate-300 rounded-md" />
        <input id="snapshot-scope" data-needs="manage_producers" data-needs-product="${escapeAttr(s.product)}" value="${escapeAttr(s.scope)}" placeholder="main or a sanshain-branch" class="px-2 py-1 border border-slate-300 rounded-md font-mono text-xs" />
        <button type="button" data-click="saveProductSnapshot" data-needs="manage_producers" data-needs-product="${escapeAttr(s.product)}" class="px-3 py-1 bg-indigo-600 text-white rounded-lg text-sm">Save</button>
        <span id="snapshot-save-result" class="text-xs text-slate-500"></span>
      </div>
      <table class="w-full text-sm"><thead class="text-slate-500"><tr>
        <th class="text-left font-semibold px-2 py-1">Member</th>
        <th class="text-left font-semibold px-2 py-1">Chart</th>
        <th class="text-left font-semibold px-2 py-1">Provides</th>
        <th class="text-left font-semibold px-2 py-1">Require set</th>
        <th class="text-left font-semibold px-2 py-1">Pins</th>
      </tr></thead><tbody>${rows}</tbody></table>
    </article>`;
}

async function saveProductSnapshot() {
  if (!openSheet) return;
  const result = snapshotEl("snapshot-save-result");
  const salesName = /** @type {HTMLInputElement} */ (snapshotEl("snapshot-sales-name")).value;
  const scope = /** @type {HTMLInputElement} */ (snapshotEl("snapshot-scope")).value.trim();
  const body = /** @type {{ sales_name: string | null, scope?: string }} */ ({
    sales_name: salesName.trim() || null,
  });
  if (scope && scope !== openSheet.scope) body.scope = scope;
  const res = await apiCall(
    `/admin/products/${encodeURIComponent(snapshotProduct)}/snapshots/${encodeURIComponent(openSheet.chart_version)}`,
    { method: "PUT", body },
  );
  if (!res.ok) {
    result.textContent = "✗ " + (await errorMessage(res));
    return;
  }
  const version = openSheet.chart_version;
  await loadProductSnapshots(snapshotProduct);
  await openProductSnapshot(version);
  snapshotEl("snapshot-save-result").textContent = "✓ Saved";
}

/** One CSV field, quoted when it needs to be. @param {string} value */
function csvField(value) {
  return /[",\r\n]/.test(value) ? `"${value.replaceAll('"', '""')}"` : value;
}

function downloadProductSheet() {
  if (!openSheet) return;
  const s = openSheet;
  const lines = [["Member", "Chart version", "Provides", "Require set", "Pins"]];
  for (const m of s.members) {
    lines.push([
      m.system,
      m.chart_version ?? "",
      m.provides.map((p) => `${p.api_type} ${p.version}`).join(" "),
      m.require_set ? String(m.require_set.number) : "",
      m.pins
        .map((p) => `${p.producer} ${p.version}${p.mismatch ? ` (ships ${p.shipped ?? ""})` : ""}`)
        .join(" "),
    ]);
  }
  const csv = lines.map((l) => l.map(csvField).join(",")).join("\r\n") + "\r\n";
  const blob = new Blob([csv], { type: "text/csv" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = `sanshain-${s.product}-${s.chart_version}.csv`;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

function printProductSheet() {
  window.print();
}

async function compareProductSnapshots() {
  const from = /** @type {HTMLSelectElement} */ (snapshotEl("snapshot-compare-from")).value;
  const to = /** @type {HTMLSelectElement} */ (snapshotEl("snapshot-compare-to")).value;
  const out = snapshotEl("snapshot-comparison");
  /** @type {MemberComparisonView[]} */
  let members;
  try {
    members = /** @type {MemberComparisonView[]} */ (
      await fetchJSON(
        `/admin/products/${encodeURIComponent(snapshotProduct)}/snapshots/compare?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}`,
      )
    );
  } catch (err) {
    out.innerHTML = `<p class="text-red-600 text-sm">${escapeHtml(/** @type {Error} */ (err).message)}</p>`;
    return;
  }
  const list = (/** @type {string} */ kind, /** @type {EndpointRef[]} */ refs) =>
    refs.length
      ? `<span class="text-xs">${kind}: ${refs.map((e) => escapeHtml(`${e.method} ${e.path}`)).join(", ")}</span>`
      : "";
  out.innerHTML = `<table class="w-full text-sm"><tbody>${members
    .map((m) => {
      const moved = m.moved
        .map(
          (l) =>
            `<div data-moved-line="${escapeAttr(l.api_type)}"><span class="font-mono">${escapeHtml(l.api_type)} ${escapeHtml(l.from ?? "–")} → ${escapeHtml(l.to ?? "–")}</span> ${[
              list("added", l.added),
              list("changed", l.changed),
              list("removed", l.removed),
            ]
              .filter(Boolean)
              .join(" · ")}</div>`,
        )
        .join("");
      return `<tr class="border-t border-slate-100 align-top" data-compare-member="${escapeAttr(m.system)}">
        <td class="px-2 py-1 font-semibold">${escapeHtml(m.system)}</td>
        <td class="px-2 py-1 font-mono text-xs">${escapeHtml(m.from_chart ?? "–")} → ${escapeHtml(m.to_chart ?? "–")}${m.only_in ? ` (only in ${escapeHtml(m.only_in)})` : ""}</td>
        <td class="px-2 py-1">${moved || '<span class="text-xs text-slate-400">no API moved</span>'}</td>
      </tr>`;
    })
    .join("")}</tbody></table>`;
}
