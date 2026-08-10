// Extracted from static/graph.html (ai/improvements.md #11): inline
// script moved to an external classic file so the CSP can forbid inline
// execution. Declarations stay global as when inline; the file loads at
// the same document position, so DOM timing is unchanged.

// apiCall throws on 401 after clearing the session; without this hook the
// page swallows it and leaves whatever was mid-flight looking stuck.
function onSessionExpired() {
  window.location.href = "/account.html";
}

let lastGraphReport = null;
let lastMermaidCode = "";
let currentGraphMode = "custom";
let currentGraphDirection = "TB";

function toggleGraphDirection() {
  currentGraphDirection = currentGraphDirection === "TB" ? "LR" : "TB";
  const label = document.getElementById("graph-direction-label");
  if (label) label.textContent = currentGraphDirection === "TB" ? "Vertical" : "Horizontal";
  if (currentGraphMode === "custom") {
    redrawGraph();
  }
}

function switchGraphMode(mode) {
  currentGraphMode = mode;
  ["custom", "mermaid", "detailed"].forEach((m) => {
    const btn = document.getElementById(`graph-mode-${m}`);
    if (btn) {
      btn.className =
        m === mode
          ? "px-3 py-1.5 bg-indigo-600 text-white"
          : "px-3 py-1.5 bg-white text-slate-600 hover:bg-slate-50";
    }
  });
  const dirBtn = document.getElementById("graph-direction-toggle");
  const copyBtn = document.getElementById("graph-copy");
  const dlBtn = document.getElementById("graph-download");
  const dlPngBtn = document.getElementById("graph-download-png");
  const toolbarRow = document.getElementById("graph-toolbar-row");

  if (dirBtn) dirBtn.classList.toggle("hidden", mode !== "custom");
  if (copyBtn) copyBtn.classList.remove("hidden");
  if (dlBtn) dlBtn.classList.remove("hidden");
  if (dlPngBtn) dlPngBtn.classList.remove("hidden");
  if (toolbarRow) toolbarRow.classList.remove("hidden");

  if (lastGraphReport) {
    const customSvg = document.getElementById("custom-graph");
    const mermaidDiv = document.getElementById("mermaid-graph");
    if (mode === "custom") {
      customSvg.classList.remove("hidden");
      mermaidDiv.classList.add("hidden");
      redrawGraph();
    } else {
      customSvg.classList.add("hidden");
      mermaidDiv.classList.remove("hidden");
      redrawGraph();
    }
  }
}

async function showGraph() {
  try {
    restoreGraphFiltersFromUrl();
    await loadGraph();
  } catch (err) {
    console.error("Failed to show graph:", err);
    hideLoader();
    const graphDiv = document.getElementById("mermaid-graph");
    if (graphDiv) {
      graphDiv.classList.remove("hidden");
      graphDiv.innerHTML = `<div class="text-red-500 p-4">Error: ${err.message}</div>`;
    }
  }
}

