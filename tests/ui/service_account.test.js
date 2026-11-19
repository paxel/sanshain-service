// Service accounts: an admin creates one on the admin page, issues its token
// on the account page through "For", and the token list says whose it is and
// who issued it.
import { test, expect } from "./fixtures.js";

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;
const ACCOUNT = `ci-${Date.now().toString(36)}`;

/**
 * This is the page with the answers the test queued for its prompts.
 * @typedef {import("@playwright/test").Page & { dialogAnswers: string[] }} PageWithAnswers
 */

test.beforeEach(async ({ page }) => {
  if (!adminPassword) throw new Error("INITIAL_ADMIN_PASSWORD is required for the UI tests");
  await page.route("**/api/sse/updates", (route) =>
    route.fulfill({ status: 200, contentType: "text/event-stream", body: "" }),
  );
  // Prompts answer what the test queued (an empty queue dismisses them).
  /** @type {PageWithAnswers} */ (page).dialogAnswers = [];
  page.on("dialog", (dialog) => {
    const answer = /** @type {PageWithAnswers} */ (page).dialogAnswers.shift();
    return answer === undefined ? dialog.dismiss() : dialog.accept(answer);
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

test("an admin creates a service account and issues its token", async ({ page }) => {
  await page.goto("/admin.html#users");
  await page.fill("#service-account-name", ACCOUNT);
  await page.click('#service-account-form button[type="submit"]');
  const row = page.locator("#users-list > div", { hasText: ACCOUNT });
  await expect(row.locator("[data-service-account]")).toHaveText("service account");
  // A service account gets a readable name from whoever manages users.
  /** @type {PageWithAnswers} */ (page).dialogAnswers.push("Penguin CI build");
  await row.locator('button:has-text("Name")').click();
  await expect(row.locator("[data-display-name]")).toHaveText("Penguin CI build");
  await expect(row.locator("[data-username]")).toHaveText(ACCOUNT);
  await expect(row.locator("[data-avatar]")).toHaveText("PC");

  // Every row says what kind of account it is; root just signed in locally.
  await page.fill("#users-search", "root");
  const root = page.locator("#users-list > div", {
    has: page.locator("[data-username]", { hasText: /^root$/ }),
  });
  await expect(root.locator("[data-account-kind]")).toHaveText("local");

  // Gravatar is off by default and switches on and off again.
  const gravatar = page.locator("#gravatar-toggle");
  await expect(gravatar).toHaveAttribute("aria-checked", "false");
  await gravatar.click();
  await expect(gravatar).toHaveAttribute("aria-checked", "true");
  await gravatar.click();
  await expect(gravatar).toHaveAttribute("aria-checked", "false");

  await page.goto("/account.html");
  await expect(page.locator("#token-for-row")).toBeVisible();
  await page.fill("#token-name", "deploy");
  await page.click("#token-for-input");
  await page.click(`[data-picker-option="${ACCOUNT}"]`);
  await expect(page.locator("#token-for-input")).toHaveValue(/Penguin CI build/);
  await page.click('#token-form button[type="submit"]');
  await expect(page.locator("#token-created-value")).toContainText("san_");
  const token = page.locator("#tokens-list > div", { hasText: ACCOUNT });
  await expect(token.locator("[data-token-for]")).toHaveText(`For ${ACCOUNT} · issued by root`);
});

test("an instance without accounts says so in the user list", async ({ page }) => {
  await page.route("**/admin/users", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: "[]" }),
  );
  await page.goto("/admin.html#users");
  await expect(page.locator("#users-list")).toHaveText("No users registered.");
});

test("a session that ran out brings the account page back to the sign-in card", async ({
  page,
}) => {
  await page.evaluate(() => {
    localStorage.setItem("sanshain_token", "expired-garbage");
    document.cookie = "sanshain_token=expired-garbage;path=/";
  });
  await page.goto("/account.html");
  await expect(page.locator("#auth-screen")).toBeVisible({ timeout: 10000 });
  await expect(page.locator("#login-username")).toBeVisible();
  // The page dropped the token the server refused.
  expect(await page.evaluate(() => localStorage.getItem("sanshain_token"))).toBeNull();
});
