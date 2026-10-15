/**
 * discovery.js — Shared utilities for the Sanshain discovery pages
 * (producers.html, consumers.html, graph.html, reports.html)
 */

let allServices = []; // ProducerSummary[]: { name, versions[], is_favorite, icon, domain }
let userFavorites = { services: [], clients: [] };

const YAML_PAGE_SIZE = 80; // lines per page for YAML viewer

async function fetchJSON(url) {
  const res = await apiCall(url);
  if (!res.ok) throw new Error(`Fetch error: ${res.status}`);
  return res.json();
}

// Load every Producer with its version lines (all API types) in one request.
async function loadAllProducers() {
  allServices = (await fetchJSON("/admin/producers")) || [];
}

function producerByName(name) {
  return allServices.find((s) => s.name === name) || null;
}

function producerVersions(name) {
  const svc = producerByName(name);
  return svc ? svc.versions || [] : [];
}

// A Producer "serves something" once at least one entry of its version lines
// has at least one endpoint. Used by the discovery view (producers.html) to
// hide Producers that provide nothing — e.g. a spec published with no paths.
function producerHasAnyEndpoints(name) {
  return producerVersions(name).some((v) => (v.endpoint_count || 0) > 0);
}

// --- Semver helpers (versions are strict MAJOR.MINOR.PATCH strings) ---

function parseSemver(v) {
  return String(v)
    .split(".")
    .map((n) => parseInt(n, 10) || 0);
}

function compareSemver(a, b) {
  const pa = parseSemver(a);
  const pb = parseSemver(b);
  for (let i = 0; i < 3; i++) {
    if ((pa[i] || 0) !== (pb[i] || 0)) return (pa[i] || 0) - (pb[i] || 0);
  }
  return 0;
}

// The highest GA version of one line (an array of version entries), or null
// when the line has no GA yet.
function latestGaVersion(versions) {
  const gas = versions.filter((v) => v.stability === "ga").map((v) => v.version);
  gas.sort(compareSemver);
  return gas.length > 0 ? gas[gas.length - 1] : null;
}

// SNAPSHOT / GA badge markup for a version entry.
function stabilityBadge(stability) {
  return stability === "ga"
    ? '<span class="text-[10px] text-green-700 bg-green-100 px-2 py-0.5 rounded-full font-bold uppercase tracking-tighter">GA</span>'
    : '<span class="text-[10px] text-amber-700 bg-amber-100 px-2 py-0.5 rounded-full font-bold uppercase tracking-tighter">Snapshot</span>';
}

async function loadUserFavorites() {
  try {
    const favorites = await fetchJSON("/auth/favorites");
    userFavorites = favorites || { services: [], clients: [] };
  } catch (err) {
    console.error("Failed to load user favorites:", err);
    userFavorites = { services: [], clients: [] };
  }
}

async function toggleFavorite(event, itemType, itemName, currentIsFavorite) {
  if (event) {
    event.stopPropagation();
    event.preventDefault();
  }
  const token = localStorage.getItem("sanshain_token");
  const headers = {};
  if (token) headers["Authorization"] = `Bearer ${token}`;

  const method = currentIsFavorite ? "DELETE" : "POST";
  try {
    const res = await fetch(`/auth/favorites/${itemType}/${encodeURIComponent(itemName)}`, {
      method,
      headers,
    });
    if (!res.ok) throw new Error(`Failed to toggle favorite: ${res.status}`);

    // Update local cache
    const list = itemType === "service" ? userFavorites.services : userFavorites.clients;
    if (currentIsFavorite) {
      const idx = list.indexOf(itemName);
      if (idx !== -1) list.splice(idx, 1);
    } else {
      if (!list.includes(itemName)) list.push(itemName);
    }

    // Refresh page/lists
    if (window.loadData) {
      await window.loadData();
    } else if (window.location.reload) {
      window.location.reload();
    }
  } catch (err) {
    console.error("Error toggling favorite:", err);
    alert("Could not update favorite. Please try again.");
  }
}

function getMethodColor(m) {
  const colors = {
    GET: "bg-blue-100 text-blue-700",
    POST: "bg-green-100 text-green-700",
    PUT: "bg-amber-100 text-amber-700",
    DELETE: "bg-red-100 text-red-700",
    PATCH: "bg-purple-100 text-purple-700",
  };
  return colors[m.toUpperCase()] || "bg-slate-100 text-slate-700";
}

