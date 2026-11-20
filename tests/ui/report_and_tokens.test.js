// Two small pages no other suite opens: the report viewer, which renders a
// Markdown report from this server only, and the token dashboard at /dashboard.
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

test("the report viewer renders a report, copies and downloads it", async ({ page }) => {
  const markdown = "# Drift\n\n| Service | Status |\n|---|---|\n| orders | ok |\n";
  await onPath(page, "/reports/fixture-drift", (route) =>
    route.fulfill({ status: 200, contentType: "text/markdown", body: markdown }),
  );
  await page.goto("/report-viewer.html?src=/reports/fixture-drift&title=Drift");
  await expect(page).toHaveTitle("Drift — Sanshain");
  const content = page.locator("#report-content");
  await expect(content.locator("h1")).toHaveText("Drift");
  // Tables scroll inside their own wrapper.
  await expect(content.locator(".table-wrapper table")).toHaveCount(1);

  await stubClipboard(page);
  await page.click("#copy-btn");
  await expect(page.locator("#copy-btn")).toContainText("Copied!");
  expect(
    await page.evaluate(
      () => /** @type {CopiedWindow} */ (/** @type {unknown} */ (window)).__copied,
    ),
  ).toBe(markdown);

  const download = page.waitForEvent("download");
  await page.click("#download-btn");
  expect((await download).suggestedFilename()).toBe("fixture-drift.md");
});

test("the report viewer refuses what it cannot show", async ({ page }) => {
  await page.goto("/report-viewer.html");
  await expect(page.locator("#error-detail")).toHaveText("No report source specified.");

  // The request would carry the token, so another server is never asked.
  await page.goto("/report-viewer.html?src=https://elsewhere.example/report");
  await expect(page.locator("#error-detail")).toHaveText(
    "The report source must be on this server.",
  );

  await onPath(page, "/reports/fixture-locked", (route) => route.fulfill({ status: 403 }));
  await page.goto("/report-viewer.html?src=/reports/fixture-locked");
  await expect(page.locator("#error-detail")).toHaveText(
    "Authentication required. Please log in first.",
  );

  await onPath(page, "/reports/fixture-broken", (route) =>
    route.fulfill({ status: 500, body: "" }),
  );
  await page.goto("/report-viewer.html?src=/reports/fixture-broken");
  await expect(page.locator("#error-detail")).toContainText("Server returned 500");
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
