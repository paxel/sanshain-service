// san-1021: the statistics page on /reports.html: its tabs, the range in the
// URL, the share link, and the exports of each panel. The tab tickets add
// their panels below.
import { test, expect } from "./fixtures.js";

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

/** @param {import("@playwright/test").Page} page */
async function signIn(page) {
  await page.route("**/api/sse/updates", (route) =>
    route.fulfill({ status: 200, contentType: "text/event-stream", body: "" }),
  );
  await page.goto("/account.html");
  await page.waitForSelector("#login-username", { state: "visible" });
  await page.fill("#login-username", "root");
  await page.fill("#login-password", /** @type {string} */ (adminPassword));
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#dashboard")).toBeVisible({ timeout: 10000 });
}

test("the page carries its tab and range in the URL", async ({ page }) => {
  await signIn(page);
  await page.goto("/reports.html");
  await expect(page.locator("#statistics-tab-landscape")).toHaveAttribute("aria-selected", "true");
  await expect(page).toHaveURL(/tab=landscape&from=\d{4}-\d{2}-\d{2}&to=\d{4}-\d{2}-\d{2}/);
  await expect(page.locator("#statistics-preset")).toHaveValue("90");

  await page.click("#statistics-tab-activity");
  await expect(page).toHaveURL(/tab=activity/);
  await expect(page.locator("#statistics-tab-activity")).toHaveAttribute("aria-selected", "true");

  await page.selectOption("#statistics-preset", "30");
  await expect(page.locator("#statistics-preset")).toHaveValue("30");
  const from = await page.locator("#statistics-from").inputValue();
  const to = await page.locator("#statistics-to").inputValue();
  const days = (Date.parse(to) - Date.parse(from)) / 86400000 + 1;
  expect(days).toBe(30);

  // A custom range is kept as it is, and a reload shows the same.
  await page.fill("#statistics-from", "2026-01-01");
  await page.locator("#statistics-from").dispatchEvent("change");
  await expect(page).toHaveURL(/from=2026-01-01/);
  await page.reload();
  await expect(page.locator("#statistics-preset")).toHaveValue("custom");
  await expect(page.locator("#statistics-from")).toHaveValue("2026-01-01");
  await expect(page.locator("#statistics-tab-activity")).toHaveAttribute("aria-selected", "true");
});

test("an unknown tab in the URL falls back to Landscape", async ({ page }) => {
  await signIn(page);
  await page.goto("/reports.html?tab=nope");
  await expect(page.locator("#statistics-tab-landscape")).toHaveAttribute("aria-selected", "true");
});
