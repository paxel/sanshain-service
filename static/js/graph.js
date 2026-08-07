/**
 * Custom dagre-based SVG dependency graph renderer.
 * Replaces Mermaid as the default graph view with full interactivity:
 * - Topological layout (clients on top, services below)
 * - Color-coded nodes (client-only, service-only, both)
 * - Red cycle edges
 * - Hover tooltips on edges showing HTTP method + path
 * - Click-to-highlight connected subgraph
 * - Zoom and pan
 */

window.graphRedrawMode = "all"; // 'all' or 'circular'
window.graphFocusTags = []; // focus filter (multi-service tag cloud)
window.graphProtocolFilters = {
  openapi: true,
  asyncapi: true,
  proto: true,
};
// Independent display-state highlights (never resolution inputs):
// outdated = Pin semantically below the line's latest GA;
// snapshot = Pin currently served from a Snapshot.
window.graphHighlightFilters = {
  outdated: false,
  snapshot: false,
};
// Which stream the graph shows (ADR-0004): 'dev' is the classic latest-activity
// view; 'main' draws the trunk pin set and producers at their trunk version.
window.graphStreamView = "dev";

// Per (service, api_type): the newest trunk-marked version of the line —
// what the main view labels producers with and checks conflicts against.
function graphTrunkVersionMap(report) {
  const map = new Map();
  (report.services_detailed || []).forEach((svc) => {
    (svc.versions || []).forEach((v) => {
      if (!v.trunk_provided_at) return;
      const key = `${svc.name}|${(v.api_type || "openapi").toLowerCase()}`;
      const existing = map.get(key);
      if (!existing || _graphCompareSemver(v.version, existing) > 0) {
        map.set(key, v.version);
      }
    });
  });
  return map;
}
window.graphTrunkVersionMap = graphTrunkVersionMap;

const GRAPH_OUTDATED_COLOR = "#f43f5e";
const GRAPH_SNAPSHOT_COLOR = "#f59e0b";

function _graphCompareSemver(a, b) {
  const pa = String(a)
    .split(".")
    .map((n) => parseInt(n, 10) || 0);
  const pb = String(b)
    .split(".")
    .map((n) => parseInt(n, 10) || 0);
  for (let i = 0; i < 3; i++) {
    if ((pa[i] || 0) !== (pb[i] || 0)) return (pa[i] || 0) - (pb[i] || 0);
  }
  return 0;
}

// Latest GA per version line, from the Producer listing that rides along in
// report.services_detailed: Map "producer|api_type" -> "x.y.z" (absent when
// the line has no GA yet).
function graphLatestGaMap(report) {
  const map = new Map();
  (report.services_detailed || []).forEach((svc) => {
    (svc.versions || []).forEach((v) => {
      if (v.stability !== "ga") return;
      const key = `${svc.name}|${(v.api_type || "openapi").toLowerCase()}`;
      const cur = map.get(key);
      if (!cur || _graphCompareSemver(v.version, cur) > 0) map.set(key, v.version);
    });
  });
  return map;
}

// Per aggregated edge ("client-->service"): does any dependency on it carry an
// Outdated or Snapshot-pinned Pin? Map key -> { outdated, snapshot }.
function graphEdgeFlags(report, latestGaMap, trunkVersionMap) {
  const flags = new Map();
  (report.dependency_graph || []).forEach((d) => {
    const key = `${d.client}-->${d.service}`;
    if (!flags.has(key)) flags.set(key, { outdated: false, snapshot: false, conflict: false });
    const f = flags.get(key);
    if (d.stability === "snapshot") f.snapshot = true;
    const typeKey = `${d.service}|${(d.api_type || "openapi").toLowerCase()}`;
    const latestGa = latestGaMap.get(typeKey);
    if (latestGa && _graphCompareSemver(d.version, latestGa) < 0) f.outdated = true;
    // Main view: a pin a whole major behind the producer's trunk version is a
    // conflict, drawn loud (ADR-0004).
    if (trunkVersionMap) {
      const trunkV = trunkVersionMap.get(typeKey);
      if (trunkV && parseInt(d.version, 10) < parseInt(trunkV, 10)) f.conflict = true;
    }
  });
  return flags;
}
window.graphLatestGaMap = graphLatestGaMap;
window.graphEdgeFlags = graphEdgeFlags;

function updateHighlightButtons() {
  const styles = [
    ["highlight-outdated", "outdated", "bg-rose-500 text-white"],
    ["highlight-snapshot", "snapshot", "bg-amber-500 text-white"],
  ];
  for (const [id, filter, activeClasses] of styles) {
    const btn = document.getElementById(id);
    if (!btn) continue;
    btn.className = window.graphHighlightFilters[filter]
      ? `px-2.5 py-1.5 ${activeClasses} font-medium`
      : "px-2.5 py-1.5 bg-white text-slate-600 hover:bg-slate-50 font-medium";
  }
}
window.updateHighlightButtons = updateHighlightButtons;

function toggleHighlightFilter(filter) {
  window.graphHighlightFilters[filter] = !window.graphHighlightFilters[filter];
  updateHighlightButtons();
  if (window.updateGraphUrl) window.updateGraphUrl(true);
  // Mermaid encodes the highlight in its generated code, so it needs a full
  // redraw; the custom renderer just restyles in place.
  const mode = typeof currentGraphMode !== "undefined" ? currentGraphMode : "custom";
  if (mode === "custom") {
    applyGraphHighlights();
  } else {
    redrawGraph();
  }
}
window.toggleHighlightFilter = toggleHighlightFilter;

// Restyle the rendered custom graph for the active highlight toggles: matching
// edges get their highlight color, everything else is dimmed. With both
// toggles off every element returns to its base styling.
function applyGraphHighlights() {
  const nodes = window._graphNodeElements;
  const edges = window._graphEdgeElements;
  if (!nodes || !edges) return;

  const { outdated, snapshot } = window.graphHighlightFilters;
  if (!outdated && !snapshot) {
    edges.forEach((els) => {
      els.path.style.opacity = "1";
      els.path.setAttribute("stroke", els.path.dataset.baseStroke);
      els.path.setAttribute("stroke-width", els.path.dataset.baseWidth);
    });
    nodes.forEach((el) => {
      el.style.opacity = "1";
    });
    return;
  }

  const litNodes = new Set();
  edges.forEach((els, key) => {
    const isOutdated = outdated && els.path.dataset.outdated === "1";
    const isSnapshot = snapshot && els.path.dataset.snapshot === "1";
    if (isOutdated || isSnapshot) {
      els.path.style.opacity = "1";
      els.path.setAttribute("stroke", isOutdated ? GRAPH_OUTDATED_COLOR : GRAPH_SNAPSHOT_COLOR);
      els.path.setAttribute("stroke-width", "3");
      const [from, to] = key.split("-->");
      litNodes.add(from);
      litNodes.add(to);
    } else {
      els.path.style.opacity = "0.12";
      els.path.setAttribute("stroke", els.path.dataset.baseStroke);
      els.path.setAttribute("stroke-width", els.path.dataset.baseWidth);
    }
  });
  nodes.forEach((el, name) => {
    el.style.opacity = litNodes.has(name) ? "1" : "0.2";
  });
}
window.applyGraphHighlights = applyGraphHighlights;

// ── Redraw Coordinator ───────────────────────────────────────────────

function redrawGraph() {
  if (typeof lastGraphReport === "undefined" || !lastGraphReport) return;

  const mode = typeof currentGraphMode !== "undefined" ? currentGraphMode : "custom";
  const direction = typeof currentGraphDirection !== "undefined" ? currentGraphDirection : "TB";

  const filteredReport = getFilteredReport(lastGraphReport);

  const customSvg = document.getElementById("custom-graph");
  const mermaidDiv = document.getElementById("mermaid-graph");

  if (mode === "custom" && customSvg) {
    renderCustomGraph(filteredReport, customSvg, direction);
  } else if (mermaidDiv) {
    // renderGraph is defined in service.html and it uses lastGraphReport globally.
    // We should change it to accept a report parameter if possible,
    // but for now we can temporarily swap lastGraphReport or just accept it's only for custom graph.
    // User asked to "redraw the graph", implying it should work for all modes.

    // Actually, Mermaid renderGraph takes a second parameter for detailed mode.
    // Let's modify service.html's renderGraph to accept report as first arg.
    if (typeof renderGraph === "function") {
      renderGraph(filteredReport, mode === "detailed");
    }
  }
}
window.redrawGraph = redrawGraph;

