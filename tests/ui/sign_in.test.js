// The sign-in card follows the instance's settings: the register tab only
// while registration is open, a text that does not promise a wait when
// accounts are acknowledged automatically, the SSO button only while OIDC
// is the source, and a maintenance notice while only administrators get in.
import { test, expect } from "@playwright/test";
import { createHash } from "node:crypto";

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
  await expect(page.locator("#dashboard")).toBeVisible({ timeout: 10000 });
  expect((await authMode(request, "local")).ok()).toBeTruthy();
});

test("signing in lands on the dashboard, whose names only a signed-in user sees", async ({
  page,
}) => {
  expect((await authMode(page.request, "local")).ok()).toBeTruthy();
  await page.goto("/");
  await page.evaluate(() => localStorage.clear());
  await page.reload();
  await expect(page.locator("#stat-producers")).toHaveText(/^\d+$/);
  await expect(page.locator("#activity-chart > rect")).toHaveCount(30);
  // A day without provides is a quiet bar in both themes, not a white dash.
  const quiet = page.locator('#activity-chart > rect[data-provides="0"]').first();
  await page.evaluate(() => {
    document.cookie = "sanshain_theme=dark;path=/";
  });
  await page.reload();
  await expect(quiet).toHaveCSS("fill", "rgb(51, 65, 85)");
  await page.evaluate(() => {
    document.cookie = "sanshain_theme=light;path=/";
  });
  await page.reload();
  await expect(page.locator("#recent-provides-card")).toBeHidden();

  await openSignIn(page);
  await page.fill("#login-username", "root");
  await page.fill("#login-password", adminPassword);
  await page.click('#login-panel button[type="submit"]');
  await page.waitForURL((url) => url.pathname === "/");
  await expect(page.locator("#stat-versions")).toHaveText(/^\d+$/);
  await expect(page.locator("#recent-provides-card")).toBeVisible();
});

test("a local user names themselves on the account page and the banner shows it", async ({
  page,
}) => {
  expect((await authMode(page.request, "local")).ok()).toBeTruthy();
  await openSignIn(page);
  await page.fill("#login-username", "root");
  await page.fill("#login-password", adminPassword);
  await page.click('#login-panel button[type="submit"]');
  await page.waitForURL((url) => url.pathname === "/");
  await page.goto("/account.html");
  await expect(page.locator("#display-name-form")).toBeVisible();
  await page.fill("#display-name-input", "Root Admin");
  await page.click('#display-name-form button[type="submit"]');
  await expect(page.locator("#display-name-status")).toHaveText("Saved.");
  await expect(page.locator("#banner-username")).toContainText("Root Admin");
  await expect(page.locator("#banner-username [data-avatar]")).toHaveText("RA");
  // Leave root as the other suites expect it.
  await page.fill("#display-name-input", "");
  await page.click('#display-name-form button[type="submit"]');
  await expect(page.locator("#banner-username [data-avatar]")).toHaveText("R");
});

test("a person picks their own Gravatar address, and only its key is kept", async ({
  page,
  request,
}) => {
  expect((await authMode(request, "local")).ok()).toBeTruthy();
  await openSignIn(page);
  await page.fill("#login-username", "root");
  await page.fill("#login-password", adminPassword);
  await page.click('#login-panel button[type="submit"]');
  await page.waitForURL((url) => url.pathname === "/");

  // Off: the account page offers nothing.
  expect((await post(request, "/admin/settings/gravatar", { enabled: false })).ok()).toBeTruthy();
  await page.goto("/account.html");
  await expect(page.locator("#display-name-form")).toBeVisible();
  await expect(page.locator("#gravatar-section")).toBeHidden();

  expect((await post(request, "/admin/settings/gravatar", { enabled: true })).ok()).toBeTruthy();
  try {
    await page.reload();
    await expect(page.locator("#gravatar-section")).toBeVisible();
    await expect(page.locator("#gravatar-section")).toContainText("never stored");
    await page.fill("#gravatar-email", "Root@Home.example");
    await page.click('#gravatar-form button[type="submit"]');
    await expect(page.locator("#gravatar-status")).toHaveText("Saved.");
    await expect(page.locator("#gravatar-email")).toHaveValue("");
    await expect(page.locator("#gravatar-own")).toBeVisible();
    const key = createHash("sha256").update("root@home.example").digest("hex");
    await expect(page.locator("#gravatar-preview img")).toHaveAttribute(
      "src",
      new RegExp(`/avatar/${key}\\?`),
    );

    await page.click("#gravatar-remove");
    await expect(page.locator("#gravatar-status")).toHaveText("Removed.");
    await expect(page.locator("#gravatar-own")).toBeHidden();
  } finally {
    await post(request, "/admin/settings/gravatar", { enabled: false });
  }
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
  await expect(page.locator("#dashboard")).toBeVisible({ timeout: 10000 });
  const openAdmin = async () => {
    await page.goto("/admin.html#sign-in");
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
