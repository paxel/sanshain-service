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
  // Peripheral Resources (#46) — shown by default, hideable like a protocol.
  periphery: true,
  // The "(unrequired)" box: producers of the scope nobody requires and that
  // require nothing — shown by default, hideable like a protocol.
  unrequired: true,
};
// Independent display-state highlights (never resolution inputs):
// outdated = Pin below the line's latest GA within the same major (a Pin to a
//   superseded Snapshot included);
// breakingOutdated = Pin a whole major behind it — the same binary "outdated"
//   before, which made one patch behind and three majors behind look identical;
// snapshot = Pin currently served from a Snapshot.
window.graphHighlightFilters = {
  outdated: false,
  breakingOutdated: false,
  snapshot: false,
};
// Which stream the graph shows (ADR-0004/0005): 'dev' is the classic
// latest-activity view; 'main' draws the trunk pin set; 'branch' draws one
// sanshain-branch's graph (window.graphBranchName / window.graphBranchPins).
window.graphStreamView = "dev";
window.graphBranchName = null;
window.graphBranchPins = [];
// Timeline (ADR-0005): null = now; otherwise the selected instant, with the
// fetched at-date pins overriding the live sets.
window.graphTimelineAt = null;
window.graphTimelinePins = null;
window.graphTimelineDates = [];
window.graphUserPermissions = [];
// Periphery (#46) of the main/branch view at the timeline position, fetched
// per scope; the dev view reads the report's own set instead.
window.graphScopePeriphery = null;
// The scope's producers with their version lines and its harvested
// subscriptions at the timeline position (GET /admin/graph/scope), fetched per
// view and instant like the periphery.
window.graphScopeData = null;

// A Peripheral Resource node is keyed by its full Backstage ref; it is shown
// as `name`, or `namespace/name` outside the default namespace.
function graphPeripheralLabel(ref) {
  const rest = ref.replace(/^resource:/, "");
  return rest.startsWith("default/") ? rest.slice("default/".length) : rest;
}
window.graphPeripheralLabel = graphPeripheralLabel;

// Fetch the periphery of the current main/branch view at the current
// timeline position. A past instant gets the periphery as it was then —
// today's set must never be drawn onto a reconstructed graph.
async function graphFetchScopePeriphery(view, branchName, at) {
  if (view === "dev") return null;
  let scope = view === "main" ? "main" : branchName;
  if (at) scope += `@${at}`;
  try {
    const res = await apiCall(`/admin/periphery?scope=${encodeURIComponent(scope)}`);
    return res.ok ? await res.json() : [];
  } catch (e) {
    console.warn("Could not load the periphery:", e);
    return [];
  }
}

// Fetch the scope data of the current view at the current timeline position:
// its producers with their version lines, its harvested subscriptions
// (dev = all, live main = trunk-flagged, anything else = none) and the
// context of every system that pushed a catalog-info (#48).
async function graphFetchScopeData(view, branchName, at) {
  let scope = view === "dev" ? "dev" : view === "main" ? "main" : branchName;
  if (at && view !== "dev") scope += `@${at}`;
  const empty = { producers: [], harvested_subscriptions: [], systems: {} };
  try {
    const res = await apiCall(`/admin/graph/scope?scope=${encodeURIComponent(scope)}`);
    return res.ok ? await res.json() : empty;
  } catch (e) {
    console.warn("Could not load the graph scope:", e);
    return empty;
  }
}
window.graphFetchScopeData = graphFetchScopeData;

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
// A major behind is a different kind of problem, so it gets its own colour.
const GRAPH_BREAKING_OUTDATED_COLOR = "#be123c";

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

// A Pin to a Snapshot whose version is below the latest GA of its line is
// "superseded": the GA it should move to, or null. Such a Pin is Outdated (or
// Major behind) like any Pin below the latest GA, and says so in the tooltip.
function graphSupersededBy(d, latestGaMap) {
  if (d.stability !== "snapshot") return null;
  const latestGa = latestGaMap.get(`${d.service}|${(d.api_type || "openapi").toLowerCase()}`);
  return latestGa && _graphCompareSemver(d.version, latestGa) < 0 ? latestGa : null;
}
window.graphSupersededBy = graphSupersededBy;

