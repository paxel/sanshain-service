// Experimental domain view (z, `level=domains`): an arc42 level-0 picture of
// the graph. Each domain is one node in its domain colour; services without a
// domain stay nodes of their own. The pins between two of them are one arrow
// per direction, labelled with their count and coloured by the strongest
// warning among them; the pins inside a domain are a count on its node.
// Clicking a domain node opens the full graph focused on that domain.

// From weakest to strongest; the strongest among an edge's pins colours it.
const GRAPH_DOMAIN_SEVERITIES = [
  { name: "none", color: "#94a3b8" },
  { name: "snapshot", color: "#f59e0b" },
  { name: "stale", color: "#f59e0b" },
  { name: "outdated", color: "#f43f5e" },
  { name: "breakingOutdated", color: "#be123c" },
  { name: "dangling", color: "#ea580c" },
  { name: "missing", color: "#f97316" },
  { name: "conflict", color: "#dc2626" },
];

// The severity index of one pin from its edge flags and the missing edges.
// Outdated, a major behind and snapshot-pinned colour only while their
// highlight is on, as in the full graph.
function graphDomainPinSeverity(flags, missing, highlights) {
  let level = 0;
  const raise = (name) => {
    const i = GRAPH_DOMAIN_SEVERITIES.findIndex((s) => s.name === name);
    if (i > level) level = i;
  };
  if (flags) {
    for (const name of ["snapshot", "outdated", "breakingOutdated"]) {
      if (flags[name] && highlights[name]) raise(name);
    }
    for (const name of ["stale", "dangling"]) {
      if (flags[name]) raise(name);
    }
    if (flags.conflict) raise("conflict");
  }
  if (missing) raise("missing");
  return level;
}

// Collapse a report's pins onto domains: the units (domain or lone service),
// the edges between units, and the pins inside each domain.
function graphCollapseDomains(report, edgeFlags, highlights = window.graphHighlightFilters || {}) {
  const domainOf = new Map();
  for (const s of report.services_detailed || []) {
    if (s.domain) domainOf.set(s.name, s.domain);
  }
  const unitOf = (name) => (domainOf.has(name) ? `domain:${domainOf.get(name)}` : name);
  const missing = new Set(
    (report.missing_endpoints || []).map((m) => `${m.client}-->${m.service}`),
  );
  const units = new Map();
  const unit = (id) => {
    if (!units.has(id)) {
      const isDomain = id.startsWith("domain:");
      units.set(id, {
        id,
        domain: isDomain ? id.slice("domain:".length) : null,
        services: new Set(),
        internal: 0,
        internalLevel: 0,
        peripheral: false,
      });
    }
    return units.get(id);
  };
  const edges = new Map();
  for (const d of report.dependency_graph || []) {
    const from = unit(unitOf(d.client));
    const to = unit(unitOf(d.service));
    from.services.add(d.client);
    to.services.add(d.service);
    if (d.peripheral) to.peripheral = true;
    const key = `${d.client}-->${d.service}`;
    const level = graphDomainPinSeverity(
      edgeFlags.get(d.flag_key || key),
      missing.has(key),
      highlights,
    );
    if (from === to) {
      if (!from.domain) continue;
      from.internal += 1;
      from.internalLevel = Math.max(from.internalLevel, level);
      continue;
    }
    const edgeKey = `${from.id}-->${to.id}`;
    if (!edges.has(edgeKey)) edges.set(edgeKey, { from: from.id, to: to.id, count: 0, level: 0 });
    const edge = edges.get(edgeKey);
    edge.count += 1;
    edge.level = Math.max(edge.level, level);
  }
  return { units: [...units.values()], edges: [...edges.values()] };
}
window.graphCollapseDomains = graphCollapseDomains;

const SVG_NS = "http://www.w3.org/2000/svg";
function graphDomainSvg(tag, attrs, text) {
  const el = document.createElementNS(SVG_NS, tag);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, String(v));
  if (text !== undefined) el.textContent = text;
  return el;
}