function getFilteredReport(report) {
  if (!report) return null;
  let deps = [...(report.dependency_graph || [])];
  let missing = report.missing_endpoints;

  // Main view (ADR-0004): the edges are the current trunk pin set, not the
  // accumulated dev activity; the missing-endpoints overlay is dev-view data.
  if (window.graphStreamView === "main") {
    const stabilityOf = (t) => {
      const svc = (report.services_detailed || []).find((s) => s.name === t.service);
      const v = svc
        ? (svc.versions || []).find(
            (x) =>
              x.version === t.version &&
              (x.api_type || "openapi").toLowerCase() === (t.api_type || "openapi").toLowerCase(),
          )
        : null;
      return v ? v.stability : "ga";
    };
    deps = (report.trunk_graph || []).map((t) => ({
      api_type: t.api_type,
      client: t.client,
      service: t.service,
      version: t.version,
      stability: stabilityOf(t),
      path: t.path,
      method: t.method,
      deprecated: false,
    }));
    missing = [];
  }

  // 1. Circular dependencies filter
  if (window.graphRedrawMode === "circular") {
    const adjMap = new Map();
    deps.forEach((d) => {
      if (!adjMap.has(d.client)) adjMap.set(d.client, []);
      if (!adjMap.get(d.client).includes(d.service)) adjMap.get(d.client).push(d.service);
    });
    const cycleEdges = graphDetectCycles(adjMap);
    const cycleNodes = new Set();
    cycleEdges.forEach((e) => {
      const [from, to] = e.split("-->");
      cycleNodes.add(from);
      cycleNodes.add(to);
    });
    // Only keep edges where BOTH nodes are in a cycle
    deps = deps.filter((d) => cycleNodes.has(d.client) && cycleNodes.has(d.service));
  }

  // 2. Focus filter (multi-service tag cloud)
  if (window.graphFocusTags && window.graphFocusTags.length > 0) {
    const focusNodes = new Set();
    for (const tag of window.graphFocusTags) {
      const target = tag.toLowerCase();
      const exactMatch = deps.find(
        (d) => d.client.toLowerCase() === target || d.service.toLowerCase() === target,
      );
      if (exactMatch) {
        const realName =
          exactMatch.client.toLowerCase() === target ? exactMatch.client : exactMatch.service;
        focusNodes.add(realName);
        deps.forEach((d) => {
          if (d.client === realName) focusNodes.add(d.service);
          if (d.service === realName) focusNodes.add(d.client);
        });
      }
    }
    if (focusNodes.size > 0) {
      deps = deps.filter((d) => focusNodes.has(d.client) && focusNodes.has(d.service));
    }
  }

  // 3. Protocol filter
  deps = deps.filter((d) => {
    const type = (d.api_type || "").toLowerCase();
    if (type === "openapi" && !window.graphProtocolFilters.openapi) return false;
    if (type === "asyncapi" && !window.graphProtocolFilters.asyncapi) return false;
    if (type === "proto" && !window.graphProtocolFilters.proto) return false;
    return true;
  });

  return { ...report, dependency_graph: deps, missing_endpoints: missing };
}
window.getFilteredReport = getFilteredReport;

function setGraphRedrawMode(mode) {
  window.graphRedrawMode = mode;
  ["all", "circular"].forEach((m) => {
    const btn = document.getElementById(`graph-redraw-${m}`);
    if (btn) {
      btn.className =
        m === mode
          ? "px-3 py-1.5 bg-indigo-600 text-white font-medium"
          : "px-3 py-1.5 text-slate-600 hover:bg-slate-50 font-medium";
    }
  });
  redrawGraph();
}
window.setGraphRedrawMode = setGraphRedrawMode;

// Switch between the dev and main stream views (ADR-0004). The legend is
// view-aware: entries carrying data-stream-only show only in their view.
function setStreamView(view) {
  window.graphStreamView = view;
  ["dev", "main"].forEach((v) => {
    const btn = document.getElementById(`stream-${v}`);
    if (btn) {
      btn.className =
        v === view
          ? "px-3 py-1.5 bg-indigo-600 text-white font-medium"
          : "px-3 py-1.5 text-slate-600 hover:bg-slate-50 font-medium";
    }
  });
  document.querySelectorAll("[data-stream-only]").forEach((el) => {
    el.classList.toggle("hidden", el.dataset.streamOnly !== view);
  });
  redrawGraph();
}
window.setStreamView = setStreamView;

function addFocusTag(name) {
  if (!name || !name.trim()) return;
  const trimmed = name.trim();
  if (window.graphFocusTags.includes(trimmed)) return;
  window.graphFocusTags.push(trimmed);
  renderFocusTags();
  if (window.updateGraphUrl) window.updateGraphUrl(true);
  redrawGraph();
}
window.addFocusTag = addFocusTag;

function removeFocusTag(name) {
  window.graphFocusTags = window.graphFocusTags.filter((t) => t !== name);
  renderFocusTags();
  if (window.updateGraphUrl) window.updateGraphUrl(true);
  redrawGraph();
}
window.removeFocusTag = removeFocusTag;

function renderFocusTags() {
  const container = document.getElementById("focus-tags-container");
  if (!container) return;
  container.innerHTML = "";
  window.graphFocusTags.forEach((tag) => {
    const pill = document.createElement("span");
    pill.className =
      "inline-flex items-center gap-1 px-2 py-0.5 bg-indigo-100 text-indigo-700 rounded-full text-xs font-medium";
    pill.innerHTML = `${_graphEscapeHtml(tag)}<button onclick="removeFocusTag('${tag.replace(/'/g, "\\'")}')"
            class="hover:text-indigo-900 cursor-pointer text-indigo-400 font-bold leading-none">&times;</button>`;
    container.appendChild(pill);
  });
}
window.renderFocusTags = renderFocusTags;

function applyGraphFocus() {
  const input = document.getElementById("graph-service-filter");
  if (!input) return;
  const val = input.value.trim();
  input.value = "";
  if (val) addFocusTag(val);
}
window.applyGraphFocus = applyGraphFocus;

function toggleProtocolFilter(protocol) {
  window.graphProtocolFilters[protocol] = !window.graphProtocolFilters[protocol];

  const btn = document.getElementById(`filter-${protocol}`);
  if (btn) {
    if (window.graphProtocolFilters[protocol]) {
      btn.className = "px-2.5 py-1.5 bg-indigo-600 text-white font-medium";
    } else {
      btn.className = "px-2.5 py-1.5 bg-white text-slate-600 hover:bg-slate-50 font-medium";
    }
  }
  if (window.updateGraphUrl) window.updateGraphUrl(true);
  redrawGraph();
}
window.toggleProtocolFilter = toggleProtocolFilter;

// ── Helpers ──────────────────────────────────────────────────────────

function _graphEscapeHtml(s) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function _measureNode(name, hasIcon) {
  const fontSize = 16;
  const charW = 9; // estimated average char width for 16px bold sans
  const idealCharsPerLine = 16;
  const maxLines = 3;
  const iconPadding = hasIcon ? 30 : 0;

  if (name.length <= idealCharsPerLine) {
    return {
      width: Math.max(100, Math.ceil(name.length * charW) + 40 + iconPadding),
      height: 48,
      lines: [name],
    };
  }

  // Wrap long names
  const lines = [];
  let remaining = name;
  while (remaining.length > 0 && lines.length < maxLines) {
    if (remaining.length <= idealCharsPerLine) {
      lines.push(remaining);
      remaining = "";
    } else {
      // Try to find a good break point (e.g. at capitals or symbols)
      let breakIdx = idealCharsPerLine;
      // Scan backwards from ideal for a capital or underscore/dash
      for (let j = idealCharsPerLine; j > idealCharsPerLine / 2; j--) {
        const char = remaining[j];
        if (/[A-Z0-9_\-\.]/.test(char)) {
          breakIdx = j;
          break;
        }
      }
      lines.push(remaining.slice(0, breakIdx));
      remaining = remaining.slice(breakIdx);
    }
  }
  if (remaining.length > 0) {
    lines[lines.length - 1] = lines[lines.length - 1].slice(0, -1) + "…";
  }

  const maxLineLen = Math.max(...lines.map((l) => l.length));
  return {
    width: Math.max(100, Math.ceil(maxLineLen * charW) + 40 + iconPadding),
    height: lines.length * 20 + 24,
    lines: lines,
  };
}

