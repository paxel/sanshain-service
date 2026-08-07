import { test, expect } from "@playwright/test";
import path from "path";

test.describe("Sanshain Screenshot Capture", () => {
  const screenshotDir = "docs/images";
  const token = process.env.SANSHAIN_TOKEN;

  test.beforeEach(async ({ page, context }, testInfo) => {
    // Set viewport size for high-quality screenshots
    await page.setViewportSize({ width: 1440, height: 900 });

    // Enable console logging
    page.on("console", (msg) => console.log(`[BROWSER ${testInfo.title}]: ${msg.text()}`));

    // Public pages don't need auth
    const isPublic =
      testInfo.title.includes("landing") ||
      testInfo.title.includes("login") ||
      testInfo.title.includes("register");

    if (!isPublic) {
      // Inject token and flags via Init Script (runs before any other script)
      await page.addInitScript((t) => {
        if (t) {
          localStorage.setItem("sanshain_token", t);
          document.cookie = `sanshain_token=${t};path=/;max-age=3600`;
        }
        localStorage.setItem("sanshain_theme", "light");
        window.SANSHAIN_FAST_SCREENSHOT = true;
      }, token);

      // Also inject cookie via context for API calls from common.js
      if (token) {
        await context.addCookies([
          {
            name: "sanshain_token",
            value: token,
            domain: "localhost",
            path: "/",
          },
        ]);
      }
    }
  });

  const ensureLoaderHidden = async (page) => {
    // Fast check for loader hidden state
    try {
      await page.waitForSelector("#app-loader", { state: "hidden", timeout: 5000 });
    } catch (e) {
      // Force hide if it's stuck
      await page.evaluate(() => {
        const loader = document.getElementById("app-loader");
        if (loader) {
          loader.classList.add("hidden");
          loader.style.display = "none";
        }
      });
    }
  };

  const setTheme = async (page, theme) => {
    await page.evaluate((t) => {
      document.cookie = `sanshain_theme=${t};path=/;max-age=31536000;SameSite=Lax`;
      if (t === "dark") {
        document.documentElement.classList.add("dark");
      } else {
        document.documentElement.classList.remove("dark");
      }
      // Re-trigger gimmicks if any
      if (window.applySockeGimmick) window.applySockeGimmick(t);
    }, theme);
    await page.waitForTimeout(500); // Wait for theme transition
  };

  test("Capture landing page", async ({ page }) => {
    await page.goto("/", { waitUntil: "domcontentloaded" });
    await page.waitForSelector('img[alt="Sanshain Logo"]');
    await page.waitForTimeout(1000);
    await page.screenshot({ path: path.join(screenshotDir, "landing.png") });

    // Dark mode landing
    await setTheme(page, "dark");
    await page.waitForTimeout(1000);
    await page.screenshot({ path: path.join(screenshotDir, "landing_dark.png") });
  });

  test("Capture login page", async ({ page, context }) => {
    // Clear cookies and localStorage for this test to show login
    await context.clearCookies();
    await page.goto("/account.html", { waitUntil: "domcontentloaded" });
    await page.evaluate(() => localStorage.clear());
    await page.reload({ waitUntil: "domcontentloaded" });

    await ensureLoaderHidden(page);
    await page.waitForSelector("#auth-screen:not(.hidden)");
    await page.screenshot({ path: path.join(screenshotDir, "login.png") });

    // Switch to register
    await page.click("#tab-register");
    await page.waitForTimeout(500);
    await page.screenshot({ path: path.join(screenshotDir, "register.png") });
  });

  test("Capture user dashboard", async ({ page }) => {
    await page.goto("/account.html", { waitUntil: "domcontentloaded" });
    await ensureLoaderHidden(page);
    await page.waitForSelector("#account-dashboard:not(.hidden)");
    await page.screenshot({ path: path.join(screenshotDir, "user_dashboard.png") });
  });

  test("Capture services and drill-down", async ({ page }) => {
    await page.goto("/producers.html", { waitUntil: "domcontentloaded" });
    await ensureLoaderHidden(page);
    await page.waitForSelector(".endpoint-card", { timeout: 30000 });
    await page.screenshot({ path: path.join(screenshotDir, "producers_list.png") });

    // Click on config-service (from demo2)
    const svcCard = page.locator(".endpoint-card", { hasText: "config-service" }).first();
    await svcCard.click();
    await page.waitForFunction(() => document.body.innerText.includes("config-service"), {
      timeout: 20000,
    });
    await page.waitForTimeout(1000);
    await page.screenshot({ path: path.join(screenshotDir, "producer_versions.png") });

    // Click on the newest version entry of the timeline
    const versionCard = page.locator(".endpoint-card", { hasText: /\d+\.\d+\.\d+/ }).first();
    await versionCard.click();
    await page.waitForSelector(".endpoint-card", { timeout: 30000 });
    await page.waitForTimeout(1000);
    await page.screenshot({ path: path.join(screenshotDir, "version_endpoints.png") });
  });

  test("Capture YAML viewer", async ({ page }) => {
    test.setTimeout(300000); // 5 minutes
    // Navigate directly to a YAML view; without a version param the viewer
    // loads the endpoint's blame trail and selects the newest version.
    await page.goto("/yaml.html?service=ml-inference&api_type=openapi&path=/predict&method=POST", {
      waitUntil: "domcontentloaded",
    });
    await ensureLoaderHidden(page);

    // Optional network idle, don't fail if it doesn't happen
    await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});

    // Wait for content that indicates YAML is loaded - use locator for more stability
    await page.locator("text=openapi:").first().waitFor({ timeout: 60000 });

    await page.waitForTimeout(3000);
    await page.screenshot({ path: path.join(screenshotDir, "yaml_viewer.png") });
  });

  test("Capture dependency graph", async ({ page }) => {
    test.setTimeout(300000); // 5 minutes
    await page.goto("/graph.html", { waitUntil: "domcontentloaded" });
    await ensureLoaderHidden(page);
    await page.waitForSelector("#custom-graph", { timeout: 30000 });

    // Wait for ANY graph node
    await page.waitForSelector("g.graph-node", { timeout: 120000 });

    // Give it plenty of time for the layout to stabilize and zoom-to-fit
    await page.waitForTimeout(15000);
    await page.screenshot({ path: path.join(screenshotDir, "graph.png") });

    /*
    // Click on a node to show details - use dispatchEvent for more reliability on SVG
    const node = page.locator('g.graph-node').filter({ hasText: /api-middleware/i }).first();
    if (await node.isVisible()) {
        await node.evaluate(el => el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true })));
        await page.waitForSelector('.node-popup', { state: 'visible', timeout: 30000 });
        await page.waitForTimeout(2000);
        await page.screenshot({ path: path.join(screenshotDir, 'graph_details.png') });
        // Close popup by clicking outside
        await page.mouse.click(10, 10);
        await page.waitForTimeout(1000);
    }
    */

    // Horizontal mode
    await page.click("#graph-direction-toggle");
    await page.waitForTimeout(5000);
    await page.screenshot({ path: path.join(screenshotDir, "graph_horizontal.png") });

    // Dark mode
    await setTheme(page, "dark");
    await page.waitForTimeout(2000);
    await page.screenshot({ path: path.join(screenshotDir, "graph_dark.png") });
  });

  test("Capture release graphs (main, branch, timeline, diff)", async ({ page }) => {
    test.setTimeout(300000);
    await page.goto("/graph.html", { waitUntil: "domcontentloaded" });
    await ensureLoaderHidden(page);
    await page.waitForSelector("#custom-graph", { timeout: 30000 });
    await page.waitForSelector("g.graph-node", { timeout: 120000 });

    // Toolbar crop (Focus, Dev/Main toggle, branch selector, filters) — the
    // cropped shots keep the controls legible on mobile-width doc pages.
    await page.locator("#graph-toolbar-row").screenshot({
      path: path.join(screenshotDir, "graph_toolbar_crop.png"),
    });

    // Main view: trunk pin set, red major-lag conflict, dangling pin.
    await page.click("#stream-main");
    await page.waitForTimeout(8000);
    await page.screenshot({ path: path.join(screenshotDir, "graph_main.png") });
    await page.locator("#graph-legend").screenshot({
      path: path.join(screenshotDir, "graph_legend_main_crop.png"),
    });

    // Timeline slider (visible in the main and branch views).
    const timelineRow = page.locator("#graph-timeline-row");
    if (await timelineRow.isVisible()) {
      await timelineRow.screenshot({
        path: path.join(screenshotDir, "graph_timeline_crop.png"),
      });
    }

    // Branch view: the release cut's pin set, hotfix + dangling included.
    await page.selectOption("#graph-branch-select", "release-maribou");
    await page.waitForTimeout(8000);
    await page.screenshot({ path: path.join(screenshotDir, "graph_branch.png") });

    // Compare: branch vs main, structured diff rendered below the panel.
    await page.selectOption("#diff-left", "release-maribou");
    await page.selectOption("#diff-right", "main");
    await page.click('button:has-text("Diff")');
    await page.waitForFunction(
      () => document.getElementById("graph-diff-output").innerText.trim().length > 0,
      { timeout: 20000 },
    );
    await page.waitForTimeout(500);
    // One crop spanning the Compare controls and the diff output under them.
    const compareBox = await page.locator("#graph-compare-row").boundingBox();
    const outputBox = await page.locator("#graph-diff-output").boundingBox();
    if (compareBox && outputBox) {
      await page.screenshot({
        path: path.join(screenshotDir, "graph_diff_crop.png"),
        clip: {
          x: compareBox.x,
          y: compareBox.y,
          width: Math.max(compareBox.width, outputBox.width),
          height: outputBox.y + outputBox.height - compareBox.y,
        },
      });
    }
  });

  test("Capture producer branch chips", async ({ page }) => {
    await page.goto("/producers.html", { waitUntil: "domcontentloaded" });
    await ensureLoaderHidden(page);
    await page.waitForSelector(".endpoint-card", { timeout: 30000 });
    const svcCard = page.locator(".endpoint-card", { hasText: "notification-hub" }).first();
    await svcCard.click();
    await page.waitForFunction(() => document.body.innerText.includes("notification-hub"), {
      timeout: 20000,
    });
    await page.waitForTimeout(1500);
    await page.screenshot({ path: path.join(screenshotDir, "producer_branches.png") });
    // Crop the newest version card: stability, trunk badge, branch chips.
    const versionCard = page.locator(".endpoint-card", { hasText: /\d+\.\d+\.\d+/ }).first();
    if (await versionCard.isVisible()) {
      await versionCard.screenshot({
        path: path.join(screenshotDir, "producer_version_card_crop.png"),
      });
    }
  });

  test("Capture report scope selector", async ({ page }) => {
    await page.goto("/reports.html", { waitUntil: "domcontentloaded" });
    await ensureLoaderHidden(page);
    const scope = page.locator("#report-scope");
    await scope.waitFor({ timeout: 15000 });
    const box = await scope.boundingBox();
    if (box) {
      await page.screenshot({
        path: path.join(screenshotDir, "report_scope_crop.png"),
        clip: {
          x: Math.max(0, box.x - 220),
          y: Math.max(0, box.y - 16),
          width: Math.min(box.width + 760, 1440 - Math.max(0, box.x - 220)),
          height: box.height + 32,
        },
      });
    }
  });

  test("Capture audit timeline", async ({ page }) => {
    await page.goto("/audit.html", { waitUntil: "domcontentloaded" });
    await ensureLoaderHidden(page);

    // Wait for entries or error or empty message
    await page
      .waitForFunction(
        () => {
          return (
            !!document.querySelector(".timeline-item") ||
            document.body.innerText.includes("No specification updates found") ||
            document.body.innerText.includes("Error loading timeline")
          );
        },
        { timeout: 15000 },
      )
      .catch(() => console.log("Audit timeline timeout, proceeding anyway"));

    await page.waitForTimeout(2000);
    await page.screenshot({ path: path.join(screenshotDir, "audit.png") });

    // Filter bar crop — date range, action type, wildcards and the Stream
    // filter (trunk / branch tag) in one mobile-friendly strip.
    await page.locator("#audit-filters").screenshot({
      path: path.join(screenshotDir, "audit_filters_crop.png"),
    });

    // Open a diff
    const diffBtn = page.locator('button:has-text("View Changes")').first();
    if (await diffBtn.isVisible()) {
      await diffBtn.click();
      await page
        .waitForSelector(".d2h-wrapper", { state: "visible", timeout: 10000 })
        .catch(() => {});
      await page.waitForTimeout(1000);
      await page.screenshot({ path: path.join(screenshotDir, "audit_diff.png") });
    }

    // Dark mode audit
    await setTheme(page, "dark");
    await page.waitForTimeout(2000);
    await page.screenshot({ path: path.join(screenshotDir, "audit_dark.png") });
  });

  test("Capture reports and observability", async ({ page }) => {
    await page.goto("/reports.html", { waitUntil: "domcontentloaded" });
    await ensureLoaderHidden(page);

    // Dashboard view
    await page
      .waitForSelector('h3:has-text("Full Dependency Report")', { timeout: 15000 })
      .catch(async () => {
        console.log("Reports dashboard timeout, forcing unhide");
        await page.evaluate(() => {
          const dash = document.getElementById("report-dashboard");
          if (dash) dash.classList.remove("hidden");
        });
      });

    await page.waitForTimeout(2000);
    await page.screenshot({ path: path.join(screenshotDir, "reports_dashboard.png") });

    // Actual report view
    const viewReportBtn = page.locator("text=View Markdown Report");
    if (await viewReportBtn.isVisible()) {
      await viewReportBtn.click();
      await page.waitForSelector("#report-container table", { timeout: 20000 }).catch(() => {});
      await page.waitForTimeout(2000);
      await page.screenshot({ path: path.join(screenshotDir, "reports.png") });
    }

    await page.goto("/observability.html", { waitUntil: "domcontentloaded" });
    await ensureLoaderHidden(page);
    await page.waitForSelector('h3:has-text("Health Check")', { timeout: 15000 }).catch(() => {});
    await page.waitForTimeout(2000);
    await page.screenshot({ path: path.join(screenshotDir, "observability.png") });
  });

  test("Capture clients list", async ({ page }) => {
    await page.goto("/consumers.html", { waitUntil: "domcontentloaded" });
    await ensureLoaderHidden(page);
    await page.waitForSelector(".endpoint-card", { timeout: 20000 });
    await page.screenshot({ path: path.join(screenshotDir, "consumers_list.png") });

    // Click on a client to show its requirements
    const clientCard = page.locator(".endpoint-card", { hasText: "InventoryService" }).first();
    if (await clientCard.isVisible()) {
      await clientCard.click();
      await page.waitForSelector('span:has-text("InventoryService")');
      await page.waitForTimeout(1000);
      await page.screenshot({ path: path.join(screenshotDir, "client_details.png") });
    }
  });

  test("Capture admin dashboard sections", async ({ page }) => {
    await page.goto("/admin.html", { waitUntil: "domcontentloaded" });
    await ensureLoaderHidden(page);
    await page.waitForSelector("#admin-dashboard:not(.hidden)", { timeout: 30000 });

    // General overview
    await page.screenshot({ path: path.join(screenshotDir, "admin_overview.png") });

    // Scroll to Users section - use exact match to avoid ambiguity
    const usersHeader = page.locator("h2", { hasText: /^Users$/ });
    await usersHeader.scrollIntoViewIfNeeded({ timeout: 10000 });
    await page.waitForTimeout(1000);
    await page.screenshot({ path: path.join(screenshotDir, "admin_users.png") });

    // Scroll to Snapshot Cleanup
    const cleanupHeader = page.locator("h2", { hasText: /^Snapshot Cleanup$/ });
    await cleanupHeader.scrollIntoViewIfNeeded({ timeout: 10000 });
    await page.waitForTimeout(1000);
    await page.screenshot({ path: path.join(screenshotDir, "admin_snapshot_cleanup.png") });

    // Scroll to Authentication
    const authHeader = page.locator("h2", { hasText: /^Authentication$/ });
    await authHeader.scrollIntoViewIfNeeded({ timeout: 10000 });
    // Switch to LDAP to show fields
    await page.click('input[value="ldap"]');
    await page.waitForSelector("#ldap-config-form:not(.hidden)", { timeout: 10000 });
    await page.waitForTimeout(1000);
    await page.screenshot({ path: path.join(screenshotDir, "admin_auth_ldap.png") });
  });
});
