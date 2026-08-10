// Extracted from static/reports.html (ai/improvements.md #11): inline
// script moved to an external classic file so the CSP can forbid inline
// execution. Declarations stay global as when inline; the file loads at
// the same document position, so DOM timing is unchanged.

async function showReports() {
  showLoader();
  try {
    loadReports();
    hideLoader();
  } catch (err) {
    console.error("Failed to show reports:", err);
    hideLoader();
    const grid = document.getElementById("reports-grid");
    if (grid) {
      grid.innerHTML = `<div class="text-red-500 text-center py-10 col-span-2">Error: ${err.message}</div>`;
    }
  }
}

// Reports cover the whole instance: every dependency is a Consumer's exact
// Pin on a Producer version, so there is no further axis to pick.
async function loadReports() {
  const grid = document.getElementById("reports-grid");
  // Graph scope (ADR-0005): dev (accumulated activity), main (trunk
  // pins), or one sanshain-branch. The selector feeds the report links.
  let branchNames = [];
  try {
    const res = await apiCall("/admin/branches");
    if (res.ok) branchNames = (await res.json()).map((b) => b.name);
  } catch (_) {
    /* branch listing is optional for reports */
  }
  const scopeOptions = ["dev", "main", ...branchNames]
    .map(
      (s) =>
        `<option value="${escapeHtml(s)}">${escapeHtml(s === "dev" ? "Dev (recorded activity)" : s === "main" ? "Main (trunk pins)" : s)}</option>`,
    )
    .join("");
  const scopeParam = () => {
    const v = document.getElementById("report-scope").value;
    return v === "dev" ? "" : `?scope=${encodeURIComponent(v)}`;
  };
  window.openScopedReport = (src, title) => {
    const scope = document.getElementById("report-scope").value;
    const label = scope === "dev" ? title : `${title} — ${scope}`;
    window.location.href = `/report-viewer.html?src=${encodeURIComponent(src + scopeParam())}&title=${encodeURIComponent(label)}`;
  };
  grid.innerHTML = `
            <div class="md:col-span-2 bg-white p-4 rounded-2xl border border-slate-200 shadow-sm flex items-center gap-3">
                <label for="report-scope" class="text-sm font-semibold text-slate-600">Graph scope</label>
                <select id="report-scope" class="border border-slate-300 rounded-lg px-2 py-1.5 text-sm bg-slate-50 focus:outline-none focus:ring-2 focus:ring-indigo-500">${scopeOptions}</select>
                <span class="text-xs text-slate-400">dev = recorded activity · main = trunk pins · or a sanshain-branch</span>
            </div>
            <div class="bg-white p-6 rounded-2xl border border-slate-200 shadow-sm hover:shadow-md transition-shadow">
                <div class="bg-indigo-100 text-indigo-700 w-12 h-12 rounded-xl flex items-center justify-center mb-4">
                    <svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 17v-2m3 2v-4m3 2v-6m-8-2h8a2 2 0 012 2v9a2 2 0 01-2 2H7a2 2 0 01-2-2V5a2 2 0 012-2h8z"></path>
                    </svg>
                </div>
                <h3 class="text-lg font-bold text-slate-800 mb-2">Producer Isolation Report</h3>
                <p class="text-sm text-slate-600 mb-6">Detailed overview of service-to-service communication, identifying which services "talk" to each other.</p>
                <button onclick="openScopedReport('/report/isolation', 'Service Isolation Report')"
                   class="inline-flex items-center justify-center w-full bg-indigo-600 text-white py-2.5 rounded-lg font-medium hover:bg-indigo-700 transition-colors">
                    View Isolation Report
                </button>
            </div>

            <div class="bg-white p-6 rounded-2xl border border-slate-200 shadow-sm hover:shadow-md transition-shadow">
                <div class="bg-slate-100 text-slate-700 w-12 h-12 rounded-xl flex items-center justify-center mb-4">
                    <svg class="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"></path>
                    </svg>
                </div>
                <h3 class="text-lg font-bold text-slate-800 mb-2">Full Dependency Report</h3>
                <p class="text-sm text-slate-600 mb-6">Complete inventory of all provided and required endpoints across all services, with each dependency's pinned version.</p>
                <button onclick="openScopedReport('/report/markdown', 'Full Dependency Report')"
                   class="inline-flex items-center justify-center w-full bg-indigo-600 text-white py-2.5 rounded-lg font-medium hover:bg-indigo-700 transition-colors">
                    View Markdown Report
                </button>
            </div>
        `;
}

window.addEventListener("sanshain-update", () => {
  console.log("Live update received, refreshing reports...");
  showReports();
});

function initReports() {
  checkDiscoveryAuth(() => showReports());
}
if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", initReports);
} else {
  initReports();
}
