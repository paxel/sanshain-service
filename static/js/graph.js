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

window.graphRedrawMode = 'all'; // 'all' or 'circular'
window.graphFocusTags = []; // focus filter (multi-service tag cloud)
window.graphProtocolFilters = {
    openapi: true,
    asyncapi: true,
    proto: true
};

// ── Redraw Coordinator ───────────────────────────────────────────────

function redrawGraph() {
    if (typeof lastGraphReport === 'undefined' || !lastGraphReport) return;
    
    const mode = typeof currentGraphMode !== 'undefined' ? currentGraphMode : 'custom';
    const direction = typeof currentGraphDirection !== 'undefined' ? currentGraphDirection : 'TB';

    const filteredReport = getFilteredReport(lastGraphReport);
    
    const customSvg = document.getElementById('custom-graph');
    const mermaidDiv = document.getElementById('mermaid-graph');
    
    if (mode === 'custom' && customSvg) {
        renderCustomGraph(filteredReport, customSvg, direction);
    } else if (mermaidDiv) {
        // renderGraph is defined in service.html and it uses lastGraphReport globally.
        // We should change it to accept a report parameter if possible, 
        // but for now we can temporarily swap lastGraphReport or just accept it's only for custom graph.
        // User asked to "redraw the graph", implying it should work for all modes.
        
        // Actually, Mermaid renderGraph takes a second parameter for detailed mode.
        // Let's modify service.html's renderGraph to accept report as first arg.
        if (typeof renderGraph === 'function') {
            renderGraph(filteredReport, mode === 'detailed');
        }
    }
}
window.redrawGraph = redrawGraph;

function getFilteredReport(report) {
    if (!report) return null;
    let deps = [...(report.dependency_graph || [])];
    
    // 1. Circular dependencies filter
    if (window.graphRedrawMode === 'circular') {
        const adjMap = new Map();
        deps.forEach(d => {
            if (!adjMap.has(d.client)) adjMap.set(d.client, []);
            if (!adjMap.get(d.client).includes(d.service)) adjMap.get(d.client).push(d.service);
        });
        const cycleEdges = graphDetectCycles(adjMap);
        const cycleNodes = new Set();
        cycleEdges.forEach(e => {
            const [from, to] = e.split('-->');
            cycleNodes.add(from);
            cycleNodes.add(to);
        });
        // Only keep edges where BOTH nodes are in a cycle
        deps = deps.filter(d => cycleNodes.has(d.client) && cycleNodes.has(d.service));
    }
    
    // 2. Focus filter (multi-service tag cloud)
    if (window.graphFocusTags && window.graphFocusTags.length > 0) {
        const focusNodes = new Set();
        for (const tag of window.graphFocusTags) {
            const target = tag.toLowerCase();
            const exactMatch = deps.find(d => d.client.toLowerCase() === target || d.service.toLowerCase() === target);
            if (exactMatch) {
                const realName = exactMatch.client.toLowerCase() === target ? exactMatch.client : exactMatch.service;
                focusNodes.add(realName);
                deps.forEach(d => {
                    if (d.client === realName) focusNodes.add(d.service);
                    if (d.service === realName) focusNodes.add(d.client);
                });
            }
        }
        if (focusNodes.size > 0) {
            deps = deps.filter(d => focusNodes.has(d.client) && focusNodes.has(d.service));
        }
    }

    // 3. Protocol filter
    deps = deps.filter(d => {
        const type = (d.api_type || '').toLowerCase();
        if (type === 'openapi' && !window.graphProtocolFilters.openapi) return false;
        if (type === 'asyncapi' && !window.graphProtocolFilters.asyncapi) return false;
        if (type === 'proto' && !window.graphProtocolFilters.proto) return false;
        return true;
    });
    
    return { ...report, dependency_graph: deps };
}
window.getFilteredReport = getFilteredReport;