async function loadGraph() {
  showLoader();
  const customSvg = document.getElementById("custom-graph");
  const graphDiv = document.getElementById("mermaid-graph");
  if (currentGraphMode === "custom") {
    while (customSvg.firstChild) customSvg.removeChild(customSvg.firstChild);
    customSvg.classList.remove("hidden");
    graphDiv.classList.add("hidden");
  } else {
    customSvg.classList.add("hidden");
    graphDiv.classList.remove("hidden");
    graphDiv.innerHTML =
      '<div class="py-10 text-slate-500 italic">Generating dependency graph...</div>';
  }

  try {
    const report = await fetchJSON("/report");

    // Fetch Producer details for icons, domains and — decisive for the
    // Outdated highlight — the version lines, whose latest GA is the
    // reference an edge's Pin is compared against.
    try {
      const servicesDetailed = await fetchJSON("/admin/producers");
      report.services_detailed = servicesDetailed;
    } catch (e) {
      console.warn("Could not load service details:", e);
    }

    // The caller's permissions gate the create-branch-here affordance
    // (a usability affordance; the server's releaser gate decides).
    try {
      const me = await apiCall("/auth/me");
      if (me.ok) {
        const data = await me.json();
        window.graphUserPermissions = data.permissions || [];
      }
    } catch (e) {
      console.warn("Could not load permissions:", e);
    }

    // Branch selector (ADR-0005): every sanshain-branch is a view.
    try {
      const branches = await fetchJSON("/admin/branches");
      const sel = document.getElementById("graph-branch-select");
      if (sel) {
        const current = sel.value;
        sel.innerHTML = '<option value="">Branch…</option>';
        for (const b of branches) {
          const opt = document.createElement("option");
          opt.value = b.name;
          opt.textContent = b.name;
          sel.appendChild(opt);
        }
        sel.value = current;
      }
      // The compare panel offers the same selections on both sides.
      for (const id of ["diff-left", "diff-right"]) {
        const dsel = document.getElementById(id);
        if (!dsel) continue;
        const current = dsel.value;
        dsel.innerHTML = '<option value="main">main</option>';
        for (const b of branches) {
          const opt = document.createElement("option");
          opt.value = b.name;
          opt.textContent = b.name;
          dsel.appendChild(opt);
        }
        if (current) dsel.value = current;
      }
    } catch (e) {
      console.warn("Could not load branches:", e);
    }

    lastGraphReport = report;

    const datalist = document.getElementById("graph-services-list");
    if (datalist) {
      const services = new Set();
      report.dependency_graph.forEach((d) => {
        services.add(d.client);
        services.add(d.service);
      });
      datalist.innerHTML = "";
      [...services].sort().forEach((s) => {
        const opt = document.createElement("option");
        opt.value = s;
        datalist.appendChild(opt);
      });
    }

    if (report.dependency_graph.length === 0) {
      customSvg.classList.add("hidden");
      graphDiv.classList.remove("hidden");
      graphDiv.innerHTML =
        '<div class="py-10 text-slate-500 italic">No dependencies found to visualize.</div>';
      document.getElementById("graph-cycle-warning").classList.add("hidden");
      hideLoader();
      return;
    }

    updateGraphUrl(false);

    await new Promise((r) => setTimeout(r, 10));
    redrawGraph();
    hideLoader();
  } catch (err) {
    hideLoader();
    customSvg.classList.add("hidden");
    graphDiv.classList.remove("hidden");
    graphDiv.innerHTML = `<div class="text-red-500 p-4">Error generating graph: ${err.message}</div>`;
  }
}

function detectCycles(adjMap) {
  const WHITE = 0,
    GRAY = 1,
    BLACK = 2;
  const color = {};
  const parent = {};
  const cycleEdges = new Set();

  for (const node of adjMap.keys()) color[node] = WHITE;

  function dfs(u) {
    color[u] = GRAY;
    for (const v of adjMap.get(u) || []) {
      if (color[v] === undefined) color[v] = WHITE;
      if (color[v] === GRAY) {
        cycleEdges.add(`${u}-->${v}`);
        let cur = u;
        while (cur && cur !== v) {
          const p = parent[cur];
          if (p !== undefined) cycleEdges.add(`${p}-->${cur}`);
          cur = p;
        }
      } else if (color[v] === WHITE) {
        parent[v] = u;
        dfs(v);
      }
    }
    color[u] = BLACK;
  }

  for (const node of adjMap.keys()) {
    if (color[node] === WHITE) dfs(node);
  }
  return cycleEdges;
}

function sanitizeId(name) {
  return name.replace(/[^a-zA-Z0-9_]/g, "_");
}

