// The graph with many domains and switches: domain frames never overlap and
// hold their own nodes, the Topics switch draws a channel nobody requires,
// and Rainbow gives every source its own colour, the catalog's dependsOn
// lines included.
import { test, expect } from "./fixtures.js";

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;
const P = "g10";
const DOMAINS = { alpha: [`${P}-a1`, `${P}-a2`], beta: [`${P}-b1`, `${P}-b2`] };
const CONSUMERS = [`${P}-c1`, `${P}-c2`];
const LONELY = `${P}-lonely`;
const CHANNEL = `${P}.lonely.events`;

const openapi = (title) =>
  [
    "openapi: 3.0.3",
    "info:",
    `  title: ${title}`,
    "  version: 1.0.0",
    "paths:",
    "  /thing:",
    "    get:",
    "      responses:",
    "        '200':",
    "          description: OK",
  ].join("\n");

test.beforeAll(async ({ request }) => {
  if (!adminPassword) throw new Error("INITIAL_ADMIN_PASSWORD is required for the UI tests");
  const login = await request.post("/auth/login", {
    data: { username: "root", password: adminPassword },
  });
  const headers = { Authorization: `Bearer ${(await login.json()).token}` };
  const ok = async (res, what) => {
    if (!res.ok()) throw new Error(`${what} failed: ${res.status()} ${await res.text()}`);
  };
  for (const [domain, producers] of Object.entries(DOMAINS)) {
    for (const name of producers) {
      await ok(
        await request.post("/provide", {
          headers,
          data: { producername: name, stability: "snapshot", openapi_yaml: openapi(name) },
        }),
        `provide ${name}`,
      );
      await ok(
        await request.post("/admin/producers/metadata", {
          headers,
          data: { name, domain },
        }),
        `domain of ${name}`,
      );
    }
  }
  // Each consumer requires one producer of each domain, so dagre interleaves
  // the domains on the producers' rank.
  const pairs = [
    [CONSUMERS[0], DOMAINS.alpha[0]],
    [CONSUMERS[0], DOMAINS.beta[0]],
    [CONSUMERS[1], DOMAINS.alpha[1]],
    [CONSUMERS[1], DOMAINS.beta[1]],
  ];
  for (const [consumer, producer] of pairs) {
    await ok(
      await request.get("/require", {
        headers,
        params: {
          consumername: consumer,
          producername: producer,
          version: "1.0.0",
          path: "/thing",
          method: "GET",
        },
      }),
      `require ${consumer} -> ${producer}`,
    );
  }
  await ok(
    await request.put(`/catalog/${CONSUMERS[0]}`, {
      headers,
      data: {
        catalog_info_yaml: [
          "apiVersion: backstage.io/v1alpha1",
          "kind: Component",
          "metadata:",
          `  name: ${CONSUMERS[0]}`,
          "spec:",
          "  type: service",
          "  dependsOn:",
          `    - resource:${P}-db`,
        ].join("\n"),
      },
    }),
    "catalog push",
  );
  await ok(
    await request.post("/provide/asyncapi", {
      headers,
      data: {
        producername: LONELY,
        stability: "snapshot",
        asyncapi_yaml: [
          "asyncapi: 2.6.0",
          "info:",
          `  title: ${LONELY}`,
          "  version: 1.0.0",
          "channels:",
          `  ${CHANNEL}:`,
          "    publish:",
          "      message:",
          "        name: Lonely",
          "        payload:",
          "          type: object",
        ].join("\n"),
      },
    }),
    "provide the lonely publisher",
  );
});

const openGraph = async (page) => {
  await page.route("**/api/sse/updates", (route) =>
    route.fulfill({ status: 200, contentType: "text/event-stream", body: "" }),
  );
  await page.goto("/account.html");
  await page.evaluate(() => localStorage.clear());
  await page.reload();
  await page.fill("#login-username", "root");
  await page.fill("#login-password", adminPassword);
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#dashboard")).toBeVisible({ timeout: 10000 });
  await page.goto("/graph.html");
  await expect(page.locator("#custom-graph")).toBeVisible({ timeout: 10000 });
};
const node = (page, name) => page.locator(`#custom-graph g.graph-node[data-node="${name}"]`);

