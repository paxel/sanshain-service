// The endpoint lists of the producer and consumer views: ordered by path,
// then method; a search box over path, method and summary; Used / Unused /
// Deprecated chips; the filter kept in the URL, so Back from an endpoint
// returns to the filtered list.
import { test, expect } from "@playwright/test";

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;
const stamp = Date.now().toString(36);
const PRODUCER = `filter-shop-${stamp}`;
const CONSUMER = `filter-app-${stamp}`;

const SPEC = [
  "openapi: 3.0.0",
  "info:",
  "  title: shop",
  "  version: 1.0.0",
  "paths:",
  "  /refunds:",
  "    get:",
  "      summary: Pay money back",
  "      responses:",
  "        '200': { description: ok }",
  "  /carts/{id}:",
  "    delete:",
  "      summary: Drop a cart",
  "      deprecated: true",
  "      parameters:",
  "        - { name: id, in: path, required: true, schema: { type: string } }",
  "      responses:",
  "        '204': { description: gone }",
  "  /carts:",
  "    post:",
  "      summary: Open a cart",
  "      responses:",
  "        '201': { description: created }",
  "    get:",
  "      summary: List carts",
  "      responses:",
  "        '200': { description: ok }",
].join("\n");

test.beforeAll(async ({ request }) => {
  if (!adminPassword) throw new Error("INITIAL_ADMIN_PASSWORD is required for the UI tests");
  const login = await request.post("/auth/login", {
    data: { username: "root", password: adminPassword },
  });
  const { token } = await login.json();
  const headers = { Authorization: `Bearer ${token}` };
  const res = await request.post("/provide", {
    headers,
    data: { producername: PRODUCER, stability: "ga", openapi_yaml: SPEC },
  });
  expect(res.ok(), `provide: ${await res.text()}`).toBeTruthy();
  for (const [path, method] of [
    ["/carts", "GET"],
    ["/carts/{id}", "DELETE"],
  ]) {
    const req = await request.get("/require", {
      headers,
      params: { consumername: CONSUMER, producername: PRODUCER, version: "1.0.0", path, method },
    });
    expect(req.ok(), `require ${method} ${path}: ${await req.text()}`).toBeTruthy();
  }
});

test.beforeEach(async ({ page }) => {
  await page.route("**/api/sse/updates", (route) =>
    route.fulfill({ status: 200, contentType: "text/event-stream", body: "" }),
  );
  await page.goto("/account.html");
  await page.waitForSelector("#login-username", { state: "visible" });
  await page.fill("#login-username", "root");
  await page.fill("#login-password", adminPassword);
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });
});

// "METHOD /path" of every visible endpoint card, in page order.
const visibleEndpoints = (page, list) =>
  page.locator(`${list} .endpoint-card:visible`).evaluateAll((cards) =>
    cards.map((card) => {
      const spans = card.querySelectorAll("span.font-mono, span.uppercase");
      const method = [...spans].find(
        (s) => /^[A-Z]+$/.test(s.textContent.trim()) && s.textContent.trim() !== "OPENAPI",
      );
      const path = card.querySelector("span.font-mono");
      return `${method ? method.textContent.trim() : "?"} ${path.textContent.trim()}`;
    }),
  );

test("the producer view orders endpoints and filters them by text and chips", async ({ page }) => {
  await page.goto(`/producers.html?service=${PRODUCER}&api_type=openapi&version=1.0.0`);
  const list = "#services-list";
  await expect(page.locator(`${list} .endpoint-card`)).toHaveCount(4);
  expect(await visibleEndpoints(page, list)).toEqual([
    "GET /carts",
    "POST /carts",
    "DELETE /carts/{id}",
    "GET /refunds",
  ]);

  const box = page.locator("[data-endpoint-filter-q]");
  await box.fill("money");
  expect(await visibleEndpoints(page, list)).toEqual(["GET /refunds"]);
  await expect(page.locator("[data-endpoint-filter-count]")).toHaveText("· 1 of 4 shown");
  await box.fill("post cart");
  expect(await visibleEndpoints(page, list)).toEqual(["POST /carts"]);
  expect(new URL(page.url()).searchParams.get("q")).toBe("post cart");
  await box.fill("");

  await page.click('[data-endpoint-filter-chip="unused"]');
  expect(await visibleEndpoints(page, list)).toEqual(["POST /carts", "GET /refunds"]);
  await page.click('[data-endpoint-filter-chip="deprecated"]');
  await expect(page.locator("[data-endpoint-filter-empty]")).toBeVisible();
  await page.click("[data-endpoint-filter-clear]");
  await expect(page.locator("[data-endpoint-filter-empty]")).toBeHidden();
  expect(await visibleEndpoints(page, list)).toHaveLength(4);
  expect(new URL(page.url()).search).not.toContain("show=");

  await page.click('[data-endpoint-filter-chip="used"]');
  await page.click('[data-endpoint-filter-chip="deprecated"]');
  expect(new URL(page.url()).searchParams.get("show")).toBe("used,deprecated");
  expect(await visibleEndpoints(page, list)).toEqual(["DELETE /carts/{id}"]);

  // Open the endpoint and come back: the filter is still on.
  await page.locator(`${list} .endpoint-card:visible`).first().click();
  await page.waitForURL(/yaml\.html/);
  await page.goBack();
  await expect(page.locator(`${list} .endpoint-card`)).toHaveCount(4);
  expect(await visibleEndpoints(page, list)).toEqual(["DELETE /carts/{id}"]);
  await expect(page.locator('[data-endpoint-filter-chip="used"]')).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

test("the consumer view filters its endpoints and hides empty groups", async ({ page }) => {
  await page.goto(`/consumers.html?name=${CONSUMER}&q=drop`);
  const list = "#clients-list";
  await expect(page.locator("[data-endpoint-filter-q]")).toHaveValue("drop");
  expect(await visibleEndpoints(page, list)).toEqual(["DELETE /carts/{id}"]);
  await expect(page.locator(`${list} .endpoint-summary:visible`)).toHaveText("Drop a cart");

  await page.locator("[data-endpoint-filter-q]").fill("");
  expect(await visibleEndpoints(page, list)).toEqual(["GET /carts", "DELETE /carts/{id}"]);
  await page.click('[data-endpoint-filter-chip="deprecated"]');
  expect(await visibleEndpoints(page, list)).toEqual(["DELETE /carts/{id}"]);

  await page.locator("[data-endpoint-filter-q]").fill("nothing-like-this");
  await expect(page.locator("[data-endpoint-filter-empty]")).toBeVisible();
  await expect(page.locator(`${list} [data-view-producer]:visible`)).toHaveCount(0);
});