async function renderGraph(report, detailed) {
  const graphDiv = document.getElementById("mermaid-graph");
  const warningDiv = document.getElementById("graph-cycle-warning");

  const allNodes = new Set();
  const adjMap = new Map();
  const edgeLabels = new Map();

  report.dependency_graph.forEach((d) => {
    const from = d.client;
    const to = d.service;
    allNodes.add(from);
    allNodes.add(to);
    if (!adjMap.has(from)) adjMap.set(from, []);
    if (!adjMap.get(from).includes(to)) adjMap.get(from).push(to);
    const key = `${from}-->${to}`;
    if (!edgeLabels.has(key)) edgeLabels.set(key, new Set());
    edgeLabels.get(key).add(`[${d.api_type || "REST"}] ${d.method} ${d.path} @${d.version}`);
  });

  const cycleEdges = detectCycles(adjMap);
  const hasCycles = cycleEdges.size > 0;

  if (hasCycles) {
    warningDiv.classList.remove("hidden");
  } else {
    warningDiv.classList.add("hidden");
  }

  const clientNodes = new Set(report.dependency_graph.map((d) => d.client));
  const serviceNodes = new Set(report.dependency_graph.map((d) => d.service));

  // Per-edge highlight flags (Outdated / Snapshot-pinned).
  const latestGaMap = graphLatestGaMap(report);
  const edgeFlags = graphEdgeFlags(report, latestGaMap);

  let mmd = "graph TD\n";

  mmd += "  classDef serviceOnly fill:#e0e7ff,stroke:#6366f1,stroke-width:2px,color:#3730a3\n";
  mmd += "  classDef clientOnly fill:#ccfbf1,stroke:#14b8a6,stroke-width:2px,color:#115e59\n";
  mmd += "  classDef both fill:#fef3c7,stroke:#f59e0b,stroke-width:2px,color:#92400e\n";
  mmd += "  classDef cycleEdge stroke:#ef4444,stroke-width:3px\n";
  mmd += "  linkStyle default stroke:#94a3b8,stroke-width:2px\n";

  for (const node of allNodes) {
    const id = sanitizeId(node);
    const isClient = clientNodes.has(node);
    const isService = serviceNodes.has(node);
    if (isClient && isService) {
      mmd += `  ${id}["${node}"]\n`;
    } else if (isService) {
      mmd += `  ${id}["${node}"]\n`;
    } else {
      mmd += `  ${id}("${node}")\n`;
    }
  }

  for (const node of allNodes) {
    const id = sanitizeId(node);
    const isClient = clientNodes.has(node);
    const isService = serviceNodes.has(node);
    if (isClient && isService) {
      mmd += `  class ${id} both\n`;
    } else if (isService) {
      mmd += `  class ${id} serviceOnly\n`;
    } else {
      mmd += `  class ${id} clientOnly\n`;
    }
  }

  let edgeIndex = 0;
  const cycleEdgeIndices = [];
  const outdatedEdgeIndices = [];
  const snapshotEdgeIndices = [];
  const uniqueEdges = new Set();
  report.dependency_graph.forEach((d) => {
    const key = `${d.client}-->${d.service}`;
    if (uniqueEdges.has(key)) return;
    uniqueEdges.add(key);

    const fromId = sanitizeId(d.client);
    const toId = sanitizeId(d.service);
    const labels = edgeLabels.get(key);
    const isCycle = cycleEdges.has(key);

    if (detailed && labels && labels.size > 0) {
      const labelText =
        [...labels].slice(0, 5).join("\\n") +
        (labels.size > 5 ? `\\n+${labels.size - 5} more` : "");
      mmd += `  ${fromId} -->|"${labelText}"| ${toId}\n`;
    } else {
      mmd += `  ${fromId} --> ${toId}\n`;
    }

    if (isCycle) cycleEdgeIndices.push(edgeIndex);
    const flags = edgeFlags.get(key);
    if (flags && flags.outdated) outdatedEdgeIndices.push(edgeIndex);
    if (flags && flags.snapshot) snapshotEdgeIndices.push(edgeIndex);
    edgeIndex++;
  });

  for (const idx of cycleEdgeIndices) {
    mmd += `  linkStyle ${idx} stroke:#ef4444,stroke-width:3px\n`;
  }
  // Highlight toggles: snapshot first, so an edge that is both keeps the
  // Outdated color, matching the custom renderer's precedence.
  if (window.graphHighlightFilters.snapshot) {
    for (const idx of snapshotEdgeIndices) {
      mmd += `  linkStyle ${idx} stroke:#f59e0b,stroke-width:3px\n`;
    }
  }
  if (window.graphHighlightFilters.outdated) {
    for (const idx of outdatedEdgeIndices) {
      mmd += `  linkStyle ${idx} stroke:#f43f5e,stroke-width:3px\n`;
    }
  }

  lastMermaidCode = mmd;

  graphDiv.removeAttribute("data-processed");
  graphDiv.innerHTML = mmd;
  await mermaid.run({ nodes: [graphDiv] });
}