// Per aggregated edge ("client-->service"): does any dependency on it carry an
// Outdated or Snapshot-pinned Pin? Map key -> { outdated, snapshot }.
function graphEdgeFlags(report, latestGaMap, trunkVersionMap) {
  const flags = new Map();
  // Staleness and major-lag are statements about *now*: both are measured
  // against today's boundary and today's trunk versions. A reconstructed past
  // graph must not be judged by them, or every historical edge reads as
  // forgotten and conflicts appear against versions that did not yet exist.
  const historical = !!window.graphTimelineAt;
  const staleBefore = trunkVersionMap && !historical ? report.trunk_stale_before : null;
  const conflictMap = historical ? null : trunkVersionMap;
  (report.dependency_graph || []).forEach((d) => {
    const key = `${d.client}-->${d.service}`;
    if (!flags.has(key))
      flags.set(key, {
        outdated: false,
        breakingOutdated: false,
        snapshot: false,
        conflict: false,
        stale: false,
      });
    const f = flags.get(key);
    // Harvested subscriptions (#6) are version-less consumer edges: no version
    // to be outdated/behind, and no last_required_at to be stale. Skip the
    // version-based classification for them.
    if (d.harvested) return;
    // Peripheral edges (#46) carry no version either.
    if (d.peripheral) return;
    // Main view: a pin not refreshed since half the trunk TTL is a
    // forgotten thing — shown as such before the cleanup culls it.
    if (staleBefore && d.last_required_at && d.last_required_at < staleBefore) f.stale = true;
    // Branch view: the pinned version no longer exists (deleted) — dangling
    // until the number is re-provided.
    if (d.dangling) f.dangling = true;
    if (d.stability === "snapshot") f.snapshot = true;
    const typeKey = `${d.service}|${(d.api_type || "openapi").toLowerCase()}`;
    const latestGa = latestGaMap.get(typeKey);
    if (latestGa && _graphCompareSemver(d.version, latestGa) < 0) {
      // Two tiers: behind within the same major is an upgrade waiting to
      // happen; a whole major behind means the contract moved underneath.
      if (parseInt(d.version, 10) < parseInt(latestGa, 10)) f.breakingOutdated = true;
      else f.outdated = true;
    }
    // Main view: a pin a whole major behind the producer's trunk version is a
    // conflict, drawn loud (ADR-0004).
    if (conflictMap) {
      const trunkV = conflictMap.get(typeKey);
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
    ["highlight-breaking-outdated", "breakingOutdated", "bg-rose-800 text-white"],
    ["highlight-snapshot", "snapshot", "bg-amber-500 text-white"],
  ];
  for (const [id, filter, activeClasses] of styles) {
    const btn = document.getElementById(id);
    if (!btn) continue;
    // nowrap: three toggles with multi-word labels otherwise wrap mid-label.
    btn.className = window.graphHighlightFilters[filter]
      ? `px-2.5 py-1.5 whitespace-nowrap ${activeClasses} font-medium`
      : "px-2.5 py-1.5 whitespace-nowrap bg-white text-slate-600 hover:bg-slate-50 font-medium";
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

  const { outdated, breakingOutdated, snapshot } = window.graphHighlightFilters;
  if (!outdated && !breakingOutdated && !snapshot) {
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
    const isBreaking = breakingOutdated && els.path.dataset.breakingOutdated === "1";
    const isSnapshot = snapshot && els.path.dataset.snapshot === "1";
    if (isOutdated || isBreaking || isSnapshot) {
      els.path.style.opacity = "1";
      const lit = isBreaking
        ? GRAPH_BREAKING_OUTDATED_COLOR
        : isOutdated
          ? GRAPH_OUTDATED_COLOR
          : GRAPH_SNAPSHOT_COLOR;
      els.path.setAttribute("stroke", lit);
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

// ── Provides/requires badges ──────────────────────────────────────
// One fixed reading order in both renderers: REST, gRPC, messaging.
const GRAPH_BADGE_ORDER = ["rest", "grpc", "msg"];
const GRAPH_BADGE_LABELS = { rest: "REST", grpc: "gRPC", msg: "MSG" };

function graphBadgeKind(apiType) {
  const t = (apiType || "openapi").toLowerCase();
  if (t === "openapi") return "rest";
  if (t === "proto") return "grpc";
  if (t === "asyncapi") return "msg";
  return null;
}

// Per node: what it provides and how it requires others, derived live for
// the current view and instant — the sticky service_tags table plays no part.
// Provides come from the producer's version lines in the view's scope at the
// viewed instant (GET /admin/graph/scope): dev = every line, main = trunk's
// member versions, a branch = its member and pinned versions. Requires come
// from the node's own outgoing edges (harvested subscriptions are messaging);
// Peripheral Resources provide the hexagon.
// Returns Map node -> { provides: [...], requires: [...] }.
function graphNodeBadges(report, deps) {
  const provides = new Map();
  const requires = new Map();
  const add = (map, node, kind) => {
    if (!kind) return;
    if (!map.has(node)) map.set(node, new Set());
    map.get(node).add(kind);
  };
  ((window.graphScopeData && window.graphScopeData.producers) || []).forEach((p) => {
    (p.versions || []).forEach((v) => add(provides, p.name, graphBadgeKind(v.api_type)));
  });
  deps.forEach((d) => {
    if (d.peripheral) {
      add(provides, d.service, "resource");
      return;
    }
    add(requires, d.client, graphBadgeKind(d.api_type));
  });
  const ordered = (set) =>
    set ? [...GRAPH_BADGE_ORDER, "resource"].filter((k) => set.has(k)) : [];
  const nodes = new Set([...provides.keys(), ...requires.keys()]);
  const result = new Map();
  nodes.forEach((n) =>
    result.set(n, { provides: ordered(provides.get(n)), requires: ordered(requires.get(n)) }),
  );
  return result;
}
window.graphNodeBadges = graphNodeBadges;

// The mermaid renderer's second label line: "▲ REST · MSG   ▼ gRPC"
// (▲ provides, ▼ requires); a side without badges is left out.
function graphBadgeLine(badges) {
  if (!badges) return "";
  const side = (arrow, kinds) => {
    const labels = kinds.filter((k) => GRAPH_BADGE_LABELS[k]).map((k) => GRAPH_BADGE_LABELS[k]);
    return labels.length ? `${arrow} ${labels.join(" · ")}` : "";
  };
  return [side("▲", badges.provides), side("▼", badges.requires)].filter(Boolean).join("   ");
}
window.graphBadgeLine = graphBadgeLine;

// 24x24 line icons (stroke only) for the badges.
const GRAPH_BADGE_ICONS = {
  rest: [
    ["path", { d: "M9 4c-2 0-3 1-3 3v2c0 1.5-1 2.5-2.5 3C5 12.5 6 13.5 6 15v2c0 2 1 3 3 3" }],
    ["path", { d: "M15 4c2 0 3 1 3 3v2c0 1.5 1 2.5 2.5 3-1.5.5-2.5 1.5-2.5 3v2c0 2-1 3-3 3" }],
  ],
  msg: [
    ["rect", { x: "3", y: "6", width: "18", height: "13", rx: "2" }],
    ["path", { d: "M3.5 7l8.5 6.5L20.5 7" }],
  ],
  grpc: [
    ["path", { d: "M4 9h14M14 5l4 4-4 4" }],
    ["path", { d: "M20 15H6M10 11l-4 4 4 4" }],
  ],
  resource: [["path", { d: "M12 3l7.8 4.5v9L12 21l-7.8-4.5v-9z" }]],
};

// One badge: a provides badge is solid (dark circle, white ring and icon), a
// requires badge outlined (white circle, dark ring and icon).
function graphDrawBadge(parent, side, kind, cx, cy) {
  const ns = "http://www.w3.org/2000/svg";
  const solid = side === "provides";
  const dark = "#1e293b";
  const g = document.createElementNS(ns, "g");
  g.dataset.badge = `${side}-${kind}`;
  const circle = document.createElementNS(ns, "circle");
  circle.setAttribute("cx", cx);
  circle.setAttribute("cy", cy);
  circle.setAttribute("r", solid ? "11" : "10");
  circle.setAttribute("fill", solid ? dark : "#fff");
  circle.setAttribute("stroke", solid ? "#fff" : dark);
  circle.setAttribute("stroke-width", "2");
  g.appendChild(circle);
  const size = solid ? 13 : 12;
  const icon = document.createElementNS(ns, "g");
  icon.setAttribute(
    "transform",
    `translate(${cx - size / 2},${cy - size / 2}) scale(${size / 24})`,
  );
  icon.setAttribute("fill", "none");
  icon.setAttribute("stroke", solid ? "#fff" : dark);
  icon.setAttribute("stroke-width", "2.2");
  icon.setAttribute("stroke-linecap", "round");
  icon.setAttribute("stroke-linejoin", "round");
  (GRAPH_BADGE_ICONS[kind] || []).forEach(([tag, attrs]) => {
    const el = document.createElementNS(ns, tag);
    Object.entries(attrs).forEach(([k, v]) => el.setAttribute(k, v));
    icon.appendChild(el);
  });
  g.appendChild(icon);
  parent.appendChild(g);
}

// The node an unowned harvested subscription points at: the channel name with
// a question mark, drawn in the missing-node style by both renderers.
function graphUnownedChannelNode(channel) {
  return `${channel} ?`;
}
window.graphUnownedChannelNode = graphUnownedChannelNode;

// The "(unrequired)" box members: producers of the scope with no service edge
// (nobody requires them, they require nothing, no harvested subscription) and
// no resource edge. `deps` must be the scope's edges before any filter, or
// hiding a protocol would push its producers into the box.
function graphUnrequiredMembers(producers, deps) {
  const connected = new Set();
  deps.forEach((d) => {
    connected.add(d.client);
    if (!d.peripheral) connected.add(d.service);
  });
  return (producers || []).filter((p) => !connected.has(p.name));
}
window.graphUnrequiredMembers = graphUnrequiredMembers;

// A chip's version-line line: "openapi 1.4.0 ga · asyncapi 0.3.0 snapshot".
function graphVersionLinesText(versions) {
  return (versions || [])
    .map((v) => `${v.api_type} ${v.version} ${v.stability || "deleted"}`)
    .join(" · ");
}
window.graphVersionLinesText = graphVersionLinesText;

// When a producer was last provided: the latest of its lines' provides.
function graphLastProvided(versions) {
  const stamps = (versions || []).map((v) => v.last_provided_at).filter(Boolean);
  return stamps.length ? stamps.sort().pop() : null;
}
window.graphLastProvided = graphLastProvided;

// The node panel's provides/requires rows, from the same badges the renderer
// draws (never from the sticky service_tags table).
function graphBadgeSummaryHtml(badges) {
  const label = (k) => (k === "resource" ? "Resource" : GRAPH_BADGE_LABELS[k]);
  const row = (title, kinds, solid) =>
    kinds.length
      ? `<div class="flex flex-wrap items-center gap-1.5 mt-1" data-panel="${title.toLowerCase()}"><span class="text-[10px] text-slate-400 w-14">${title}</span>${kinds
          .map(
            (k) =>
              `<span class="px-1.5 py-0.5 rounded-sm text-[10px] ${solid ? "bg-slate-200 text-slate-900" : "bg-slate-800 text-slate-200 border border-slate-500"}">${_graphEscapeHtml(label(k))}</span>`,
          )
          .join("")}</div>`
      : "";
  const b = badges || { provides: [], requires: [] };
  return row("Provides", b.provides, true) + row("Requires", b.requires, false);
}
window.graphBadgeSummaryHtml = graphBadgeSummaryHtml;

function getFilteredReport(report) {
  if (!report) return null;
  let deps = [...(report.dependency_graph || [])];
  let missing = report.missing_endpoints;

  // Pin-set views show the version line's stored stability, looked up from
  // the report's detailed services (a dangling pin's line is gone → "ga").
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
  const pinAsDep = (t) => ({
    api_type: t.api_type,
    client: t.client,
    service: t.service,
    version: t.version,
    stability: stabilityOf(t),
    path: t.path,
    method: t.method,
    deprecated: false,
    last_required_at: t.last_required_at,
    dangling: !!t.dangling,
  });

  // Branch view (ADR-0005): the edges are the branch's pin set, dangling
  // references included; overlays are dev-view data. The circular/focus/
  // protocol filters below apply to pin sets like to any other view.
  if (window.graphStreamView === "branch") {
    deps = (window.graphTimelinePins || window.graphBranchPins || []).map(pinAsDep);
    missing = [];
  }

  // Main view (ADR-0004): the edges are the current trunk pin set, not the
  // accumulated dev activity; the missing-endpoints overlay is dev-view data.
  if (window.graphStreamView === "main") {
    deps = (window.graphTimelinePins || report.trunk_graph || []).map(pinAsDep);
    missing = [];
  }

  // Harvested AsyncAPI subscriptions (#6/ADR-0006): version-less consumer edges.
  // The scope data carries them scoped by the server to match the view (dev =
  // all, live main = trunk-only, anything else = none), so append them to
  // whichever deps we built. A subscription with no resolved owner (nobody provides the channel
  // yet) points at a missing node named after the channel, so the pending
  // consumption stays visible and every subscriber of that channel meets there.
  ((window.graphScopeData && window.graphScopeData.harvested_subscriptions) || []).forEach((h) => {
    deps.push({
      api_type: "asyncapi",
      client: h.client,
      service: h.owner || graphUnownedChannelNode(h.channel),
      version: null,
      stability: null,
      path: h.channel,
      method: "SUB",
      deprecated: false,
      harvested: true,
      unfulfilled: !h.owner,
    });
  });

  // Peripheral Resources (#46): edges from a system to what its Backstage
  // catalog-info lists under dependsOn — their own node kind, no version.
  const periphery =
    window.graphStreamView === "dev"
      ? report.peripheral_dependencies || []
      : window.graphScopePeriphery || [];
  periphery.forEach((p) => {
    deps.push({
      api_type: "periphery",
      client: p.system,
      service: p.resource,
      version: null,
      stability: null,
      path: p.resource,
      method: "DEPENDS",
      deprecated: false,
      peripheral: true,
    });
  });

  // The "(unrequired)" box, decided on the scope's unfiltered edges; the
  // filters below then narrow its members on their own terms.
  let unrequired = graphUnrequiredMembers(
    window.graphScopeData && window.graphScopeData.producers,
    deps,
  );

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
    // Nothing in the box is part of a cycle.
    unrequired = [];
  }

  // 2. Focus filter (multi-service tag cloud)
  if (window.graphFocusTags && window.graphFocusTags.length > 0) {
    const focusNodes = new Set();
    const tags = window.graphFocusTags.map((t) => t.toLowerCase());
    // The box keeps only the members a focus tag names; a named member also
    // narrows the edges like any other focused node.
    unrequired = unrequired.filter((p) => tags.includes(p.name.toLowerCase()));
    unrequired.forEach((p) => focusNodes.add(p.name));
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
    if (type === "periphery" && !window.graphProtocolFilters.periphery) return false;
    return true;
  });
  // A member stays while any of its API types is shown.
  unrequired = unrequired.filter((p) =>
    (p.versions || []).some((v) => window.graphProtocolFilters[v.api_type] !== false),
  );
  if (!window.graphProtocolFilters.unrequired) unrequired = [];

  return { ...report, dependency_graph: deps, missing_endpoints: missing, unrequired };
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
  if (view !== "branch") {
    window.graphBranchName = null;
    const sel = document.getElementById("graph-branch-select");
    if (sel) sel.value = "";
  }
  ["dev", "main"].forEach((v) => {
    const btn = document.getElementById(`stream-${v}`);
    if (btn) {
      btn.className =
        v === view
          ? "px-3 py-1.5 bg-indigo-600 text-white font-medium"
          : "px-3 py-1.5 text-slate-600 hover:bg-slate-50 font-medium";
    }
  });
  // Space-separated: a marker can belong to more than one view — a dangling
  // pin is drawn in both the main and the branch graph.
  document.querySelectorAll("[data-stream-only]").forEach((el) => {
    const views = el.dataset.streamOnly.split(/\s+/);
    el.classList.toggle("hidden", !views.includes(view));
  });
  window.graphTimelineAt = null;
  window.graphTimelinePins = null;
  window.graphScopePeriphery = null;
  window.graphScopeData = null;
  // A view switch outdates every timeline/periphery request still in flight.
  const token = (window._graphTimelineToken || 0) + 1;
  window._graphTimelineToken = token;
  initTimelineSlider();
  refreshTimeline();
  // One redraw once the scope's periphery and producers are both in: drawing
  // the pins first would show the box and badges of no scope at all.
  Promise.all([
    graphFetchScopePeriphery(view, window.graphBranchName, null),
    graphFetchScopeData(view, window.graphBranchName, null),
  ]).then(([periphery, scopeData]) => {
    if (token !== window._graphTimelineToken) return;
    window.graphScopePeriphery = periphery;
    window.graphScopeData = scopeData;
    redrawGraph();
  });
}
window.setStreamView = setStreamView;

// ── Timeline (ADR-0005) ──────────────────────────────────────────────
// The slider indexes the graph's change dates; the last position is "now".

async function refreshTimeline() {
  const row = document.getElementById("graph-timeline-row");
  if (!row) return;
  const view = window.graphStreamView;
  if (view === "dev") {
    row.classList.add("hidden");
    window.graphTimelineAt = null;
    window.graphTimelinePins = null;
    return;
  }
  const url =
    view === "main"
      ? "/admin/trunk/timeline"
      : `/admin/branches/${encodeURIComponent(window.graphBranchName)}/timeline`;
  try {
    const res = await apiCall(url);
    window.graphTimelineDates = res.ok ? await res.json() : [];
  } catch (e) {
    console.warn("Could not load the timeline:", e);
    window.graphTimelineDates = [];
  }
  const slider = document.getElementById("graph-timeline-slider");
  slider.max = String(window.graphTimelineDates.length);
  slider.value = slider.max;
  document.getElementById("graph-timeline-label").textContent = "now";
  const createBtn = document.getElementById("graph-create-branch-here");
  createBtn.classList.toggle("hidden", !window.graphUserPermissions.includes("release_ga"));
  row.classList.remove("hidden");
}

async function setTimelinePosition(at) {
  const slider = document.getElementById("graph-timeline-slider");
  const label = document.getElementById("graph-timeline-label");
  if (at === null) {
    const token = (window._graphTimelineToken || 0) + 1;
    window._graphTimelineToken = token;
    slider.value = slider.max;
    label.textContent = "now";
    const [periphery, scopeData] = await Promise.all([
      graphFetchScopePeriphery(window.graphStreamView, window.graphBranchName, null),
      graphFetchScopeData(window.graphStreamView, window.graphBranchName, null),
    ]);
    if (token !== window._graphTimelineToken) return;
    window.graphTimelineAt = null;
    window.graphTimelinePins = null;
    window.graphScopePeriphery = periphery;
    window.graphScopeData = scopeData;
    redrawGraph();
    return;
  }
  const view = window.graphStreamView;
  const url =
    view === "main"
      ? `/admin/trunk/graph?at=${encodeURIComponent(at)}`
      : `/admin/branches/${encodeURIComponent(window.graphBranchName)}/graph?at=${encodeURIComponent(at)}`;
  // Dragging the slider fires one request per instant it crosses, and they can
  // land out of order — on a slow link an earlier response arriving last would
  // leave the graph on an instant the user already scrubbed past. Only the
  // newest request may apply its result.
  const token = (window._graphTimelineToken || 0) + 1;
  window._graphTimelineToken = token;
  // The label tracks the handle immediately; the graph follows when it loads.
  label.textContent = at.slice(0, 16).replace("T", " ");
  try {
    const res = await apiCall(url);
    if (!res.ok || token !== window._graphTimelineToken) return;
    const pins = await res.json();
    if (token !== window._graphTimelineToken) return;
    const [periphery, scopeData] = await Promise.all([
      graphFetchScopePeriphery(view, window.graphBranchName, at),
      graphFetchScopeData(view, window.graphBranchName, at),
    ]);
    if (token !== window._graphTimelineToken) return;
    window.graphTimelinePins = pins;
    window.graphTimelineAt = at;
    window.graphScopePeriphery = periphery;
    window.graphScopeData = scopeData;
    redrawGraph();
  } catch (e) {
    console.warn("Could not load the graph at that instant:", e);
  }
}
window.setTimelinePosition = setTimelinePosition;

function initTimelineSlider() {
  const slider = document.getElementById("graph-timeline-slider");
  if (!slider || slider.dataset.wired) return;
  slider.dataset.wired = "1";
  slider.addEventListener("input", () => {
    const idx = parseInt(slider.value, 10);
    if (idx >= window.graphTimelineDates.length) {
      setTimelinePosition(null);
    } else {
      setTimelinePosition(window.graphTimelineDates[idx]);
    }
  });
}

// "Create branch here": the retroactive cut, from the selected instant.
async function createBranchHere() {
  const at = window.graphTimelineAt;
  const name = prompt(
    at
      ? `Create a sanshain-branch off the graph as of ${at}:`
      : "Create a sanshain-branch off the current graph:",
  );
  if (!name) return;
  const body = { name };
  if (at) body.as_of = at;
  if (window.graphStreamView === "branch") body.source = window.graphBranchName;
  const res = await apiCall("/admin/branches", { method: "POST", body });
  if (!res.ok) {
    alert("Could not create the branch: " + (await errorMessage(res)));
    return;
  }
  alert(`Sanshain-branch '${name}' created.`);
}
window.createBranchHere = createBranchHere;

// Enter the branch view (ADR-0005): fetch the branch's pin set — dangling
// references included — and draw it with the shared pipeline.
async function selectBranchView(name) {
  if (!name) {
    setStreamView("dev");
    return;
  }
  try {
    const res = await apiCall(`/admin/branches/${encodeURIComponent(name)}/graph`);
    if (!res.ok) {
      failBranchSelection(name, `HTTP ${res.status}`);
      return;
    }
    window.graphBranchPins = await res.json();
    window.graphBranchName = name;
    setStreamView("branch");
  } catch (e) {
    failBranchSelection(name, e.message || String(e));
  }
}

/// The select must not keep showing a branch the view never switched to — put
/// it back where the graph actually is, and say why out loud rather than only
/// in the console.
function failBranchSelection(name, reason) {
  const sel = document.getElementById("graph-branch-select");
  if (sel) sel.value = window.graphBranchName || "";
  console.warn(`Could not load branch '${name}': ${reason}`);
  // Visible, like a failed branch creation in this file: the user chose this
  // branch, so a silent console line is not an answer.
  alert(`Could not load sanshain-branch '${name}': ${reason}`);
}
window.selectBranchView = selectBranchView;

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
    pill.innerHTML = `${_graphEscapeHtml(tag)}<button data-click="removeFocusTag" data-click-args="${attrJson([tag])}"
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

// Fit-to-view is bound to a closure created only once the graph has rendered
// (it captures the current pan/scale). Expose a stable top-level entry point so
// the toolbar button's data-click resolves at page load; it forwards to the
// live implementation once render has set it, and is a no-op before then (there
// is nothing to fit yet).
let _graphFitToView = null;
function graphFitToView() {
  if (_graphFitToView) _graphFitToView();
}
window.graphFitToView = graphFitToView;

// Enter in the focus input applies it — replaces the inline keydown handler.
function applyGraphFocusOnEnter(event) {
  if (event.key === "Enter") {
    event.preventDefault();
    applyGraphFocus();
  }
}
window.applyGraphFocusOnEnter = applyGraphFocusOnEnter;

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

// ── Islands and the "(unrequired)" box ──────────────────────────────

// Cubic Bezier control points of an edge from p0 to p1, rank-axis aware.
//  - Normal cross-rank edges: control points pulled along the rank axis (Y in
//    TB, X in LR) so curves leave/enter nodes perpendicular to their border —
//    a smooth "river" flow.
//  - Same-rank / reverse edges (tiny rank delta): S-curve that bows sideways
//    along the flow axis so arrows don't cut through nodes.
function graphEdgeControls(p0, p1, F, R) {
  const dF = p1[F] - p0[F];
  const dR = p1[R] - p0[R];
  const absDR = Math.abs(dR);
  const c1 = { x: p0.x, y: p0.y };
  const c2 = { x: p1.x, y: p1.y };
  if (absDR < 30) {
    const bow = Math.max(40, Math.abs(dF) * 0.3);
    const sign = dF >= 0 ? 1 : -1;
    c1[F] = p0[F] + sign * bow;
    c2[F] = p1[F] - sign * bow;
    c1[R] = p0[R] + bow * 0.6;
    c2[R] = p1[R] + bow * 0.6;
  } else {
    const tension = Math.min(Math.max(absDR * 0.5, 40), 140);
    const dir = dR >= 0 ? 1 : -1;
    c1[R] = p0[R] + dir * tension;
    c2[R] = p1[R] - dir * tension;
  }
  return { c1, c2 };
}

// Disconnected islands: connected groups of nodes whose every edge is a
// resource edge — a system nobody requires and that requires nothing, with
// its Peripheral Resources. Returns one array of node names per island.
function graphIslands(nodes, deps) {
  const parent = new Map();
  const find = (n) => {
    while (parent.get(n) !== n) {
      parent.set(n, parent.get(parent.get(n)));
      n = parent.get(n);
    }
    return n;
  };
  nodes.forEach((n) => parent.set(n, n));
  deps.forEach((d) => {
    if (!parent.has(d.client) || !parent.has(d.service)) return;
    parent.set(find(d.client), find(d.service));
  });
  const groups = new Map();
  nodes.forEach((n) => {
    const root = find(n);
    if (!groups.has(root)) groups.set(root, { nodes: [], allPeripheral: true });
    groups.get(root).nodes.push(n);
  });
  deps.forEach((d) => {
    if (!d.peripheral && parent.has(d.client)) groups.get(find(d.client)).allPeripheral = false;
  });
  return [...groups.values()]
    .filter((grp) => grp.allPeripheral && grp.nodes.length > 1)
    .map((grp) => grp.nodes.sort());
}
window.graphIslands = graphIslands;

// Which side of the main graph (`box` = {minX, minY, maxX, maxY}) takes what
// does not fit inside it — overflowing islands and the "(unrequired)" box:
// "right" when the graph is taller than wide, "below" otherwise, so the
// drawing squares up instead of growing along its long side. Without a main
// graph the flow direction decides.
function graphOverflowSide(box, isLR) {
  if (!box) return isLR ? "right" : "below";
  return box.maxY - box.minY > box.maxX - box.minX ? "right" : "below";
}
window.graphOverflowSide = graphOverflowSide;

// Pack islands ({w, h}) into the empty areas of the main graph's bounding box
// (`box` = {minX, minY, maxX, maxY}, or null when there is no main graph),
// avoiding the `obstacles` rects ({x0, y0, x1, y1}). Islands that do not fit
// go to `side` ("right" or "below", see graphOverflowSide) wrapped inside the
// box's height (right) / width (below), so they never lengthen the long side.
// Returns the top-left {x, y} of every island, in input order.
function graphPackIslands(box, obstacles, islands, side) {
  const CELL = 10;
  const MARGIN = 30;
  const toRight = side === "right";
  const widest = Math.max(0, ...islands.map((i) => i.w));
  const tallest = Math.max(0, ...islands.map((i) => i.h));
  const frame = box || {
    minX: 0,
    minY: 0,
    maxX: toRight ? 0 : Math.max(widest, 900),
    maxY: toRight ? Math.max(tallest, 600) : 0,
  };
  const cols = Math.max(0, Math.floor((frame.maxX - frame.minX) / CELL));
  const rows = Math.max(0, Math.floor((frame.maxY - frame.minY) / CELL));
  const used = new Uint8Array(cols * rows);
  const mark = (x0, y0, x1, y1) => {
    const c0 = Math.max(0, Math.floor((x0 - frame.minX) / CELL));
    const c1 = Math.min(cols - 1, Math.floor((x1 - frame.minX) / CELL));
    const r0 = Math.max(0, Math.floor((y0 - frame.minY) / CELL));
    const r1 = Math.min(rows - 1, Math.floor((y1 - frame.minY) / CELL));
    for (let r = r0; r <= r1; r++) for (let c = c0; c <= c1; c++) used[r * cols + c] = 1;
  };
  obstacles.forEach((o) => mark(o.x0 - MARGIN, o.y0 - MARGIN, o.x1 + MARGIN, o.y1 + MARGIN));
  const fits = (r0, c0, rh, cw) => {
    for (let r = r0; r < r0 + rh; r++)
      for (let c = c0; c < c0 + cw; c++) if (used[r * cols + c]) return false;
    return true;
  };
  const positions = new Array(islands.length).fill(null);
  const order = islands
    .map((_, i) => i)
    .sort((a, b) => islands[b].w * islands[b].h - islands[a].w * islands[a].h);
  const overflow = [];
  if (box) {
    for (const i of order) {
      const cw = Math.ceil(islands[i].w / CELL);
      const rh = Math.ceil(islands[i].h / CELL);
      let spot = null;
      for (let r = 0; r + rh <= rows && !spot; r++)
        for (let c = 0; c + cw <= cols && !spot; c++) if (fits(r, c, rh, cw)) spot = { r, c };
      if (!spot) {
        overflow.push(i);
        continue;
      }
      const x = frame.minX + spot.c * CELL;
      const y = frame.minY + spot.r * CELL;
      positions[i] = { x, y };
      mark(x - MARGIN, y - MARGIN, x + islands[i].w + MARGIN, y + islands[i].h + MARGIN);
    }
  } else {
    overflow.push(...islands.map((_, i) => i));
  }
  // The chosen side, wrapped inside the frame's extent along that side.
  overflow.sort((a, b) => a - b);
  const GAP = 60;
  let along = toRight ? (box ? frame.maxX + 80 : 0) : box ? frame.maxY + 80 : 0;
  let across = toRight ? frame.minY : frame.minX;
  let lane = 0;
  const limit = toRight ? frame.maxY : frame.maxX;
  overflow.forEach((i) => {
    const size = toRight ? islands[i].h : islands[i].w;
    const depth = toRight ? islands[i].w : islands[i].h;
    const start = toRight ? frame.minY : frame.minX;
    if (across > start && across + size > limit) {
      along += lane + GAP;
      across = start;
      lane = 0;
    }
    positions[i] = toRight ? { x: along, y: across } : { x: across, y: along };
    across += size + GAP;
    lane = Math.max(lane, depth);
  });
  return positions;
}
window.graphPackIslands = graphPackIslands;

// The "(unrequired)" box on `side` of the graph ("right" or "below", see
// graphOverflowSide) with one compact chip per member in a dense grid: a
// column as tall as the graph on the right, a strip as wide as the graph below.
// `extent` = {minX, minY, maxX, maxY} of everything drawn before it.
function graphLayoutUnrequiredBox(names, extent, side) {
  const toRight = side === "right";
  const CHIP_H = 26;
  const GAP = 8;
  const PAD = 12;
  const HEADER = 26;
  const longest = Math.max(0, ...names.map((n) => n.length));
  const cellW = Math.min(220, Math.max(90, longest * 7 + 20));
  const e = extent || { minX: 0, minY: 0, maxX: 0, maxY: 0 };
  let cols;
  let x;
  let y;
  if (toRight) {
    const height = Math.max(e.maxY - e.minY, HEADER + CHIP_H + 2 * PAD);
    const rows = Math.max(
      1,
      Math.min(names.length, Math.floor((height - HEADER - 2 * PAD + GAP) / (CHIP_H + GAP))),
    );
    cols = Math.ceil(names.length / rows);
    x = e.maxX + 80;
    y = e.minY;
  } else {
    const width = Math.max(e.maxX - e.minX, cellW + 2 * PAD);
    cols = Math.max(1, Math.min(names.length, Math.floor((width - 2 * PAD + GAP) / (cellW + GAP))));
    x = e.minX;
    y = e.maxY + 80;
  }
  const rowsUsed = Math.ceil(names.length / cols);
  const chips = names.map((name, i) => ({
    name,
    x: x + PAD + (i % cols) * (cellW + GAP),
    y: y + HEADER + PAD + Math.floor(i / cols) * (CHIP_H + GAP),
    w: cellW,
    h: CHIP_H,
  }));
  const innerW = cols * (cellW + GAP) - GAP + 2 * PAD;
  return {
    x,
    y,
    w: toRight ? innerW : Math.max(innerW, e.maxX - e.minX),
    h: HEADER + 2 * PAD + rowsUsed * (CHIP_H + GAP) - GAP,
    chips,
  };
}
window.graphLayoutUnrequiredBox = graphLayoutUnrequiredBox;

// ── Main renderer ────────────────────────────────────────────────────

function renderCustomGraph(report, svgElement, direction) {
  // Clear previous content
  while (svgElement.firstChild) svgElement.removeChild(svgElement.firstChild);

  const deps = report.dependency_graph || [];
  const unrequired = report.unrequired || [];
  if (deps.length === 0 && unrequired.length === 0) {
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
  // Peripheral Resources (#46) and the edges leading to them.
  const peripheralNodes = new Set(deps.filter((d) => d.peripheral).map((d) => d.service));
  const peripheralEdges = new Set(
    deps.filter((d) => d.peripheral).map((d) => `${d.client}-->${d.service}`),
  );

  // Outdated / Snapshot-pinned flags per aggregated edge — and in the main
  // view, major-lag conflicts against the producer's trunk version.
  const isMainView = window.graphStreamView === "main";
  // Today's trunk versions describe today. On a reconstructed past graph they
  // would inject producers that were not yet trunk-marked and label the rest
  // with versions they did not carry then — so the whole map stays out, which
  // also disables the staleness and major-lag judgments downstream.
  const trunkVersionMap =
    isMainView && !window.graphTimelineAt ? graphTrunkVersionMap(report) : null;
  const latestGaMap = graphLatestGaMap(report);
  const edgeHighlightFlags = graphEdgeFlags(report, latestGaMap, trunkVersionMap);

  // Main view: a trunk-provided producer nobody pins sits in the
  // "(unrequired)" box with the rest of the scope's unrequired producers.

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

  // Provides/requires badges per node, derived live for this view.
  const nodeBadges = graphNodeBadges(report, deps);

  // An unowned harvested subscription points at a missing `<channel> ?` node:
  // its edge is drawn as a missing dependency in every view, not only in dev.
  const unownedChannelNodes = new Set();
  deps.forEach((d) => {
    if (!d.unfulfilled) return;
    unownedChannelNodes.add(d.service);
    missingEdgeKeys.add(`${d.client}-->${d.service}`);
  });

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

  // Islands (a system with only its resource edges) are laid out on their own
  // and packed into the main graph's empty areas afterwards.
  const islands = graphIslands([...allNodes], deps);
  const islandNodes = new Set(islands.flat());
  const layoutNodes = [...allNodes].filter((n) => !islandNodes.has(n));

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
  for (const node of layoutNodes) {
    const meta = serviceMetaMap.get(node);
    if (meta && meta.domain) domains.add(meta.domain);
  }

  // Set domain clusters
  domains.forEach((domain) => {
    g.setNode(`domain:${domain}`, { label: domain, clusterLabelPos: "top" });
  });

  for (const node of allNodes) {
    const meta = serviceMetaMap.get(node);
    const label = peripheralNodes.has(node) ? graphPeripheralLabel(node) : node;
    const m = _measureNode(label, !!(meta && meta.icon));
    // Main view: the version label sits bottom-right, beside the requires
    // badges on the bottom-left; keep both on the node.
    const requiresCount = (nodeBadges.get(node) || { requires: [] }).requires.length;
    if (isMainView && requiresCount > 0) m.width = Math.max(m.width, requiresCount * 24 + 70);
    nodeMetrics.set(node, m);
    if (islandNodes.has(node)) continue;
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
    if (islandNodes.has(from)) continue;
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
  for (const node of layoutNodes) {
    nodeFOffsets.set(node, 0);
  }

  const GAP = isLR ? 25 : 30; // gap between units on a rank (flow axis)
  const GROUP_GAP = isLR ? 100 : 120; // larger gap between different roles (flow axis)

  // Step 1 — group nodes by rank and calculate global lane metrics.
  const unitsByRank = new Map(); // rankR -> [{id, cf, width, role}]
  for (const node of layoutNodes) {
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

  for (const node of layoutNodes) {
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
  const anchorEdge = (e) => {
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
  };
  g.edges().forEach(anchorEdge);

  // ── Islands: packed into the main graph's empty areas ─────────────
  // The main graph's extent and what occupies it — nodes, domain clusters and
  // the drawn edge curves — so an island never lands on any of them.
  const extentOf = (ids) => {
    let box = null;
    ids.forEach((v) => {
      const n = g.node(v);
      if (!n || n.width === undefined) return;
      const b = {
        minX: n.x - n.width / 2,
        minY: n.y - n.height / 2,
        maxX: n.x + n.width / 2,
        maxY: n.y + n.height / 2,
      };
      box = box
        ? {
            minX: Math.min(box.minX, b.minX),
            minY: Math.min(box.minY, b.minY),
            maxX: Math.max(box.maxX, b.maxX),
            maxY: Math.max(box.maxY, b.maxY),
          }
        : b;
    });
    return box;
  };
  // Everything outside the main graph goes to the side with more room.
  const overflowSide = graphOverflowSide(extentOf(g.nodes()), isLR);
  if (islands.length > 0) {
    const mainBox = extentOf(g.nodes());
    const obstacles = [];
    g.nodes().forEach((v) => {
      const n = g.node(v);
      if (!n || n.width === undefined) return;
      obstacles.push({
        x0: n.x - n.width / 2,
        y0: n.y - n.height / 2,
        x1: n.x + n.width / 2,
        y1: n.y + n.height / 2,
      });
    });
    g.edges().forEach((e) => {
      const [p0, p1] = g.edge(e).points;
      const { c1, c2 } = graphEdgeControls(p0, p1, F, R);
      for (let t = 0; t <= 1; t += 0.05) {
        const u = 1 - t;
        const x = u * u * u * p0.x + 3 * u * u * t * c1.x + 3 * u * t * t * c2.x + t * t * t * p1.x;
        const y = u * u * u * p0.y + 3 * u * u * t * c1.y + 3 * u * t * t * c2.y + t * t * t * p1.y;
        obstacles.push({ x0: x - 4, y0: y - 4, x1: x + 4, y1: y + 4 });
      }
    });
    // Each island's own layout: its systems on one rank, their resources on
    // the next, centred on each other.
    const ISLAND_GAP = 30;
    const ISLAND_RANK = isLR ? 90 : 70;
    const shapes = islands.map((nodes) => {
      const ranks = [
        nodes.filter((n) => !peripheralNodes.has(n)),
        nodes.filter((n) => peripheralNodes.has(n)),
      ].map((members) => {
        const sizes = members.map((n) => {
          const m = nodeMetrics.get(n);
          return { n, f: isLR ? m.height : m.width, r: isLR ? m.width : m.height };
        });
        const span =
          sizes.reduce((a, b) => a + b.f, 0) + Math.max(0, sizes.length - 1) * ISLAND_GAP;
        const depth = Math.max(0, ...sizes.map((x) => x.r));
        return { sizes, span, depth };
      });
      const span = Math.max(ranks[0].span, ranks[1].span);
      const depth = ranks[0].depth + ISLAND_RANK + ranks[1].depth;
      return { ranks, span, depth, w: isLR ? depth : span, h: isLR ? span : depth };
    });
    const spots = graphPackIslands(mainBox, obstacles, shapes, overflowSide);
    shapes.forEach((shape, i) => {
      const spot = spots[i];
      let rankStart = 0;
      shape.ranks.forEach((rank) => {
        let cursor = (shape.span - rank.span) / 2;
        rank.sizes.forEach(({ n, f }) => {
          const m = nodeMetrics.get(n);
          const along = rankStart + rank.depth / 2; // rank axis, centred on the rank
          const across = cursor + f / 2;
          g.setNode(n, {
            label: n,
            width: m.width,
            height: m.height,
            x: spot.x + (isLR ? along : across),
            y: spot.y + (isLR ? across : along),
          });
          cursor += f + ISLAND_GAP;
        });
        rankStart += rank.depth + ISLAND_RANK;
      });
    });
    islands.forEach((nodes) => {
      nodes.forEach((from) => {
        (adjMap.get(from) || []).forEach((to) => {
          g.setEdge(from, to);
          anchorEdge({ v: from, w: to });
        });
      });
    });
  }

  // ── The "(unrequired)" box, beside or below the graph ────────────
  const unrequiredBox =
    unrequired.length > 0
      ? graphLayoutUnrequiredBox(
          unrequired.map((p) => p.name).sort(),
          extentOf(g.nodes()),
          overflowSide,
        )
      : null;

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

  if (unrequiredBox) {
    minX = Math.min(minX, unrequiredBox.x);
    minY = Math.min(minY, unrequiredBox.y);
    maxX = Math.max(maxX, unrequiredBox.x + unrequiredBox.w);
    maxY = Math.max(maxY, unrequiredBox.y + unrequiredBox.h);
  }

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

  // ── Render the "(unrequired)" box ────────────────────────────────
  const unrequiredByName = new Map(unrequired.map((p) => [p.name, p]));
  if (unrequiredBox) {
    const ns = "http://www.w3.org/2000/svg";
    const boxG = document.createElementNS(ns, "g");
    boxG.setAttribute("id", "graph-unrequired-box");
    boxG.dataset.unrequiredBox = "1";
    const frame = document.createElementNS(ns, "rect");
    frame.setAttribute("x", unrequiredBox.x);
    frame.setAttribute("y", unrequiredBox.y);
    frame.setAttribute("width", unrequiredBox.w);
    frame.setAttribute("height", unrequiredBox.h);
    frame.setAttribute("rx", "12");
    frame.setAttribute("fill", "#f8fafc");
    frame.setAttribute("stroke", "#94a3b8");
    frame.setAttribute("stroke-width", "1.5");
    frame.setAttribute("stroke-dasharray", "6 4");
    boxG.appendChild(frame);
    const title = document.createElementNS(ns, "text");
    title.setAttribute("x", unrequiredBox.x + 12);
    title.setAttribute("y", unrequiredBox.y + 20);
    title.setAttribute("fill", "#64748b");
    title.setAttribute("font-size", "12");
    title.setAttribute("font-weight", "bold");
    title.textContent = "(unrequired)";
    boxG.appendChild(title);
    unrequiredBox.chips.forEach((chip) => {
      const chipG = document.createElementNS(ns, "g");
      chipG.setAttribute("class", "graph-unrequired-chip");
      chipG.dataset.chip = chip.name;
      chipG.style.cursor = "pointer";
      const r = document.createElementNS(ns, "rect");
      r.setAttribute("x", chip.x);
      r.setAttribute("y", chip.y);
      r.setAttribute("width", chip.w);
      r.setAttribute("height", chip.h);
      r.setAttribute("rx", "6");
      r.setAttribute("fill", "#e0e7ff");
      r.setAttribute("stroke", "#6366f1");
      r.setAttribute("stroke-width", "1");
      chipG.appendChild(r);
      const maxChars = Math.floor((chip.w - 16) / 7);
      const t = document.createElementNS(ns, "text");
      t.setAttribute("x", chip.x + chip.w / 2);
      t.setAttribute("y", chip.y + chip.h / 2);
      t.setAttribute("text-anchor", "middle");
      t.setAttribute("dominant-baseline", "central");
      t.setAttribute("fill", "#3730a3");
      t.setAttribute("font-size", "12");
      t.setAttribute("font-weight", "600");
      t.textContent =
        chip.name.length > maxChars ? `${chip.name.slice(0, maxChars - 1)}…` : chip.name;
      chipG.appendChild(t);
      const tip = document.createElementNS(ns, "title");
      tip.textContent = chip.name;
      chipG.appendChild(tip);
      boxG.appendChild(chipG);
    });
    mainG.appendChild(boxG);
  }

  // ── Render edges ─────────────────────────────────────────────────
  const edgeElements = new Map(); // "from-->to" -> { path, hitArea }

  g.edges().forEach((e) => {
    const edgeData = g.edge(e);
    const key = `${e.v}-->${e.w}`;
    const isCycle = cycleEdges.has(key);
    const points = edgeData.points;
    const p0 = points[0];
    const p1 = points[points.length - 1];

    // Elegant cubic Bezier routing, rank-axis aware (see graphEdgeControls).
    const { c1, c2 } = graphEdgeControls(p0, p1, F, R);
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
    const hFlags = edgeHighlightFlags.get(key);
    // One decision for colour, width and dash together: they were previously
    // set in separate passes, and the later pass overwrote the dash the
    // stale/dangling branches had just chosen.
    let edgeColor, edgeWidth, markerEnd;
    let dashArray = null;
    const isPeripheral = peripheralEdges.has(key);
    if (isPeripheral) {
      // Outside Sanshain: a thin dashed slate line, never a pin colour.
      edgeColor = "#64748b";
      edgeWidth = "1.5";
      dashArray = "4 3";
      markerEnd = "url(#arrow)";
    } else if (isCycle) {
      edgeColor = "#a855f7";
      edgeWidth = "3";
      markerEnd = "url(#arrow-red)";
    } else if (isMissing) {
      edgeColor = "#f97316";
      edgeWidth = "2";
      dashArray = "6 3";
      markerEnd = "url(#arrow-orange)";
    } else if (hFlags && hFlags.dangling) {
      // The pinned version no longer exists (deleted); heals on re-provide.
      // Ranked above the messaging shapes below: those describe how an edge is
      // wired, this says the edge points at nothing. ADR-0005's "never
      // silently healthy" fails if a dangling AsyncAPI pin can be drawn as an
      // ordinary pub/sub link.
      edgeColor = "#ea580c";
      edgeWidth = "2";
      dashArray = "2 5";
      markerEnd = "url(#arrow-orange)";
    } else if (hFlags && hFlags.stale) {
      // Main view: the pin was not refreshed since the staleness boundary.
      edgeColor = "#f59e0b";
      edgeWidth = "2";
      dashArray = "3 4";
      markerEnd = "url(#arrow)";
    } else if (isBidirectionalPubSub) {
      edgeColor = "#94a3b8";
      edgeWidth = "2";
      dashArray = "6 3";
      markerEnd = "";
    } else if (hFlags && hFlags.conflict) {
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
    if (isPeripheral) path.dataset.edgeType = "peripheral";
    else if (isCycle) path.dataset.edgeType = "circular";
    else if (isMissing) path.dataset.edgeType = "missing";
    else if (isBidirectionalPubSub) path.dataset.edgeType = "pubsub-bidir";
    else path.dataset.edgeType = "normal";
    // Independent highlight flags + base styling, so the Outdated /
    // Snapshot-pinned toggles can restyle and restore without a redraw. The
    // stale/dangling colors live in edgeColor above, so baseStroke keeps them.
    path.dataset.outdated = hFlags && hFlags.outdated ? "1" : "0";
    path.dataset.breakingOutdated = hFlags && hFlags.breakingOutdated ? "1" : "0";
    path.dataset.snapshot = hFlags && hFlags.snapshot ? "1" : "0";
    path.dataset.conflict = hFlags && hFlags.conflict ? "1" : "0";
    path.dataset.stale = hFlags && hFlags.stale ? "1" : "0";
    path.dataset.dangling = hFlags && hFlags.dangling ? "1" : "0";
    path.dataset.baseStroke = edgeColor;
    path.dataset.baseWidth = edgeWidth;
    if (dashArray) path.setAttribute("stroke-dasharray", dashArray);
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
    const badges = nodeBadges.get(node) || { provides: [], requires: [] };

    // Determine node colors
    let fill, stroke, textColor;

    const isPeripheralNode = peripheralNodes.has(node);
    if (isPeripheralNode) {
      fill = "#f1f5f9";
      stroke = "#64748b";
      textColor = "#334155";
    } else if (isMissingService) {
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
    group.dataset.provides = badges.provides.join(",");
    group.dataset.requires = badges.requires.join(",");
    // Set role for legend highlighting
    if (isPeripheralNode) group.dataset.role = "peripheral";
    else if (isMissingService) group.dataset.role = "missing";
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
    if (isPeripheralNode) rect.setAttribute("stroke-dasharray", "4 3");
    group.appendChild(rect);

    // Top-left "!" for a missing service.
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

    // Double border for "both" nodes
    if (isClient && isService && !isPeripheralNode) {
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

    if (isPeripheralNode || node.length > 25) {
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
        const svcMeta = serviceMetaMap.get(node);
        const staleBefore = report.trunk_stale_before;
        const newestTrunkStamp = ((svcMeta && svcMeta.versions) || [])
          .filter((v) => v.trunk_provided_at)
          .map((v) => v.trunk_provided_at)
          .sort()
          .pop();
        const isStale = staleBefore && newestTrunkStamp && newestTrunkStamp < staleBefore;
        const vText = document.createElementNS("http://www.w3.org/2000/svg", "text");
        // Bottom-right when requires badges take the bottom-left.
        const besideBadges = badges.requires.length > 0;
        vText.setAttribute("x", besideBadges ? nd.x + nd.width / 2 - 8 : nd.x);
        vText.setAttribute("y", nd.y + nd.height / 2 - 7);
        vText.setAttribute("text-anchor", besideBadges ? "end" : "middle");
        vText.setAttribute("fill", isStale ? "#f59e0b" : textColor);
        vText.setAttribute("font-size", "10");
        vText.setAttribute("font-family", "ui-monospace, monospace");
        vText.setAttribute("opacity", "0.9");
        vText.textContent = isStale ? `⚠ v${trunkV}` : `v${trunkV}`;
        group.appendChild(vText);
      }
    }

    // Badges straddle the node's edges, drawn last so they sit on top:
    // provides solid on the top edge right-aligned, requires outlined on the
    // bottom edge left-aligned, both in the fixed REST, gRPC, MSG order.
    const nTop = nd.y - nd.height / 2;
    const nRight = nd.x + nd.width / 2;
    const nLeft = nd.x - nd.width / 2;
    badges.provides.forEach((kind, i) => {
      const fromRight = badges.provides.length - 1 - i;
      graphDrawBadge(group, "provides", kind, nRight - 4 - fromRight * 24, nTop + 2);
    });
    badges.requires.forEach((kind, i) => {
      graphDrawBadge(group, "requires", kind, nLeft + 4 + i * 24, nTop + nd.height - 2);
    });

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

            if (d.peripheral) {
              return `<div class="text-slate-300 text-[11px] py-1">depends on <span class="font-mono">${_graphEscapeHtml(path)}</span> <span class="text-slate-500">(Backstage)</span></div>`;
            }

            if (d.unfulfilled) {
              return `<div class="text-slate-300 text-[11px] py-1"><span class="font-mono text-indigo-400 font-bold">${_graphEscapeHtml(method)}</span> <span class="font-mono">${_graphEscapeHtml(path)}</span> <span class="text-slate-500">(nobody provides this channel yet)</span></div>`;
            }

            const url = `/yaml.html?service=${encodeURIComponent(to)}&api_type=${encodeURIComponent(type)}&version=${encodeURIComponent(d.version || "")}&path=${encodeURIComponent(path)}&method=${encodeURIComponent(method)}`;

            let methodClass = "text-indigo-400";
            if (method === "POST" || method === "PUB") methodClass = "text-emerald-400";
            if (method === "DELETE") methodClass = "text-rose-400";
            if (method === "PUT") methodClass = "text-amber-400";

            const supersededBy = graphSupersededBy(d, latestGaMap);
            const deprecatedBadge = d.deprecated
              ? '<span class="px-1 py-0.5 rounded-sm bg-amber-500/20 text-amber-400 border border-amber-500/30 text-[9px] font-bold ml-1">DEPRECATED</span>'
              : "";

            return `
            <a href="${url}" class="group block p-1.5 rounded-sm bg-white/5 hover:bg-indigo-500/20 border border-transparent hover:border-indigo-500/30 transition-all">
              <div class="flex items-center justify-between gap-2">
                <div class="flex items-center gap-1">
                  <span class="font-mono text-[11px] ${methodClass} font-bold">${_graphEscapeHtml(method)}</span>
                  ${deprecatedBadge}
                </div>
                <span class="text-[10px] text-slate-500 group-hover:text-indigo-300 transition-colors">${_graphEscapeHtml(type)} ${_graphEscapeHtml(String(d.version || ""))}${d.stability === "snapshot" ? ' <span class="text-amber-400 font-bold">SNAP</span>' : ""}</span>
              </div>
              <div class="font-mono text-[11px] text-slate-300 truncate group-hover:text-white transition-colors" title="${_graphEscapeHtml(path)}">${_graphEscapeHtml(path)}</div>
              ${supersededBy ? `<div data-superseded class="text-[10px] text-rose-400 font-semibold">superseded by ${_graphEscapeHtml(supersededBy)} ga</div>` : ""}
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

  // `member`: the scope producer behind an "(unrequired)" chip, whose panel
  // adds its version lines and when it was last provided.
  function showNodeTooltip(e, name, member) {
    if (tooltipHideTimer) {
      clearTimeout(tooltipHideTimer);
      tooltipHideTimer = null;
    }

    if (unownedChannelNodes.has(name)) {
      tooltip.innerHTML = `
        <div class="text-[10px] text-slate-400 font-bold uppercase tracking-wider mb-0.5">Unprovided Channel</div>
        <div class="font-bold text-white break-all">${_graphEscapeHtml(name)}</div>
        <div class="text-[11px] text-slate-400 mt-1">Subscribed to in an AsyncAPI document, but no Producer publishes this channel yet.</div>
      `;
      tooltip.style.display = "block";
      tooltip.style.left = Math.max(10, e.clientX + 8) + "px";
      tooltip.style.top = Math.max(10, e.clientY + 8) + "px";
      tooltip.style.opacity = "1";
      return;
    }

    if (name.startsWith("resource:")) {
      tooltip.innerHTML = `
        <div class="text-[10px] text-slate-400 font-bold uppercase tracking-wider mb-0.5">Peripheral Resource</div>
        <div class="font-bold text-white break-all">${_graphEscapeHtml(name)}</div>
        <div class="text-[11px] text-slate-400 mt-1">Outside Sanshain — declared in a system's Backstage catalog-info.yaml (dependsOn).</div>
      `;
      tooltip.style.display = "block";
      tooltip.style.left = Math.max(10, e.clientX + 8) + "px";
      tooltip.style.top = Math.max(10, e.clientY + 8) + "px";
      tooltip.style.opacity = "1";
      return;
    }

    // What the node provides and requires, as the badges show it — a chip's
    // producer has no edges, so it only provides its scoped lines.
    const badges = member
      ? graphNodeBadges({}, []).get(name) || { provides: [], requires: [] }
      : nodeBadges.get(name);
    const lastProvided = member ? graphLastProvided(member.versions) : null;
    const memberLines = member
      ? `<div class="mt-2 text-[11px] text-slate-300 font-mono break-words" data-panel="versions">${_graphEscapeHtml(graphVersionLinesText(member.versions))}</div>
         <div class="text-[10px] text-slate-400" data-panel="last-provided">${lastProvided ? `last provided ${_graphEscapeHtml(lastProvided.slice(0, 16).replace("T", " "))}` : "last provided: unknown at this instant"}</div>`
      : "";
    // Owner, description and link chips from the system's catalog-info
    // (#48), below the version lines; rides the scope payload.
    const systems = (window.graphScopeData && window.graphScopeData.systems) || {};
    const contextBlock = systemContextHtml(systems[name], true);
    const contextLines = contextBlock ? `<div class="mt-2">${contextBlock}</div>` : "";

    // The graph draws Producers and Consumers as the same kind of node, but the
    // two have different pages. `services_detailed` lists Producers only, so a
    // node missing from it consumes without providing — sending it to
    // producers.html asks for a version line that does not exist and lands on
    // "Fetch error: 404".
    const isProducer = (report?.services_detailed || []).some((s) => s.name === name);
    const href = isProducer
      ? `/producers.html?service=${encodeURIComponent(name)}`
      : `/consumers.html?name=${encodeURIComponent(name)}`;

    tooltip.innerHTML = `
      <div class="mb-3 pb-2 border-b border-slate-700/50">
        <div class="text-[10px] text-slate-400 font-bold uppercase tracking-wider mb-0.5">${isProducer ? "Producer" : "Consumer"} Node</div>
        <div class="font-bold text-lg text-white truncate mb-1">${_graphEscapeHtml(name)}</div>
        ${graphBadgeSummaryHtml(badges)}
        ${memberLines}
        ${contextLines}
      </div>
      <div class="space-y-3">
        <a href="${href}" class="flex items-center justify-center gap-2 w-full py-2 px-3 rounded-lg bg-indigo-600 hover:bg-indigo-500 text-white font-medium transition-all shadow-lg shadow-indigo-500/20">
          <span>View ${isProducer ? "Service" : "Consumer"} Details</span>
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

  // An "(unrequired)" chip opens the node panel on click.
  mainG.querySelectorAll(".graph-unrequired-chip").forEach((el) => {
    el.addEventListener("click", (e) => {
      e.stopPropagation();
      showNodeTooltip(e, el.dataset.chip, unrequiredByName.get(el.dataset.chip));
    });
    el.addEventListener("mousedown", (e) => e.stopPropagation());
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

  _graphFitToView = fitToView;
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

  // Exports carry their scope (ADR-0005): view, branch and instant.
  const view = window.graphStreamView || "dev";
  let stamp = view === "branch" ? `branch ${window.graphBranchName}` : view;
  if (window.graphTimelineAt) stamp += ` @ ${window.graphTimelineAt}`;
  const filename = `dependency_graph_${stamp.replace(/[^a-zA-Z0-9.@-]+/g, "_")}.png`;

  // Clone the SVG to avoid modifying the live one
  const svgClone = svg.cloneNode(true);
  const stampText = document.createElementNS("http://www.w3.org/2000/svg", "text");
  stampText.setAttribute("x", "8");
  stampText.setAttribute("y", "16");
  stampText.setAttribute("font-size", "12");
  stampText.setAttribute("fill", "#64748b");
  stampText.setAttribute("font-family", "ui-monospace, monospace");
  stampText.textContent = `Sanshain — ${stamp}`;
  svgClone.appendChild(stampText);
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
