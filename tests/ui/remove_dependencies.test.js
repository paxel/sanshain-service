// san-1018: the Remove button of a Producer group on the Consumer page removes
// every dependency of that Consumer on that Producer and shows the page again.
import { test, expect, json } from "./fixtures.js";

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;
const stamp = Date.now().toString(36);
const KEPT = `remove-kept-${stamp}`;
const GONE = `remove-gone-${stamp}`;
const CONSUMER = `remove-app-${stamp}`;

/** @param {string} title */
const spec = (title) =>
  [
    "openapi: 3.0.0",
    "info:",
    `  title: ${title}`,
    "  version: 1.0.0",
    "paths:",
    "  /items:",
    "    get:",
    "      responses:",
    "        '200': { description: ok }",
  ].join("\n");

test.beforeAll(async ({ request }) => {
  if (!adminPassword) throw new Error("INITIAL_ADMIN_PASSWORD is required for the UI tests");
  const login = await request.post("/auth/login", {
    data: { username: "root", password: adminPassword },
  });
  const { token } = /** @type {{ token: string }} */ (await json(login));
  const headers = { Authorization: `Bearer ${token}` };
  for (const producer of [KEPT, GONE]) {
    const res = await request.post("/provide", {
      headers,
      data: { producername: producer, stability: "ga", openapi_yaml: spec(producer) },
    });
    expect(res.ok(), `provide: ${await res.text()}`).toBeTruthy();
    const req = await request.get("/require", {
      headers,
      params: {
        consumername: CONSUMER,
        producername: producer,
        version: "1.0.0",
        path: "/items",
        method: "GET",
        trunk: "true",
      },
    });
    expect(req.ok(), `require: ${await req.text()}`).toBeTruthy();
  }
});

test.beforeEach(async ({ page }) => {
  await page.route("**/api/sse/updates", (route) =>
    route.fulfill({ status: 200, contentType: "text/event-stream", body: "" }),
  );
  await page.goto("/account.html");
  await page.waitForSelector("#login-username", { state: "visible" });
  await page.fill("#login-username", "root");
  await page.fill("#login-password", /** @type {string} */ (adminPassword));
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#dashboard")).toBeVisible({ timeout: 10000 });
});

test("Remove on a Producer group drops the Consumer's dependencies on it", async ({ page }) => {
  await page.goto(`/consumers.html?name=${CONSUMER}`);
  const groups = page.locator("#clients-list [data-remove-dependencies]");
  await expect(groups).toHaveCount(2);

  // Dismissed: nothing changes.
  page.once("dialog", (dialog) => void dialog.dismiss());
  const gone = page
    .locator("#clients-list > div")
    .filter({ has: page.locator("[data-group-service]", { hasText: GONE }) });
  await gone.locator("[data-remove-dependencies]").click();
  await expect(groups).toHaveCount(2);

  page.once("dialog", (dialog) => void dialog.accept());
  await gone.locator("[data-remove-dependencies]").click();
  await expect(groups).toHaveCount(1);
  await expect(page.locator("#clients-list [data-group-service]")).toHaveText(KEPT);
});

test("a refused Remove says why", async ({ page }) => {
  await page.route(`**/admin/consumers/${CONSUMER}/dependencies/**`, (route) =>
    route.fulfill({
      status: 403,
      contentType: "application/json",
      body: JSON.stringify({ error: "Missing permission manage_consumers" }),
    }),
  );
  await page.goto(`/consumers.html?name=${CONSUMER}`);
  /** @type {string[]} */
  const messages = [];
  page.on("dialog", (dialog) => {
    messages.push(dialog.message());
    void dialog.accept();
  });
  await page.locator("#clients-list [data-remove-dependencies]").first().click();
  await expect.poll(() => messages.length).toBe(2);
  expect(messages[1]).toContain("Could not remove the dependencies");
});
