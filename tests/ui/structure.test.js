// The spec viewer's Structure view: the endpoint as a foldable tree with
// types, flags, descriptions, the version that last changed each node and a
// copyable example per body; folded endpoints in the full-spec viewer that
// load when opened; and the choice remembered in the browser.
import { test, expect } from "@playwright/test";

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;
const PRODUCER = `structure-${Date.now().toString(36)}`;

const spec = (version, extra) =>
  [
    "openapi: 3.0.0",
    "info:",
    "  title: shop",
    `  version: ${version}`,
    "paths:",
    "  /carts:",
    "    post:",
    "      summary: Open a cart",
    "      requestBody:",
    "        content:",
    "          application/json:",
    "            schema:",
    "              $ref: '#/components/schemas/Cart'",
    "      responses:",
    "        '201':",
    "          description: created",
    "components:",
    "  schemas:",
    "    Cart:",
    "      type: object",
    "      required: [owner]",
    "      properties:",
    "        owner:",
    "          type: string",
    "          format: email",
    "          description: Who pays for the cart.",
    ...extra,
  ].join("\n");

test.beforeAll(async ({ request }) => {
  if (!adminPassword) throw new Error("INITIAL_ADMIN_PASSWORD is required for the UI tests");
  const login = await request.post("/auth/login", {
    data: { username: "root", password: adminPassword },
  });
  const { token } = await login.json();
  const headers = { Authorization: `Bearer ${token}` };
  const currency = ["        currency:", "          type: string", "          default: EUR"];
  for (const [version, extra] of [
    ["1.0.0", []],
    ["1.1.0", currency],
  ]) {
    const res = await request.post("/provide", {
      headers,
      data: { producername: PRODUCER, stability: "snapshot", openapi_yaml: spec(version, extra) },
    });
    expect(res.ok(), `provide ${version}: ${await res.text()}`).toBeTruthy();
  }
});

test.beforeEach(async ({ page }) => {
  await page.route("**/api/sse/updates", (route) =>
    route.fulfill({ status: 200, contentType: "text/event-stream", body: "" }),
  );
  await page.goto("/account.html");
  await page.evaluate(() => localStorage.removeItem("sanshain_spec_view"));
  await page.waitForSelector("#login-username", { state: "visible" });
  await page.fill("#login-username", "root");
  await page.fill("#login-password", adminPassword);
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });
});

const endpointUrl = () =>
  `/yaml.html?service=${PRODUCER}&api_type=openapi&version=1.1.0&path=%2Fcarts&method=post`;

test("an endpoint reads as a tree with types, versions and a copyable example", async ({
  page,
}) => {
  await page.goto(endpointUrl());
  await page.click("#view-structure-btn");
  const root = page.locator("[data-structure-root]");
  await expect(root).toBeVisible();
  await expect(page.locator("#blame-toggle-container")).toBeHidden();

  const owner = root.locator('[data-structure-node="owner"]');
  await expect(owner.locator("summary")).toContainText("string (email)");
  await expect(owner.locator("summary")).toContainText("required");
  await expect(owner.locator("summary")).toContainText("1.0.0");
  await owner.locator("summary").click();
  await expect(owner.locator("[data-structure-description]")).toHaveText("Who pays for the cart.");
  const currency = root.locator('[data-structure-node="currency"]');
  await expect(currency.locator("summary")).toContainText("1.1.0");

  const example = root.locator("[data-structure-example]");
  await example.locator("summary").click();
  // The members' order, not the alphabet.
  await expect(example.locator("pre")).toHaveText(
    '{\n  "owner": "user@example.com",\n  "currency": "EUR"\n}',
  );

  await root.locator("[data-structure-collapse]").click();
  await expect(owner.locator("[data-structure-description]")).toBeHidden();
  await root.locator("[data-structure-expand]").click();
  await expect(owner.locator("[data-structure-description]")).toBeVisible();
});

test("the full-spec viewer loads a folded endpoint when it is opened", async ({ page }) => {
  await page.goto(endpointUrl());
  await page.click("#view-structure-btn");
  await page.goto(`/yaml.html?service=${PRODUCER}&api_type=openapi&version=1.1.0`);
  // Remembered from the endpoint page.
  const root = page.locator("[data-structure-root]");
  await expect(root).toBeVisible();
  const carts = root.locator('[data-structure-node="POST /carts"]');
  await expect(carts.locator("summary").first()).toContainText("inside: 1.1.0");
  await expect(carts.locator('[data-structure-node="owner"]')).toHaveCount(0);
  await carts.locator("summary").first().click();
  await expect(carts.locator('[data-structure-node="owner"]')).toBeVisible();

  await page.click("#view-yaml-btn");
  await page.reload();
  await expect(page.locator("[data-structure-root]")).toHaveCount(0);
});