test("domain frames do not overlap and hold their own nodes", async ({ page }) => {
  await openGraph(page);
  await expect(node(page, DOMAINS.beta[1])).toHaveCount(1, { timeout: 20000 });
  const layout = await page.evaluate(() => {
    const rect = (el) => el.getBoundingClientRect();
    const frames = {};
    document.querySelectorAll("#custom-graph rect[data-domain]").forEach((el) => {
      const r = rect(el);
      frames[el.dataset.domain] = { x0: r.left, y0: r.top, x1: r.right, y1: r.bottom };
    });
    const nodes = {};
    document.querySelectorAll("#custom-graph g.graph-node").forEach((el) => {
      const r = rect(el);
      nodes[el.dataset.node] = { x0: r.left, y0: r.top, x1: r.right, y1: r.bottom };
    });
    return { frames, nodes };
  });
  const { alpha, beta } = layout.frames;
  expect(alpha && beta).toBeTruthy();
  const overlap = (a, b) => a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1;
  expect(overlap(alpha, beta)).toBe(false);
  const inside = (n, f) => n.x0 >= f.x0 && n.x1 <= f.x1 && n.y0 >= f.y0 && n.y1 <= f.y1;
  for (const [domain, producers] of Object.entries(DOMAINS)) {
    const frame = layout.frames[domain];
    for (const name of producers) expect(inside(layout.nodes[name], frame), name).toBe(true);
    const other = layout.frames[domain === "alpha" ? "beta" : "alpha"];
    for (const name of producers) expect(overlap(layout.nodes[name], other), name).toBe(false);
  }
});

test("a domain frame and its name are drawn in the domain's own colour", async ({ page }) => {
  await openGraph(page);
  await expect(node(page, DOMAINS.beta[1])).toHaveCount(1, { timeout: 20000 });
  const drawn = await page.evaluate(() => {
    const out = {};
    for (const domain of ["alpha", "beta"]) {
      const frame = document.querySelector(`#custom-graph rect[data-domain="${domain}"]`);
      const label = document.querySelector(`#custom-graph text[data-domain-label="${domain}"]`);
      out[domain] = {
        stroke: frame.getAttribute("stroke"),
        fill: frame.getAttribute("fill"),
        dashed: frame.hasAttribute("stroke-dasharray"),
        text: label.getAttribute("fill"),
        size: label.getAttribute("font-size"),
        expected: window.graphDomainColor(domain),
      };
    }
    // The same name always gets the same colour, and no warning colour is used.
    const again = window.graphDomainColor("alpha").border === out.alpha.expected.border;
    const warnings = ["#9333ea", "#dc2626", "#ea580c", "#d97706"];
    const clean = window.GRAPH_DOMAIN_PALETTE.every((c) => !warnings.includes(c.border));
    return { out, again, clean };
  });
  expect(drawn.again).toBe(true);
  expect(drawn.clean).toBe(true);
  for (const d of Object.values(drawn.out)) {
    expect(d.stroke).toBe(d.expected.border);
    expect(d.fill).toBe(d.expected.fill);
    expect(d.text).toBe(d.expected.text);
    expect(d.dashed).toBe(false);
    expect(d.size).toBe("12");
  }
});

test("ungrouped, the domain is written on the node and no frame is drawn", async ({ page }) => {
  await openGraph(page);
  await expect(node(page, DOMAINS.beta[1])).toHaveCount(1, { timeout: 20000 });
  await expect(page.locator("#graph-domains-toggle")).toHaveAttribute("aria-pressed", "true");
  await page.click("#graph-domains-toggle");
  await expect(page.locator("#graph-domains-toggle")).toHaveAttribute("aria-pressed", "false");
  await expect(page.locator("#custom-graph rect[data-domain]")).toHaveCount(0);
  const a1 = node(page, DOMAINS.alpha[0]);
  await expect(a1).toHaveAttribute("data-node-domain", "alpha");
  await expect(a1.locator("text[data-domain-line]")).toHaveText("alpha");
  const edge = await a1
    .locator("rect[data-domain-edge]")
    .evaluate((el) => [el.getAttribute("fill"), window.graphDomainColor("alpha").border]);
  expect(edge[0]).toBe(edge[1]);
  // A service without a domain carries no domain line.
  await expect(node(page, LONELY).locator("text[data-domain-line]")).toHaveCount(0);
  await expect.poll(() => new URL(page.url()).searchParams.get("domains")).toBe("node");
  // Remembered: a plain visit stays ungrouped; a link that says group wins.
  await page.goto("/graph.html");
  await expect(node(page, DOMAINS.beta[1])).toHaveCount(1, { timeout: 20000 });
  await expect(page.locator("#custom-graph rect[data-domain]")).toHaveCount(0);
  await page.goto("/graph.html?domains=group");
  await expect(page.locator('#custom-graph rect[data-domain="alpha"]')).toHaveCount(1, {
    timeout: 20000,
  });
});

