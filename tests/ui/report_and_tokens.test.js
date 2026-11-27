// A small page no other suite opens: the token dashboard at /dashboard. (The
// report viewer it also covered went with san-1028.)
import { test, expect } from "./fixtures.js";

/** @typedef {{ __copied?: string }} CopiedWindow The stubbed clipboard keeps what it got here. */

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

/**
 * Answers the request for exactly this path, never the page whose address names it.
 * @param {import("@playwright/test").Page} page
 * @param {string} pathname
 * @param {(route: import("@playwright/test").Route) => Promise<void>} handler
 */
const onPath = (page, pathname, handler) => page.route((url) => url.pathname === pathname, handler);

/**
 * A headless browser may refuse the clipboard; the stub records what was copied.
 * @param {import("@playwright/test").Page} page
 */
const stubClipboard = (page) =>
  page.evaluate(() => {
    navigator.clipboard.writeText = (text) => {
      /** @type {CopiedWindow} */ (/** @type {unknown} */ (window)).__copied = text;
      return Promise.resolve();
    };
  });

test.beforeEach(async ({ page }) => {
  if (!adminPassword) throw new Error("INITIAL_ADMIN_PASSWORD is required for the UI tests");
  await page.route("**/api/sse/updates", (route) =>
    route.fulfill({ status: 200, contentType: "text/event-stream", body: "" }),
  );
  page.on("dialog", (dialog) => dialog.accept());
  await page.goto("/account.html");
  await page.evaluate(() => {
    localStorage.clear();
  });
  await page.reload();
  await page.fill("#login-username", "root");
  await page.fill("#login-password", adminPassword);
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#dashboard")).toBeVisible({ timeout: 10000 });
});

test("the token dashboard issues a token once and revokes it", async ({ page }) => {
  // The page is rendered for a bearer token, which a navigation does not send by itself.
  const token = await page.evaluate(() => localStorage.getItem("sanshain_token"));
  await onPath(page, "/dashboard", (route) =>
    route.continue({
      headers: { ...route.request().headers(), authorization: `Bearer ${String(token)}` },
    }),
  );
  await page.goto("/dashboard");
  await expect(page.locator("#dashboard-main")).toBeVisible({ timeout: 10000 });
  const name = `ui-dashboard-${Date.now().toString(36)}`;
  await page.fill("#tokenName", name);
  await page.click('#createTokenForm button[type="submit"]');
  await expect(page.locator("#tokenModal")).toBeVisible();
  const value = await page.locator("#tokenValue").textContent();
  expect(value).toMatch(/^san_/);
  await expect(page.locator("#tokenValueUsage")).toHaveText(/** @type {string} */ (value));

  await stubClipboard(page);
  await page.click("#copyTokenBtn");
  await expect(page.locator("#copyTokenBtn")).toHaveText("✅ Copied!");
  await page.click("#closeModal");
  await expect(page.locator("#tokenModal")).toBeHidden();

  const row = page.locator("#tokenTableBody tr", { hasText: name });
  await expect(row).toBeVisible();
  await row.locator('button:has-text("Revoke")').click();
  await expect(row).toHaveCount(0);
});
