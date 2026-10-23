// The UI steps of the recorded walkthroughs (issue #57). Run by
// scripts/record-walkthroughs.sh against the instance it started, after the
// HTTP steps have filled the catalog:
//   BASE_URL        the instance
//   SANSHAIN_TOKEN  a root session token
//   EVIDENCE_DIR    site/evidence; the story screenshots go to <story>/<nn>-<step>.png
// Light theme, 1280 wide; a dark twin (`-dark.png`) where the picture's colours
// carry meaning. The chapter screenshots the 2.6.0 chapters reference are taken
// here too, into site/assets/images/.
import { test } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";

const evidenceDir = process.env.EVIDENCE_DIR || "site/evidence";
const imagesDir = "site/assets/images";
const token = process.env.SANSHAIN_TOKEN;

const dev = (name) => path.join(evidenceDir, "developer", name);
const ops = (name) => path.join(evidenceDir, "operator", name);
const img = (name) => path.join(imagesDir, name);

test.describe("Recorded walkthroughs", () => {
  test.beforeAll(() => {
    if (!token) throw new Error("SANSHAIN_TOKEN (a root session token) is required");
    for (const dir of [path.join(evidenceDir, "developer"), path.join(evidenceDir, "operator")]) {
      fs.mkdirSync(dir, { recursive: true });
    }
  });

  test.beforeEach(async ({ page, context }, testInfo) => {
    await page.setViewportSize({ width: 1280, height: 800 });
    page.on("dialog", (dialog) => dialog.dismiss().catch(() => {}));
    if (testInfo.title.includes("signed out")) return;
    await page.addInitScript((t) => {
      localStorage.setItem("sanshain_token", t);
      document.cookie = `sanshain_token=${t};path=/;max-age=3600`;
      localStorage.setItem("sanshain_theme", "light");
      window.SANSHAIN_FAST_SCREENSHOT = true;
    }, token);
    await context.addCookies([
      { name: "sanshain_token", value: token, domain: "127.0.0.1", path: "/" },
      { name: "sanshain_token", value: token, domain: "localhost", path: "/" },
    ]);
  });

  const loaderGone = async (page) => {
    await page.waitForSelector("#app-loader", { state: "hidden", timeout: 10000 }).catch(() =>
      page.evaluate(() => {
        const loader = document.getElementById("app-loader");
        if (loader) loader.style.display = "none";
      }),
    );
  };

  const setTheme = async (page, theme) => {
    await page.evaluate((t) => {
      document.cookie = `sanshain_theme=${t};path=/;max-age=31536000;SameSite=Lax`;
      localStorage.setItem("sanshain_theme", t);
      document.documentElement.classList.toggle("dark", t === "dark");
      if (window.applySockeGimmick) window.applySockeGimmick(t);
    }, theme);
    await page.waitForTimeout(400);
  };

  const section = (page, title) =>
    page.locator("section", { has: page.locator(`h2:text-is("${title}")`) });

  // ---------------------------------------------------------------- developer

  test("developer 03: the graph shows orders pinning payments", async ({ page }) => {
    await page.goto("/graph.html", { waitUntil: "domcontentloaded" });
    await loaderGone(page);
    await page.waitForSelector("g.graph-node", { timeout: 60000 });
    await page.waitForTimeout(4000);
    await page.screenshot({ path: dev("03-graph.png") });
    await setTheme(page, "dark");
    await page.waitForTimeout(1500);
    await page.screenshot({ path: dev("03-graph-dark.png") });
    await setTheme(page, "light");
  });

  test("developer 06 and 10: the Producer page with its version line and catalog context", async ({
    page,
  }) => {
    await page.goto("/producers.html?service=payments", { waitUntil: "domcontentloaded" });
    await loaderGone(page);
    await page.waitForFunction(() => document.body.innerText.includes("2.0.0"), {
      timeout: 30000,
    });
    await page.waitForSelector("[data-context='owner']", { timeout: 30000 });
    await page.waitForTimeout(1200);
    await page.screenshot({ path: dev("06-producer-versions.png") });
    const context = page
      .locator("[data-context='owner']")
      .locator("xpath=ancestor::div[contains(@class,'rounded-xl')][1]");
    await context.screenshot({ path: dev("10-catalog-context.png") });
    await context.screenshot({ path: img("producer_context.png") });
    const webhooks = page
      .locator("[data-webhook-panel]")
      .locator("xpath=ancestor::div[contains(@class,'rounded-xl')][1]");
    await webhooks.waitFor({ state: "visible", timeout: 20000 });
    await webhooks.screenshot({ path: img("producer_webhooks_card.png") });
  });

  test("developer 08: one endpoint's version history in the spec viewer", async ({ page }) => {
    await page.goto("/yaml.html?service=payments&api_type=openapi&path=/payments&method=POST", {
      waitUntil: "domcontentloaded",
    });
    await loaderGone(page);
    await page.locator("text=openapi:").first().waitFor({ timeout: 60000 });
    await page.waitForTimeout(2000);
    await page.screenshot({ path: dev("08-history.png") });
  });

  test("developer 09: the reports page with the branch in the scope selector", async ({ page }) => {
    await page.goto("/reports.html", { waitUntil: "domcontentloaded" });
    await loaderGone(page);
    const scope = page.locator("#report-scope");
    await scope.waitFor({ timeout: 20000 });
    await scope.selectOption("release-2026.10");
    await page.waitForTimeout(1500);
    await page.screenshot({ path: dev("09-report.png") });
    await scope.selectOption("dev");
    await page.waitForTimeout(800);
    await page.screenshot({ path: img("reports.png") });
  });

  test("chapter: the graph node panel with the catalog context", async ({ page }) => {
    await page.goto("/graph.html", { waitUntil: "domcontentloaded" });
    await loaderGone(page);
    const node = page.locator("g.graph-node[data-node='payments']");
    await node.waitFor({ timeout: 60000 });
    await page.waitForTimeout(3000);
    await node.hover();
    await page.waitForSelector("#graph-tooltip", { state: "visible", timeout: 10000 });
    await page.waitForTimeout(800);
    await page.screenshot({ path: img("graph_node_panel.png") });
  });

  test("chapter: the consumers list", async ({ page }) => {
    await page.goto("/consumers.html", { waitUntil: "domcontentloaded" });
    await loaderGone(page);
    await page.waitForSelector(".endpoint-card", { timeout: 20000 });
    await page.waitForTimeout(800);
    await page.screenshot({ path: img("consumers_list.png") });
  });

  test("chapter: the account page with the read-only box and the token list", async ({ page }) => {
    await page.goto("/account.html", { waitUntil: "domcontentloaded" });
    await loaderGone(page);
    await page.waitForSelector("#account-dashboard:not(.hidden)", { timeout: 20000 });
    await page.waitForSelector("[data-token-scope]", { timeout: 20000 });
    await page.locator("#token-form").scrollIntoViewIfNeeded();
    await page.waitForTimeout(800);
    await page.screenshot({ path: img("account_tokens.png") });
  });

  // ----------------------------------------------------------------- operator

  test("operator 02: the sign-in form, signed out", async ({ page, context }) => {
    await context.clearCookies();
    await page.goto("/account.html", { waitUntil: "domcontentloaded" });
    await page.evaluate(() => localStorage.clear());
    await page.reload({ waitUntil: "domcontentloaded" });
    await loaderGone(page);
    await page.waitForSelector("#auth-screen:not(.hidden)", { timeout: 20000 });
    await page.waitForTimeout(500);
    await page.screenshot({ path: ops("02-login.png") });
  });

  test("operator 04 to 08 and the admin overview", async ({ page }) => {
    await page.goto("/admin.html#general", { waitUntil: "domcontentloaded" });
    await loaderGone(page);
    await page.waitForSelector("#admin-dashboard:not(.hidden)", { timeout: 30000 });
    await page.waitForSelector("#db-snapshot-btn", { timeout: 30000 });
    await page.waitForSelector("[data-webhook-list]", { timeout: 30000 });
    await page.waitForTimeout(1500);
    await page.screenshot({ path: img("admin_overview.png") });

    const webhooks = page.locator("#webhooks-section");
    await webhooks.scrollIntoViewIfNeeded();
    await page.waitForTimeout(500);
    await webhooks.screenshot({ path: ops("06-webhooks.png") });
    await webhooks.screenshot({ path: img("admin_webhooks.png") });
    await section(page, "Snapshot Cleanup").screenshot({ path: ops("07-snapshot-cleanup.png") });
    await section(page, "Database Configuration").screenshot({ path: ops("08-database.png") });

    // The Users tab lists the pending accounts only; the approved chip shows
    // maria and root.
    await page.goto("/admin.html#users", { waitUntil: "domcontentloaded" });
    await page.click('[data-user-chip="status:approved"]');
    await page.waitForFunction(
      () => document.getElementById("users-list").innerText.includes("maria"),
      { timeout: 30000 },
    );
    await page.waitForTimeout(500);
    await section(page, "Users").screenshot({ path: ops("04-users.png") });
  });

  test("operator 09: the audit timeline", async ({ page }) => {
    await page.goto("/audit.html", { waitUntil: "domcontentloaded" });
    await loaderGone(page);
    await page.waitForSelector(".timeline-item", { timeout: 20000 }).catch(() => {});
    await page.waitForTimeout(1500);
    await page.screenshot({ path: ops("09-audit.png") });
  });

  test("operator 10: the observability page", async ({ page }) => {
    await page.goto("/observability.html", { waitUntil: "domcontentloaded" });
    await loaderGone(page);
    await page.waitForSelector("#stat-uptime", { timeout: 20000 });
    await page.waitForTimeout(2500);
    await page.screenshot({ path: ops("10-observability.png") });
  });
});
