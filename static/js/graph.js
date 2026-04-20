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

// ── Helpers ──────────────────────────────────────────────────────────

function _graphEscapeHtml(s) {
    return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function _identifyNoiseWords(allNodes) {
    const splitWords = (s) => s.split(/(?=[A-Z])|[^a-zA-Z0-9]/).filter(w => w.length > 1).map(w => w.toLowerCase());
    const wordCounts = {};
    const nodesArray = Array.from(allNodes);
    nodesArray.forEach(s => {
        const words = new Set(splitWords(s));
        words.forEach(w => {
            wordCounts[w] = (wordCounts[w] || 0) + 1;
        });
    });

    const noise = new Set();
    const threshold = Math.max(2, nodesArray.length * 0.4); 
    for (const [w, count] of Object.entries(wordCounts)) {
        if (count >= threshold) noise.add(w);
    }
    return noise;
}

function _getClusterLabel(members, noiseWords) {
    if (members.length < 2) return "";
    const splitWords = (s) => s.split(/(?=[A-Z])|[^a-zA-Z0-9]/).filter(w => w.length > 1).map(w => w.toLowerCase());
    const allMembersWords = members.map(splitWords);

    let commonWords = allMembersWords[0];
    for (let i = 1; i < allMembersWords.length; i++) {
        commonWords = commonWords.filter(w => allMembersWords[i].includes(w));
    }

    const filtered = commonWords.filter(w => !noiseWords.has(w));
    if (filtered.length === 0) return "";

    // Capitalize and join, but keep it short
    return filtered.slice(0, 3).map(w => w.charAt(0).toUpperCase() + w.slice(1)).join(' ');
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
    if (!deps || deps.length === 0) return;

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

    // ── Dagre layout & Clustering ────────────────────────────────────
    const noiseWords = _identifyNoiseWords(allNodes);
    const serviceToClients = new Map();
    deps.forEach(d => {
        if (!serviceToClients.has(d.service)) serviceToClients.set(d.service, new Set());
        serviceToClients.get(d.service).add(d.client);
    });

    const groups = new Map();
    for (const [service, clients] of serviceToClients) {
        const key = Array.from(clients).sort().join('|');
        if (!groups.has(key)) groups.set(key, []);
        groups.get(key).push(service);
    }

    const clusters = [];
    const clusterMap = new Map();
    for (const [key, members] of groups) {
        if (members.length > 1) {
            const label = _getClusterLabel(members, noiseWords);
            const cid = 'cluster_' + clusters.length;
            clusters.push({ id: cid, label, members });
            members.forEach(m => clusterMap.set(m, cid));
        }
    }

    const g = new dagre.graphlib.Graph({ compound: true });
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

    const nodeW = 160, nodeH = 40;
    for (const node of allNodes) {
        g.setNode(node, { label: node, width: nodeW, height: nodeH });
        if (clusterMap.has(node)) g.setParent(node, clusterMap.get(node));
    }
    clusters.forEach(c => {
        g.setNode(c.id, { label: c.label });
    });

    for (const [from, tos] of adjMap) {
        for (const to of tos) g.setEdge(from, to);
    }
    dagre.layout(g);

    // ── Structured layout (math-based compaction) ────────────────────
    // After Dagre has assigned coordinates, we post-process each rank to:
    //  (a) compact cluster members into an overlapping "bricks" grid
    //  (b) re-pack all rank elements (standalone nodes + clusters as
    //      single units) with a uniform gap so empty space is reclaimed.
    // All offsets relative to Dagre output are recorded in node(X)Offsets
    // maps so edges can be interpolated precisely.

    // Axis abstraction: F = along-flow (within a rank), R = rank axis.
    // TB: F=x, R=y, along-flow node extent = nodeW.
    // LR: F=y, R=x, along-flow node extent = nodeH.
    const F = isLR ? 'y' : 'x';
    const R = isLR ? 'x' : 'y';
    const nodeFlowExtent = isLR ? nodeH : nodeW;

    const nodeFOffsets = new Map();  // delta along flow axis
    const nodeROffsets = new Map();  // delta along rank axis (for brick stagger)
    for (const node of allNodes) {
        nodeFOffsets.set(node, 0);
        nodeROffsets.set(node, 0);
    }

    // Step 1 — compute intra-cluster compacted geometry (members become a
    // 2-row brick pattern centered at the cluster's original flow-center).
    // Flow-axis spacings must exceed the node's flow-axis extent, otherwise
    // bricks collide. In LR the flow extent is nodeH (40), so gaps can be
    // tighter than TB (flow extent nodeW=160).
    const GAP = isLR ? 25 : 30;           // gap between units on a rank (flow axis)
    const BRICK_GAP = isLR ? 12 : 15;     // gap between adjacent bricks (flow axis)
    // Brick stagger along the rank axis. In LR this shifts X, and must
    // leave room next to nodeW-wide neighbours — bump it up.
    // In TB the rank axis is Y and nodes are 40px tall, so we need >40 to
    // avoid vertical overlap between staggered brick rows; use 70 for a
    // comfortable 30px clearance.
    // In LR the rank axis is X and nodes are nodeW=160 wide, so the second
    // brick row must be shifted by more than nodeW to clear the first row
    // horizontally. 200 gives a comfortable 40px X-clearance between
    // staggered members. TB keeps 70 (nodeH=40 + 30px gap).
    const BRICK_DR = isLR ? 200 : 70;
    const clusterGeom = new Map(); // cid -> {origCenterF, rankR, width, members}

    clusters.forEach(c => {
        const sortedMembers = [...c.members].sort((a, b) => g.node(a)[F] - g.node(b)[F]);
        const origFs = sortedMembers.map(m => g.node(m)[F]);
        const origCenterF = (Math.min(...origFs) + Math.max(...origFs)) / 2;
        const rankR = g.node(sortedMembers[0])[R]; // all cluster members share a rank

        // In LR (horizontal) mode we skip brick weaving entirely: cluster
        // members stack straight under each other along the flow axis (Y),
        // no rank-axis (X) stagger. This gives a cleaner, readable column
        // per cluster. In TB we keep the 2-row brick pattern to reduce
        // horizontal footprint.
        const stepF = isLR
            ? nodeFlowExtent + BRICK_GAP       // full node + gap, no overlap
            : nodeFlowExtent / 2 + BRICK_GAP;  // half-step for brick overlap
        const n = sortedMembers.length;
        const totalSpan = (n - 1) * stepF;
        const startF = -totalSpan / 2;
        const width = totalSpan + nodeFlowExtent;

        const members = sortedMembers.map((m, i) => ({
            id: m,
            relF: startF + i * stepF,                  // relative to cluster flow-center
            dr: isLR ? 0 : (i % 2) * BRICK_DR,         // no stagger in LR
        }));

        clusterGeom.set(c.id, { origCenterF, rankR, width, members });
    });

    // Step 2 — group layout units by rank and re-pack along the flow axis.
    const unitsByRank = new Map(); // rankR -> [{type, id, cf, width}]
    for (const node of allNodes) {
        if (clusterMap.has(node)) continue;
        const nd = g.node(node);
        const key = Math.round(nd[R]);
        if (!unitsByRank.has(key)) unitsByRank.set(key, []);
        const w = isLR ? nd.height : nd.width;
        unitsByRank.get(key).push({ type: 'node', id: node, cf: nd[F], width: w });
    }
    clusterGeom.forEach((geom, cid) => {
        const key = Math.round(geom.rankR);
        if (!unitsByRank.has(key)) unitsByRank.set(key, []);
        unitsByRank.get(key).push({ type: 'cluster', id: cid, cf: geom.origCenterF, width: geom.width });
    });

    unitsByRank.forEach(units => {
        units.sort((a, b) => a.cf - b.cf);
        const totalWidth = units.reduce((s, u) => s + u.width, 0) + GAP * (units.length - 1);
        const oldMid = (units[0].cf + units[units.length - 1].cf) / 2;
        let cursor = oldMid - totalWidth / 2;
        units.forEach(u => {
            u.newCf = cursor + u.width / 2;
            cursor += u.width + GAP;
        });
    });

    // Step 3 — apply computed positions.
    unitsByRank.forEach(units => {
        units.forEach(u => {
            if (u.type === 'node') {
                const nd = g.node(u.id);
                nodeFOffsets.set(u.id, u.newCf - nd[F]);
            } else {
                const geom = clusterGeom.get(u.id);
                geom.members.forEach(m => {
                    const nd = g.node(m.id);
                    const targetF = u.newCf + m.relF;
                    const targetR = geom.rankR + m.dr;
                    nodeFOffsets.set(m.id, targetF - nd[F]);
                    nodeROffsets.set(m.id, targetR - nd[R]);
                });
                geom.finalCenterF = u.newCf;
            }
        });
    });

    for (const node of allNodes) {
        const nd = g.node(node);
        nd[F] += nodeFOffsets.get(node) || 0;
        nd[R] += nodeROffsets.get(node) || 0;
    }

    // Step 4 — update cluster bounding boxes to enclose their (now moved)
    // members, with padding and space reserved for the label.
    clusters.forEach(c => {
        const nd = g.node(c.id);
        if (!nd) return;
        let minX = Infinity, maxX = -Infinity, minY = Infinity, maxY = -Infinity;
        c.members.forEach(m => {
            const mnd = g.node(m);
            minX = Math.min(minX, mnd.x - mnd.width / 2);
            maxX = Math.max(maxX, mnd.x + mnd.width / 2);
            minY = Math.min(minY, mnd.y - mnd.height / 2);
            maxY = Math.max(maxY, mnd.y + mnd.height / 2);
        });
        const padX = 20;
        const padY = 20;
        const labelPad = c.label ? 20 : 0;
        // Label lives on the rank-axis start side: top in TB, left in LR.
        if (isLR) {
            nd.width = (maxX - minX) + 2 * padX + labelPad;
            nd.height = (maxY - minY) + 2 * padY;
            nd.x = (minX + maxX) / 2 - labelPad / 2;
            nd.y = (minY + maxY) / 2;
        } else {
            nd.width = (maxX - minX) + 2 * padX;
            nd.height = (maxY - minY) + 2 * padY + labelPad;
            nd.x = (minX + maxX) / 2;
            nd.y = (minY + maxY) / 2 + labelPad / 2;
        }
    });

    // Recompute edge endpoints from final node positions. Anchor on the
    // rank-axis border of each node (top/bottom for TB, left/right for LR).
    // Rank-axis node extent: nodeH in TB, nodeW in LR.
    const nodeRankExtent = isLR ? nodeW : nodeH;
    g.edges().forEach(e => {
        const edgeData = g.edge(e);
        const nv = g.node(e.v);
        const nw = g.node(e.w);
        const vIsBefore = nv[R] <= nw[R];
        const p0 = { x: nv.x, y: nv.y };
        const p1 = { x: nw.x, y: nw.y };
        p0[R] += vIsBefore ? nodeRankExtent / 2 : -nodeRankExtent / 2;
        p1[R] += vIsBefore ? -nodeRankExtent / 2 : nodeRankExtent / 2;
        edgeData.points = [p0, p1];
    });

    const graphInfo = g.graph();
    const svgW = graphInfo.width + (isLR ? 160 : 40);
    const svgH = graphInfo.height + (isLR ? 40 : 160); // Extra room for brick staggering

    svgElement.setAttribute('viewBox', `0 0 ${svgW} ${svgH}`);
    svgElement.style.minHeight = Math.min(svgH + 40, 800) + 'px';

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

    // ── Render clusters (first, so they are in background) ────────────
    clusters.forEach(c => {
        const nd = g.node(c.id);
        if (!nd) return;
        const group = document.createElementNS('http://www.w3.org/2000/svg', 'g');
        group.setAttribute('class', 'graph-cluster');
        const rect = document.createElementNS('http://www.w3.org/2000/svg', 'rect');
        rect.setAttribute('x', nd.x - nd.width / 2);
        rect.setAttribute('y', nd.y - nd.height / 2);
        rect.setAttribute('width', nd.width);
        rect.setAttribute('height', nd.height);
        rect.setAttribute('rx', '12'); rect.setAttribute('ry', '12');
        rect.setAttribute('fill', '#f8fafc');
        rect.setAttribute('stroke', '#cbd5e1');
        rect.setAttribute('stroke-width', '2');
        rect.setAttribute('stroke-dasharray', '5 5');
        group.appendChild(rect);
        if (c.label) {
            const text = document.createElementNS('http://www.w3.org/2000/svg', 'text');
            text.setAttribute('fill', '#64748b');
            text.setAttribute('font-size', '10');
            text.setAttribute('font-weight', 'bold');
            if (isLR) {
                // Vertical label on the left edge of the cluster.
                const lx = nd.x - nd.width / 2 + 12;
                const ly = nd.y;
                text.setAttribute('x', lx);
                text.setAttribute('y', ly);
                text.setAttribute('text-anchor', 'middle');
                text.setAttribute('transform', `rotate(-90 ${lx} ${ly})`);
            } else {
                text.setAttribute('x', nd.x);
                text.setAttribute('y', nd.y - nd.height / 2 + 15);
                text.setAttribute('text-anchor', 'middle');
            }
            text.textContent = c.label.toUpperCase() + ' CLUSTER';
            group.appendChild(text);
        }
        mainG.appendChild(group);
    });

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
        rect.setAttribute('x', nd.x - nodeW / 2);
        rect.setAttribute('y', nd.y - nodeH / 2);
        rect.setAttribute('width', nodeW);
        rect.setAttribute('height', nodeH);
        rect.setAttribute('rx', '8');
        rect.setAttribute('ry', '8');
        rect.setAttribute('fill', fill);
        rect.setAttribute('stroke', stroke);
        rect.setAttribute('stroke-width', '2');
        group.appendChild(rect);

        // Double border for "both" nodes
        if (isClient && isService) {
            const inner = document.createElementNS('http://www.w3.org/2000/svg', 'rect');
            inner.setAttribute('x', nd.x - nodeW / 2 + 4);
            inner.setAttribute('y', nd.y - nodeH / 2 + 4);
            inner.setAttribute('width', nodeW - 8);
            inner.setAttribute('height', nodeH - 8);
            inner.setAttribute('rx', '5');
            inner.setAttribute('ry', '5');
            inner.setAttribute('fill', 'none');
            inner.setAttribute('stroke', stroke);
            inner.setAttribute('stroke-width', '1');
            inner.setAttribute('stroke-opacity', '0.5');
            group.appendChild(inner);
        }

        const text = document.createElementNS('http://www.w3.org/2000/svg', 'text');
        text.setAttribute('x', nd.x);
        text.setAttribute('y', nd.y);
        text.setAttribute('text-anchor', 'middle');
        text.setAttribute('dominant-baseline', 'central');
        text.setAttribute('fill', textColor);
        text.setAttribute('font-size', '13');
        text.setAttribute('font-family', 'ui-sans-serif, system-ui, sans-serif');
        text.setAttribute('font-weight', '600');
        // Truncate long names
        const displayName = node.length > 18 ? node.slice(0, 16) + '…' : node;
        text.textContent = displayName;
        if (node.length > 18) {
            const title = document.createElementNS('http://www.w3.org/2000/svg', 'title');
            title.textContent = node;
            group.appendChild(title);
        }
        group.appendChild(text);

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
    let scale = 1, panX = 0, panY = 0;
    let isPanning = false, startX = 0, startY = 0;

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