function renderPaginatedYaml(container, yamlText, filename) {
  filename = filename || "endpoint.yaml";
  const lines = yamlText.split("\n");
  const totalPages = Math.max(1, Math.ceil(lines.length / YAML_PAGE_SIZE));
  let currentPage = 1;

  function render() {
    const start = (currentPage - 1) * YAML_PAGE_SIZE;
    const end = Math.min(start + YAML_PAGE_SIZE, lines.length);
    const pageLines = lines.slice(start, end);
    const escaped = pageLines.join("\n").replace(/</g, "&lt;").replace(/>/g, "&gt;");

    let paginationHtml = "";
    if (totalPages > 1) {
      paginationHtml = `
                <div class="flex items-center justify-between mt-4">
                    <button id="yaml-prev" class="px-3 py-1.5 rounded-lg text-sm font-medium ${currentPage <= 1 ? "bg-slate-100 text-slate-500 cursor-not-allowed" : "bg-indigo-600 text-white hover:bg-indigo-700"}" ${currentPage <= 1 ? "disabled" : ""}>← Previous</button>
                    <span class="text-sm text-slate-500">Page ${currentPage} of ${totalPages} &middot; Lines ${start + 1}–${end} of ${lines.length}</span>
                    <button id="yaml-next" class="px-3 py-1.5 rounded-lg text-sm font-medium ${currentPage >= totalPages ? "bg-slate-100 text-slate-500 cursor-not-allowed" : "bg-indigo-600 text-white hover:bg-indigo-700"}" ${currentPage >= totalPages ? "disabled" : ""}>Next →</button>
                </div>`;
    }

    container.innerHTML = `
            <div class="flex items-center justify-between mb-2">
                <span class="text-xs text-slate-500">${lines.length} lines total</span>
                <div class="flex gap-2">
                    <button id="yaml-copy" class="inline-flex items-center gap-1 px-2.5 py-1 rounded-lg text-xs font-medium bg-slate-100 text-slate-600 hover:bg-slate-200 transition-colors" title="Copy to clipboard">
                        <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><rect x="9" y="9" width="13" height="13" rx="2" ry="2" stroke-width="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1" stroke-width="2"/></svg>
                        Copy
                    </button>
                    <button id="yaml-download" class="inline-flex items-center gap-1 px-2.5 py-1 rounded-lg text-xs font-medium bg-slate-100 text-slate-600 hover:bg-slate-200 transition-colors" title="Download YAML file">
                        <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 16v2a2 2 0 002 2h12a2 2 0 002-2v-2M7 10l5 5 5-5M12 15V3"/></svg>
                        Download
                    </button>
                </div>
            </div>
            <pre class="bg-slate-800 text-green-300 p-4 rounded-xl text-sm overflow-x-auto whitespace-pre-wrap leading-relaxed">${escaped}</pre>
            ${paginationHtml}`;

    if (totalPages > 1) {
      const prev = document.getElementById("yaml-prev");
      const next = document.getElementById("yaml-next");
      if (prev && currentPage > 1)
        prev.onclick = () => {
          currentPage--;
          render();
        };
      if (next && currentPage < totalPages)
        next.onclick = () => {
          currentPage++;
          render();
        };
    }

    const copyBtn = document.getElementById("yaml-copy");
    if (copyBtn)
      copyBtn.onclick = () => {
        navigator.clipboard.writeText(yamlText).then(() => {
          copyBtn.innerHTML = `<svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/></svg> Copied!`;
          setTimeout(() => {
            copyBtn.innerHTML = `<svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><rect x="9" y="9" width="13" height="13" rx="2" ry="2" stroke-width="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1" stroke-width="2"/></svg> Copy`;
          }, 2000);
        });
      };

    const dlBtn = document.getElementById("yaml-download");
    if (dlBtn)
      dlBtn.onclick = () => {
        const blob = new Blob([yamlText], { type: "application/x-yaml" });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = filename;
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
      };
  }
  render();
}

// --- Diff views (Producers page line diff, yaml.html) ---------------------
// Both views render the server's `similar` diff (format=json hunks from
// /admin/producers/{name}/diff). Two switches: Ignore whitespace (off by
// default, per view) and Unified / Side by side (side by side by default on
// wide screens, remembered per browser; narrow screens always unified).

const DIFF_LAYOUT_STORAGE_KEY = "sanshain.diffLayout";
const DIFF_NARROW_QUERY = "(max-width: 767px)";
const diffViewStates = new WeakMap();

function storedDiffLayout() {
  try {
    return localStorage.getItem(DIFF_LAYOUT_STORAGE_KEY);
  } catch (e) {
    return null;
  }
}

function storeDiffLayout(layout) {
  try {
    localStorage.setItem(DIFF_LAYOUT_STORAGE_KEY, layout);
  } catch (e) {
    // Storage unavailable: the choice lasts until the page is left.
  }
}

function isNarrowDiffScreen() {
  return window.matchMedia(DIFF_NARROW_QUERY).matches;
}

function effectiveDiffLayout() {
  if (isNarrowDiffScreen()) return "unified";
  return storedDiffLayout() === "unified" ? "unified" : "split";
}

