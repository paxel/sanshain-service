// Extracted from static/graph.html: inline
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

let currentGraphDirection = readGraphViewPref("direction") === "LR" ? "LR" : "TB";
window.graphDensity = graphRememberedDensity();
window.graphViewSwitches.rainbow = readGraphViewPref("rainbow") === "1";
window.graphViewSwitches.topics = readGraphViewPref("topics") === "1";
window.graphDomainsMode = readGraphViewPref("domains") === "node" ? "node" : "group";

const GRAPH_SWITCH_BASE =
  "inline-flex items-center gap-1 px-2.5 py-1.5 rounded-lg text-xs font-medium border transition-colors shadow-xs";
const GRAPH_SWITCH_ON = "bg-indigo-600 border-indigo-600 text-white";
const GRAPH_SWITCH_OFF = "bg-white border-slate-300 text-slate-600 hover:bg-slate-50";

// Reflect the remembered switches in the toolbar.
// The remembered density; a browser that only remembers the former Compact
// switch gets Compact.
function graphRememberedDensity() {
  const density = readGraphViewPref("density");
  if (window.GRAPH_DENSITIES.includes(density)) return density;
  return readGraphViewPref("compact") === "1" ? "compact" : "normal";
}

function stepGraphDensity() {
  const all = window.GRAPH_DENSITIES;
  window.graphDensity = all[(all.indexOf(window.graphDensity) + 1) % all.length];
  writeGraphViewPref("density", window.graphDensity);
  syncGraphViewButtons();
  updateGraphUrl(false);
  if (currentGraphMode === "custom") {
    redrawGraph();
  }
}

function syncGraphViewButtons() {
  const label = document.getElementById("graph-direction-label");
  if (label) label.textContent = currentGraphDirection === "TB" ? "Vertical" : "Horizontal";
  const densityLabel = document.getElementById("graph-density-label");
  if (densityLabel) {
    densityLabel.textContent =
      window.graphDensity.charAt(0).toUpperCase() + window.graphDensity.slice(1);
  }
  const pressed = {
    density: window.graphDensity !== "normal",
    rainbow: window.graphViewSwitches.rainbow,
    topics: window.graphViewSwitches.topics,
    domains: window.graphDomainsMode === "group",
  };
  for (const [name, on] of Object.entries(pressed)) {
    const btn = document.getElementById(`graph-${name}-toggle`);
    if (!btn) continue;
    btn.className = `${GRAPH_SWITCH_BASE} ${on ? GRAPH_SWITCH_ON : GRAPH_SWITCH_OFF}`;
    btn.setAttribute("aria-pressed", on ? "true" : "false");
  }
}

// z: the experimental domain view and back.
function toggleGraphLevel() {
  window.graphLevel = window.graphLevel === "domains" ? "services" : "domains";
  updateGraphUrl(false);
  if (currentGraphMode === "custom") redrawGraph();
}

// A click on a domain node: the full graph, focused on that domain.
function openGraphDomain(domain) {
  window.graphLevel = "services";
  window.graphFocusTags = [`domain:${domain}`];
  renderFocusTags();
  updateGraphUrl(true);
  redrawGraph();
}
window.openGraphDomain = openGraphDomain;

function toggleGraphDomains() {
  window.graphDomainsMode = window.graphDomainsMode === "group" ? "node" : "group";
  writeGraphViewPref("domains", window.graphDomainsMode);
  syncGraphViewButtons();
  updateGraphUrl(false);
  if (currentGraphMode === "custom") {
    redrawGraph();
  }
}

function toggleGraphDirection() {
  currentGraphDirection = currentGraphDirection === "TB" ? "LR" : "TB";
  writeGraphViewPref("direction", currentGraphDirection);
  syncGraphViewButtons();
  updateGraphUrl(false);
  if (currentGraphMode === "custom") {
    redrawGraph();
  }
}

