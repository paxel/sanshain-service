import { test, expect } from "@playwright/test";

// Link crawler — the net the two 2.2 navigation bugs slipped through.
//
// The de-inlining audit checks the DOM at page load, but both real breakages
// lived one step further: an endpoint card rendered only after drilling into a
// version carried unparseable action args, and the graph tooltip linked every
// node to the producers page whether or not the node was a Producer. Neither is
// visible on a freshly loaded page.
//
// So this test walks the UI the way a user does: seed real data, log in,
// collect every same-origin link every page renders (drill-down deep links
// included), visit them all, and require of each destination that
//   - the URL carries no "undefined" or "null" query value (an interpolation
//     that lost its variable),
//   - the page throws no script fault and the dispatcher logs no dead handler
//     or unparseable args,
//   - no fetch-error panel is rendered,
//   - every data-*-args attribute anywhere in the resulting DOM parses.
//
// It asserts nothing about layout or content — only that navigation works.

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

const PRODUCER = "crawl-producer";
const CONSUMER = "crawl-consumer";
const SPEC = [
  "openapi: 3.0.0",
  "info:",
  "  title: Crawl Fixture",
  "  version: 1.0.0",
  "paths:",
  "  /crawl/things:",
  "    get:",
  "      responses:",
  "        '200':",
  "          description: OK",
  "",
].join("\n");

// Entry points: every page in the nav, plus the deep links the UI builds
// dynamically — seeded above so they resolve. yaml.html and the drilled
// producers views are exactly where the endpoint-card bug lived.
const SEEDS = [
  "/",
  "/dashboard",
  "/account.html",
  "/admin.html",
  "/producers.html",
  `/producers.html?service=${PRODUCER}`,
  `/producers.html?service=${PRODUCER}&api_type=openapi&version=1.0.0`,
  "/consumers.html",
  `/consumers.html?name=${CONSUMER}`,
  "/graph.html",
  "/reports.html",
  "/audit.html",
  "/observability.html",
  `/yaml.html?service=${PRODUCER}&api_type=openapi&version=1.0.0&path=${encodeURIComponent("/crawl/things")}&method=GET`,
];

// Script faults plus the dispatcher's own diagnostics: a dead data-action or
// args that fail to parse are logged, not thrown, so the log is the signal.
const FAULT =
  /refused to (execute|apply|load)|content security policy|is not defined|referenceerror|typeerror|syntaxerror|is not a function|resolves to no function|args are not valid JSON/i;

// Pages behind the crawl cap still get seen eventually via other runs; the cap
// only bounds runtime against a page that generates unbounded links.
const MAX_PAGES = 80;

function normalize(href, base) {
  const url = new URL(href, base);
  if (url.origin !== new URL(base).origin) return null;
  // Logout tears the session down for every page after it; export downloads
  // navigate nowhere.
  if (url.pathname.startsWith("/auth/") || url.pathname.startsWith("/export")) return null;
  url.hash = "";
  return url.pathname + url.search;
}

test.describe("Link crawl", () => {
  test.beforeAll(async ({ request }) => {
    if (!adminPassword) {
      throw new Error("INITIAL_ADMIN_PASSWORD is required for the UI safety-net tests");
    }
    const login = await request.post("/auth/login", {
      data: { username: "root", password: adminPassword },
    });
    const { token } = await login.json();
    const auth = { Authorization: `Bearer ${token}` };
    await request.post("/provide", {
      headers: auth,
      data: { producername: PRODUCER, stability: "ga", openapi_yaml: SPEC },
    });
    await request.get("/require", {
      headers: auth,
      params: {
        consumername: CONSUMER,
        producername: PRODUCER,
        version: "1.0.0",
        path: "/crawl/things",
        method: "GET",
      },
    });
  });

  test("every reachable link leads to a working page", async ({ page }) => {
    test.setTimeout(300000);
    page.on("dialog", async (d) => await d.dismiss());

    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill("#login-username", "root");
    await page.fill("#login-password", adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });

    const faults = [];
    page.on("pageerror", (e) => faults.push(`${page.url()} pageerror: ${e.message}`));
    page.on("console", (msg) => {
      if (msg.type() === "error" && FAULT.test(msg.text())) {
        faults.push(`${page.url()} console: ${msg.text()}`);
      }
    });

    const queue = [...SEEDS];
    const visited = new Set();

    while (queue.length > 0 && visited.size < MAX_PAGES) {
      const target = queue.shift();
      if (visited.has(target)) continue;
      visited.add(target);

      await page.goto(target);
      await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
      await page.waitForTimeout(500);

      // An interpolation that lost its variable produces a literal
      // "undefined"/"null" — in this page's own URL or in any link it renders.
      const url = new URL(page.url());
      for (const [key, value] of url.searchParams) {
        expect(value, `${target}: query param '${key}' is the string '${value}'`).not.toMatch(
          /^(undefined|null)$/,
        );
      }

      // The uniform failure panel both real bugs rendered.
      const errorPanel = await page
        .locator("text=/Fetch error: \\d+/")
        .count()
        .catch(() => 0);
      expect(errorPanel, `${target} renders a fetch-error panel`).toBe(0);

      // Every action-args attribute in the final DOM must parse — this is the
      // page-load audit repeated where it matters: after dynamic rendering.
      const badArgs = await page.evaluate(() => {
        const bad = [];
        for (const el of document.querySelectorAll("*")) {
          for (const attr of el.attributes) {
            if (!/^data-(click|change|input|submit|keydown)-args$/.test(attr.name)) continue;
            try {
              if (!Array.isArray(JSON.parse(attr.value))) {
                bad.push(`${attr.name} is not an array: ${attr.value}`);
              }
            } catch {
              bad.push(`${attr.name} is not valid JSON: ${attr.value}`);
            }
          }
        }
        return bad;
      });
      expect(badArgs, `${target} has unparseable action args:\n${badArgs.join("\n")}`).toEqual([]);

      // Some links exist only while hovering — the graph's node tooltip
      // builds its anchor on mouseenter, which is exactly where the
      // consumer-linked-as-producer bug lived. Hover every node so those
      // anchors are in the DOM when links are collected.
      const nodes = page.locator("[data-node]");
      const nodeCount = Math.min(await nodes.count(), 15);
      for (let i = 0; i < nodeCount; i++) {
        await nodes
          .nth(i)
          .hover({ timeout: 2000 })
          .catch(() => {});
        await page.waitForTimeout(150);
        for (const href of await page
          .locator("a[href]")
          .evaluateAll((els) => els.map((a) => a.getAttribute("href")))) {
          if (!href || href.startsWith("#")) continue;
          const next = normalize(href, url.origin);
          if (next && !visited.has(next)) queue.push(next);
        }
      }

      // Collect where this page can take the user next.
      const hrefs = await page
        .locator("a[href]")
        .evaluateAll((els) => els.map((a) => a.getAttribute("href")));
      for (const href of hrefs) {
        if (!href || href.startsWith("#")) continue;
        const next = normalize(href, url.origin);
        if (next && !visited.has(next)) queue.push(next);
      }
    }

    expect(faults, `script faults during the crawl:\n${faults.join("\n")}`).toEqual([]);
    // The crawl must have actually gone somewhere beyond its seeds.
    expect(visited.size).toBeGreaterThan(SEEDS.length);
  });
});
