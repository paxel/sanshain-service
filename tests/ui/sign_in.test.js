// The sign-in card follows the instance's settings: the register tab only
// while registration is open, a text that does not promise a wait when
// accounts are acknowledged automatically, the SSO button only while OIDC
// is the source, and a maintenance notice while only administrators get in.
import { test, expect } from "@playwright/test";

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;
const USER = `signin-${Date.now().toString(36)}`;

let headers;
const post = (request, url, data) => request.post(url, { headers, data });
const authMode = (request, mode) =>
  request.put("/admin/auth-config", { headers, data: { auth_mode: mode } });

test.beforeAll(async ({ request }) => {
  if (!adminPassword) throw new Error("INITIAL_ADMIN_PASSWORD is required for the UI tests");
  const login = await request.post("/auth/login", {
    data: { username: "root", password: adminPassword },
  });
  headers = { Authorization: `Bearer ${(await login.json()).token}` };
});

// Leave the instance as the other suites expect it: Local, registration closed.
test.afterAll(async ({ request }) => {
  await authMode(request, "local");
  await post(request, "/admin/settings/registration", { enabled: false });
  await post(request, "/admin/settings/auto-approve", { enabled: false });
});

test.beforeEach(async ({ page }) => {
  await page.route("**/api/sse/updates", (route) =>
    route.fulfill({ status: 200, contentType: "text/event-stream", body: "" }),
  );
});

const openSignIn = async (page) => {
  await page.goto("/account.html");
  await page.evaluate(() => localStorage.clear());
  await page.reload();
  await expect(page.locator("#login-username")).toBeVisible();
};

test("the register tab and its text follow the registration switches", async ({
  page,
  request,
}) => {
  await post(request, "/admin/settings/registration", { enabled: false });
  await openSignIn(page);
  await expect(page.locator("#tab-register")).toBeHidden();
  await expect(page.locator("#sso-login")).toBeHidden();
  await expect(page.locator("#auth-notice")).toBeHidden();

  await post(request, "/admin/settings/registration", { enabled: true });
  await post(request, "/admin/settings/auto-approve", { enabled: true });
  await openSignIn(page);
  await page.click("#tab-register");
  await expect(page.locator("#register-help")).toHaveText(
    "You can sign in as soon as your account is created.",
  );
  await page.fill("#register-username", USER);
  await page.fill("#register-password", "secret-pass-1");
  await page.fill("#register-password-confirm", "secret-pass-1");
  await page.click('#register-form button[type="submit"]');
  await expect(page.locator("#register-success")).toHaveText(
    "Account created. You can sign in now.",
  );

  await post(request, "/admin/settings/auto-approve", { enabled: false });
  await openSignIn(page);
  await page.click("#tab-register");
  await expect(page.locator("#register-help")).toContainText("an admin must approve");
});

test("in maintenance the card says so and refuses everyone but administrators", async ({
  page,
  request,
}) => {
  // USER exists from the test above, auto-acknowledged.
  expect((await authMode(request, "off")).ok()).toBeTruthy();
  await openSignIn(page);
  await expect(page.locator("#auth-notice")).toHaveText(
    "Sanshain is in maintenance. Only administrators can sign in.",
  );
  await expect(page.locator("#tab-register")).toBeHidden();

  await page.fill("#login-username", USER);
  await page.fill("#login-password", "secret-pass-1");
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#login-error")).toContainText("maintenance");

  await page.fill("#login-username", "root");
  await page.fill("#login-password", adminPassword);
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });
  expect((await authMode(request, "local")).ok()).toBeTruthy();
});

// Reported from a real instance: once switched off, auto-acknowledge could not
// be switched on again. Each click must reach the server, and the switch must
// show the stored value after a reload.
test("the auto-acknowledge switch turns off and on again and keeps its state", async ({
  page,
  request,
}) => {
  expect((await authMode(request, "local")).ok()).toBeTruthy();
  await post(request, "/admin/settings/auto-approve", { enabled: true });
  const stored = async () =>
    (await (await request.get("/admin/settings/auto-approve", { headers })).json())
      .auto_approve_users;

  await openSignIn(page);
  await page.fill("#login-username", "root");
  await page.fill("#login-password", adminPassword);
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });
  const openAdmin = async () => {
    await page.goto("/admin.html");
    await expect(page.locator("#admin-dashboard:not(.hidden)")).toBeVisible({ timeout: 30000 });
    await expect(page.locator("#local-users-panel")).toBeVisible();
  };
  await openAdmin();

  const toggle = page.locator("#auto-approve-toggle");
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  expect(await stored()).toBe(false);
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  expect(await stored()).toBe(true);

  await openAdmin();
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await openAdmin();
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  expect(await stored()).toBe(true);
});
