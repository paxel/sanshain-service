// A producer's page beyond the happy path: the list search, the full API
// download of a version and what deleting a version asks first. Requests that
// would change the instance are stubbed.
import { test, expect, json } from "./fixtures.js";

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;
const SERVICE = "producer-page-fixture";

/**
 * Answers the request for exactly this path, never the page itself.
 * @param {import("@playwright/test").Page} page
 * @param {string} pathname
 * @param {(route: import("@playwright/test").Route) => Promise<void>} handler
 */
const onPath = (page, pathname, handler) => page.route((url) => url.pathname === pathname, handler);

test.beforeAll(async ({ request }) => {
  if (!adminPassword) throw new Error("INITIAL_ADMIN_PASSWORD is required for the UI tests");
  const login = await request.post("/auth/login", {
    data: { username: "root", password: adminPassword },
  });
  const headers = {
    Authorization: `Bearer ${/** @type {{ token: string }} */ (await json(login)).token}`,
  };
  const res = await request.post("/provide", {
    headers,
    data: {
      producername: SERVICE,
      stability: "snapshot",
      openapi_yaml: [
        "openapi: 3.0.3",
        "info:",
        "  title: Producer page fixture",
        "  version: 1.0.0",
        "paths:",
        "  /stock:",
        "    get:",
        "      responses:",
        "        '200':",
        "          description: OK",
      ].join("\n"),
    },
  });
  if (!res.ok()) throw new Error(`provide: ${String(res.status())}`);
});

/** @type {string[]} */
let alerts;

test.beforeEach(async ({ page }) => {
  await page.route("**/api/sse/updates", (route) =>
    route.fulfill({ status: 200, contentType: "text/event-stream", body: "" }),
  );
  alerts = [];
  page.on("dialog", (dialog) => {
    alerts.push(dialog.message());
    return dialog.accept();
  });
  await page.goto("/account.html");
  await page.evaluate(() => {
    localStorage.clear();
  });
  await page.reload();
  await page.fill("#login-username", "root");
  await page.fill("#login-password", /** @type {string} */ (adminPassword));
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#dashboard")).toBeVisible({ timeout: 10000 });
});

test("the producer list narrows to what is searched", async ({ page }) => {
  await page.goto("/producers.html");
  const search = page.locator("#service-search");
  await search.fill(SERVICE);
  await expect(page.getByText(SERVICE, { exact: true }).first()).toBeVisible({ timeout: 10000 });
  await search.fill("no-producer-is-called-like-this");
  await expect(page.getByText(SERVICE, { exact: true })).toHaveCount(0);
  await search.fill("");
  await expect(page.getByText(SERVICE, { exact: true }).first()).toBeVisible();
});

test("a version downloads its full API and asks before it is deleted", async ({ page }) => {
  await page.goto(`/producers.html?service=${SERVICE}`);
  const card = page.locator("[data-download]").first();
  await expect(card).toBeVisible({ timeout: 10000 });

  const download = page.waitForEvent("download");
  await card.click();
  expect((await download).suggestedFilename()).toBe(`${SERVICE}-1.0.0.yaml`);

  const fullSpec = `/admin/producers/${SERVICE}/full-spec`;
  await onPath(page, fullSpec, (route) => route.fulfill({ status: 500, body: "" }));
  await card.click();
  await expect.poll(() => alerts).toContain("Failed to load the full API spec for this version.");
  await page.unroute((url) => url.pathname === fullSpec);

  const dependents = `/admin/producers/${SERVICE}/versions/openapi/1.0.0/dependents`;
  await onPath(page, dependents, (route) => route.fulfill({ status: 500, body: "" }));
  await page.locator("[data-delete]").first().click();
  await expect
    .poll(() => alerts.some((a) => a.startsWith("Could not check dependents")))
    .toBe(true);
  await page.unrouteAll();

  // With nobody pinned, the confirmation says so; the deletion itself is refused here.
  await onPath(page, `/admin/producers/${SERVICE}/versions/openapi/1.0.0`, (route) =>
    route.request().method() === "DELETE"
      ? route.fulfill({ status: 500, contentType: "application/json", body: "{}" })
      : route.continue(),
  );
  await page.locator("[data-delete]").first().click();
  await expect(page.locator("#confirm-message")).toContainText("No Consumers are pinned");
  await page.click("#confirm-yes");
  await expect.poll(() => alerts.some((a) => a.startsWith("Failed to delete:"))).toBe(true);
});