test("the Topics switch draws a channel nobody requires", async ({ page }) => {
  await openGraph(page);
  await expect(page.locator("#graph-unrequired-box")).toContainText(LONELY, { timeout: 20000 });
  await page.click("#graph-topics-toggle");
  const topic = node(page, `topic:${CHANNEL}`);
  await expect(topic).toHaveAttribute("data-role", "topic");
  await expect(
    page.locator(
      `#custom-graph path.graph-edge[data-from="${LONELY}"][data-to="topic:${CHANNEL}"]`,
    ),
  ).toHaveCount(1);
  await expect(node(page, LONELY)).toHaveCount(1);
  await expect(page.locator("#graph-unrequired-box", { hasText: LONELY })).toHaveCount(0);
});

test("Rainbow gives every source its own colour, catalog lines included", async ({ page }) => {
  await openGraph(page);
  await expect(node(page, CONSUMERS[0])).toHaveCount(1, { timeout: 20000 });
  await page.click("#graph-rainbow-toggle");
  await expect(page.locator("#custom-graph")).toHaveAttribute("data-rainbow", "1");
  const edges = await page.evaluate(() =>
    [...document.querySelectorAll("#custom-graph path.graph-edge")].map((p) => ({
      from: p.dataset.from,
      type: p.dataset.edgeType,
      stroke: p.getAttribute("stroke"),
      dash: p.getAttribute("stroke-dasharray"),
    })),
  );
  const coloured = edges.filter((e) => e.type === "normal" || e.type === "peripheral");
  const bySource = new Map();
  for (const e of coloured) {
    if (bySource.has(e.from)) expect(bySource.get(e.from)).toBe(e.stroke);
    bySource.set(e.from, e.stroke);
  }
  // Every source its own colour.
  expect(new Set(bySource.values()).size).toBe(bySource.size);
  const catalog = edges.find((e) => e.type === "peripheral" && e.from === CONSUMERS[0]);
  expect(catalog.stroke).toBe(bySource.get(CONSUMERS[0]));
  expect(catalog.stroke).not.toBe("#64748b");
  expect(catalog.dash).toBe("4 3");
});

test("hotkeys flip the switches, list themselves and stay out of fields", async ({ page }) => {
  await openGraph(page);
  await expect(node(page, DOMAINS.beta[1])).toHaveCount(1, { timeout: 20000 });
  await page.locator("body").press("d");
  await expect(page.locator("#graph-density-label")).toHaveText("Compact");
  await page.locator("body").press("g");
  await expect(page.locator("#graph-domains-toggle")).toHaveAttribute("aria-pressed", "false");
  await page.locator("body").press("r");
  await expect(page.locator("#graph-rainbow-toggle")).toHaveAttribute("aria-pressed", "true");
  await page.locator("body").press("t");
  await expect(page.locator("#graph-topics-toggle")).toHaveAttribute("aria-pressed", "true");
  await page.locator("body").press("v");
  await expect(page.locator("#graph-direction-label")).toHaveText("Horizontal");
  await page.locator("body").press("l");
  await expect(page.locator("#graph-legend")).toBeHidden();
  await page.locator("body").press("l");
  await expect(page.locator("#graph-legend")).toBeVisible();

  // ? lists the keys; Esc closes the list.
  await page.locator("body").press("?");
  const overlay = page.locator("#graph-keys-overlay");
  await expect(overlay).toBeVisible();
  await expect(overlay).toContainText("Density: Normal, Compact, Dense");
  await page.locator("body").press("Escape");
  await expect(overlay).toBeHidden();

  // Typing in a field is typing, not a hotkey.
  await page.locator("#graph-service-filter").press("d");
  await expect(page.locator("#graph-density-label")).toHaveText("Compact");

  // [ steps the timeline back to its left edge when nothing changed in between.
  await page.goto("/graph.html?view=main&from=2020-01-01&to=2020-01-31");
  await expect(page.locator("#graph-timeline-label")).toHaveText("2020-01-31 23:59", {
    timeout: 20000,
  });
  await page.locator("body").press("[");
  await expect(page.locator("#graph-timeline-label")).toHaveText("2020-01-01 00:00");
  await page.locator("body").press("]");
  await expect(page.locator("#graph-timeline-label")).toHaveText("2020-01-31 23:59");
});

