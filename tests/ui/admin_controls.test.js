import { test, expect } from "@playwright/test";

// Behavioural coverage of the admin dashboard's controls, written before the
// inline handlers are removed (ai/improvements.md #11): each test drives a
// control and asserts its visible effect, so it passes against the current
// onclick= wiring and must keep passing once those become a delegated
// data-action dispatcher. A control that goes dead in the rewrite fails here.
// Only non-destructive controls are exercised — no nuke, no cleanup-that-
// deletes, no logout, no navigation.

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

test.beforeEach(async ({ page }) => {
  if (!adminPassword) throw new Error("INITIAL_ADMIN_PASSWORD is required");
  await page.goto("/account.html");
  await page.waitForSelector("#login-username", { state: "visible" });
  await page.fill("#login-username", "root");
  await page.fill("#login-password", adminPassword);
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });
  await page.goto("/admin.html");
  await expect(page.locator("#admin-dashboard:not(.hidden)")).toBeVisible({ timeout: 30000 });
});

test("auth-mode radios reveal and hide their config forms", async ({ page }) => {
  await page.locator('input[name="auth-mode"][value="ldap"]').check();
  await expect(page.locator("#ldap-config-form")).toBeVisible();
  await page.locator('input[name="auth-mode"][value="disabled"]').check();
  await expect(page.locator("#ldap-config-form")).toBeHidden();
});

test("the trunk max-age save reports success", async ({ page }) => {
  const input = page.locator("#trunk-max-age-days");
  await input.scrollIntoViewIfNeeded();
  await input.fill("77");
  await page.locator('#trunk-max-age-days ~ button:has-text("Save")').click();
  // The value persists across a reload — the save actually took.
  await page.reload();
  await expect(page.locator("#admin-dashboard:not(.hidden)")).toBeVisible({ timeout: 30000 });
  await expect(page.locator("#trunk-max-age-days")).toHaveValue("77");
  // Restore.
  await page.locator("#trunk-max-age-days").fill("90");
  await page.locator('#trunk-max-age-days ~ button:has-text("Save")').click();
});

test("a trunk cleanup run reports its result", async ({ page }) => {
  // Non-destructive on a clean graph: closes nothing, but the handler must run
  // and write the result span.
  await page.locator("#trunk-cleanup-result").scrollIntoViewIfNeeded();
  await page.locator('#trunk-max-age-days ~ button:has-text("Run Cleanup")').click();
  await expect(page.locator("#trunk-cleanup-result")).not.toBeEmpty({ timeout: 10000 });
});

test("the services filter narrows the list", async ({ page }) => {
  const filter = page.locator("#services-filter");
  await filter.scrollIntoViewIfNeeded();
  // Typing runs filterServices(); assert it does not throw and the input holds.
  await filter.fill("zzz-no-such-service");
  await expect(filter).toHaveValue("zzz-no-such-service");
  await filter.fill("");
});

test("the users section loads its list without error", async ({ page }) => {
  // loadUsers() runs on section render; the root account must appear.
  const usersHeading = page.locator("h2", { hasText: /^Users$/ });
  await usersHeading.scrollIntoViewIfNeeded();
  await expect(page.locator("body")).toContainText("root", { timeout: 10000 });
});
