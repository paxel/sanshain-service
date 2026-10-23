// Service accounts: an admin creates one on the admin page, issues its token
// on the account page through "For", and the token list says whose it is and
// who issued it.
import { test, expect } from "@playwright/test";

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;
const ACCOUNT = `ci-${Date.now().toString(36)}`;

test.beforeEach(async ({ page }) => {
  if (!adminPassword) throw new Error("INITIAL_ADMIN_PASSWORD is required for the UI tests");
  await page.route("**/api/sse/updates", (route) =>
    route.fulfill({ status: 200, contentType: "text/event-stream", body: "" }),
  );
  page.on("dialog", (dialog) => dialog.dismiss());
  await page.goto("/account.html");
  await page.evaluate(() => localStorage.clear());
  await page.reload();
  await page.fill("#login-username", "root");
  await page.fill("#login-password", adminPassword);
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#dashboard")).toBeVisible({ timeout: 10000 });
});

test("an admin creates a service account and issues its token", async ({ page }) => {
  await page.goto("/admin.html");
  await page.fill("#service-account-name", ACCOUNT);
  await page.click('#service-account-form button[type="submit"]');
  const row = page.locator("#users-list > div", { hasText: ACCOUNT });
  await expect(row.locator("[data-service-account]")).toHaveText("service account");

  await page.goto("/account.html");
  await expect(page.locator("#token-for-row")).toBeVisible();
  await page.fill("#token-name", "deploy");
  await page.selectOption("#token-for", ACCOUNT);
  await page.click('#token-form button[type="submit"]');
  await expect(page.locator("#token-created-value")).toContainText("san_");
  const token = page.locator("#tokens-list > div", { hasText: ACCOUNT });
  await expect(token.locator("[data-token-for]")).toHaveText(`For ${ACCOUNT} · issued by root`);
});
