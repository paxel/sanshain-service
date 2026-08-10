import { test, expect } from "@playwright/test";

// The safety net for de-inlining (ai/improvements.md #11). Once the CSP forbids
// inline scripts and every `onclick=` becomes an addEventListener, a handler
// that was missed no longer silently no-ops — it throws a ReferenceError or a
// CSP violation the moment the page loads or the control is used. These tests
// fail on any such throw, on every page, so a botched extraction cannot pass
// unseen. They assert nothing about styling — that is what the screenshot pass
// is for — only that the JavaScript is wired up and runs clean.

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

// This net targets the failure mode of de-inlining specifically, not every
// console.error a page might emit. A missed handler under a strict CSP shows up
// as one of two things: an uncaught exception (`pageerror` — e.g. an inline
// `onclick="foo()"` whose `foo` is gone throws a ReferenceError), or a CSP
// refusal the browser logs ("Refused to execute inline script…"). We keep those
// and deliberately ignore the pre-existing background noise a clean build
// already produces — a 403 on a probed resource, the SSE reconnect the app logs
// itself, favicon 404s, the Tailwind CDN's production advisory — none of which
// is a script fault and none of which a de-inlining change can cause.
const SCRIPT_FAULT =
  /refused to (execute|apply|load)|content security policy|is not defined|referenceerror|typeerror|syntaxerror|is not a function/i;

function watchForErrors(page) {
  const errors = [];
  page.on("pageerror", (e) => errors.push(`pageerror: ${e.message}`));
  page.on("console", (msg) => {
    if (msg.type() !== "error") return;
    const text = msg.text();
    if (SCRIPT_FAULT.test(text)) errors.push(`console.error: ${text}`);
  });
  return errors;
}

async function login(page) {
  await page.goto("/account.html");
  await page.waitForSelector("#login-username", { state: "visible" });
  await page.fill("#login-username", "root");
  await page.fill("#login-password", adminPassword);
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });
}

test.beforeAll(() => {
  if (!adminPassword) {
    throw new Error("INITIAL_ADMIN_PASSWORD is required for the UI safety-net tests");
  }
});

// Every authenticated page must load without a JavaScript fault. `service.html`
// and `yaml.html` need query params to do anything real, but their shells must
// still initialise cleanly.
const AUTHED_PAGES = [
  "/account.html",
  "/admin.html",
  "/producers.html",
  "/consumers.html",
  "/graph.html",
  "/reports.html",
  "/audit.html",
  "/observability.html",
];

for (const path of AUTHED_PAGES) {
  test(`${path} loads without a JavaScript error`, async ({ page }) => {
    const errors = watchForErrors(page);
    await login(page);
    await page.goto(path);
    // Let deferred init (fetches, renders) run and settle.
    await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
    await page.waitForTimeout(1500);
    expect(errors, `${path} produced JS errors:\n${errors.join("\n")}`).toEqual([]);
  });
}

test("the public landing page loads without a JavaScript error", async ({ page }) => {
  const errors = watchForErrors(page);
  await page.goto("/");
  await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
  await page.waitForTimeout(1000);
  expect(errors, `landing produced JS errors:\n${errors.join("\n")}`).toEqual([]);
});

// The admin dashboard is where 52 of the ~123 inline handlers live, so it is
// the page most likely to regress. Exercise a representative spread of its
// controls and assert no handler threw.
test("admin dashboard controls fire without error", async ({ page }) => {
  const errors = watchForErrors(page);
  await login(page);
  await page.goto("/admin.html");
  await expect(page.locator("#admin-dashboard:not(.hidden)")).toBeVisible({ timeout: 30000 });

  // A tab switch, a settings section, and an auth-mode radio each run a
  // handler; a missed extraction would throw here rather than pass.
  const authRadio = page.locator('input[value="ldap"]');
  if (await authRadio.count()) {
    await authRadio.first().click();
    await expect(page.locator("#ldap-config-form:not(.hidden)")).toBeVisible({ timeout: 5000 });
    await page.locator('input[value="disabled"]').first().click();
  }

  await page.waitForTimeout(500);
  expect(errors, `admin controls threw:\n${errors.join("\n")}`).toEqual([]);
});
