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

    // ── Dagre layout ─────────────────────────────────────────────────
    const g = new dagre.graphlib.Graph();
    g.setGraph({ rankdir: 'TB', nodesep: 50, ranksep: 70, edgesep: 25, marginx: 20, marginy: 20 });
    g.setDefaultEdgeLabel(() => ({}));

    const nodeW = 160, nodeH = 40;
    for (const node of allNodes) {
        g.setNode(node, { label: node, width: nodeW, height: nodeH });
    }
    for (const [from, tos] of adjMap) {
        for (const to of tos) g.setEdge(from, to);
    }
    dagre.layout(g);

    const graphInfo = g.graph();
    const svgW = graphInfo.width + 40;
    const svgH = graphInfo.height + 40;

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

    // ── Render edges ─────────────────────────────────────────────────
    const edgeElements = new Map(); // "from-->to" -> { path, hitArea }

    g.edges().forEach(e => {
        const edgeData = g.edge(e);
        const key = `${e.v}-->${e.w}`;
        const isCycle = cycleEdges.has(key);
        const points = edgeData.points;

        // Build path string
        let d = `M ${points[0].x} ${points[0].y}`;
        if (points.length === 2) {
            d += ` L ${points[1].x} ${points[1].y}`;
        } else {
            for (let i = 1; i < points.length; i++) {
                d += ` L ${points[i].x} ${points[i].y}`;
            }
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

        // Zoom toward cursor
        const rect = svgElement.getBoundingClientRect();
        const mx = e.clientX - rect.left;
        const my = e.clientY - rect.top;
        panX = mx - (mx - panX) * (newScale / scale);
        panY = my - (my - panY) * (newScale / scale);
        scale = newScale;
        updateTransform();
    }, { passive: false });

    svgElement.addEventListener('mousedown', e => {
        if (e.button !== 0) return;
        isPanning = true;
        startX = e.clientX - panX;
        startY = e.clientY - panY;
        svgElement.style.cursor = 'grabbing';
    });

    window.addEventListener('mousemove', e => {
        if (!isPanning) return;
        panX = e.clientX - startX;
        panY = e.clientY - startY;
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