function toggleGraphViewSwitch(name) {
  window.graphViewSwitches[name] = !window.graphViewSwitches[name];
  writeGraphViewPref(name, window.graphViewSwitches[name] ? "1" : "0");
  syncGraphViewButtons();
  updateGraphUrl(false);
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
  const compactBtn = document.getElementById("graph-density-toggle");
  const rainbowBtn = document.getElementById("graph-rainbow-toggle");
  const topicsBtn = document.getElementById("graph-topics-toggle");
  const copyBtn = document.getElementById("graph-copy");
  const dlBtn = document.getElementById("graph-download");
  const dlPngBtn = document.getElementById("graph-download-png");
  const toolbarRow = document.getElementById("graph-toolbar-row");

  if (dirBtn) dirBtn.classList.toggle("hidden", mode !== "custom");
  if (compactBtn) compactBtn.classList.toggle("hidden", mode !== "custom");
  if (rainbowBtn) rainbowBtn.classList.toggle("hidden", mode !== "custom");
  if (topicsBtn) topicsBtn.classList.toggle("hidden", mode !== "custom");
  const domainsBtn = document.getElementById("graph-domains-toggle");
  if (domainsBtn) domainsBtn.classList.toggle("hidden", mode !== "custom");
  if (copyBtn) copyBtn.classList.remove("hidden");
  if (dlBtn) dlBtn.classList.remove("hidden");
  if (dlPngBtn) dlPngBtn.classList.remove("hidden");
  if (toolbarRow) toolbarRow.classList.remove("hidden");
  updateGraphUrl(false);

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

// Load the page's state from the address, draw, then open the view the
// address names. Also what back and forward do.
async function showGraph() {
  try {
    const pending = restoreGraphFiltersFromUrl();
    await loadGraph();
    await applyGraphViewFromUrl(pending);
  } catch (err) {
    console.error("Failed to show graph:", err);
    hideLoader();
    const graphDiv = document.getElementById("mermaid-graph");
    if (graphDiv) {
      graphDiv.classList.remove("hidden");
      // As text: a message can quote what the server sent (a JSON parse error
      // quotes the body), so it never becomes markup.
      const message = document.createElement("div");
      message.className = "text-red-500 p-4";
      message.textContent = `Error: ${err.message}`;
      graphDiv.replaceChildren(message);
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
      window.graphBranchNames = branches.map((b) => b.name);
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

    // The scope data (producers with version lines, harvested subscriptions)
    // of the view and instant on screen — badges, the "(unrequired)" box and
    // the harvested edges all read it.
    window.graphScopeData = await graphFetchScopeData(
      window.graphStreamView,
      window.graphBranchName,
      window.graphTimelineAt,
    );

    lastGraphReport = report;

    const services = new Set();
    report.dependency_graph.forEach((d) => {
      services.add(d.client);
      services.add(d.service);
    });
    graphFocusServices = [...services].sort(byCodeUnit);
    if (graphFocusPicker) graphFocusPicker.refresh();

    const anyProducer = ((window.graphScopeData && window.graphScopeData.producers) || []).length;
    if (report.dependency_graph.length === 0 && !anyProducer) {
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
    edgeLabels
      .get(key)
      .add(
        d.peripheral
          ? "depends on"
          : `[${d.api_type || "REST"}] ${d.method} ${d.path}${d.version ? ` @${d.version}` : ""}`,
      );
  });

  const cycleEdges = graphDetectCycles(adjMap);
  const hasCycles = cycleEdges.size > 0;

  if (hasCycles) {
    warningDiv.classList.remove("hidden");
  } else {
    warningDiv.classList.add("hidden");
  }

  const clientNodes = new Set(report.dependency_graph.map((d) => d.client));
  const serviceNodes = new Set(report.dependency_graph.map((d) => d.service));
  // Peripheral Resources (#46): drawn as cylinders with a dashed outline.
  const peripheralNodes = new Set(
    report.dependency_graph.filter((d) => d.peripheral).map((d) => d.service),
  );

  // Unowned harvested subscriptions point at a missing `<channel> ?` node.
  const missingChannelNodes = new Set(
    report.dependency_graph.filter((d) => d.unfulfilled).map((d) => d.service),
  );

  // Second label line: what the node provides (▲) and requires (▼).
  const nodeBadges = graphNodeBadges(report, report.dependency_graph);
  const withBadges = (node, text) => {
    const line = graphBadgeLine(nodeBadges.get(node));
    return line ? `${text}<br/>${line}` : text;
  };

  // Per-edge highlight flags (Outdated / Snapshot-pinned).
  const latestGaMap = graphLatestGaMap(report);
  const edgeFlags = graphEdgeFlags(report, latestGaMap);

  let mmd = "graph TD\n";

  mmd += "  classDef serviceOnly fill:#e0e7ff,stroke:#6366f1,stroke-width:2px,color:#3730a3\n";
  mmd += "  classDef clientOnly fill:#ccfbf1,stroke:#14b8a6,stroke-width:2px,color:#115e59\n";
  mmd += "  classDef both fill:#fef3c7,stroke:#f59e0b,stroke-width:2px,color:#92400e\n";
  mmd += "  classDef cycleEdge stroke:#ef4444,stroke-width:3px\n";
  mmd +=
    "  classDef peripheral fill:#f1f5f9,stroke:#64748b,stroke-width:1.5px,stroke-dasharray:4 3,color:#334155\n";
  mmd +=
    "  classDef missing fill:#fff7ed,stroke:#f97316,stroke-width:2px,stroke-dasharray:6 3,color:#9a3412\n";
  mmd += "  linkStyle default stroke:#94a3b8,stroke-width:2px\n";

  for (const node of allNodes) {
    const id = sanitizeId(node);
    const isClient = clientNodes.has(node);
    const isService = serviceNodes.has(node);
    if (peripheralNodes.has(node)) {
      mmd += `  ${id}[("${graphPeripheralLabel(node)}")]\n`;
    } else if (isClient && isService) {
      mmd += `  ${id}["${withBadges(node, node)}"]\n`;
    } else if (isService) {
      mmd += `  ${id}["${withBadges(node, node)}"]\n`;
    } else {
      mmd += `  ${id}("${withBadges(node, node)}")\n`;
    }
  }

  for (const node of allNodes) {
    const id = sanitizeId(node);
    const isClient = clientNodes.has(node);
    const isService = serviceNodes.has(node);
    if (peripheralNodes.has(node)) {
      mmd += `  class ${id} peripheral\n`;
    } else if (missingChannelNodes.has(node)) {
      mmd += `  class ${id} missing\n`;
    } else if (isClient && isService) {
      mmd += `  class ${id} both\n`;
    } else if (isService) {
      mmd += `  class ${id} serviceOnly\n`;
    } else {
      mmd += `  class ${id} clientOnly\n`;
    }
  }

  // The "(unrequired)" box: one subgraph holding the scope's producers nobody
  // requires and that require nothing.
  const unrequired = report.unrequired || [];
  if (unrequired.length > 0) {
    mmd += "  classDef unrequired fill:#e0e7ff,stroke:#6366f1,stroke-width:1px,color:#3730a3\n";
    mmd += '  subgraph unrequired_box["(unrequired)"]\n';
    for (const p of [...unrequired].sort((a, b) => a.name.localeCompare(b.name))) {
      mmd += `    ${sanitizeId(p.name)}["${withBadges(p.name, p.name)}"]\n`;
    }
    mmd += "  end\n";
    for (const p of unrequired) mmd += `  class ${sanitizeId(p.name)} unrequired\n`;
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
  void navigator.clipboard.writeText(content).then(() => {
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
  for (const p of diff.periphery_added || [])
    rows.push(
      `<div class="text-green-700">+ ${escapeHtml(p.system)} ⇢ ${escapeHtml(p.resource)}</div>`,
    );
  for (const p of diff.periphery_removed || [])
    rows.push(
      `<div class="text-red-700">− ${escapeHtml(p.system)} ⇢ ${escapeHtml(p.resource)}</div>`,
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

// Clear button for the graph-diff panel — replaces an inline innerHTML reset.
function clearGraphDiffOutput() {
  const out = document.getElementById("graph-diff-output");
  if (out) out.innerHTML = "";
}
window.clearGraphDiffOutput = clearGraphDiffOutput;

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

    if (type === "node-role" || type === "node-badge" || type === "node-external") {
      // node-badge:<kind> matches nodes that provide or require that kind.
      const hasBadge = (el) =>
        `${el.dataset.provides || ""},${el.dataset.requires || ""}`.split(",").includes(value);
      const matchedNodes = new Set();
      nodes.forEach((el, name) => {
        const match =
          type === "node-role"
            ? el.dataset.role === value
            : type === "node-external"
              ? el.dataset.external === value
              : hasBadge(el);
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
    } else if (type === "unrequired") {
      // The box itself stays lit; the graph around it steps back.
      nodes.forEach((el) => {
        el.style.opacity = "0.12";
        el.style.transition = "opacity 0.2s";
      });
      edges.forEach((els) => {
        els.path.style.opacity = "0.08";
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
  if (!window.graphProtocolFilters.periphery) hiddenProtocols.push("periphery");
  if (!window.graphProtocolFilters.unrequired) hiddenProtocols.push("unrequired");
  const hide_protocols = hiddenProtocols.join(",");

  const highlights = [];
  if (window.graphHighlightFilters.outdated) highlights.push("outdated");
  if (window.graphHighlightFilters.breakingOutdated) highlights.push("breakingOutdated");
  if (window.graphHighlightFilters.snapshot) highlights.push("snapshot");

  const params = new URLSearchParams();
  if (focus) params.set("focus", focus);
  if (hide_protocols) params.set("hide_protocols", hide_protocols);
  if (highlights.length) params.set("highlight", highlights.join(","));

  // The view, its range and instant: what a shared link must reproduce.
  const view = window.graphStreamView;
  if (view !== "dev") params.set("view", view);
  if (view === "branch" && window.graphBranchName) params.set("branch", window.graphBranchName);
  if (window.graphTimelineRangeChosen) {
    params.set("from", window.graphTimelineRange.from);
    params.set("to", window.graphTimelineRange.to);
  }
  if (view !== "dev" && window.graphTimelineAt) params.set("at", window.graphTimelineAt);
  // The display switches, always: the receiver's remembered ones must not
  // change what the sender saw.
  params.set("density", window.graphDensity);
  for (const name of ["rainbow", "topics"]) {
    params.set(name, window.graphViewSwitches[name] ? "1" : "0");
  }
  params.set("domains", window.graphDomainsMode);
  if (window.graphLevel === "domains") params.set("level", "domains");
  params.set("direction", currentGraphDirection);
  params.set("mode", currentGraphMode);
  if (window.graphRedrawMode === "circular") params.set("redraw", "circular");

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
  void window.graphLoadFocusProducts();

  const hide_protocols = params.get("hide_protocols");
  const hiddenSet = new Set(hide_protocols ? hide_protocols.split(",").filter(Boolean) : []);

  window.graphProtocolFilters = {
    openapi: !hiddenSet.has("openapi"),
    asyncapi: !hiddenSet.has("asyncapi"),
    proto: !hiddenSet.has("proto"),
    // A link that says which protocols it hides wins; a plain visit takes
    // the browser's remembered Periphery switch, shown unless switched off.
    periphery: hide_protocols
      ? !hiddenSet.has("periphery")
      : readGraphViewPref("periphery") !== "0",
    unrequired: !hiddenSet.has("unrequired"),
  };

  ["openapi", "asyncapi", "proto", "periphery", "unrequired"].forEach((p) => {
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

  // A link that sets a display switch wins; a plain visit takes the
  // browser's remembered one.
  const density = params.get("density");
  window.graphDensity = window.GRAPH_DENSITIES.includes(density)
    ? density
    : graphRememberedDensity();
  for (const name of ["rainbow", "topics"]) {
    const value = params.get(name);
    window.graphViewSwitches[name] =
      value === null ? readGraphViewPref(name) === "1" : value === "1";
  }
  window.graphLevel = params.get("level") === "domains" ? "domains" : "services";
  const domains = params.get("domains");
  window.graphDomainsMode =
    domains === "group" || domains === "node"
      ? domains
      : readGraphViewPref("domains") === "node"
        ? "node"
        : "group";
  const direction = params.get("direction");
  currentGraphDirection =
    direction === "LR" || direction === "TB"
      ? direction
      : readGraphViewPref("direction") === "LR"
        ? "LR"
        : "TB";
  syncGraphViewButtons();

  const pending = restoreGraphTimelineFromParams(params);
  // Both write the address, so they come once all of its state is restored.
  // The mode is not remembered per browser: custom unless the link names one.
  const mode = params.get("mode");
  switchGraphMode(["mermaid", "detailed"].includes(mode) ? mode : "custom");
  setGraphRedrawMode(params.get("redraw") === "circular" ? "circular" : "all");
  return pending;
}

// The range and the instant of the address, and the view it names. A range
// that is no pair of days in order gives the default; an instant outside the
// range widens it to the instant's day. Returns the view to open once the
// graph is loaded (see applyGraphViewFromUrl).
function restoreGraphTimelineFromParams(params) {
  const fallback = graphDefaultTimelineRange();
  const from = params.get("from");
  const to = params.get("to");
  const range = { from: from || fallback.from, to: to || fallback.to };
  let chosen = from !== null || to !== null;
  if (!graphIsTimelineDay(range.from) || !graphIsTimelineDay(range.to) || range.from > range.to) {
    range.from = fallback.from;
    range.to = fallback.to;
    chosen = false;
  }

  let view = params.get("view");
  const branch = params.get("branch");
  if (view === "branch" && !branch) view = "main";
  if (view !== "main" && view !== "branch") view = "dev";

  const rawAt = params.get("at");
  // Only an RFC 3339 instant, as the server takes it; Date.parse alone also
  // reads forms like "June 1 2025".
  const rfc3339 = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$/;
  const atMs = rawAt && rfc3339.test(rawAt) ? Date.parse(rawAt) : Number.NaN;
  const at = view !== "dev" && !Number.isNaN(atMs) ? rawAt : null;
  if (at) {
    const day = graphUtcDay(new Date(atMs));
    if (day < range.from) {
      range.from = day;
      chosen = true;
    }
    if (day > range.to) {
      range.to = day;
      chosen = true;
    }
  }
  window.graphTimelineRange = range;
  window.graphTimelineRangeChosen = chosen;
  // Set before the graph loads, which reads them and writes them back into
  // the address.
  window.graphStreamView = view;
  window.graphBranchName = view === "branch" ? branch : null;
  window.graphTimelineAt = at;
  return { view, branch, at, switches: window._graphUserViewSwitches || 0 };
}

function showGraphNotice(text) {
  const notice = document.getElementById("graph-url-notice");
  if (!notice) return;
  notice.textContent = text || "";
  notice.classList.toggle("hidden", !text);
}

// Open the view the address named, unless the user switched views while the
// page loaded. A branch that no longer exists opens main with the same range
// and instant, and says so.
async function applyGraphViewFromUrl(pending) {
  const userSwitched = () => (window._graphUserViewSwitches || 0) !== pending.switches;
  if (userSwitched()) return;
  showGraphNotice("");
  let view = pending.view;
  if (view === "branch") {
    let pins = null;
    if ((window.graphBranchNames || []).includes(pending.branch)) {
      try {
        const res = await apiCall(`/admin/branches/${encodeURIComponent(pending.branch)}/graph`);
        if (res.ok) pins = await res.json();
      } catch (e) {
        console.warn(`Could not load branch '${pending.branch}':`, e);
      }
    }
    if (userSwitched()) return;
    if (pins === null) {
      showGraphNotice(`There is no sanshain-branch '${pending.branch}': showing main instead.`);
      view = "main";
    } else {
      window.graphBranchPins = pins;
      window.graphBranchName = pending.branch;
      const sel = document.getElementById("graph-branch-select");
      if (sel) sel.value = pending.branch;
    }
  }
  await setStreamView(view, pending.at, false);
  updateGraphUrl(false);
}

// Handle back/forward navigation
window.addEventListener("popstate", () => {
  void showGraph();
});

window.addEventListener("sanshain-update", async () => {
  console.log("Detected spec update, refreshing graph...");
  await loadGraph();
});

// The Focus picker: tick services to spotlight them, untick to let go. Text
// that names no service (such as `tag:payments` or `product:shop`) is taken
// with Enter.
let graphFocusServices = [];
function initGraphFocusPicker() {
  const root = document.getElementById("graph-focus-picker");
  if (!root) return;
  graphFocusPicker = createPicker(root, {
    id: "graph-service-filter",
    placeholder: "Add service...",
    multiple: true,
    options: () => {
      const extra = window.graphFocusTags.filter((t) => !graphFocusServices.includes(t));
      return graphFocusServices.concat(extra).map((s) => ({ value: s, label: s }));
    },
    isSelected: (value) => window.graphFocusTags.includes(value),
    onToggle: (value, checked) =>
      checked ? window.addFocusTag(value) : window.removeFocusTag(value),
    onFreeText: (text) => window.addFocusTag(text),
  });
}

// ── Fullscreen ───────────────────────────────────────────────────────
// Level 1 fills the browser window with the graph, level 2 the screen (the
// browser's own fullscreen). The look is CSS on body[data-graph-fs]; this
// keeps the level, the open panel and the space the column, the legend and
// the timeline bar take.
let graphFsLevel = 0;

function graphFsBody() {
  return document.body;
}

// The free space for the graph, as the CSS variables the fullscreen rules use.
function layoutGraphFullscreen() {
  const body = graphFsBody();
  const visible = (el) => !!el && el.offsetWidth > 0 && el.offsetHeight > 0;
  const rail = document.getElementById("graph-fs-rail");
  const legend = document.getElementById("graph-legend");
  const timeline = document.getElementById("graph-timeline-row");
  const left = graphFsLevel && visible(rail) ? rail.offsetWidth : 0;
  const right = graphFsLevel && visible(legend) ? legend.offsetWidth : 0;
  const bottom = graphFsLevel && visible(timeline) ? timeline.offsetHeight : 0;
  const next = `${left}|${right}|${bottom}`;
  if (body.dataset.fsLayout === next) return;
  body.dataset.fsLayout = next;
  body.style.setProperty("--fs-left", `${left}px`);
  body.style.setProperty("--fs-right", `${right}px`);
  body.style.setProperty("--fs-bottom", `${bottom}px`);
  // The canvas takes its size from the space it is given.
  if (currentGraphMode === "custom") redrawGraph();
}

function setGraphFsLevel(level) {
  graphFsLevel = level;
  const body = graphFsBody();
  if (level) body.dataset.graphFs = String(level);
  else {
    delete body.dataset.graphFs;
    closeGraphFsPanel();
  }
  const wantScreen = level === 2;
  const inScreen = !!document.fullscreenElement;
  if (wantScreen && !inScreen && document.documentElement.requestFullscreen) {
    // Refused without a key press or click: stay in the window.
    document.documentElement.requestFullscreen().catch(() => setGraphFsLevel(1));
  } else if (!wantScreen && inScreen && document.exitFullscreen) {
    void document.exitFullscreen().catch(() => {});
  }
  delete body.dataset.fsLayout;
  layoutGraphFullscreen();
}

// f: window, then screen, then back to the page.
function stepGraphFullscreen() {
  setGraphFsLevel((graphFsLevel + 1) % 3);
}

function exitGraphFullscreen() {
  setGraphFsLevel(0);
}

function closeGraphFsPanel() {
  document.querySelectorAll("[data-fs-shown]").forEach((el) => delete el.dataset.fsShown);
  document
    .querySelectorAll("[data-fs-icon]")
    .forEach((el) => el.classList.remove("bg-indigo-100", "text-indigo-700"));
}

function graphFsPanelOpen() {
  return !!document.querySelector("[data-fs-shown]");
}

// One panel at a time; its icon again closes it.
function toggleGraphFsPanel(name) {
  const panel = document.querySelector(`[data-fs-panel="${name}"]`);
  const wasOpen = !!panel && panel.dataset.fsShown !== undefined;
  closeGraphFsPanel();
  if (!panel || wasOpen) return;
  panel.dataset.fsShown = "1";
  const icon = document.querySelector(`[data-fs-icon="${name}"]`);
  if (icon) icon.classList.add("bg-indigo-100", "text-indigo-700");
}

// c: the icon column (and its open panel) shown or hidden.
function toggleGraphFsRail() {
  const body = graphFsBody();
  if (body.dataset.fsRailHidden !== undefined) delete body.dataset.fsRailHidden;
  else body.dataset.fsRailHidden = "1";
  layoutGraphFullscreen();
}

// b: the timeline bar shown or hidden, in and out of fullscreen.
function toggleGraphTimelineBar() {
  const body = graphFsBody();
  if (body.dataset.graphTimelineHidden !== undefined) delete body.dataset.graphTimelineHidden;
  else body.dataset.graphTimelineHidden = "1";
  layoutGraphFullscreen();
}

// Esc: the key list, then an open panel, then one fullscreen level.
function graphEscape() {
  if (graphKeysOpen()) {
    hideGraphKeys();
    return true;
  }
  if (graphFsPanelOpen()) {
    closeGraphFsPanel();
    return true;
  }
  if (graphFsLevel) {
    setGraphFsLevel(graphFsLevel - 1);
    return true;
  }
  return false;
}

function initGraphFullscreen() {
  // The browser leaves its fullscreen on its own Esc: back to the window.
  document.addEventListener("fullscreenchange", () => {
    if (!document.fullscreenElement && graphFsLevel === 2) setGraphFsLevel(1);
  });
  window.addEventListener("resize", () => layoutGraphFullscreen());
  if (typeof ResizeObserver !== "undefined") {
    const observer = new ResizeObserver(() => layoutGraphFullscreen());
    for (const id of ["graph-timeline-row", "graph-legend", "graph-fs-rail"]) {
      const el = document.getElementById(id);
      if (el) observer.observe(el);
    }
  }
  // A click into the graph closes the open panel.
  const container = document.getElementById("graph-container");
  if (container) container.addEventListener("mousedown", () => closeGraphFsPanel());
}

// ── Hotkeys ──────────────────────────────────────────────────────────
// Single keys without Ctrl, Alt or Cmd, ignored while typing in a field. The
// ? overlay lists this table, so it always shows what the keys do.
const GRAPH_HOTKEYS = [
  { key: "f", label: "Fullscreen: window, screen, back", run: () => stepGraphFullscreen() },
  { key: "c", label: "Control column shown or hidden", run: () => toggleGraphFsRail() },
  { key: "b", label: "Timeline bar shown or hidden", run: () => toggleGraphTimelineBar() },
  { key: "z", label: "Domain view (experimental)", run: () => toggleGraphLevel() },
  { key: "d", label: "Density: Normal, Compact, Dense", run: () => stepGraphDensity() },
  { key: "g", label: "Domain frames on or off", run: () => toggleGraphDomains() },
  { key: "r", label: "Rainbow", run: () => toggleGraphViewSwitch("rainbow") },
  { key: "t", label: "Topics", run: () => toggleGraphViewSwitch("topics") },
  { key: "v", label: "Vertical or horizontal", run: () => toggleGraphDirection() },
  { key: "l", label: "Legend shown or hidden", run: () => toggleGraphLegend() },
  { key: "[", label: "Previous change on the timeline", run: () => graphStepTimeline(-1) },
  { key: "]", label: "Next change on the timeline", run: () => graphStepTimeline(1) },
  { key: "0", label: "Fit to view", run: () => graphFitToView() },
  { key: "?", label: "This list", run: () => toggleGraphKeys() },
];

function toggleGraphLegend() {
  const legend = document.getElementById("graph-legend");
  if (legend) legend.classList.toggle("hidden");
  layoutGraphFullscreen();
}

function graphKeysOpen() {
  const overlay = document.getElementById("graph-keys-overlay");
  return !!overlay && !overlay.classList.contains("hidden");
}

function toggleGraphKeys() {
  const overlay = document.getElementById("graph-keys-overlay");
  const list = document.getElementById("graph-keys-list");
  if (!overlay || !list) return;
  if (!list.childElementCount) {
    const esc = { key: "Esc", label: "Back: this list, a panel, a fullscreen level" };
    for (const hotkey of [...GRAPH_HOTKEYS, esc]) {
      const dt = document.createElement("dt");
      const kbd = document.createElement("kbd");
      kbd.className =
        "inline-block min-w-[1.5rem] text-center px-1.5 py-0.5 rounded-sm border border-slate-300 bg-slate-50 font-mono text-xs text-slate-700";
      kbd.textContent = hotkey.key;
      dt.appendChild(kbd);
      const dd = document.createElement("dd");
      dd.className = "text-slate-600";
      dd.textContent = hotkey.label;
      list.append(dt, dd);
    }
  }
  overlay.classList.toggle("hidden");
}

function hideGraphKeys() {
  const overlay = document.getElementById("graph-keys-overlay");
  if (overlay) overlay.classList.add("hidden");
}

function graphTypingIn(target) {
  if (!target || !target.tagName) return false;
  return (
    ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName) || target.isContentEditable === true
  );
}

function onGraphHotkey(e) {
  if (e.ctrlKey || e.altKey || e.metaKey) return;
  // Esc goes back even from inside a field of an open panel.
  if (e.key === "Escape") {
    if (graphEscape()) e.preventDefault();
    return;
  }
  // The timeline slider has its own arrow keys; letters still work there.
  if (graphTypingIn(e.target) && e.target.id !== "graph-timeline-slider") return;
  const hotkey = GRAPH_HOTKEYS.find((h) => h.key === e.key);
  if (!hotkey) return;
  e.preventDefault();
  hotkey.run();
}

function initGraph() {
  document.addEventListener("keydown", onGraphHotkey);
  initGraphFullscreen();
  initGraphFocusPicker();
  syncGraphViewButtons();
  void checkDiscoveryAuth(() => showGraph());
}
if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", initGraph);
} else {
  initGraph();
}