document.getElementById("graph-copy").onclick = () => {
  let content;
  if (currentGraphMode === "custom") {
    const svg = document.getElementById("custom-graph");
    if (!svg.firstChild) return;
    content = getCustomGraphSVG(svg);
  } else {
    if (!lastMermaidCode) return;
    content = lastMermaidCode;
  }
  const btn = document.getElementById("graph-copy");
  navigator.clipboard.writeText(content).then(() => {
    btn.innerHTML = `<svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7"/></svg> Copied!`;
    setTimeout(() => {
      btn.innerHTML = `<svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><rect x="9" y="9" width="13" height="13" rx="2" ry="2" stroke-width="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1" stroke-width="2"/></svg> Copy`;
    }, 2000);
  });
};

// Exports carry their scope (ADR-0005): view, branch and instant — a
// screenshot of a release graph must not masquerade as current truth.
function graphScopeStamp() {
  const view = window.graphStreamView || "dev";
  let stamp = view === "branch" ? `branch ${window.graphBranchName}` : view;
  if (window.graphTimelineAt) stamp += ` @ ${window.graphTimelineAt}`;
  return stamp;
}

function graphScopeSlug() {
  return graphScopeStamp().replace(/[^a-zA-Z0-9.@-]+/g, "_");
}

// Compare two graph selections (ADR-0005) and render the structured diff.
async function runGraphDiff() {
  const sel = (idSel, idAt) => {
    let v = document.getElementById(idSel).value;
    const at = document.getElementById(idAt).value;
    if (at) v += `@${new Date(at).toISOString()}`;
    return v;
  };
  const left = sel("diff-left", "diff-left-at");
  const right = sel("diff-right", "diff-right-at");
  const out = document.getElementById("graph-diff-output");
  out.innerHTML = '<div class="text-sm text-slate-500 italic">Comparing…</div>';
  let res;
  try {
    res = await apiCall(
      `/admin/graph/diff?left=${encodeURIComponent(left)}&right=${encodeURIComponent(right)}`,
    );
  } catch (e) {
    // A thrown call (expired session, network) must not leave the panel
    // reading "Comparing…" forever.
    out.innerHTML = `<div class="text-sm text-red-500">Diff failed: ${escapeHtml(e.message || String(e))}</div>`;
    return;
  }
  if (!res.ok) {
    out.innerHTML = `<div class="text-sm text-red-500">Diff failed: ${escapeHtml(await errorMessage(res))}</div>`;
    return;
  }
  const diff = await res.json();
  const rows = [];
  for (const s of diff.services_added)
    rows.push(`<div class="text-green-700">+ service ${escapeHtml(s)}</div>`);
  for (const s of diff.services_removed)
    rows.push(`<div class="text-red-700">− service ${escapeHtml(s)}</div>`);
  for (const p of diff.pins_added)
    rows.push(
      `<div class="text-green-700">+ ${escapeHtml(p.client)} → ${escapeHtml(p.service)} ${escapeHtml(p.method)} ${escapeHtml(p.path)} @ ${escapeHtml(p.version)}</div>`,
    );
  for (const p of diff.pins_removed)
    rows.push(
      `<div class="text-red-700">− ${escapeHtml(p.client)} → ${escapeHtml(p.service)} ${escapeHtml(p.method)} ${escapeHtml(p.path)} @ ${escapeHtml(p.version)}</div>`,
    );
  for (const p of diff.pins_changed)
    rows.push(
      `<div class="text-amber-700">~ ${escapeHtml(p.client)} → ${escapeHtml(p.service)} ${escapeHtml(p.method)} ${escapeHtml(p.path)}: ${escapeHtml(p.from)} → ${escapeHtml(p.to)}</div>`,
    );
  out.innerHTML = `
            <div class="bg-white border border-slate-200 rounded-xl p-4 text-sm font-mono">
                <div class="text-xs text-slate-500 mb-2 font-sans">${escapeHtml(diff.left)} → ${escapeHtml(diff.right)}</div>
                ${rows.length ? rows.join("") : '<div class="text-slate-500 italic font-sans">No differences.</div>'}
            </div>`;
}
window.runGraphDiff = runGraphDiff;