function renderDomainGraph(report, svgElement, direction) {
  while (svgElement.firstChild) svgElement.removeChild(svgElement.firstChild);
  svgElement.dataset.level = "domains";
  const isMainView = window.graphStreamView === "main";
  const trunkVersionMap =
    isMainView && !window.graphTimelineAt ? graphTrunkVersionMap(report) : null;
  const edgeFlags = graphEdgeFlags(report, graphLatestGaMap(report), trunkVersionMap);
  const { units, edges } = graphCollapseDomains(report, edgeFlags);

  const isLR = direction === "LR";
  const gaps = graphLayoutGaps(isLR, window.graphDensity);
  const g = new dagre.graphlib.Graph();
  g.setGraph({
    rankdir: isLR ? "LR" : "TB",
    nodesep: gaps.nodesep,
    ranksep: gaps.ranksep,
    edgesep: gaps.edgesep,
    marginx: 40,
    marginy: 40,
  });
  g.setDefaultEdgeLabel(() => ({}));
  const title = (u) =>
    u.domain || (u.peripheral && window.graphPeripheralLabel ? graphPeripheralLabel(u.id) : u.id);
  const subtitle = (u) => {
    const services = `${u.services.size} service${u.services.size === 1 ? "" : "s"}`;
    return `${services} · ${u.internal} internal pin${u.internal === 1 ? "" : "s"}`;
  };
  for (const u of units) {
    const sub = u.domain ? subtitle(u).length * 6.5 : 0;
    const width = Math.max(120, Math.ceil(Math.max(title(u).length * 9, sub)) + 40);
    g.setNode(u.id, { width, height: u.domain ? 64 : 44 });
  }
  for (const e of edges) g.setEdge(e.from, e.to);
  dagre.layout(g);

  const holder = svgElement.parentElement;
  const availableH =
    document.body.dataset.graphFs && holder
      ? holder.clientHeight
      : window.innerHeight - svgElement.getBoundingClientRect().top - 40;
  svgElement.style.height = `${Math.max(availableH, 300)}px`;
  const vw = svgElement.clientWidth;
  const vh = svgElement.clientHeight;
  svgElement.setAttribute("viewBox", `0 0 ${vw} ${vh}`);

  const defs = graphDomainSvg("defs", {});
  GRAPH_DOMAIN_SEVERITIES.forEach((s, i) => {
    const marker = graphDomainSvg("marker", {
      id: `domain-arrow-${i}`,
      viewBox: "0 0 10 10",
      refX: 9,
      refY: 5,
      markerWidth: 7,
      markerHeight: 7,
      orient: "auto-start-reverse",
    });
    marker.appendChild(graphDomainSvg("path", { d: "M0,0 L10,5 L0,10 z", fill: s.color }));
    defs.appendChild(marker);
  });
  svgElement.appendChild(defs);
  const mainG = graphDomainSvg("g", {});
  svgElement.appendChild(mainG);

  for (const e of edges) {
    const points = g.edge(e.from, e.to).points || [];
    if (points.length < 2) continue;
    const severity = GRAPH_DOMAIN_SEVERITIES[e.level];
    const d = points.map((p, i) => `${i ? "L" : "M"}${p.x},${p.y}`).join(" ");
    const path = graphDomainSvg("path", {
      d,
      fill: "none",
      stroke: severity.color,
      "stroke-width": e.level ? 3 : 2,
      "marker-end": `url(#domain-arrow-${e.level})`,
      class: "graph-domain-edge",
    });
    path.dataset.from = e.from;
    path.dataset.to = e.to;
    path.dataset.count = String(e.count);
    path.dataset.severity = severity.name;
    mainG.appendChild(path);
    const mid = points[Math.floor(points.length / 2)];
    const label = graphDomainSvg(
      "text",
      {
        x: mid.x,
        y: mid.y,
        "text-anchor": "middle",
        "dominant-baseline": "central",
        "font-size": 12,
        "font-weight": 700,
        fill: severity.color,
        stroke: "#ffffff",
        "stroke-width": 4,
        "paint-order": "stroke",
      },
      String(e.count),
    );
    mainG.appendChild(label);
  }

  for (const u of units) {
    const nd = g.node(u.id);
    const group = graphDomainSvg("g", { class: "graph-domain-unit" });
    group.dataset.unit = u.id;
    const x = nd.x - nd.width / 2;
    const y = nd.y - nd.height / 2;
    const color = u.domain ? graphDomainColor(u.domain) : null;
    group.appendChild(
      graphDomainSvg("rect", {
        x,
        y,
        width: nd.width,
        height: nd.height,
        rx: 10,
        fill: color ? color.fill : "#e0e7ff",
        stroke: color ? color.border : "#6366f1",
        "stroke-width": 2,
      }),
    );
    group.appendChild(
      graphDomainSvg(
        "text",
        {
          x: nd.x,
          y: u.domain ? nd.y - 10 : nd.y,
          "text-anchor": "middle",
          "dominant-baseline": "central",
          "font-size": 15,
          "font-weight": 700,
          "font-family": "ui-sans-serif, system-ui, sans-serif",
          fill: color ? color.text : "#3730a3",
        },
        title(u),
      ),
    );
    if (u.domain) {
      group.style.cursor = "pointer";
      const internal = graphDomainSvg(
        "text",
        {
          x: nd.x,
          y: nd.y + 14,
          "text-anchor": "middle",
          "dominant-baseline": "central",
          "font-size": 11,
          "font-family": "ui-sans-serif, system-ui, sans-serif",
          fill: GRAPH_DOMAIN_SEVERITIES[u.internalLevel].color,
          "font-weight": u.internalLevel ? 700 : 400,
        },
        subtitle(u),
      );
      internal.dataset.internal = String(u.internal);
      internal.dataset.severity = GRAPH_DOMAIN_SEVERITIES[u.internalLevel].name;
      group.appendChild(internal);
      group.appendChild(graphDomainSvg("title", {}, `Open the graph of ${u.domain}`));
      group.addEventListener("click", () => {
        if (window.openGraphDomain) window.openGraphDomain(u.domain);
      });
    }
    mainG.appendChild(group);
  }

  // Fit, then wheel to zoom and drag to pan, as in the full graph.
  const box = g.graph();
  let scale = Math.min(1.2, Math.min((vw - 80) / (box.width || 1), (vh - 80) / (box.height || 1)));
  let panX = vw / 2 - ((box.width || 0) / 2) * scale;
  let panY = vh / 2 - ((box.height || 0) / 2) * scale;
  const apply = () => mainG.setAttribute("transform", `translate(${panX},${panY}) scale(${scale})`);
  apply();
  svgElement.onwheel = (e) => {
    e.preventDefault();
    const rect = svgElement.getBoundingClientRect();
    const mx = ((e.clientX - rect.left) / rect.width) * vw;
    const my = ((e.clientY - rect.top) / rect.height) * vh;
    const next = Math.max(0.2, Math.min(3, scale * (e.deltaY > 0 ? 0.9 : 1.1)));
    panX = mx - (mx - panX) * (next / scale);
    panY = my - (my - panY) * (next / scale);
    scale = next;
    apply();
  };
  let drag = null;
  svgElement.onmousedown = (e) => {
    if (e.button === 0) drag = { x: e.clientX, y: e.clientY };
  };
  svgElement.onmousemove = (e) => {
    if (!drag) return;
    const rect = svgElement.getBoundingClientRect();
    panX += ((e.clientX - drag.x) / rect.width) * vw;
    panY += ((e.clientY - drag.y) / rect.height) * vh;
    drag = { x: e.clientX, y: e.clientY };
    apply();
  };
  svgElement.onmouseup = () => {
    drag = null;
  };
  svgElement.onmouseleave = () => {
    drag = null;
  };
}
window.renderDomainGraph = renderDomainGraph;
