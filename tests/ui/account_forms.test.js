// The answers of the account page's three forms: sign-in, registration and the
// password change. The server's refusals are stubbed where a real one would
// change the instance the other suites use.
import { test, expect } from "./fixtures.js";

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

/**
 * Answers the request for exactly this path.
 * @param {import("@playwright/test").Page} page
 * @param {string} pathname
 * @param {(route: import("@playwright/test").Route) => Promise<void>} handler
 */
const onPath = (page, pathname, handler) => page.route((url) => url.pathname === pathname, handler);

test.beforeEach(async ({ page }) => {
  if (!adminPassword) throw new Error("INITIAL_ADMIN_PASSWORD is required for the UI tests");
  await page.route("**/api/sse/updates", (route) =>
    route.fulfill({ status: 200, contentType: "text/event-stream", body: "" }),
  );
});

test("a wrong password is refused on the sign-in card", async ({ page }) => {
  await page.goto("/account.html");
  await page.evaluate(() => {
    localStorage.clear();
  });
  await page.reload();
  await page.fill("#login-username", "root");
  await page.fill("#login-password", "not-the-password");
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#login-error")).toHaveText("Invalid username or password.");
});

test("the registration form says what became of the request", async ({ page }) => {
  await onPath(page, "/auth/options", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        registration: true,
        sso: false,
        auto_approve: true,
        admins_only: false,
        auth_mode: "local",
      }),
    }),
  );
  /** @type {{ status: number, body?: string }[]} */
  const answers = [
    { status: 201, body: JSON.stringify({ approved: true }) },
    { status: 409 },
    { status: 403 },
    { status: 500 },
  ];
  await onPath(page, "/auth/register", (route) => {
    const answer = answers.shift() ?? { status: 500 };
    return route.fulfill({ contentType: "application/json", body: "", ...answer });
  });
  await page.goto("/account.html");
  await page.evaluate(() => {
    localStorage.clear();
  });
  await page.reload();
  await page.click("#tab-register");
  await expect(page.locator("#register-help")).toHaveText(
    "You can sign in as soon as your account is created.",
  );

  const error = page.locator("#register-error");
  const register = async (/** @type {string} */ confirm) => {
    await page.fill("#register-username", "ui-register");
    await page.fill("#register-password", "a-long-enough-password");
    await page.fill("#register-password-confirm", confirm);
    await page.click('#register-form button[type="submit"]');
  };
  await register("something-else");
  await expect(error).toHaveText("Passwords do not match.");

  await register("a-long-enough-password");
  await expect(page.locator("#register-success")).toHaveText(
    "Account created. You can sign in now.",
  );
  await register("a-long-enough-password");
  await expect(error).toHaveText("Username already taken.");
  await register("a-long-enough-password");
  await expect(error).toHaveText("Registration is currently disabled.");
  await register("a-long-enough-password");
  await expect(error).toHaveText("Registration failed (500).");
});

test("the password change says whether it worked", async ({ page }) => {
  await page.goto("/account.html");
  await page.evaluate(() => {
    localStorage.clear();
  });
  await page.reload();
  await page.fill("#login-username", "root");
  await page.fill("#login-password", /** @type {string} */ (adminPassword));
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#dashboard")).toBeVisible({ timeout: 10000 });
  await page.goto("/account.html");
  await expect(page.locator("#pw-form")).toBeVisible({ timeout: 10000 });

  const change = async () => {
    await page.fill("#pw-current", "not-the-password");
    await page.fill("#pw-new", "another-long-password");
    await page.click('#pw-form button[type="submit"]');
  };
  // The real server refuses a wrong current password.
  await change();
  await expect(page.locator("#pw-error")).toHaveText("Current password is incorrect.");

  // The other answers are stubbed, so root keeps its password.
  /** @type {{ status: number, body: string }[]} */
  const answers = [
    { status: 500, body: "" },
    { status: 200, body: "{}" },
  ];
  await onPath(page, "/auth/change-password", (route) =>
    route.fulfill({ contentType: "application/json", ...(answers.shift() ?? answers[0]) }),
  );
  await change();
  await expect(page.locator("#pw-error")).toHaveText("Error: 500");
  await change();
  await expect(page.locator("#pw-success")).toHaveText("Password updated successfully.");
});
