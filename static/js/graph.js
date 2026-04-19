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

function renderCustomGraph(report, svgElement) {
    // Clear previous content
    while (svgElement.firstChild) svgElement.removeChild(svgElement.firstChild);

    const deps = report.dependency_graph;
    if (!deps || deps.length === 0) return;

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
        rankdir: 'TB', 
        nodesep: 50, 
        ranksep: 180, 
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

    const nodeOffsets = new Map();   // dy per node
    const nodeXOffsets = new Map();  // dx per node
    for (const node of allNodes) {
        nodeOffsets.set(node, 0);
        nodeXOffsets.set(node, 0);
    }

    // Step 1 — compute intra-cluster compacted geometry (members become a
    // 2-row brick pattern centered at the cluster's original center).
    // Also compute each cluster's new effective width.
    const GAP = 30;            // horizontal gap between units on a rank
    const BRICK_GAP_X = 15;    // horizontal gap between adjacent bricks
    const BRICK_DY = 45;       // vertical offset of odd-indexed bricks
    const clusterGeom = new Map(); // cid -> {origCenterX, rankY, width, members:[{id, dx, dy}]}

    clusters.forEach(c => {
        const sortedMembers = [...c.members].sort((a, b) => g.node(a).x - g.node(b).x);
        const origXs = sortedMembers.map(m => g.node(m).x);
        const origCenterX = (Math.min(...origXs) + Math.max(...origXs)) / 2;
        const rankY = g.node(sortedMembers[0]).y; // all cluster members share a rank

        // Two-row brick layout: row 0 at dy=0, row 1 at dy=BRICK_DY, step = nodeW/2 + gap
        const stepX = nodeW / 2 + BRICK_GAP_X;
        const n = sortedMembers.length;
        const totalSpan = (n - 1) * stepX;
        const startX = -totalSpan / 2; // relative to cluster center
        const width = totalSpan + nodeW;

        const members = sortedMembers.map((m, i) => ({
            id: m,
            relX: startX + i * stepX,       // relative to cluster center
            dy: (i % 2) * BRICK_DY,
        }));

        clusterGeom.set(c.id, { origCenterX, rankY, width, members });
    });

    // Step 2 — group all layout units by rank (Y) and re-pack horizontally.
    // A "unit" is either a standalone node or a cluster (as a single block).
    // This reclaims the horizontal space freed by cluster compaction and
    // keeps gaps uniform, so standalone nodes on this rank (and visually
    // on adjacent ranks) look aligned rather than floating.
    const unitsByRank = new Map(); // rankY -> [{type, id, cx, width}]
    // Standalone nodes
    for (const node of allNodes) {
        if (clusterMap.has(node)) continue;
        const nd = g.node(node);
        const key = Math.round(nd.y);
        if (!unitsByRank.has(key)) unitsByRank.set(key, []);
        unitsByRank.get(key).push({ type: 'node', id: node, cx: nd.x, width: nd.width });
    }
    // Clusters
    clusterGeom.forEach((geom, cid) => {
        const key = Math.round(geom.rankY);
        if (!unitsByRank.has(key)) unitsByRank.set(key, []);
        unitsByRank.get(key).push({ type: 'cluster', id: cid, cx: geom.origCenterX, width: geom.width });
    });

    // Pack each rank: sort by original center-X, then place left-to-right
    // keeping the same midpoint (so the whole rank stays centered in the
    // graph and aligned with other ranks).
    unitsByRank.forEach(units => {
        units.sort((a, b) => a.cx - b.cx);
        const totalWidth = units.reduce((s, u) => s + u.width, 0) + GAP * (units.length - 1);
        const oldMid = (units[0].cx + units[units.length - 1].cx) / 2;
        let cursor = oldMid - totalWidth / 2;
        units.forEach(u => {
            const newCx = cursor + u.width / 2;
            u.newCx = newCx;
            cursor += u.width + GAP;
        });
    });

    // Step 3 — apply computed positions to every node.
    unitsByRank.forEach(units => {
        units.forEach(u => {
            if (u.type === 'node') {
                const nd = g.node(u.id);
                nodeXOffsets.set(u.id, u.newCx - nd.x);
            } else {
                const geom = clusterGeom.get(u.id);
                geom.members.forEach(m => {
                    const nd = g.node(m.id);
                    const targetX = u.newCx + m.relX;
                    const targetY = geom.rankY + m.dy;
                    nodeXOffsets.set(m.id, targetX - nd.x);
                    nodeOffsets.set(m.id, targetY - nd.y);
                });
                geom.finalCenterX = u.newCx;
            }
        });
    });

    // Apply offsets to nodes
    for (const node of allNodes) {
        const nd = g.node(node);
        nd.y += nodeOffsets.get(node) || 0;
        nd.x += nodeXOffsets.get(node) || 0;
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
        nd.width = (maxX - minX) + 2 * padX;
        nd.height = (maxY - minY) + 2 * padY + labelPad;
        nd.x = (minX + maxX) / 2;
        nd.y = (minY + maxY) / 2 + labelPad / 2;
    });

    // Recompute edge endpoints from final node positions (ignore Dagre's
    // now-stale intermediate waypoints — they routed around pre-move
    // coordinates and produce chaotic detours after compaction).
    g.edges().forEach(e => {
        const edgeData = g.edge(e);
        const nv = g.node(e.v);
        const nw = g.node(e.w);
        // Anchor on node border (top/bottom edge) for top-to-bottom flow.
        const vIsAbove = nv.y <= nw.y;
        const p0 = { x: nv.x, y: nv.y + (vIsAbove ? nodeH / 2 : -nodeH / 2) };
        const p1 = { x: nw.x, y: nw.y + (vIsAbove ? -nodeH / 2 : nodeH / 2) };
        edgeData.points = [p0, p1];
    });

    const graphInfo = g.graph();
    const svgW = graphInfo.width + 40;
    const svgH = graphInfo.height + 160; // Extra room for staggering

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
            text.setAttribute('x', nd.x);
            text.setAttribute('y', nd.y - nd.height / 2 + 15);
            text.setAttribute('text-anchor', 'middle');
            text.setAttribute('fill', '#64748b');
            text.setAttribute('font-size', '10');
            text.setAttribute('font-weight', 'bold');
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

        // Elegant cubic Bezier routing.
        //  - Normal top→bottom edges: control points pulled vertically so
        //    curves leave/enter nodes perpendicular to their border, giving
        //    a smooth "river" look without random detours.
        //  - Same-rank or reverse edges (|dy| small): use a lateral S-curve
        //    that bows outward so arrows don't cut straight through nodes.
        const dx = p1.x - p0.x;
        const dy = p1.y - p0.y;
        const absDy = Math.abs(dy);
        let d;
        if (absDy < 30) {
            // Sideways / same-rank: bow downward (or upward) via horizontal tangents.
            const bow = Math.max(40, Math.abs(dx) * 0.3);
            const sign = dx >= 0 ? 1 : -1;
            const c1x = p0.x + sign * bow;
            const c2x = p1.x - sign * bow;
            const c1y = p0.y + bow * 0.6;
            const c2y = p1.y + bow * 0.6;
            d = `M ${p0.x} ${p0.y} C ${c1x} ${c1y}, ${c2x} ${c2y}, ${p1.x} ${p1.y}`;
        } else {
            // Vertical flow: tangent strength proportional to vertical distance,
            // clamped so short hops stay gentle and long hops don't overshoot.
            const tension = Math.min(Math.max(absDy * 0.5, 40), 140);
            const dir = dy >= 0 ? 1 : -1;
            const c1 = { x: p0.x, y: p0.y + dir * tension };
            const c2 = { x: p1.x, y: p1.y - dir * tension };
            d = `M ${p0.x} ${p0.y} C ${c1.x} ${c1.y}, ${c2.x} ${c2.y}, ${p1.x} ${p1.y}`;
        }

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