function diffLineNo(n) {
  return `<span class="diff-no w-10 shrink-0 text-right pr-2 text-slate-500 select-none">${n == null ? "" : n}</span>`;
}

const DIFF_LINE_STYLES = {
  add: { cls: "bg-green-950/40 text-green-300", sign: "+" },
  del: { cls: "bg-red-950/40 text-red-300", sign: "-" },
  ctx: { cls: "text-slate-300", sign: " " },
};

function renderUnifiedDiffRows(hunks) {
  return hunks
    .map((h) => {
      const header = `<div class="diff-hunk-header text-indigo-400 px-2 py-0.5 select-none">@@ -${h.old_start},${h.old_len} +${h.new_start},${h.new_len} @@</div>`;
      const lines = h.lines
        .map((l) => {
          const style = DIFF_LINE_STYLES[l.op];
          return `<div class="diff-line flex ${style.cls}" data-diff="${l.op}">${diffLineNo(l.old_no)}${diffLineNo(l.new_no)}<span class="w-4 shrink-0 select-none">${style.sign}</span><span class="whitespace-pre">${escapeHtml(l.text)}</span></div>`;
        })
        .join("");
      return header + lines;
    })
    .join("");
}

function diffSideCell(side, no, text, op) {
  if (no == null) {
    return `<div class="diff-cell bg-slate-900/60" data-side="${side}"></div>`;
  }
  const style = DIFF_LINE_STYLES[op];
  return `<div class="diff-cell flex min-w-0 ${style.cls}" data-side="${side}">${diffLineNo(no)}<span class="whitespace-pre-wrap break-all min-w-0">${escapeHtml(text)}</span></div>`;
}

// Pair each run of deletions with the following run of insertions so a
// changed line sits next to its replacement.
function renderSplitDiffRows(hunks) {
  return hunks
    .map((h) => {
      const rows = [
        `<div class="diff-hunk-header text-indigo-400 px-2 py-0.5 select-none">@@ -${h.old_start},${h.old_len} +${h.new_start},${h.new_len} @@</div>`,
      ];
      let i = 0;
      while (i < h.lines.length) {
        const l = h.lines[i];
        if (l.op === "ctx") {
          rows.push(
            `<div class="diff-row grid grid-cols-2 divide-x divide-slate-800" data-diff="ctx">${diffSideCell("old", l.old_no, l.old_text ?? l.text, "ctx")}${diffSideCell("new", l.new_no, l.text, "ctx")}</div>`,
          );
          i++;
          continue;
        }
        const dels = [];
        const adds = [];
        while (i < h.lines.length && h.lines[i].op !== "ctx") {
          (h.lines[i].op === "del" ? dels : adds).push(h.lines[i]);
          i++;
        }
        for (let k = 0; k < Math.max(dels.length, adds.length); k++) {
          const d = dels[k];
          const a = adds[k];
          const kind = d && a ? "change" : d ? "del" : "add";
          rows.push(
            `<div class="diff-row grid grid-cols-2 divide-x divide-slate-800" data-diff="${kind}">${diffSideCell("old", d ? d.old_no : null, d ? d.text : "", "del")}${diffSideCell("new", a ? a.new_no : null, a ? a.text : "", "add")}</div>`,
          );
        }
      }
      return rows.join("");
    })
    .join("");
}

