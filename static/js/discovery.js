/**
 * discovery.js — Shared utilities for the Sanshain discovery pages
 * (producers.html, consumers.html, graph.html, reports.html)
 */

let allServices = [];
let allServiceBranches = {}; // cache: serviceName -> branches[] (unfiltered — every branch, including empty ones)
let allServiceLastPublished = {}; // cache: serviceName -> { branch: isoTimestamp }
let allServiceExpireAt = {}; // cache: serviceName -> { branch: isoTimestamp } (stale-cleanup TTL)
let allServiceEndpointCount = {}; // cache: serviceName -> { branch: count } (0 = branch serves nothing)
let userFavorites = { services: [], clients: [] };

// A branch "serves something" once it has at least one endpoint. Used by the
// discovery view (producers.html) to hide branches/services that provide
// nothing — e.g. an OpenAPI spec published with no paths. Other consumers of
// allServiceBranches (reports.html, graph.html branch selectors) intentionally
// keep offering every branch, including empty ones.
function branchHasEndpoints(serviceName, branch) {
  const counts = allServiceEndpointCount[serviceName] || {};
  return (counts[branch] || 0) > 0;
}

function serviceHasAnyEndpoints(serviceName) {
  const branches = allServiceBranches[serviceName] || [];
  return branches.some((b) => branchHasEndpoints(serviceName, b));
}
const YAML_PAGE_SIZE = 80; // lines per page for YAML viewer

async function fetchJSON(url) {
  const res = await apiCall(url);
  if (!res.ok) throw new Error(`Fetch error: ${res.status}`);
  return res.json();
}