function setGraphRedrawMode(mode) {
    window.graphRedrawMode = mode;
    ['all', 'circular'].forEach(m => {
        const btn = document.getElementById(`graph-redraw-${m}`);
        if (btn) {
            btn.className = m === mode
                ? 'px-3 py-1.5 bg-indigo-600 text-white font-medium'
                : 'px-3 py-1.5 text-slate-600 hover:bg-slate-50 font-medium';
        }
    });
    redrawGraph();
}
window.setGraphRedrawMode = setGraphRedrawMode;

function addFocusTag(name) {
    if (!name || !name.trim()) return;
    const trimmed = name.trim();
    if (window.graphFocusTags.includes(trimmed)) return;
    window.graphFocusTags.push(trimmed);
    renderFocusTags();
    redrawGraph();
}
window.addFocusTag = addFocusTag;

function removeFocusTag(name) {
    window.graphFocusTags = window.graphFocusTags.filter(t => t !== name);
    renderFocusTags();
    redrawGraph();
}
window.removeFocusTag = removeFocusTag;

function renderFocusTags() {
    const container = document.getElementById('focus-tags-container');
    if (!container) return;
    container.innerHTML = '';
    window.graphFocusTags.forEach(tag => {
        const pill = document.createElement('span');
        pill.className = 'inline-flex items-center gap-1 px-2 py-0.5 bg-indigo-100 text-indigo-700 rounded-full text-xs font-medium';
        pill.innerHTML = `${_graphEscapeHtml(tag)}<button onclick="removeFocusTag('${tag.replace(/'/g, "\\'")}')"
            class="hover:text-indigo-900 cursor-pointer text-indigo-400 font-bold leading-none">&times;</button>`;
        container.appendChild(pill);
    });
}
window.renderFocusTags = renderFocusTags;

function applyGraphFocus() {
    const input = document.getElementById('graph-service-filter');
    if (!input) return;
    const val = input.value.trim();
    input.value = '';
    if (val) addFocusTag(val);
}
window.applyGraphFocus = applyGraphFocus;

function toggleProtocolFilter(protocol) {
    window.graphProtocolFilters[protocol] = !window.graphProtocolFilters[protocol];
    
    const btn = document.getElementById(`filter-${protocol}`);
    if (btn) {
        if (window.graphProtocolFilters[protocol]) {
            btn.className = 'px-2.5 py-1.5 bg-indigo-600 text-white font-medium';
        } else {
            btn.className = 'px-2.5 py-1.5 bg-white text-slate-600 hover:bg-slate-50 font-medium';
        }
    }
    redrawGraph();
}
window.toggleProtocolFilter = toggleProtocolFilter;

// ── Helpers ──────────────────────────────────────────────────────────

