// The header, navigation and footer every page gets from site-chrome.js: the
// same links everywhere, the page's own marked, the permission-gated links
// shown only to who may follow them, and one loading screen.
import { test, expect } from "./fixtures.js";

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;
const PAGES = [
  ["/producers.html", true],
  ["/consumers.html", true],
  ["/products.html", true],
  ["/graph.html", true],
  ["/reports.html", true],
  ["/account.html", true],
  ["/audit.html", true],
  ["/observability.html", false],
  ["/admin.html", false],
];
const OPEN_LINKS = ["Producers", "Consumers", "Products", "Graph", "Reports", "Observability"];

async function signIn(page) {
  await page.goto("/account.html");
  await page.waitForSelector("#login-username", { state: "visible" });
  await page.fill("#login-username", "root");
  await page.fill("#login-password", adminPassword);
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#dashboard")).toBeVisible({ timeout: 10000 });
}

test("every page carries the same header and its footer", async ({ page }) => {
  // Signed in, so no page sends the visit on to the sign-in page.
  await signIn(page);
  for (const [path, hasFooter] of PAGES) {
    await page.goto(path);
    expect(new URL(page.url()).pathname, path).toBe(path);
    const nav = page.locator("#site-banner nav");
    await expect(nav.locator("a"), path).toHaveText([
      "Producers",
      "Consumers",
      "Products",
      "Graph",
      "Reports",
      "Audit",
      "Observability",
      "Admin",
    ]);
    await expect(page.locator("#app-loader"), path).toHaveCount(1);
    await expect(page.locator("#site-footer"), path).toHaveCount(hasFooter ? 1 : 0);
    if (hasFooter) await expect(page.locator("#site-footer"), path).toContainText("License");
  }
  // The page's own link is marked.
  await page.goto("/graph.html");
  await expect(page.locator('#site-banner nav a[href="/graph.html"]')).toHaveClass(/underline/);
});

test("the gated links show only once someone may follow them", async ({ page }) => {
  await page.goto("/producers.html");
  for (const label of OPEN_LINKS) {
    await expect(page.locator("#site-banner nav a", { hasText: label })).toBeVisible();
  }
  await expect(page.locator("#nav-admin-link")).toBeHidden();
  await expect(page.locator("#nav-audit-link")).toBeHidden();

  await signIn(page);
  await page.goto("/producers.html");
  await expect(page.locator("#nav-admin-link")).toBeVisible({ timeout: 10000 });
  await expect(page.locator("#nav-audit-link")).toBeVisible();
});