test("fullscreen hides the page, keeps the controls beside the graph and steps back", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await openGraph(page);
  await expect(node(page, DOMAINS.beta[1])).toHaveCount(1, { timeout: 20000 });
  const box = (sel) => page.locator(sel).evaluate((el) => el.getBoundingClientRect().toJSON());

  await page.locator("body").press("f");
  await expect(page.locator("body")).toHaveAttribute("data-graph-fs", "1");
  await expect(page.locator("#site-banner")).toBeHidden();
  await expect(page.locator("#graph-toolbar-row [data-fs-panel='focus']")).toBeHidden();
  const rail = page.locator("#graph-fs-rail");
  await expect(rail).toBeVisible();
  const railBox = await box("#graph-fs-rail");
  const legendBox = await box("#graph-legend");
  let graph = await box("#graph-container");
  expect(graph.top).toBe(0);
  expect(Math.round(graph.left)).toBe(Math.round(railBox.width));
  expect(Math.round(graph.right)).toBe(Math.round(legendBox.left));
  // The canvas fills the height it is given.
  const svg = await box("#custom-graph");
  expect(svg.height).toBeGreaterThan(600);
  await page.screenshot({ path: "test-results/fullscreen.png" });

  // An icon opens its panel beside the column; Esc closes it first.
  await page.click("[data-fs-icon='switches']");
  await expect(page.locator("[data-fs-panel='switches']")).toBeVisible();
  await expect(page.locator("#graph-density-toggle")).toBeVisible();
  await page.click("[data-fs-icon='focus']");
  await expect(page.locator("[data-fs-panel='switches']")).toBeHidden();
  await expect(page.locator("[data-fs-panel='focus']")).toBeVisible();
  await page.screenshot({ path: "test-results/fullscreen-panel.png" });
  await page.locator("body").press("Escape");
  await expect(page.locator("[data-fs-panel='focus']")).toBeHidden();
  await expect(page.locator("body")).toHaveAttribute("data-graph-fs", "1");

  // c and l give their space to the graph.
  await page.locator("body").press("c");
  await expect(rail).toBeHidden();
  await page.locator("body").press("l");
  await expect(page.locator("#graph-legend")).toBeHidden();
  await expect.poll(async () => (await box("#graph-container")).left).toBe(0);
  graph = await box("#graph-container");
  expect(Math.round(graph.right)).toBe(1280);
  await page.locator("body").press("c");
  await page.locator("body").press("l");

  // Esc leaves the window level: the page is back.
  await page.locator("body").press("Escape");
  await expect(page.locator("body")).not.toHaveAttribute("data-graph-fs", /./);
  await expect(page.locator("#site-banner")).toBeVisible();
  await expect(rail).toBeHidden();
  await expect(page.locator("#graph-toolbar-row [data-fs-panel='focus']")).toBeVisible();
});

test("b hides the timeline bar, in fullscreen its space goes to the graph", async ({ page }) => {
  await openGraph(page);
  await page.goto("/graph.html?view=main");
  const row = page.locator("#graph-timeline-row");
  await expect(row).toBeVisible({ timeout: 20000 });
  await page.locator("body").press("f");
  const bottom = () =>
    page.locator("#graph-container").evaluate((el) => el.getBoundingClientRect().bottom);
  const withBar = await bottom();
  await page.locator("body").press("b");
  await expect(row).toBeHidden();
  await expect.poll(bottom).toBeGreaterThan(withBar);
  await page.locator("body").press("b");
  await expect(row).toBeVisible();
});