function _graphEscapeHtml(s) {
    return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function _measureNode(name) {
    const fontSize = 16;
    const charW = 9; // estimated average char width for 16px bold sans
    const idealCharsPerLine = 16;
    const maxLines = 3;

    if (name.length <= idealCharsPerLine) {
        return {
            width: Math.max(100, Math.ceil(name.length * charW) + 40),
            height: 48,
            lines: [name]
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

    const maxLineLen = Math.max(...lines.map(l => l.length));
    return {
        width: Math.max(100, Math.ceil(maxLineLen * charW) + 40),
        height: lines.length * 20 + 24,
        lines: lines
    };
}


// ── Cycle detection (reusable from service.html) ─────────────────────

function graphDetectCycles(adjMap) {
    const WHITE = 0, GRAY = 1, BLACK = 2;
    const color = {};
    const parent = {};
    const cycleEdges = new Set();

    for (const node of adjMap.keys()) color[node] = WHITE;

    function dfs(u) {
        color[u] = GRAY;
        for (const v of (adjMap.get(u) || [])) {
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
        const warningDiv = document.getElementById('graph-cycle-warning');
        if (warningDiv) warningDiv.classList.add('hidden');
        svgElement.setAttribute('viewBox', '0 0 400 120');
        svgElement.style.height = '120px';
        const text = document.createElementNS('http://www.w3.org/2000/svg', 'text');
        text.setAttribute('x', '200');
        text.setAttribute('y', '60');
        text.setAttribute('text-anchor', 'middle');
        text.setAttribute('dominant-baseline', 'central');
        text.setAttribute('fill', '#94a3b8');
        text.setAttribute('font-size', '16');
        text.setAttribute('font-style', 'italic');
        text.textContent = 'No dependencies match the current filters';
        svgElement.appendChild(text);
        return;
    }

    // Layout direction: 'TB' (default, top→bottom) or 'LR' (left→right).
    // In LR mode the rank axis becomes X, so we swap the roles of x/y
    // across the entire post-layout pipeline (brick stagger, rank packing,
    // cluster bounds, edge tangents).
    const dir = direction === 'LR' ? 'LR' : 'TB';
    const isLR = dir === 'LR';

    // Build graph data
    const allNodes = new Set();
    const adjMap = new Map();
    const edgeLabels = new Map(); // "from-->to" -> Set<"METHOD /path">

    deps.forEach(d => {
        const from = d.client, to = d.service;
        allNodes.add(from);
        allNodes.add(to);
        if (!adjMap.has(from)) adjMap.set(from, []);
        if (!adjMap.get(from).includes(to)) adjMap.get(from).push(to);
        const key = `${from}-->${to}`;
        if (!edgeLabels.has(key)) edgeLabels.set(key, new Set());
        edgeLabels.get(key).add(`${d.method} ${d.path}`);
    });

    const clientNodes = new Set(deps.map(d => d.client));
    const serviceNodes = new Set(deps.map(d => d.service));
    const cycleEdges = graphDetectCycles(adjMap);

    // Show/hide cycle warning
    const warningDiv = document.getElementById('graph-cycle-warning');
    if (warningDiv) {
        warningDiv.classList.toggle('hidden', cycleEdges.size === 0);
    }

    // ── Dagre layout ─────────────────────────────────────────────────
    const g = new dagre.graphlib.Graph();
    g.setGraph({
        rankdir: dir,
        // In LR mode the along-rank separation is vertical; nodes are only
        // ~40px tall so they'd overlap with the default 50px. Give them room.
        nodesep: isLR ? 80 : 50,
        // LR no longer does brick staggering along X, so ranksep can stay
        // tight — just enough gap between rank columns of 160px nodes to
        // leave room for the Bezier edge curvature between layers.
        ranksep: isLR ? 110 : 180,
        edgesep: 50,
        marginx: 40,
        marginy: 60
    });
    g.setDefaultEdgeLabel(() => ({}));

    const nodeMetrics = new Map();
    for (const node of allNodes) {
        const m = _measureNode(node);
        nodeMetrics.set(node, m);
        g.setNode(node, { label: node, width: m.width, height: m.height });
    }

    const nodeRole = new Map();
    for (const node of allNodes) {
        const isC = clientNodes.has(node);
        const isS = serviceNodes.has(node);
        if (isC && isS) nodeRole.set(node, 1); // BOTH
        else if (isC) nodeRole.set(node, 0);   // CLIENT ONLY
        else nodeRole.set(node, 2);            // SERVICE ONLY
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
    const F = isLR ? 'y' : 'x';
    const R = isLR ? 'x' : 'y';

    const nodeFOffsets = new Map();  // delta along flow axis
    for (const node of allNodes) {
        nodeFOffsets.set(node, 0);
    }

    const GAP = isLR ? 25 : 30;           // gap between units on a rank (flow axis)
    const GROUP_GAP = isLR ? 100 : 120;   // larger gap between different roles (flow axis)

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
    unitsByRank.forEach(units => {
        units.sort((a, b) => {
            if (a.role !== b.role) return a.role - b.role;
            return a.cf - b.cf;
        });
        const roleGroups = [[], [], []];
        units.forEach(u => roleGroups[u.role].push(u));
        for (let i = 0; i < 3; i++) {
            if (roleGroups[i].length > 0) {
                const w = roleGroups[i].reduce((acc, u) => acc + u.width, 0) + (roleGroups[i].length - 1) * GAP;
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
    unitsByRank.forEach(units => {
        const roleGroups = [[], [], []];
        units.forEach(u => roleGroups[u.role].push(u));
        for (let i = 0; i < 3; i++) {
            // All groups of the same role across all ranks now share the same 
            // starting offset, ensuring "leftest nodes same level".
            let cursor = laneStart[i] + globalOffset;
            roleGroups[i].forEach(u => {
                u.newCf = cursor + u.width / 2;
                cursor += u.width + GAP;
            });
        }
    });

    // Step 2 — apply computed positions.
    unitsByRank.forEach(units => {
        units.forEach(u => {
            const nd = g.node(u.id);
            nodeFOffsets.set(u.id, u.newCf - nd[F]);
        });
    });

    for (const node of allNodes) {
        const nd = g.node(node);
        nd[F] += nodeFOffsets.get(node) || 0;
    }

    // Recompute edge endpoints from final node positions. Anchor on the
    // rank-axis border of each node (top/bottom for TB, left/right for LR).
    g.edges().forEach(e => {
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

    const graphInfo = g.graph();
    const svgW = graphInfo.width + 40;
    const svgH = graphInfo.height + 40;

    // Viewport-sized canvas: fill available window height
    const availableH = window.innerHeight - svgElement.getBoundingClientRect().top - 40;
    svgElement.style.height = Math.max(availableH, 300) + 'px';
    const vw = svgElement.clientWidth;
    const vh = svgElement.clientHeight;
    svgElement.setAttribute('viewBox', `0 0 ${vw} ${vh}`);

    // Defs for arrowheads
    const defs = document.createElementNS('http://www.w3.org/2000/svg', 'defs');
    ['#94a3b8', '#ef4444'].forEach((color, i) => {
        const marker = document.createElementNS('http://www.w3.org/2000/svg', 'marker');
        marker.setAttribute('id', i === 0 ? 'arrow' : 'arrow-red');
        marker.setAttribute('viewBox', '0 0 10 10');
        marker.setAttribute('refX', '10');
        marker.setAttribute('refY', '5');
        marker.setAttribute('markerWidth', '8');
        marker.setAttribute('markerHeight', '8');
        marker.setAttribute('orient', 'auto-start-reverse');
        const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
        path.setAttribute('d', 'M 0 0 L 10 5 L 0 10 z');
        path.setAttribute('fill', color);
        marker.appendChild(path);
        defs.appendChild(marker);
    });
    svgElement.appendChild(defs);

    // Main group for zoom/pan
    const mainG = document.createElementNS('http://www.w3.org/2000/svg', 'g');
    mainG.setAttribute('class', 'graph-main');
    svgElement.appendChild(mainG);

    // ── Render edges ─────────────────────────────────────────────────
    const edgeElements = new Map(); // "from-->to" -> { path, hitArea }

    g.edges().forEach(e => {
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
        const hitArea = document.createElementNS('http://www.w3.org/2000/svg', 'path');
        hitArea.setAttribute('d', d);
        hitArea.setAttribute('stroke', 'transparent');
        hitArea.setAttribute('stroke-width', '16');
        hitArea.setAttribute('fill', 'none');
        hitArea.setAttribute('class', 'edge-hit');
        hitArea.dataset.from = e.v;
        hitArea.dataset.to = e.w;
        mainG.appendChild(hitArea);

        // Visible edge
        const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
        path.setAttribute('d', d);
        path.setAttribute('stroke', isCycle ? '#ef4444' : '#94a3b8');
        path.setAttribute('stroke-width', isCycle ? '3' : '2');
        path.setAttribute('fill', 'none');
        path.setAttribute('marker-end', isCycle ? 'url(#arrow-red)' : 'url(#arrow)');
        path.setAttribute('class', 'graph-edge');
        path.dataset.from = e.v;
        path.dataset.to = e.w;
        if (isCycle) path.setAttribute('stroke-dasharray', '6 3');
        mainG.appendChild(path);

        edgeElements.set(key, { path, hitArea });
    });

    // ── Render nodes ─────────────────────────────────────────────────
    const nodeElements = new Map(); // nodeName -> group element

    for (const node of allNodes) {
        const nd = g.node(node);
        const isClient = clientNodes.has(node);
        const isService = serviceNodes.has(node);

        let fill, stroke, textColor;
        if (isClient && isService) {
            fill = '#fef3c7'; stroke = '#f59e0b'; textColor = '#92400e';
        } else if (isService) {
            fill = '#e0e7ff'; stroke = '#6366f1'; textColor = '#3730a3';
        } else {
            fill = '#ccfbf1'; stroke = '#14b8a6'; textColor = '#115e59';
        }

        const group = document.createElementNS('http://www.w3.org/2000/svg', 'g');
        group.setAttribute('class', 'graph-node');
        group.dataset.node = node;
        group.style.cursor = 'pointer';

        const rect = document.createElementNS('http://www.w3.org/2000/svg', 'rect');
        rect.setAttribute('x', nd.x - nd.width / 2);
        rect.setAttribute('y', nd.y - nd.height / 2);
        rect.setAttribute('width', nd.width);
        rect.setAttribute('height', nd.height);
        rect.setAttribute('rx', '8');
        rect.setAttribute('ry', '8');
        rect.setAttribute('fill', fill);
        rect.setAttribute('stroke', stroke);
        rect.setAttribute('stroke-width', '2');
        group.appendChild(rect);

        // Double border for "both" nodes
        if (isClient && isService) {
            const inner = document.createElementNS('http://www.w3.org/2000/svg', 'rect');
            inner.setAttribute('x', nd.x - nd.width / 2 + 4);
            inner.setAttribute('y', nd.y - nd.height / 2 + 4);
            inner.setAttribute('width', nd.width - 8);
            inner.setAttribute('height', nd.height - 8);
            inner.setAttribute('rx', '5');
            inner.setAttribute('ry', '5');
            inner.setAttribute('fill', 'none');
            inner.setAttribute('stroke', stroke);
            inner.setAttribute('stroke-width', '1');
            inner.setAttribute('stroke-opacity', '0.5');
            group.appendChild(inner);
        }

        const m = nodeMetrics.get(node);
        const lineH = 20;
        const totalTextH = m.lines.length * lineH;
        m.lines.forEach((line, i) => {
            const text = document.createElementNS('http://www.w3.org/2000/svg', 'text');
            text.setAttribute('x', nd.x);
            text.setAttribute('y', nd.y - totalTextH / 2 + (i * lineH) + lineH / 2);
            text.setAttribute('text-anchor', 'middle');
            text.setAttribute('dominant-baseline', 'central');
            text.setAttribute('fill', textColor);
            text.setAttribute('font-size', '16');
            text.setAttribute('font-family', 'ui-sans-serif, system-ui, sans-serif');
            text.setAttribute('font-weight', '600');
            text.textContent = line;
            group.appendChild(text);
        });

        if (node.length > 25) { // Show full name on hover if it was wrapped/truncated
            const title = document.createElementNS('http://www.w3.org/2000/svg', 'title');
            title.textContent = node;
            group.appendChild(title);
        }

        mainG.appendChild(group);
        nodeElements.set(node, group);
    }

    // ── Tooltip ──────────────────────────────────────────────────────
    let tooltip = document.getElementById('graph-tooltip');
    if (!tooltip) {
        tooltip = document.createElement('div');
        tooltip.id = 'graph-tooltip';
        tooltip.style.cssText = 'position:fixed;pointer-events:none;background:#1e293b;color:#f1f5f9;padding:6px 10px;border-radius:6px;font-size:12px;font-family:ui-monospace,monospace;line-height:1.5;z-index:1000;display:none;max-width:350px;white-space:pre-line;box-shadow:0 4px 12px rgba(0,0,0,0.3)';
        document.body.appendChild(tooltip);
    }

    function showTooltip(e, from, to) {
        const key = `${from}-->${to}`;
        const labels = edgeLabels.get(key);
        if (!labels || labels.size === 0) return;
        const lines = [...labels].sort().map(l => _graphEscapeHtml(l));
        tooltip.innerHTML = `<div style="font-weight:600;margin-bottom:2px;color:#a5b4fc">${_graphEscapeHtml(from)} → ${_graphEscapeHtml(to)}</div>` + lines.join('\n');
        tooltip.style.display = 'block';
        tooltip.style.left = (e.clientX + 12) + 'px';
        tooltip.style.top = (e.clientY + 12) + 'px';
    }

    function hideTooltip() {
        tooltip.style.display = 'none';
    }

    // Edge hover
    mainG.querySelectorAll('.edge-hit, .graph-edge').forEach(el => {
        el.addEventListener('mouseenter', e => showTooltip(e, el.dataset.from, el.dataset.to));
        el.addEventListener('mousemove', e => {
            tooltip.style.left = (e.clientX + 12) + 'px';
            tooltip.style.top = (e.clientY + 12) + 'px';
        });
        el.addEventListener('mouseleave', hideTooltip);
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
            const [from, to] = key.split('-->');
            if (from === nodeName || to === nodeName) {
                connected.add(from);
                connected.add(to);
                connectedEdges.add(key);
            }
        }

        // Dim everything, then highlight connected
        nodeElements.forEach((el, name) => {
            el.style.opacity = connected.has(name) ? '1' : '0.15';
            el.style.transition = 'opacity 0.2s';
        });
        edgeElements.forEach((els, key) => {
            const op = connectedEdges.has(key) ? '1' : '0.1';
            els.path.style.opacity = op;
            els.path.style.transition = 'opacity 0.2s';
        });
    }

    function clearHighlight() {
        highlighted = null;
        nodeElements.forEach(el => { el.style.opacity = '1'; el.style.transition = 'opacity 0.2s'; });
        edgeElements.forEach(els => { els.path.style.opacity = '1'; els.path.style.transition = 'opacity 0.2s'; });
    }

    mainG.querySelectorAll('.graph-node').forEach(el => {
        el.addEventListener('click', e => {
            e.stopPropagation();
            highlightNode(el.dataset.node);
        });
    });

    svgElement.addEventListener('click', clearHighlight);

    // ── Zoom & Pan ───────────────────────────────────────────────────
    // Fit-to-view initial scale and centering
    let scale = Math.min(vw / svgW, vh / svgH) * 0.9;
    let panX = (vw - svgW * scale) / 2;
    let panY = (vh - svgH * scale) / 2;
    let isPanning = false, startX = 0, startY = 0;

    updateTransform();

    function updateTransform() {
        mainG.setAttribute('transform', `translate(${panX},${panY}) scale(${scale})`);
    }

    svgElement.addEventListener('wheel', e => {
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
    }, { passive: false });

    svgElement.addEventListener('mousedown', e => {
        if (e.button !== 0) return;
        isPanning = true;
        startX = e.clientX;
        startY = e.clientY;
        svgElement.style.cursor = 'grabbing';
    });

    window.addEventListener('mousemove', e => {
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

    window.addEventListener('mouseup', () => {
        isPanning = false;
        svgElement.style.cursor = 'default';
    });
}

/**
 * Returns the SVG markup string for copy/download.
 */
function getCustomGraphSVG(svgElement) {
    return svgElement.outerHTML;
}

// ── Debounced resize handler ────────────────────────────────────────
let _graphResizeTimer = null;
window.addEventListener('resize', () => {
    clearTimeout(_graphResizeTimer);
    _graphResizeTimer = setTimeout(() => redrawGraph(), 200);
});