// ── Cycle detection (reusable from service.html) ─────────────────────

function graphDetectCycles(adjMap) {
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

// ── Main renderer ────────────────────────────────────────────────────

function renderCustomGraph(report, svgElement, direction) {
  // Clear previous content
  while (svgElement.firstChild) svgElement.removeChild(svgElement.firstChild);

  const deps = report.dependency_graph;
  if (!deps || deps.length === 0) {
    const warningDiv = document.getElementById("graph-cycle-warning");
    if (warningDiv) warningDiv.classList.add("hidden");
    svgElement.setAttribute("viewBox", "0 0 400 120");
    svgElement.style.height = "120px";
    const text = document.createElementNS("http://www.w3.org/2000/svg", "text");
    text.setAttribute("x", "200");
    text.setAttribute("y", "60");
    text.setAttribute("text-anchor", "middle");
    text.setAttribute("dominant-baseline", "central");
    text.setAttribute("fill", "#94a3b8");
    text.setAttribute("font-size", "16");
    text.setAttribute("font-style", "italic");
    text.textContent = "No dependencies match the current filters";
    svgElement.appendChild(text);
    return;
  }

  // Layout direction: 'TB' (default, top→bottom) or 'LR' (left→right).
  // In LR mode the rank axis becomes X, so we swap the roles of x/y
  // across the entire post-layout pipeline (brick stagger, rank packing,
  // cluster bounds, edge tangents).
  const dir = direction === "LR" ? "LR" : "TB";
  const isLR = dir === "LR";

  // Build missing edge keys from report.missing_endpoints
  const missingEdgeKeys = new Set();
  if (report.missing_endpoints) {
    report.missing_endpoints.forEach((me) => {
      missingEdgeKeys.add(`${me.client}-->${me.service}`);
    });
  }

  // Build graph data
  const allNodes = new Set();
  const adjMap = new Map();
  const edgeLabels = new Map(); // "from-->to" -> [dependency, ...]

  deps.forEach((d) => {
    const from = d.client,
      to = d.service;
    allNodes.add(from);
    allNodes.add(to);
    if (!adjMap.has(from)) adjMap.set(from, []);
    if (!adjMap.get(from).includes(to)) adjMap.get(from).push(to);
    const key = `${from}-->${to}`;
    if (!edgeLabels.has(key)) edgeLabels.set(key, []);
    edgeLabels.get(key).push(d);
  });

  const clientNodes = new Set(deps.map((d) => d.client));
  const serviceNodes = new Set(deps.map((d) => d.service));
  const cycleEdges = graphDetectCycles(adjMap);

  // Outdated / Snapshot-pinned flags per aggregated edge — and in the main
  // view, major-lag conflicts against the producer's trunk version.
  const isMainView = window.graphStreamView === "main";
  const trunkVersionMap = isMainView ? graphTrunkVersionMap(report) : null;
  const latestGaMap = graphLatestGaMap(report);
  const edgeHighlightFlags = graphEdgeFlags(report, latestGaMap, trunkVersionMap);

  // Main view: a trunk-provided producer belongs in the picture even before
  // anyone pins it — its trunk version is a statement on its own.
  if (isMainView && trunkVersionMap) {
    for (const key of trunkVersionMap.keys()) {
      const name = key.split("|")[0];
      allNodes.add(name);
      serviceNodes.add(name);
    }
  }

  // Detect bidirectional PUB/SUB edges (both directions between same pair)
  const pubsubBidirectional = new Set();
  const edgeMethods = new Map(); // "A-->B" -> Set of methods
  deps.forEach((d) => {
    const key = `${d.client}-->${d.service}`;
    if (!edgeMethods.has(key)) edgeMethods.set(key, new Set());
    edgeMethods.get(key).add((d.method || "").toUpperCase());
  });
  for (const [key, methods] of edgeMethods) {
    const [from, to] = key.split("-->");
    const reverseKey = `${to}-->${from}`;
    const reverseMethods = edgeMethods.get(reverseKey);
    if (reverseMethods) {
      const hasPub = methods.has("PUB") || reverseMethods.has("PUB");
      const hasSub = methods.has("SUB") || reverseMethods.has("SUB");
      if (hasPub && hasSub) {
        pubsubBidirectional.add(key);
        pubsubBidirectional.add(reverseKey);
      }
    }
  }

  // Derive client tags from dependency api_type (clients don't have service_tags)
  if (!report.service_tags) report.service_tags = {};
  deps.forEach((d) => {
    const type = (d.api_type || "").toLowerCase();
    if (type === "asyncapi") {
      if (!report.service_tags[d.client]) report.service_tags[d.client] = [];
      if (!report.service_tags[d.client].includes("messaging"))
        report.service_tags[d.client].push("messaging");
    } else if (type === "proto") {
      if (!report.service_tags[d.client]) report.service_tags[d.client] = [];
      if (!report.service_tags[d.client].includes("grpc"))
        report.service_tags[d.client].push("grpc");
    }
  });

  // Inject virtual KAFKA node if any asyncapi dependency exists
  const KAFKA_NODE = "KAFKA";
  const messagingRegisterEdges = new Set();
  const asyncClients = new Set();
  deps.forEach((d) => {
    if ((d.api_type || "").toLowerCase() === "asyncapi") {
      asyncClients.add(d.client);
      asyncClients.add(d.service);
    }
  });
  if (asyncClients.size > 0) {
    allNodes.add(KAFKA_NODE);
    serviceNodes.add(KAFKA_NODE);
    // Ensure service_tags includes messaging tag for the virtual node
    if (!report.service_tags) report.service_tags = {};
    report.service_tags[KAFKA_NODE] = ["messaging"];
    // Link all async-involved services to KAFKA
    for (const svc of asyncClients) {
      if (!adjMap.has(svc)) adjMap.set(svc, []);
      if (!adjMap.get(svc).includes(KAFKA_NODE)) adjMap.get(svc).push(KAFKA_NODE);
      const key = `${svc}-->${KAFKA_NODE}`;
      if (!edgeLabels.has(key)) edgeLabels.set(key, []);
      edgeLabels.get(key).push({ method: "PUB/SUB", path: "register", api_type: "AsyncAPI" });
      // Mark these edges as messaging-register (grey dashed, no arrows)
      messagingRegisterEdges.add(key);
      clientNodes.add(svc);
    }
  }

  // Identify missing services: services where ALL inbound edges are missing
  const missingServices = new Set();
  for (const svc of serviceNodes) {
    const inboundKeys = deps
      .filter((d) => d.service === svc)
      .map((d) => `${d.client}-->${d.service}`);
    if (inboundKeys.length > 0 && inboundKeys.every((k) => missingEdgeKeys.has(k))) {
      missingServices.add(svc);
    }
  }

  // Show/hide cycle warning
  const warningDiv = document.getElementById("graph-cycle-warning");
  if (warningDiv) {
    warningDiv.classList.toggle("hidden", cycleEdges.size === 0);
  }

  // ── Dagre layout ─────────────────────────────────────────────────
  const g = new dagre.graphlib.Graph({ compound: true });
  g.setGraph({
    rankdir: dir,
    // Increased spacing for better clarity
    nodesep: isLR ? 100 : 120,
    ranksep: isLR ? 150 : 250,
    edgesep: 80,
    marginx: 40,
    marginy: 60,
  });
  g.setDefaultEdgeLabel(() => ({}));

  const nodeMetrics = new Map();
  const serviceMetaMap = new Map();
  if (report.services_detailed) {
    report.services_detailed.forEach((s) => serviceMetaMap.set(s.name, s));
  }

  const domains = new Set();
  for (const node of allNodes) {
    const meta = serviceMetaMap.get(node);
    if (meta && meta.domain) domains.add(meta.domain);
  }

  // Set domain clusters
  domains.forEach((domain) => {
    g.setNode(`domain:${domain}`, { label: domain, clusterLabelPos: "top" });
  });

  for (const node of allNodes) {
    const meta = serviceMetaMap.get(node);
    const m = _measureNode(node, !!(meta && meta.icon));
    nodeMetrics.set(node, m);
    g.setNode(node, { label: node, width: m.width, height: m.height });
    if (meta && meta.domain) {
      g.setParent(node, `domain:${meta.domain}`);
    }
  }

  const nodeRole = new Map();
  for (const node of allNodes) {
    const isC = clientNodes.has(node);
    const isS = serviceNodes.has(node);
    if (isC && isS)
      nodeRole.set(node, 1); // BOTH
    else if (isC)
      nodeRole.set(node, 0); // CLIENT ONLY
    else nodeRole.set(node, 2); // SERVICE ONLY
  }

  for (const [from, tos] of adjMap) {
    for (const to of tos) g.setEdge(from, to);
  }
  dagre.layout(g);

  // ── Structured layout (math-based compaction) ────────────────────
  // After Dagre has assigned coordinates, we post-process each rank to
  // re-pack all nodes with a uniform gap so empty space is reclaimed
  // and nodes follow a logical "client-to-server" flow (Autobahn layout).
  // All offsets relative to Dagre output are recorded in nodeFOffsets
  // maps so edges can be interpolated precisely.

  // Axis abstraction: F = along-flow (within a rank), R = rank axis.
  // TB: F=x, R=y, along-flow node extent = node width.
  // LR: F=y, R=x, along-flow node extent = node height.
  const F = isLR ? "y" : "x";
  const R = isLR ? "x" : "y";

  const nodeFOffsets = new Map(); // delta along flow axis
  for (const node of allNodes) {
    nodeFOffsets.set(node, 0);
  }

  const GAP = isLR ? 25 : 30; // gap between units on a rank (flow axis)
  const GROUP_GAP = isLR ? 100 : 120; // larger gap between different roles (flow axis)

  // Step 1 — group nodes by rank and calculate global lane metrics.
  const unitsByRank = new Map(); // rankR -> [{id, cf, width, role}]
  for (const node of allNodes) {
    const nd = g.node(node);
    const key = Math.round(nd[R]);
    if (!unitsByRank.has(key)) unitsByRank.set(key, []);
    const w = isLR ? nd.height : nd.width;
    unitsByRank.get(key).push({ id: node, cf: nd[F], width: w, role: nodeRole.get(node) });
  }

  const maxGroupWidth = [0, 0, 0];
  let sumOldMid = 0;
  unitsByRank.forEach((units) => {
    units.sort((a, b) => {
      if (a.role !== b.role) return a.role - b.role;
      return a.cf - b.cf;
    });
    const roleGroups = [[], [], []];
    units.forEach((u) => roleGroups[u.role].push(u));
    for (let i = 0; i < 3; i++) {
      if (roleGroups[i].length > 0) {
        const w =
          roleGroups[i].reduce((acc, u) => acc + u.width, 0) + (roleGroups[i].length - 1) * GAP;
        maxGroupWidth[i] = Math.max(maxGroupWidth[i], w);
      }
    }
    sumOldMid += (units[0].cf + units[units.length - 1].cf) / 2;
  });

  // Determine lane starts based on global max widths
  let currentX = 0;
  const laneStart = [0, 0, 0];
  for (let i = 0; i < 3; i++) {
    laneStart[i] = currentX;
    if (maxGroupWidth[i] > 0) {
      currentX += maxGroupWidth[i];
      let hasRight = false;
      for (let j = i + 1; j < 3; j++) if (maxGroupWidth[j] > 0) hasRight = true;
      if (hasRight) currentX += GROUP_GAP;
    }
  }

  const totalLanesWidth = currentX;
  const globalMid = unitsByRank.size > 0 ? sumOldMid / unitsByRank.size : 0;
  const globalOffset = globalMid - totalLanesWidth / 2;

  // Step 2 — assign new positions using the global lanes.
  unitsByRank.forEach((units) => {
    const roleGroups = [[], [], []];
    units.forEach((u) => roleGroups[u.role].push(u));
    for (let i = 0; i < 3; i++) {
      // All groups of the same role across all ranks now share the same
      // starting offset, ensuring "leftest nodes same level".
      let cursor = laneStart[i] + globalOffset;
      roleGroups[i].forEach((u) => {
        u.newCf = cursor + u.width / 2;
        cursor += u.width + GAP;
      });
    }
  });

  // Step 2 — apply computed positions.
  unitsByRank.forEach((units) => {
    units.forEach((u) => {
      const nd = g.node(u.id);
      nodeFOffsets.set(u.id, u.newCf - nd[F]);
    });
  });

  for (const node of allNodes) {
    const nd = g.node(node);
    nd[F] += nodeFOffsets.get(node) || 0;
  }

  // Re-calculate cluster bounds after compaction
  g.nodes().forEach((v) => {
    if (v.startsWith("domain:")) {
      const clusterNode = g.node(v);
      const children = g.children(v);
      if (children && children.length > 0) {
        let cMinX = Infinity,
          cMinY = Infinity,
          cMaxX = -Infinity,
          cMaxY = -Infinity;
        children.forEach((childId) => {
          const child = g.node(childId);
          if (!child) return;
          const chw = child.width / 2;
          const chh = child.height / 2;
          cMinX = Math.min(cMinX, child.x - chw);
          cMaxX = Math.max(cMaxX, child.x + chw);
          cMinY = Math.min(cMinY, child.y - chh);
          cMaxY = Math.max(cMaxY, child.y + chh);
        });
        if (cMinX !== Infinity) {
          const clusterPadding = 40;
          clusterNode.width = cMaxX - cMinX + clusterPadding * 2;
          clusterNode.height = cMaxY - cMinY + clusterPadding * 2;
          clusterNode.x = (cMinX + cMaxX) / 2;
          clusterNode.y = (cMinY + cMaxY) / 2;
        }
      }
    }
  });

  // Recompute edge endpoints from final node positions. Anchor on the
  // rank-axis border of each node (top/bottom for TB, left/right for LR).
  g.edges().forEach((e) => {
    const edgeData = g.edge(e);
    const nv = g.node(e.v);
    const nw = g.node(e.w);
    const vIsBefore = nv[R] <= nw[R];
    const p0 = { x: nv.x, y: nv.y };
    const p1 = { x: nw.x, y: nw.y };

    const vRExtent = isLR ? nv.width : nv.height;
    const wRExtent = isLR ? nw.width : nw.height;
    const vFExtent = isLR ? nv.height : nv.width;
    const wFExtent = isLR ? nw.height : nw.width;

    // Anchor on the rank-axis border of each node
    p0[R] += vIsBefore ? vRExtent / 2 : -vRExtent / 2;
    p1[R] += vIsBefore ? -wRExtent / 2 : wRExtent / 2;

    // Offset along flow axis (F) to reduce overlap (66% start -> 33% end)
    p0[F] += (0.66 - 0.5) * vFExtent;
    p1[F] += (0.33 - 0.5) * wFExtent;

    edgeData.points = [p0, p1];
  });

  // Calculate the actual bounding box of all nodes and clusters after compaction
  let minX = Infinity,
    minY = Infinity,
    maxX = -Infinity,
    maxY = -Infinity;
  g.nodes().forEach((v) => {
    const node = g.node(v);
    if (!node || node.width === undefined) return;
    const hw = node.width / 2;
    const hh = node.height / 2;
    minX = Math.min(minX, node.x - hw);
    maxX = Math.max(maxX, node.x + hw);
    minY = Math.min(minY, node.y - hh);
    maxY = Math.max(maxY, node.y + hh);
  });

  // Fallback if empty
  if (minX === Infinity) {
    minX = 0;
    minY = 0;
    maxX = 800;
    maxY = 600;
  }

  const contentW = maxX - minX;
  const contentH = maxY - minY;
  const centerX = (minX + maxX) / 2;
  const centerY = (minY + maxY) / 2;

  // Viewport-sized canvas: fill available window height
  const availableH = window.innerHeight - svgElement.getBoundingClientRect().top - 40;
  svgElement.style.height = Math.max(availableH, 300) + "px";
  const vw = svgElement.clientWidth;
  const vh = svgElement.clientHeight;
  svgElement.setAttribute("viewBox", `0 0 ${vw} ${vh}`);

  // Defs for arrowheads
  const defs = document.createElementNS("http://www.w3.org/2000/svg", "defs");
  [
    ["arrow", "#94a3b8"],
    ["arrow-red", "#a855f7"],
    ["arrow-orange", "#f97316"],
    ["arrow-green", "#22c55e"],
  ].forEach(([id, color]) => {
    const marker = document.createElementNS("http://www.w3.org/2000/svg", "marker");
    marker.setAttribute("id", id);
    marker.setAttribute("viewBox", "0 0 10 10");
    marker.setAttribute("refX", "10");
    marker.setAttribute("refY", "5");
    marker.setAttribute("markerWidth", "8");
    marker.setAttribute("markerHeight", "8");
    marker.setAttribute("orient", "auto-start-reverse");
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", "M 0 0 L 10 5 L 0 10 z");
    path.setAttribute("fill", color);
    marker.appendChild(path);
    defs.appendChild(marker);
  });
  svgElement.appendChild(defs);

  // Main group for zoom/pan
  const mainG = document.createElementNS("http://www.w3.org/2000/svg", "g");
  mainG.setAttribute("class", "graph-main");
  svgElement.appendChild(mainG);

  // ── Render clusters ──────────────────────────────────────────────
  g.nodes().forEach((v) => {
    const node = g.node(v);
    if (v.startsWith("domain:")) {
      const cluster = document.createElementNS("http://www.w3.org/2000/svg", "rect");
      cluster.setAttribute("x", node.x - node.width / 2);
      cluster.setAttribute("y", node.y - node.height / 2);
      cluster.setAttribute("width", node.width);
      cluster.setAttribute("height", node.height);
      cluster.setAttribute("rx", "12");
      cluster.setAttribute("ry", "12");
      cluster.setAttribute("fill", "rgba(99, 102, 241, 0.03)");
      cluster.setAttribute("stroke", "rgba(99, 102, 241, 0.15)");
      cluster.setAttribute("stroke-width", "1.5");
      cluster.setAttribute("stroke-dasharray", "4 4");
      mainG.appendChild(cluster);

      const label = document.createElementNS("http://www.w3.org/2000/svg", "text");
      label.setAttribute("x", node.x - node.width / 2 + 10);
      label.setAttribute("y", node.y - node.height / 2 + 15);
      label.setAttribute("fill", "rgba(99, 102, 241, 0.5)");
      label.setAttribute("font-size", "10");
      label.setAttribute("font-weight", "bold");
      label.setAttribute("text-transform", "uppercase");
      label.textContent = node.label;
      mainG.appendChild(label);
    }
  });

  // ── Render edges ─────────────────────────────────────────────────
  const edgeElements = new Map(); // "from-->to" -> { path, hitArea }

  g.edges().forEach((e) => {
    const edgeData = g.edge(e);
    const key = `${e.v}-->${e.w}`;
    const isCycle = cycleEdges.has(key);
    const points = edgeData.points;
    const p0 = points[0];
    const p1 = points[points.length - 1];

    // Elegant cubic Bezier routing, rank-axis aware.
    //  - Normal cross-rank edges: control points pulled along the rank
    //    axis (Y in TB, X in LR) so curves leave/enter nodes
    //    perpendicular to their border — a smooth "river" flow.
    //  - Same-rank / reverse edges (tiny rank delta): S-curve that bows
    //    sideways along the flow axis so arrows don't cut through nodes.
    const dF = p1[F] - p0[F];
    const dR = p1[R] - p0[R];
    const absDR = Math.abs(dR);
    const c1 = { x: p0.x, y: p0.y };
    const c2 = { x: p1.x, y: p1.y };
    if (absDR < 30) {
      // Sideways / same-rank: tangents along the flow axis.
      const bow = Math.max(40, Math.abs(dF) * 0.3);
      const sign = dF >= 0 ? 1 : -1;
      c1[F] = p0[F] + sign * bow;
      c2[F] = p1[F] - sign * bow;
      c1[R] = p0[R] + bow * 0.6;
      c2[R] = p1[R] + bow * 0.6;
    } else {
      // Cross-rank: tangents along the rank axis.
      const tension = Math.min(Math.max(absDR * 0.5, 40), 140);
      const dir = dR >= 0 ? 1 : -1;
      c1[R] = p0[R] + dir * tension;
      c2[R] = p1[R] - dir * tension;
    }
    const d = `M ${p0.x} ${p0.y} C ${c1.x} ${c1.y}, ${c2.x} ${c2.y}, ${p1.x} ${p1.y}`;

    // Invisible wide hit area for hover
    const hitArea = document.createElementNS("http://www.w3.org/2000/svg", "path");
    hitArea.setAttribute("d", d);
    hitArea.setAttribute("stroke", "transparent");
    hitArea.setAttribute("stroke-width", "16");
    hitArea.setAttribute("fill", "none");
    hitArea.setAttribute("class", "edge-hit");
    hitArea.dataset.from = e.v;
    hitArea.dataset.to = e.w;
    mainG.appendChild(hitArea);

    // Visible edge
    const isMissing = missingEdgeKeys.has(key);
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", d);
    const isBidirectionalPubSub = pubsubBidirectional.has(key);
    const isMessagingRegister = messagingRegisterEdges.has(key);
    let edgeColor, edgeWidth, markerEnd;
    if (isCycle) {
      edgeColor = "#a855f7";
      edgeWidth = "3";
      markerEnd = "url(#arrow-red)";
    } else if (isMissing) {
      edgeColor = "#f97316";
      edgeWidth = "2";
      markerEnd = "url(#arrow-orange)";
    } else if (isMessagingRegister) {
      edgeColor = "#3b82f6";
      edgeWidth = "1.5";
      markerEnd = "";
    } else if (isBidirectionalPubSub) {
      edgeColor = "#94a3b8";
      edgeWidth = "2";
      markerEnd = "";
    } else if (edgeHighlightFlags.get(key) && edgeHighlightFlags.get(key).conflict) {
      // Main view: the pin lags the producer's trunk version by a major.
      edgeColor = "#dc2626";
      edgeWidth = "3";
      markerEnd = "url(#arrow-red)";
    } else {
      edgeColor = "#94a3b8";
      edgeWidth = "2";
      markerEnd = "url(#arrow)";
    }
    path.setAttribute("stroke", edgeColor);
    path.setAttribute("stroke-width", edgeWidth);
    path.setAttribute("fill", "none");
    if (markerEnd) path.setAttribute("marker-end", markerEnd);
    path.setAttribute("class", "graph-edge");
    path.dataset.from = e.v;
    path.dataset.to = e.w;
    if (isCycle) path.dataset.edgeType = "circular";
    else if (isMissing) path.dataset.edgeType = "missing";
    else if (isMessagingRegister) path.dataset.edgeType = "messaging-register";
    else if (isBidirectionalPubSub) path.dataset.edgeType = "pubsub-bidir";
    else path.dataset.edgeType = "normal";
    // Independent highlight flags + base styling, so the Outdated /
    // Snapshot-pinned toggles can restyle and restore without a redraw.
    const hFlags = edgeHighlightFlags.get(key);
    path.dataset.outdated = hFlags && hFlags.outdated ? "1" : "0";
    path.dataset.snapshot = hFlags && hFlags.snapshot ? "1" : "0";
    path.dataset.conflict = hFlags && hFlags.conflict ? "1" : "0";
    path.dataset.baseStroke = edgeColor;
    path.dataset.baseWidth = edgeWidth;
    if (isMissing || isBidirectionalPubSub || isMessagingRegister)
      path.setAttribute("stroke-dasharray", "6 3");
    mainG.appendChild(path);

    edgeElements.set(key, { path, hitArea });
  });

  // ── Render nodes ─────────────────────────────────────────────────
  const nodeElements = new Map(); // nodeName -> group element

  for (const node of allNodes) {
    const nd = g.node(node);
    const isClient = clientNodes.has(node);
    const isService = serviceNodes.has(node);

    const isMissingService = missingServices.has(node);
    const nodeTags = (report.service_tags && report.service_tags[node]) || [];
    const hasTag = (t) => nodeTags.includes(t);

    // Determine node colors
    let fill, stroke, textColor;

    if (isMissingService) {
      fill = "#fff7ed";
      stroke = "#f97316";
      textColor = "#9a3412";
    } else if (isClient && isService) {
      fill = "#fef3c7";
      stroke = "#f59e0b";
      textColor = "#92400e";
    } else if (isService) {
      fill = "#e0e7ff";
      stroke = "#6366f1";
      textColor = "#3730a3";
    } else {
      fill = "#ccfbf1";
      stroke = "#14b8a6";
      textColor = "#115e59";
    }

    const group = document.createElementNS("http://www.w3.org/2000/svg", "g");
    group.setAttribute("class", "graph-node");
    group.dataset.node = node;
    if (nodeTags.length > 0) group.dataset.tags = nodeTags.join(",");
    // Set role for legend highlighting
    if (isMissingService) group.dataset.role = "missing";
    else if (hasTag("messaging")) group.dataset.role = "tag-messaging";
    else if (hasTag("grpc")) group.dataset.role = "tag-grpc";
    else if (isClient && isService) group.dataset.role = "both";
    else if (isService) group.dataset.role = "service";
    else group.dataset.role = "client";
    group.style.cursor = "pointer";

    const cx = nd.x,
      cy = nd.y,
      hw = nd.width / 2,
      hh = nd.height / 2;

    // Draw standard rectangle
    const rect = document.createElementNS("http://www.w3.org/2000/svg", "rect");
    rect.setAttribute("x", cx - hw);
    rect.setAttribute("y", cy - hh);
    rect.setAttribute("width", nd.width);
    rect.setAttribute("height", nd.height);
    rect.setAttribute("rx", "8");
    rect.setAttribute("ry", "8");
    rect.setAttribute("fill", fill);
    rect.setAttribute("stroke", stroke);
    rect.setAttribute("stroke-width", "2");
    if (isMissingService) rect.setAttribute("stroke-dasharray", "6 3");
    group.appendChild(rect);

    // Add Symbols (Top-Left: ! for missing, Top-Right: ✉️ for AsyncAPI, 🔌 for gRPC)
    if (isMissingService) {
      const sym = document.createElementNS("http://www.w3.org/2000/svg", "text");
      sym.setAttribute("x", cx - hw + 8);
      sym.setAttribute("y", cy - hh + 13);
      sym.setAttribute("font-size", "12");
      sym.setAttribute("font-weight", "bold");
      sym.setAttribute("fill", textColor);
      sym.setAttribute("text-anchor", "middle");
      sym.textContent = "!";
      group.appendChild(sym);
    }

    const trSymbols = [];
    if (hasTag("messaging")) trSymbols.push({ type: "text", text: "🛢️" });
    if (hasTag("grpc")) trSymbols.push({ type: "text", text: "⛓️" });

    trSymbols.forEach((s, i) => {
      const x = cx + hw - 12 - i * 14;
      const y = cy - hh + 13;
      if (s.type === "text") {
        const sym = document.createElementNS("http://www.w3.org/2000/svg", "text");
        sym.setAttribute("x", x);
        sym.setAttribute("y", y);
        sym.setAttribute("font-size", "10");
        sym.setAttribute("text-anchor", "middle");
        sym.textContent = s.text;
        group.appendChild(sym);
      } else if (s.type === "svg") {
        const svgG = document.createElementNS("http://www.w3.org/2000/svg", "g");
        // Offset to center the 24x24 icon scaled by 0.45 (~11x11)
        svgG.setAttribute("transform", `translate(${x - 5.5}, ${y - 9.5}) scale(${s.scale})`);
        const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
        path.setAttribute("d", s.d);
        path.setAttribute("fill", textColor);
        svgG.appendChild(path);
        group.appendChild(svgG);
      }
    });

    // Double border for "both" nodes
    if (isClient && isService) {
      const inner = document.createElementNS("http://www.w3.org/2000/svg", "rect");
      inner.setAttribute("x", nd.x - nd.width / 2 + 4);
      inner.setAttribute("y", nd.y - nd.height / 2 + 4);
      inner.setAttribute("width", nd.width - 8);
      inner.setAttribute("height", nd.height - 8);
      inner.setAttribute("rx", "5");
      inner.setAttribute("ry", "5");
      inner.setAttribute("fill", "none");
      inner.setAttribute("stroke", stroke);
      inner.setAttribute("stroke-width", "1");
      inner.setAttribute("stroke-opacity", "0.5");
      group.appendChild(inner);
    }

    const m = nodeMetrics.get(node);
    const meta = serviceMetaMap.get(node);
    const lineH = 20;
    const totalTextH = m.lines.length * lineH;
    const hasIcon = !!(meta && meta.icon);
    const textXOffset = hasIcon ? 15 : 0;

    if (hasIcon) {
      const iconText = document.createElementNS("http://www.w3.org/2000/svg", "text");
      iconText.setAttribute("x", nd.x - m.width / 2 + 20);
      iconText.setAttribute("y", nd.y);
      iconText.setAttribute("text-anchor", "middle");
      iconText.setAttribute("dominant-baseline", "central");
      iconText.setAttribute("font-size", "20");
      iconText.textContent = meta.icon;
      group.appendChild(iconText);
    }

    m.lines.forEach((line, i) => {
      const text = document.createElementNS("http://www.w3.org/2000/svg", "text");
      text.setAttribute("x", nd.x + textXOffset);
      text.setAttribute("y", nd.y - totalTextH / 2 + i * lineH + lineH / 2);
      text.setAttribute("text-anchor", "middle");
      text.setAttribute("dominant-baseline", "central");
      text.setAttribute("fill", textColor);
      text.setAttribute("font-size", "16");
      text.setAttribute("font-family", "ui-sans-serif, system-ui, sans-serif");
      text.setAttribute("font-weight", "600");
      text.textContent = line;
      group.appendChild(text);
    });

    if (node.length > 25) {
      // Show full name on hover if it was wrapped/truncated
      const title = document.createElementNS("http://www.w3.org/2000/svg", "title");
      title.textContent = node;
      group.appendChild(title);
    }

    // Main view: label the producer with its trunk version.
    if (isMainView && trunkVersionMap) {
      const trunkV =
        trunkVersionMap.get(`${node}|openapi`) ||
        trunkVersionMap.get(`${node}|asyncapi`) ||
        trunkVersionMap.get(`${node}|proto`);
      if (trunkV) {
        const vText = document.createElementNS("http://www.w3.org/2000/svg", "text");
        vText.setAttribute("x", nd.x);
        vText.setAttribute("y", nd.y + nd.height / 2 - 7);
        vText.setAttribute("text-anchor", "middle");
        vText.setAttribute("fill", textColor);
        vText.setAttribute("font-size", "10");
        vText.setAttribute("font-family", "ui-monospace, monospace");
        vText.setAttribute("opacity", "0.75");
        vText.textContent = `v${trunkV}`;
        group.appendChild(vText);
      }
    }

    mainG.appendChild(group);
    nodeElements.set(node, group);
  }

  // ── Tooltip ──────────────────────────────────────────────────────
  let tooltipHideTimer = null;
  let tooltip = document.getElementById("graph-tooltip");
  if (!tooltip) {
    tooltip = document.createElement("div");
    tooltip.id = "graph-tooltip";
    tooltip.className =
      "fixed pointer-events-auto bg-slate-900/95 text-slate-100 p-3 rounded-xl text-xs font-sans leading-relaxed z-[1000] hidden max-w-sm shadow-2xl border border-slate-700/50 backdrop-blur-md transition-opacity duration-200";
    document.body.appendChild(tooltip);
  }

  // Re-bind tooltip mouse handlers every render so they reference the current closure
  tooltip.onmouseenter = () => {
    if (tooltipHideTimer) {
      clearTimeout(tooltipHideTimer);
      tooltipHideTimer = null;
    }
  };
  tooltip.onmouseleave = () => hideTooltip();

  function showTooltip(e, from, to) {
    if (tooltipHideTimer) {
      clearTimeout(tooltipHideTimer);
      tooltipHideTimer = null;
    }
    const key = `${from}-->${to}`;
    const endpoints = edgeLabels.get(key);
    if (!endpoints || endpoints.length === 0) return;

    tooltip.innerHTML = `
      <div class="mb-2 pb-1 border-b border-slate-700/50">
        <div class="text-[10px] text-slate-400 font-bold uppercase tracking-wider mb-0.5">Dependency</div>
        <div class="font-bold text-indigo-300 truncate">${_graphEscapeHtml(from)} <span class="text-slate-500 mx-0.5">→</span> ${_graphEscapeHtml(to)}</div>
      </div>
      <div class="space-y-1.5 max-h-64 overflow-y-auto pr-1 custom-scrollbar">
        ${endpoints
          .sort((a, b) => (a.path + a.method).localeCompare(b.path + b.method))
          .map((d) => {
            const method = d.method || "GET";
            const path = d.path || "/";
            const type = d.api_type || "OpenAPI";
            const isVirtual = path === "register";

            if (isVirtual) {
              return `<div class="text-slate-400 italic text-[11px] py-1 border-b border-slate-800/50 last:border-0">${_graphEscapeHtml(method)} ${_graphEscapeHtml(path)}</div>`;
            }

            const url = `/yaml.html?service=${encodeURIComponent(to)}&api_type=${encodeURIComponent(type)}&version=${encodeURIComponent(d.version || "")}&path=${encodeURIComponent(path)}&method=${encodeURIComponent(method)}`;

            let methodClass = "text-indigo-400";
            if (method === "POST" || method === "PUB") methodClass = "text-emerald-400";
            if (method === "DELETE") methodClass = "text-rose-400";
            if (method === "PUT") methodClass = "text-amber-400";

            const deprecatedBadge = d.deprecated
              ? '<span class="px-1 py-0.5 rounded bg-amber-500/20 text-amber-400 border border-amber-500/30 text-[9px] font-bold ml-1">DEPRECATED</span>'
              : "";

            return `
            <a href="${url}" class="group block p-1.5 rounded bg-white/5 hover:bg-indigo-500/20 border border-transparent hover:border-indigo-500/30 transition-all">
              <div class="flex items-center justify-between gap-2">
                <div class="flex items-center gap-1">
                  <span class="font-mono text-[11px] ${methodClass} font-bold">${_graphEscapeHtml(method)}</span>
                  ${deprecatedBadge}
                </div>
                <span class="text-[10px] text-slate-500 group-hover:text-indigo-300 transition-colors">${_graphEscapeHtml(type)} ${_graphEscapeHtml(String(d.version || ""))}${d.stability === "snapshot" ? ' <span class="text-amber-400 font-bold">SNAP</span>' : ""}</span>
              </div>
              <div class="font-mono text-[11px] text-slate-300 truncate group-hover:text-white transition-colors" title="${_graphEscapeHtml(path)}">${_graphEscapeHtml(path)}</div>
            </a>
          `;
          })
          .join("")}
      </div>
    `;
    tooltip.style.display = "block";
    tooltip.style.opacity = "0";

    // Position — overlap cursor by 2px so mouse can reach tooltip without gap
    let x = e.clientX + 8;
    let y = e.clientY + 8;

    tooltip.style.left = x + "px";
    tooltip.style.top = y + "px";

    // Check for viewport overflow
    const rect = tooltip.getBoundingClientRect();
    if (x + rect.width > window.innerWidth - 20) {
      x = e.clientX - rect.width - 4;
    }
    if (y + rect.height > window.innerHeight - 20) {
      y = e.clientY - rect.height - 4;
    }

    tooltip.style.left = Math.max(10, x) + "px";
    tooltip.style.top = Math.max(10, y) + "px";
    tooltip.style.opacity = "1";
  }

  function showNodeTooltip(e, name) {
    if (tooltipHideTimer) {
      clearTimeout(tooltipHideTimer);
      tooltipHideTimer = null;
    }

    const tags = report?.service_tags?.[name] || [];

    tooltip.innerHTML = `
      <div class="mb-3 pb-2 border-b border-slate-700/50">
        <div class="text-[10px] text-slate-400 font-bold uppercase tracking-wider mb-0.5">Service Node</div>
        <div class="font-bold text-lg text-white truncate mb-1">${_graphEscapeHtml(name)}</div>
        <div class="flex flex-wrap gap-1.5 mt-2">
          ${tags.map((t) => `<span class="px-1.5 py-0.5 rounded bg-slate-800 text-slate-300 border border-slate-700 text-[10px]">${_graphEscapeHtml(t)}</span>`).join("")}
        </div>
      </div>
      <div class="space-y-3">
        <a href="/producers.html?service=${encodeURIComponent(name)}" class="flex items-center justify-center gap-2 w-full py-2 px-3 rounded-lg bg-indigo-600 hover:bg-indigo-500 text-white font-medium transition-all shadow-lg shadow-indigo-500/20">
          <span>View Service Details</span>
          <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M14 5l7 7m0 0l-7 7m7-7H3"/></svg>
        </a>
      </div>
    `;

    tooltip.style.display = "block";
    tooltip.style.opacity = "0";

    // Position — overlap cursor by 2px so mouse can reach tooltip without gap
    let x = e.clientX + 8;
    let y = e.clientY + 8;
    tooltip.style.left = x + "px";
    tooltip.style.top = y + "px";

    const rect = tooltip.getBoundingClientRect();
    if (x + rect.width > window.innerWidth - 20) x = e.clientX - rect.width - 4;
    if (y + rect.height > window.innerHeight - 20) y = e.clientY - rect.height - 4;

    tooltip.style.left = Math.max(10, x) + "px";
    tooltip.style.top = Math.max(10, y) + "px";
    tooltip.style.opacity = "1";
  }

  function hideTooltip() {
    if (tooltipHideTimer) clearTimeout(tooltipHideTimer);
    tooltipHideTimer = setTimeout(() => {
      tooltip.style.display = "none";
    }, 300);
  }

  // Edge hover
  mainG.querySelectorAll(".edge-hit, .graph-edge").forEach((el) => {
    el.addEventListener("mouseenter", (e) => showTooltip(e, el.dataset.from, el.dataset.to));
    el.addEventListener("mouseleave", hideTooltip);
  });

  // Node hover
  mainG.querySelectorAll(".graph-node").forEach((el) => {
    el.addEventListener("mouseenter", (e) => showNodeTooltip(e, el.dataset.node));
    el.addEventListener("mouseleave", hideTooltip);
  });

  // ── Click-to-highlight ───────────────────────────────────────────
  let highlighted = null;

  function highlightNode(nodeName) {
    if (highlighted === nodeName) {
      clearHighlight();
      return;
    }
    highlighted = nodeName;

    // Find connected nodes (direct neighbors)
    const connected = new Set([nodeName]);
    const connectedEdges = new Set();
    for (const [key, labels] of edgeLabels) {
      const [from, to] = key.split("-->");
      if (from === nodeName || to === nodeName) {
        connected.add(from);
        connected.add(to);
        connectedEdges.add(key);
      }
    }

    // Dim everything, then highlight connected
    nodeElements.forEach((el, name) => {
      el.style.opacity = connected.has(name) ? "1" : "0.15";
      el.style.transition = "opacity 0.2s";
    });
    edgeElements.forEach((els, key) => {
      const op = connectedEdges.has(key) ? "1" : "0.1";
      els.path.style.opacity = op;
      els.path.style.transition = "opacity 0.2s";
    });
  }

  function clearHighlight() {
    highlighted = null;
    nodeElements.forEach((el) => {
      el.style.opacity = "1";
      el.style.transition = "opacity 0.2s";
    });
    edgeElements.forEach((els) => {
      els.path.style.opacity = "1";
      els.path.style.transition = "opacity 0.2s";
    });
  }

  mainG.querySelectorAll(".graph-node").forEach((el) => {
    el.addEventListener("click", (e) => {
      e.stopPropagation();
      highlightNode(el.dataset.node);
    });
  });

  svgElement.addEventListener("click", clearHighlight);

  // ── Legend hover-to-highlight ────────────────────────────────────
  // Expose nodeElements/edgeElements so the legend hover code can access them
  window._graphNodeElements = nodeElements;
  window._graphEdgeElements = edgeElements;

  // Re-apply the persistent Outdated / Snapshot-pinned highlight toggles.
  applyGraphHighlights();

  // ── Zoom & Pan ───────────────────────────────────────────────────
  let scale = 1;
  let panX = 0;
  let panY = 0;
  let isPanning = false,
    startX = 0,
    startY = 0;

  function fitToView() {
    const curVw = svgElement.viewBox.baseVal.width || vw;
    const curVh = svgElement.viewBox.baseVal.height || vh;
    const pad = 40; // use a reasonable padding
    scale = Math.min((curVw - pad * 2) / contentW, (curVh - pad * 2) / contentH);

    // Limit max scale to avoid pixelated nodes on tiny graphs
    if (scale > 1.2) scale = 1.2;

    // Pan to center the content
    panX = curVw / 2 - centerX * scale;
    panY = curVh / 2 - centerY * scale;

    updateTransform();
  }

  window.graphFitToView = fitToView;
  fitToView();

  function updateTransform() {
    mainG.setAttribute("transform", `translate(${panX},${panY}) scale(${scale})`);
  }

  svgElement.addEventListener(
    "wheel",
    (e) => {
      e.preventDefault();
      const delta = e.deltaY > 0 ? 0.9 : 1.1;
      const newScale = Math.max(0.2, Math.min(3, scale * delta));

      // Convert mouse position from screen pixels to SVG viewBox coordinates
      const rect = svgElement.getBoundingClientRect();
      const vb = svgElement.viewBox.baseVal;
      const mx = ((e.clientX - rect.left) / rect.width) * vb.width;
      const my = ((e.clientY - rect.top) / rect.height) * vb.height;

      // Zoom toward cursor in SVG space
      panX = mx - (mx - panX) * (newScale / scale);
      panY = my - (my - panY) * (newScale / scale);
      scale = newScale;
      updateTransform();
    },
    { passive: false },
  );

  svgElement.addEventListener("mousedown", (e) => {
    if (e.button !== 0) return;
    isPanning = true;
    startX = e.clientX;
    startY = e.clientY;
    svgElement.style.cursor = "grabbing";
  });

  window.addEventListener("mousemove", (e) => {
    if (!isPanning) return;
    const rect = svgElement.getBoundingClientRect();
    const vb = svgElement.viewBox.baseVal;
    const dx = ((e.clientX - startX) / rect.width) * vb.width;
    const dy = ((e.clientY - startY) / rect.height) * vb.height;
    panX += dx;
    panY += dy;
    startX = e.clientX;
    startY = e.clientY;
    updateTransform();
  });

  window.addEventListener("mouseup", () => {
    isPanning = false;
    svgElement.style.cursor = "default";
  });
}

/**
 * Returns the SVG markup string for copy/download.
 */
function getCustomGraphSVG(svgElement) {
  const clone = svgElement.cloneNode(true);
  if (!clone.getAttribute("xmlns")) {
    clone.setAttribute("xmlns", "http://www.w3.org/2000/svg");
  }
  return clone.outerHTML;
}

/**
 * Exports the current graph to a PNG image.
 */
function exportToPng(currentGraphMode) {
  let svg;
  if (currentGraphMode === "custom") {
    svg = document.getElementById("custom-graph");
  } else {
    const mermaidDiv = document.getElementById("mermaid-graph");
    svg = mermaidDiv ? mermaidDiv.querySelector("svg") : null;
  }
  if (!svg) return;
  if (currentGraphMode === "custom" && !svg.firstChild) return;

  const filename = "dependency_graph.png";

  // Clone the SVG to avoid modifying the live one
  const svgClone = svg.cloneNode(true);
  if (!svgClone.getAttribute("xmlns")) {
    svgClone.setAttribute("xmlns", "http://www.w3.org/2000/svg");
  }

  let width, height;

  if (currentGraphMode === "custom") {
    // For custom graph, we want to export the whole graph content, not just visible area
    const mainG = svg.querySelector("g");
    if (!mainG) return;

    const bbox = mainG.getBBox();
    width = bbox.width + 80; // Add some margin
    height = bbox.height + 80;

    // Reset transform in clone
    const cloneG = svgClone.querySelector("g");
    cloneG.setAttribute("transform", `translate(${-bbox.x + 40}, ${-bbox.y + 40})`);

    svgClone.setAttribute("width", width);
    svgClone.setAttribute("height", height);
    svgClone.setAttribute("viewBox", `0 0 ${width} ${height}`);
  } else {
    // For Mermaid, use its own viewBox/dimensions
    if (svg.viewBox && svg.viewBox.baseVal && svg.viewBox.baseVal.width) {
      width = svg.viewBox.baseVal.width;
      height = svg.viewBox.baseVal.height;
    } else {
      width = svg.clientWidth || 800;
      height = svg.clientHeight || 600;
    }
    svgClone.setAttribute("width", width);
    svgClone.setAttribute("height", height);
  }

  // Dynamic scale factor for high resolution (extra huge for detailed export)
  // We target ~10000 pixels on the longest side for maximum readability,
  // but cap it to avoid browser canvas limits (usually 16k-32k).
  const targetLongSide = 10000;
  const maxSafeDim = 16384;
  let factor = targetLongSide / Math.max(width, height);

  // Ensure we don't scale down below 3.0x (minimum high-res)
  // and don't scale up above 10.0x (prevent pixelation if tiny)
  factor = Math.max(3.0, Math.min(10.0, factor));

  // Final safety check for absolute dimensions
  if (width * factor > maxSafeDim) factor = maxSafeDim / width;
  if (height * factor > maxSafeDim) factor = Math.min(factor, maxSafeDim / height);

  // Ensure factor is at least 1.0 (never scale down)
  factor = Math.max(1.0, factor);

  // For Mermaid, we need to strip interactive elements that might taint the canvas
  // (like links or external resource references in foreignObject)
  if (currentGraphMode !== "custom") {
    svgClone.querySelectorAll("a").forEach((a) => {
      // Replace <a> with a <g> or just remove href
      a.removeAttribute("href");
      a.removeAttribute("xlink:href");
      a.style.cursor = "default";
    });
  }

  const svgData = new XMLSerializer().serializeToString(svgClone);
  const canvas = document.createElement("canvas");
  canvas.width = width * factor;
  canvas.height = height * factor;
  const ctx = canvas.getContext("2d");

  // Fill background white
  ctx.fillStyle = "white";
  ctx.fillRect(0, 0, canvas.width, canvas.height);

  const img = new Image();
  // Using a data URL instead of a Blob URL can sometimes help with tainted canvas issues
  // in some browsers, but let's stick to blob if it works, and just add error handling.
  const blob = new Blob([svgData], { type: "image/svg+xml;charset=utf-8" });
  const url = URL.createObjectURL(blob);

  img.onload = () => {
    try {
      ctx.drawImage(img, 0, 0, width * factor, height * factor);
      const pngUrl = canvas.toDataURL("image/png");
      const a = document.createElement("a");
      a.href = pngUrl;
      a.download = filename;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
    } catch (err) {
      console.error("Failed to export PNG:", err);
      alert(
        "Failed to export PNG. This usually happens because of browser security restrictions with Mermaid graphs. Try using the default 'Graph' mode for export.",
      );
    } finally {
      URL.revokeObjectURL(url);
    }
  };
  img.onerror = (err) => {
    console.error("Failed to load SVG for PNG export:", err);
    alert("Failed to load graph for export.");
    URL.revokeObjectURL(url);
  };
  img.src = url;
}
window.exportToPng = exportToPng;

// ── Debounced resize handler ────────────────────────────────────────
let _graphResizeTimer = null;
window.addEventListener("resize", () => {
  clearTimeout(_graphResizeTimer);
  _graphResizeTimer = setTimeout(() => redrawGraph(), 200);
});