async function loadAllServiceBranches() {
  const services = await fetchJSON("/admin/producers");
  allServices = services || [];
  allServiceBranches = {};
  allServiceLastPublished = {};
  allServiceExpireAt = {};
  allServiceEndpointCount = {};
  for (const svc of allServices) {
    allServiceBranches[svc.name] = svc.branches || [];
    allServiceLastPublished[svc.name] = svc.branches_last_published || {};
    allServiceExpireAt[svc.name] = svc.branches_expire_at || {};
    allServiceEndpointCount[svc.name] = svc.branches_endpoint_count || {};
  }
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

let allBranchesMetadata = [];

async function loadBranchesMetadata() {
  try {
    allBranchesMetadata = await fetchJSON("/branches/metadata");
  } catch (err) {
    console.error("Failed to load branches metadata:", err);
    allBranchesMetadata = [];
  }
}

function isBranchProtected(branchName, protectedPatterns) {
  if (!protectedPatterns || !Array.isArray(protectedPatterns)) return false;
  return protectedPatterns.some((pattern) => {
    if (pattern === branchName) return true;
    const regexStr =
      "^" + pattern.replace(/[-\/\\^$+?.()|[\]{}]/g, "\\$&").replace(/\*/g, ".*") + "$";
    const regex = new RegExp(regexStr);
    return regex.test(branchName);
  });
}

function sortBranchNames(branchNames, protectedBranches) {
  const list = Array.from(branchNames);
  const lastModifiedMap = {};
  for (const b of allBranchesMetadata) {
    lastModifiedMap[b.name] = b.last_modified;
  }

  return list.sort((a, b) => {
    const aProt = isBranchProtected(a, protectedBranches);
    const bProt = isBranchProtected(b, protectedBranches);

    if (aProt && !bProt) return -1;
    if (!aProt && bProt) return 1;

    const aTime = lastModifiedMap[a] || "1970-01-01T00:00:00Z";
    const bTime = lastModifiedMap[b] || "1970-01-01T00:00:00Z";

    if (aTime !== bTime) {
      return bTime.localeCompare(aTime); // descending (newest first)
    }
    return a.localeCompare(b);
  });
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

function renderVersionHistory(container, versions) {
  versions.sort((a, b) => a.version - b.version);

  let html = `<div class="space-y-3">
        <div class="flex items-center gap-3 mb-4">
            <label class="text-sm font-medium text-slate-600">Compare:</label>
            <select id="diff-from" class="text-sm border border-slate-300 rounded-lg px-2 py-1">
                ${versions.map((v) => `<option value="${v.version}"${v.version === versions[0].version ? " selected" : ""}>v${v.version} (${v.created_at})</option>`).join("")}
            </select>
            <span class="text-slate-500">→</span>
            <select id="diff-to" class="text-sm border border-slate-300 rounded-lg px-2 py-1">
                ${versions.map((v) => `<option value="${v.version}"${v.version === versions[versions.length - 1].version ? " selected" : ""}>v${v.version} (${v.created_at})</option>`).join("")}
            </select>
            <button id="diff-go" class="px-3 py-1 text-sm font-medium bg-indigo-600 text-white rounded-lg hover:bg-indigo-700">Diff</button>
        </div>
        <div id="diff-output"></div>
        <h4 class="text-sm font-semibold text-slate-700 mt-6 mb-2">All Versions</h4>`;

  for (const v of [...versions].reverse()) {
    const hasDiff = v.diff_from_previous && v.diff_from_previous.trim().length > 0;
    html += `
        <div class="bg-white border border-slate-200 rounded-xl p-3">
            <div class="flex items-center justify-between">
                <div>
                    <span class="text-sm font-semibold text-slate-700">Version ${v.version}</span>
                    <span class="text-xs text-slate-500 ml-2">${v.created_at}</span>
                </div>
                <div class="flex gap-2">
                    <button class="ver-yaml-btn text-xs px-2 py-1 bg-slate-100 text-slate-600 rounded hover:bg-slate-200" data-version="${v.version}">View YAML</button>
                    ${hasDiff ? `<button class="ver-diff-btn text-xs px-2 py-1 bg-amber-100 text-amber-700 rounded hover:bg-amber-200" data-version="${v.version}">Diff from v${v.version - 1}</button>` : ""}
                </div>
            </div>
            <div id="ver-detail-${v.version}" class="hidden mt-3"></div>
        </div>`;
  }
  html += "</div>";
  container.innerHTML = html;

  const versionMap = {};
  versions.forEach((v) => (versionMap[v.version] = v));

  document.getElementById("diff-go").onclick = () => {
    const fromV = parseInt(document.getElementById("diff-from").value);
    const toV = parseInt(document.getElementById("diff-to").value);
    const output = document.getElementById("diff-output");
    if (fromV === toV) {
      output.innerHTML = '<div class="text-sm text-slate-500 italic">Same version selected.</div>';
      return;
    }
    const vFrom = versionMap[fromV];
    const vTo = versionMap[toV];
    if (!vFrom || !vTo) {
      output.innerHTML = '<div class="text-sm text-red-500">Version not found.</div>';
      return;
    }
    const diff = simpleDiff(vFrom.yaml_content, vTo.yaml_content);
    const adds = diff.filter((d) => d.type === "add").length;
    const dels = diff.filter((d) => d.type === "del").length;
    output.innerHTML = `
            <div class="text-xs text-slate-500 mb-1">v${fromV} → v${toV}: <span class="text-green-600">+${adds}</span> / <span class="text-red-600">-${dels}</span> lines</div>
            <pre class="diff-pre bg-slate-800 p-4 rounded-xl text-sm overflow-x-auto whitespace-pre-wrap leading-relaxed">${renderDiffHtml(diff)}</pre>`;
  };

  container.querySelectorAll(".ver-yaml-btn").forEach((btn) => {
    btn.onclick = () => {
      const ver = parseInt(btn.dataset.version);
      const detail = document.getElementById(`ver-detail-${ver}`);
      if (!detail.classList.contains("hidden")) {
        detail.classList.add("hidden");
        return;
      }
      detail.classList.remove("hidden");
      const v = versionMap[ver];
      const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
      detail.innerHTML = `<pre class="bg-slate-800 text-green-300 p-4 rounded-xl text-sm overflow-x-auto whitespace-pre-wrap leading-relaxed">${esc(v.yaml_content)}</pre>`;
    };
  });

  container.querySelectorAll(".ver-diff-btn").forEach((btn) => {
    btn.onclick = () => {
      const ver = parseInt(btn.dataset.version);
      const detail = document.getElementById(`ver-detail-${ver}`);
      if (!detail.classList.contains("hidden")) {
        detail.classList.add("hidden");
        return;
      }
      detail.classList.remove("hidden");
      const v = versionMap[ver];
      const prev = versionMap[ver - 1];
      if (prev) {
        const diff = simpleDiff(prev.yaml_content, v.yaml_content);
        const adds = diff.filter((d) => d.type === "add").length;
        const dels = diff.filter((d) => d.type === "del").length;
        detail.innerHTML = `
                    <div class="text-xs text-slate-500 mb-1">v${ver - 1} → v${ver}: <span class="text-green-600">+${adds}</span> / <span class="text-red-600">-${dels}</span></div>
                    <pre class="diff-pre bg-slate-800 p-4 rounded-xl text-sm overflow-x-auto whitespace-pre-wrap leading-relaxed">${renderDiffHtml(diff)}</pre>`;
      } else if (v.diff_from_previous) {
        const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
        const coloredDiff = v.diff_from_previous
          .split("\n")
          .map((line) => {
            if (line.startsWith("+"))
              return `<span class="text-green-400">` + esc(line) + `</span>`;
            if (line.startsWith("-")) return `<span class="text-red-400">` + esc(line) + `</span>`;
            return `<span class="text-slate-300">` + esc(line) + `</span>`;
          })
          .join("\n");
        detail.innerHTML = `<pre class="diff-pre bg-slate-800 p-4 rounded-xl text-sm overflow-x-auto whitespace-pre-wrap leading-relaxed">${coloredDiff}</pre>`;
      }
    };
  });
}

async function checkDiscoveryAuth(onSuccess) {
  if (window.SANSHAIN_FAST_SCREENSHOT) {
    console.log("Fast screenshot mode: bypassing auth check");
    if (onSuccess) await onSuccess();
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
        if (onSuccess) await onSuccess();
        return;
      }
    } catch (err) {
      console.error("Auth check failed:", err);
    }
  }

  // Fallback: check if dev mode is enabled
  try {
    const devRes = await fetch("/admin/settings/dev-mode");
    if (devRes.ok) {
      const devData = await devRes.json();
      if (devData.enabled) {
        renderBanner(null);
        if (onSuccess) await onSuccess();
        return;
      }
    }
  } catch (e) {}

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
