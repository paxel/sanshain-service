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

function simpleDiff(a, b) {
  const aLines = a.split("\n");
  const bLines = b.split("\n");
  const result = [];
  const m = aLines.length,
    n = bLines.length;
  if (m + n > 5000) {
    let ai = 0,
      bi = 0;
    while (ai < m || bi < n) {
      if (ai < m && bi < n && aLines[ai] === bLines[bi]) {
        result.push({ type: "ctx", line: aLines[ai] });
        ai++;
        bi++;
      } else if (bi < n) {
        result.push({ type: "add", line: bLines[bi] });
        bi++;
      } else {
        result.push({ type: "del", line: aLines[ai] });
        ai++;
      }
    }
    return result;
  }
  const dp = Array.from({ length: m + 1 }, () => new Uint16Array(n + 1));
  for (let i = m - 1; i >= 0; i--)
    for (let j = n - 1; j >= 0; j--)
      dp[i][j] =
        aLines[i] === bLines[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
  let i = 0,
    j = 0;
  while (i < m || j < n) {
    if (i < m && j < n && aLines[i] === bLines[j]) {
      result.push({ type: "ctx", line: aLines[i] });
      i++;
      j++;
    } else if (j < n && (i >= m || dp[i][j + 1] >= dp[i + 1][j])) {
      result.push({ type: "add", line: bLines[j] });
      j++;
    } else {
      result.push({ type: "del", line: aLines[i] });
      i++;
    }
  }
  return result;
}

function renderDiffHtml(diffLines) {
  const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  return diffLines
    .map((d) => {
      if (d.type === "add") return `<span class="text-green-400">+ ${esc(d.line)}</span>`;
      if (d.type === "del") return `<span class="text-red-400">- ${esc(d.line)}</span>`;
      return `<span class="text-slate-500">  ${esc(d.line)}</span>`;
    })
    .join("\n");
}

// Colorize a server-produced unified diff (text/plain from
// /admin/producers/{name}/diff) for display in a dark <pre> block.
function renderUnifiedDiffHtml(diffText) {
  const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  return diffText
    .split("\n")
    .map((line) => {
      if (line.startsWith("+++") || line.startsWith("---"))
        return `<span class="text-slate-300 font-bold">${esc(line)}</span>`;
      if (line.startsWith("@@")) return `<span class="text-indigo-400">${esc(line)}</span>`;
      if (line.startsWith("+")) return `<span class="text-green-400">${esc(line)}</span>`;
      if (line.startsWith("-")) return `<span class="text-red-400">${esc(line)}</span>`;
      return `<span class="text-slate-400">${esc(line)}</span>`;
    })
    .join("\n");
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