// Mount (or refresh) a diff view in `container`. `options.load(ignoreWhitespace)`
// resolves to the JSON diff; `options.caption` is trusted HTML shown in the
// toolbar; `options.emptyHtml` replaces the body when nothing differs. The
// whitespace switch survives re-mounts into the same container.
async function mountDiffView(container, options) {
  const state = diffViewStates.get(container) || { ignoreWhitespace: false };
  diffViewStates.set(container, state);
  state.options = options;
  // `options.ignoreWhitespaceSwitch === false` omits the switch (and compares
  // whitespace) where it could have no effect.
  const wsSwitch = options.ignoreWhitespaceSwitch !== false;
  if (!wsSwitch) state.ignoreWhitespace = false;
  const token = {};
  state.token = token;

  container.innerHTML = `<div class="diff-view flex flex-col border border-slate-800 rounded-lg overflow-hidden bg-slate-950 font-mono text-xs">
      <div class="diff-toolbar flex flex-wrap items-center justify-between gap-3 bg-slate-900 px-4 py-2 border-b border-slate-800 text-slate-400 font-sans">
        <div class="diff-caption">${options.caption || ""}</div>
        <div class="flex flex-wrap items-center gap-3">
          ${
            wsSwitch
              ? `<label class="inline-flex items-center gap-1.5 cursor-pointer select-none">
            <input type="checkbox" class="diff-ignore-ws" ${state.ignoreWhitespace ? "checked" : ""} />
            <span>Ignore whitespace</span>
          </label>`
              : ""
          }
          <div class="diff-layout-switch inline-flex rounded-md bg-slate-800 p-0.5">
            <button type="button" data-layout="unified" class="diff-layout-btn px-2 py-0.5 rounded-sm">Unified</button>
            <button type="button" data-layout="split" class="diff-layout-btn px-2 py-0.5 rounded-sm">Side by side</button>
          </div>
        </div>
      </div>
      <div class="diff-body overflow-x-auto py-1"><div class="text-slate-500 italic px-4 py-2">Loading diff…</div></div></div>`;
  const body = container.querySelector(".diff-body");
  const view = container.querySelector(".diff-view");
  const layoutSwitch = container.querySelector(".diff-layout-switch");

  const renderBody = () => {
    const layout = effectiveDiffLayout();
    view.dataset.layout = layout;
    layoutSwitch.classList.toggle("hidden", isNarrowDiffScreen());
    layoutSwitch.querySelectorAll(".diff-layout-btn").forEach((btn) => {
      const active = btn.dataset.layout === layout;
      btn.classList.toggle("bg-slate-600", active);
      btn.classList.toggle("text-white", active);
      btn.setAttribute("aria-pressed", active ? "true" : "false");
    });
    if (!state.diff) return;
    if (state.diff.hunks.length === 0) {
      body.innerHTML = `<div class="diff-empty text-slate-400 italic px-4 py-2 font-sans">${options.emptyHtml || "No differences."}</div>`;
      return;
    }
    body.innerHTML =
      layout === "split"
        ? renderSplitDiffRows(state.diff.hunks)
        : renderUnifiedDiffRows(state.diff.hunks);
  };

  const load = async () => {
    state.diff = null;
    renderBody();
    try {
      const diff = await options.load(state.ignoreWhitespace);
      if (state.token !== token) return; // a newer mount replaced this one
      state.diff = diff;
      renderBody();
    } catch (e) {
      if (state.token !== token) return;
      body.innerHTML = `<div class="text-red-400 px-4 py-2 font-sans">Failed to load diff: ${escapeHtml(e.message)}</div>`;
    }
  };

  const wsToggle = container.querySelector(".diff-ignore-ws");
  if (wsToggle) {
    wsToggle.addEventListener("change", (e) => {
      state.ignoreWhitespace = e.target.checked;
      load();
    });
  }
  layoutSwitch.querySelectorAll(".diff-layout-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      storeDiffLayout(btn.dataset.layout);
      renderBody();
    });
  });
  const narrow = window.matchMedia(DIFF_NARROW_QUERY);
  const onResize = () => {
    if (!container.isConnected || state.token !== token) {
      narrow.removeEventListener("change", onResize);
      return;
    }
    renderBody();
  };
  narrow.addEventListener("change", onResize);

  await load();
}

// Fetch one version-to-version diff of a Producer's line from the server.
// `extra` adds query parameters (path, method, context, ...).
async function fetchVersionDiff(serviceName, apiType, from, to, extra) {
  const q = new URLSearchParams({ api_type: apiType, from, to, ...extra });
  const res = await apiCall(`/admin/producers/${encodeURIComponent(serviceName)}/diff?${q}`);
  if (!res.ok) {
    throw new Error(`HTTP ${res.status}`);
  }
  return extra && extra.format === "json" ? res.json() : res.text();
}

/// Resolve who the caller is, then hand that to `onSuccess`.
///
/// The callback ALWAYS receives the resolved user, or `null` when the caller is
/// not identified. Callers that gate admin-only affordances depend on it:
/// invoking the callback with no argument leaves their `user` parameter
/// `undefined`, which reads as not-an-admin and silently disables the feature
/// for everyone.
async function checkDiscoveryAuth(onSuccess) {
  if (window.SANSHAIN_FAST_SCREENSHOT) {
    console.log("Fast screenshot mode: bypassing auth check");
    if (onSuccess) await onSuccess(null);
    return;
  }
  const token = getSanshainToken();

  // If we have a token, we always try to use it
  if (token) {
    try {
      const res = await apiCall("/auth/me");
      if (res.ok) {
        const user = await res.json();
        renderBanner(user);
        if (onSuccess) await onSuccess(user);
        return;
      }
    } catch (err) {
      console.error("Auth check failed:", err);
    }
  }

  if (window.SANSHAIN_FAST_SCREENSHOT) {
    console.log("Fast screenshot mode: skipping auth redirect");
    return;
  }
  window.location.href = "/account.html";
}

function closeModal() {
  document.getElementById("modal").classList.add("hidden");
  const reportBtn = document.getElementById("md-report-btn");
  if (reportBtn) reportBtn.style.display = "";
}