document.getElementById("graph-download").onclick = () => {
  let content, filename, mime;
  if (currentGraphMode === "custom") {
    const svg = document.getElementById("custom-graph");
    if (!svg.firstChild) return;
    content = getCustomGraphSVG(svg);
    // Stamp the scope into the document itself, not only the name.
    // The stamp carries a user-chosen branch name into XML text, so it
    // is escaped — an unescaped '&' alone already breaks the document.
    content = content.replace(
      "</svg>",
      `<text x="8" y="16" font-size="12" fill="#64748b" font-family="ui-monospace, monospace">Sanshain — ${escapeHtml(graphScopeStamp())}</text></svg>`,
    );
    filename = `dependency_graph_${graphScopeSlug()}.svg`;
    mime = "image/svg+xml";
  } else {
    if (!lastMermaidCode) return;
    content = lastMermaidCode;
    filename = `dependency_graph_${graphScopeSlug()}.mmd`;
    mime = "text/plain";
  }
  const blob = new Blob([content], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
};

document.getElementById("graph-download-png").onclick = () => {
  exportToPng(currentGraphMode);
};

// Legend hover-to-highlight
(function initLegendHighlight() {
  const legend = document.getElementById("graph-legend");
  if (!legend) return;

  function applyLegendHighlight(filter) {
    const nodes = window._graphNodeElements;
    const edges = window._graphEdgeElements;
    if (!nodes || !edges) return;

    const [type, value] = filter.split(":");

    if (type === "node-role") {
      const matchedNodes = new Set();
      nodes.forEach((el, name) => {
        const match = el.dataset.role === value;
        el.style.opacity = match ? "1" : "0.12";
        el.style.transition = "opacity 0.2s";
        if (match) matchedNodes.add(name);
      });
      edges.forEach((els, key) => {
        const [from, to] = key.split("-->");
        const connected = matchedNodes.has(from) || matchedNodes.has(to);
        els.path.style.opacity = connected ? "1" : "0.08";
        els.path.style.transition = "opacity 0.2s";
      });
    } else if (type === "edge-type" || type === "edge-flag") {
      const connectedNodes = new Set();
      edges.forEach((els, key) => {
        const match =
          type === "edge-type"
            ? els.path.dataset.edgeType === value
            : els.path.dataset[value] === "1";
        els.path.style.opacity = match ? "1" : "0.08";
        els.path.style.transition = "opacity 0.2s";
        if (match) {
          const [from, to] = key.split("-->");
          connectedNodes.add(from);
          connectedNodes.add(to);
        }
      });
      nodes.forEach((el, name) => {
        el.style.opacity = connectedNodes.has(name) ? "1" : "0.12";
        el.style.transition = "opacity 0.2s";
      });
    }
  }

  function clearLegendHighlight() {
    const nodes = window._graphNodeElements;
    const edges = window._graphEdgeElements;
    if (!nodes || !edges) return;
    nodes.forEach((el) => {
      el.style.opacity = "1";
      el.style.transition = "opacity 0.2s";
    });
    edges.forEach((els) => {
      els.path.style.opacity = "1";
      els.path.style.transition = "opacity 0.2s";
    });
    // Re-assert the persistent highlight toggles the hover overrode.
    if (window.applyGraphHighlights) window.applyGraphHighlights();
  }

  legend.querySelectorAll(".legend-item[data-legend-highlight]").forEach((item) => {
    item.addEventListener("mouseenter", () => {
      const filter = item.dataset.legendHighlight;
      if (filter) applyLegendHighlight(filter);
    });
    item.addEventListener("mouseleave", () => {
      clearLegendHighlight();
    });
  });
})();

function updateGraphUrl(push = false) {
  const focus = window.graphFocusTags.join(",");

  const hiddenProtocols = [];
  if (!window.graphProtocolFilters.openapi) hiddenProtocols.push("openapi");
  if (!window.graphProtocolFilters.asyncapi) hiddenProtocols.push("asyncapi");
  if (!window.graphProtocolFilters.proto) hiddenProtocols.push("proto");
  const hide_protocols = hiddenProtocols.join(",");

  const highlights = [];
  if (window.graphHighlightFilters.outdated) highlights.push("outdated");
  if (window.graphHighlightFilters.breakingOutdated) highlights.push("breakingOutdated");
  if (window.graphHighlightFilters.snapshot) highlights.push("snapshot");

  const params = new URLSearchParams();
  if (focus) params.set("focus", focus);
  if (hide_protocols) params.set("hide_protocols", hide_protocols);
  if (highlights.length) params.set("highlight", highlights.join(","));

  const newUrl = params.toString() ? `?${params.toString()}` : window.location.pathname;
  const state = {
    focus: window.graphFocusTags,
    hide_protocols: hiddenProtocols,
    highlight: highlights,
  };

  if (push) {
    history.pushState(state, "", newUrl);
  } else {
    history.replaceState(state, "", newUrl);
  }
}
window.updateGraphUrl = updateGraphUrl;

function restoreGraphFiltersFromUrl() {
  const params = new URLSearchParams(window.location.search);

  const focus = params.get("focus");
  if (focus) {
    window.graphFocusTags = focus.split(",").filter(Boolean);
  } else {
    window.graphFocusTags = [];
  }
  renderFocusTags();

  const hide_protocols = params.get("hide_protocols");
  const hiddenSet = new Set(hide_protocols ? hide_protocols.split(",").filter(Boolean) : []);

  window.graphProtocolFilters = {
    openapi: !hiddenSet.has("openapi"),
    asyncapi: !hiddenSet.has("asyncapi"),
    proto: !hiddenSet.has("proto"),
  };

  ["openapi", "asyncapi", "proto"].forEach((p) => {
    const btn = document.getElementById(`filter-${p}`);
    if (btn) {
      if (window.graphProtocolFilters[p]) {
        btn.className = "px-2.5 py-1.5 bg-indigo-600 text-white font-medium";
      } else {
        btn.className = "px-2.5 py-1.5 bg-white text-slate-600 hover:bg-slate-50 font-medium";
      }
    }
  });

  const highlightSet = new Set((params.get("highlight") || "").split(",").filter(Boolean));
  window.graphHighlightFilters = {
    outdated: highlightSet.has("outdated"),
    breakingOutdated: highlightSet.has("breakingOutdated"),
    snapshot: highlightSet.has("snapshot"),
  };
  updateHighlightButtons();
}

// Handle back/forward navigation
window.addEventListener("popstate", async () => {
  restoreGraphFiltersFromUrl();
  await loadGraph();
});

window.addEventListener("sanshain-update", async () => {
  console.log("Detected spec update, refreshing graph...");
  await loadGraph();
});

function initGraph() {
  checkDiscoveryAuth(() => showGraph());
}
if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", initGraph);
} else {
  initGraph();
}