test("z collapses the graph onto domains, counting and colouring the pins", async ({ page }) => {
  await openGraph(page);
  await expect(node(page, DOMAINS.beta[1])).toHaveCount(1, { timeout: 20000 });

  // The collapse itself, on a report of its own: two pins from a lone service
  // into one domain make one arrow of 2, coloured by the worse pin; a pin
  // inside a domain is counted on it.
  const collapsed = await page.evaluate(() => {
    const report = {
      services_detailed: [
        { name: "a", domain: "pay" },
        { name: "b", domain: "pay" },
        { name: "x", domain: "ship" },
      ],
      dependency_graph: [
        { client: "lone", service: "a" },
        { client: "lone", service: "b" },
        { client: "a", service: "b" },
        { client: "x", service: "a" },
      ],
      missing_endpoints: [{ client: "lone", service: "b" }],
    };
    const flags = new Map([["a-->b", { outdated: true }]]);
    // Outdated colours only while its highlight is on.
    const off = window
      .graphCollapseDomains(report, flags, {})
      .units.find((u) => u.id === "domain:pay").internalLevel;
    const { units, edges } = window.graphCollapseDomains(report, flags, { outdated: true });
    return {
      off,
      units: units.map((u) => [u.id, u.internal, u.internalLevel]).sort(),
      edges: edges.map((e) => [e.from, e.to, e.count, e.level]).sort(),
    };
  });
  expect(collapsed.off).toBe(0);
  expect(collapsed.units).toEqual([
    ["domain:pay", 1, 3],
    ["domain:ship", 0, 0],
    ["lone", 0, 0],
  ]);
  expect(collapsed.edges).toEqual([
    ["domain:ship", "domain:pay", 1, 0],
    ["lone", "domain:pay", 2, 6],
  ]);

  await page.locator("body").press("z");
  const svg = page.locator("#custom-graph");
  await expect(svg).toHaveAttribute("data-level", "domains");
  await expect.poll(() => new URL(page.url()).searchParams.get("level")).toBe("domains");
  const alpha = page.locator('#custom-graph g[data-unit="domain:alpha"]');
  await expect(alpha).toHaveCount(1);
  await expect(page.locator(`#custom-graph g[data-unit="${CONSUMERS[0]}"]`)).toHaveCount(1);
  await expect(
    page.locator(`#custom-graph path[data-from="${CONSUMERS[0]}"][data-to="domain:alpha"]`),
  ).toHaveAttribute("data-count", "1");

  // A domain node opens the full graph focused on that domain.
  await alpha.click();
  await expect(svg).toHaveAttribute("data-level", "services");
  await expect.poll(() => new URL(page.url()).searchParams.get("focus")).toBe("domain:alpha");
  expect(new URL(page.url()).searchParams.get("level")).toBeNull();
  await expect(node(page, DOMAINS.alpha[0])).toHaveCount(1);
  await expect(node(page, DOMAINS.beta[0])).toHaveCount(0);
});

test("redrawing the graph does not pile up window listeners", async ({ page }) => {
  await openGraph(page);
  await expect(node(page, DOMAINS.beta[1])).toHaveCount(1, { timeout: 20000 });
  const cdp = await page.context().newCDPSession(page);
  const windowListeners = async (type) => {
    const { result } = await cdp.send("Runtime.evaluate", { expression: "window" });
    const { listeners } = await cdp.send("DOMDebugger.getEventListeners", {
      objectId: result.objectId,
    });
    return listeners.filter((l) => l.type === type).length;
  };
  const before = [await windowListeners("mousemove"), await windowListeners("mouseup")];
  // Each switch redraws; the domain view and back take the pan off and on.
  for (const key of ["r", "r", "d", "d", "d", "z", "z"]) {
    await page.locator("body").press(key);
  }
  await expect(node(page, DOMAINS.beta[1])).toHaveCount(1);
  expect([await windowListeners("mousemove"), await windowListeners("mouseup")]).toEqual(before);
  // Panning still works after the redraws.
  const transform = () => page.locator("#custom-graph > g").first().getAttribute("transform");
  const start = await transform();
  const box = await page.locator("#custom-graph").boundingBox();
  await page.mouse.move(box.x + 50, box.y + 50);
  await page.mouse.down();
  await page.mouse.move(box.x + 150, box.y + 120);
  await page.mouse.up();
  expect(await transform()).not.toBe(start);
});

test("a failed graph load shows its message as text, never as markup", async ({ page }) => {
  // A body that is no JSON: the parse error quotes it.
  await page.route("**/report", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: '<img src=x id="injected">',
    }),
  );
  await page.route("**/api/sse/updates", (route) =>
    route.fulfill({ status: 200, contentType: "text/event-stream", body: "" }),
  );
  await page.goto("/account.html");
  await page.evaluate(() => localStorage.clear());
  await page.reload();
  await page.fill("#login-username", "root");
  await page.fill("#login-password", adminPassword);
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#dashboard")).toBeVisible({ timeout: 10000 });
  await page.goto("/graph.html");
  const shown = page.locator("#mermaid-graph .text-red-500");
  await expect(shown).toContainText("Error:", { timeout: 10000 });
  await expect(shown).toContainText("<img");
  await expect(page.locator("#mermaid-graph img")).toHaveCount(0);
});

test("a value escaped for the graph panel cannot leave its attribute", async ({ page }) => {
  await openGraph(page);
  const result = await page.evaluate(() => {
    const path = `/orders" onmouseover="alert(1)' x='<b>&`;
    const holder = document.createElement("div");
    holder.innerHTML = `<div title="${window._graphEscapeHtml(path)}">${window._graphEscapeHtml(path)}</div>`;
    const element = holder.firstElementChild;
    return {
      attributes: element.getAttributeNames(),
      title: element.getAttribute("title"),
      text: element.textContent,
    };
  });
  const path = `/orders" onmouseover="alert(1)' x='<b>&`;
  expect(result).toEqual({ attributes: ["title"], title: path, text: path });
});
