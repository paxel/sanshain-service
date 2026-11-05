// The observability page's log level filter: the view and the copied text
// both take the chosen level and above, oldest first.
import { test, expect } from "./fixtures.js";

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

test("the log filter keeps the chosen level and above, oldest first", async ({ page }) => {
  await page.goto("/account.html");
  await page.waitForSelector("#login-username", { state: "visible" });
  await page.fill("#login-username", "root");
  await page.fill("#login-password", adminPassword);
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#dashboard")).toBeVisible({ timeout: 10000 });
  await page.goto("/observability.html");
  await expect(page.locator("#log-filter")).toBeVisible({ timeout: 10000 });
  const picked = await page.evaluate(() => {
    const entry = (timestamp, message) => ({ timestamp, message });
    currentLogs = {
      errors: [entry("2026-01-04", "e")],
      warnings: [entry("2026-01-02", "w")],
      infos: [entry("2026-01-03", "i")],
      debugs: [entry("2026-01-01", "d")],
    };
    const names = (filter) =>
      filteredLogs(filter)
        .map((l) => l.message)
        .join("");
    return ["ALL", "INFO", "WARN", "ERROR"].map(names);
  });
  expect(picked).toEqual(["dwie", "wie", "we", "e"]);
});
