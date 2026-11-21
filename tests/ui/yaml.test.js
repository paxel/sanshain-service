import { test, expect, json } from "./fixtures.js";

/** @typedef {{ __copied?: string }} CopiedWindow The stubbed clipboard keeps what it got here. */

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;
// A producer of its own, with two versions of one endpoint, so the viewer has a
// trail to show and two versions to compare.
const SERVICE = "yaml-viewer-fixture";

/** @param {string} version @param {string} summary */
const spec = (version, summary) =>
  [
    "openapi: 3.0.3",
    "info:",
    "  title: YAML viewer fixture",
    `  version: ${version}`,
    "paths:",
    "  /users:",
    "    get:",
    `      summary: ${summary}`,
    "      responses:",
    "        '200':",
    "          description: OK",
  ].join("\n");

test.beforeAll(async ({ request }) => {
  if (!adminPassword) throw new Error("INITIAL_ADMIN_PASSWORD is required for the UI tests");
  const login = await request.post("/auth/login", {
    data: { username: "root", password: adminPassword },
  });
  const headers = {
    Authorization: `Bearer ${/** @type {{ token: string }} */ (await json(login)).token}`,
  };
  for (const [version, summary] of [
    ["1.0.0", "List the users"],
    ["1.1.0", "List the users, newest first"],
  ]) {
    const res = await request.post("/provide", {
      headers,
      data: {
        producername: SERVICE,
        stability: "snapshot",
        openapi_yaml: spec(/** @type {string} */ (version), /** @type {string} */ (summary)),
      },
    });
    if (!res.ok()) throw new Error(`provide ${String(version)}: ${String(res.status())}`);
  }
});

test.beforeEach(async ({ page }) => {
  await page.route("**/api/sse/updates", (route) =>
    route.fulfill({ status: 200, contentType: "text/event-stream", body: "" }),
  );
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

test.describe("Sanshain YAML Viewer UI Tests", () => {
  test.use({ actionTimeout: 5000, navigationTimeout: 15000 });
  test("YAML viewer page loaded with correct elements", async ({ page }) => {
    // Without a version param the viewer loads the endpoint's full blame trail
    // and selects the newest version that carries the endpoint.
    await page.goto(`/yaml.html?service=${SERVICE}&api_type=openapi&path=/users&method=GET`);

    // Header should be visible
    await expect(page.locator("header#site-banner")).toBeVisible();

    // Breadcrumbs should contain service name and the version line
    const breadcrumbService = page.locator("#breadcrumb-service");
    await expect(breadcrumbService).toBeVisible({ timeout: 10000 });
    await expect(breadcrumbService).toContainText(SERVICE);
    await expect(page.locator("#breadcrumb-version")).toContainText("openapi");

    // Endpoint header details should be present
    await expect(page.locator("#endpoint-method")).toContainText("GET");
    await expect(page.locator("#endpoint-path")).toContainText("/users");

    // Right panel action buttons should be visible
    await expect(page.locator("#view-yaml-btn")).toBeVisible();
    await expect(page.locator("#view-diff-btn")).toBeVisible();
    await expect(page.locator("#blame-toggle-container")).toBeVisible();
    await expect(page.locator("#copy-btn")).toBeVisible();
    await expect(page.locator("#download-btn")).toBeVisible();

    // The newest version shows its summary; it copies and downloads as YAML.
    const viewer = page.locator("#viewer-container");
    await expect(viewer).toContainText("newest first", { timeout: 10000 });
    await page.evaluate(() => {
      navigator.clipboard.writeText = (text) => {
        /** @type {CopiedWindow} */ (/** @type {unknown} */ (window)).__copied = text;
        return Promise.resolve();
      };
    });
    await page.click("#copy-btn");
    await expect
      .poll(() =>
        page.evaluate(() => /** @type {CopiedWindow} */ (/** @type {unknown} */ (window)).__copied),
      )
      .toContain("newest first");
    const download = page.waitForEvent("download", { timeout: 5000 });
    await page.click("#download-btn");
    expect((await download).suggestedFilename()).toBe(`${SERVICE}_1.1.0.yaml`);

    // Blame on and off again, then the diff of the two versions.
    // The switch's styled track covers the checkbox, as a pointer sees it.
    await page.check("#blame-toggle", { force: true });
    await expect(page.locator("#blame-toggle")).toBeChecked();
    await page.uncheck("#blame-toggle", { force: true });
    await page.click("#view-diff-btn");
    await expect(viewer).toContainText("newest first", { timeout: 5000 });
  });
});
