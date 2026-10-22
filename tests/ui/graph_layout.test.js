// The graph with many domains and switches: domain frames never overlap and
// hold their own nodes, the Topics switch draws a channel nobody requires,
// and Rainbow gives every source its own colour, the catalog's dependsOn
// lines included.
import { test, expect } from "@playwright/test";

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
  await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });
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
