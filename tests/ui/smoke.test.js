import { test, expect } from "@playwright/test";

// The live-update stream stays open by design, so "networkidle" would never
// arrive: these tests get an empty stream, which the page retries quietly.
test.beforeEach(async ({ page }) => {
  await page.route("**/api/sse/updates", (route) =>
    route.fulfill({ status: 200, contentType: "text/event-stream", body: "" }),
  );
});

test.describe("Sanshain UI Smoke Test", () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

  test("Landing page loads and has correct title", async ({ page }) => {
    await page.goto("/");
    await expect(page).toHaveTitle(/Sanshain/);

    // Check for critical logo
    const logo = page.locator('img[alt="Sanshain Logo"]').first();
    await expect(logo).toBeVisible();

    // Verify logo dimensions are not broken (intrinsic vs display)
    const box = await logo.boundingBox();
    expect(box.height).toBeLessThan(100); // Should be h-9 (~36px)
  });

  test("Login and Admin settings persistence", async ({ page }) => {
    // Clear storage before starting to avoid stale session or banner issues
    await page.goto("/");
    await page.evaluate(() => {
      sessionStorage.clear();
      localStorage.clear();
    });

    // Handle ANY unexpected dialogs by dismissing them to prevent hangs
    page.on("dialog", async (dialog) => {
      console.log(`[UI Test] Auto-dismissing dialog: ${dialog.message()}`);
      await dialog.dismiss();
    });

    // Explicitly check for and dismiss the reload banner if it's a DOM element
    const dismissReloadBanner = async () => {
      const banner = page.locator("#sanshain-reload-banner");
      if (await banner.isVisible()) {
        await banner.locator('button:has-text("×")').click();
      }
    };

    await page.goto("/account.html");
    await dismissReloadBanner();

    // Login - Use specific selectors to avoid ambiguity with the "Sign In" tab
    if (!adminPassword) {
      throw new Error("INITIAL_ADMIN_PASSWORD environment variable is required for tests");
    }
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');

    // Wait for redirect or UI change with better error info
    try {
      await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });
    } catch (e) {
      const errorText = await page.locator("#login-error").textContent();
      if (errorText && errorText.trim().length > 0) {
        throw new Error(`Login failed with error: ${errorText}`);
      }
      throw e;
    }
    await expect(page.locator("#banner-username")).toContainText("root");

    // Navigate to Admin
    await page.goto("/admin.html");
    await dismissReloadBanner();
    await page.waitForSelector("#admin-dashboard");

    // Local shows its two switches; Dev is greyed out without the server's gate.
    await expect(page.locator('input[name="auth-mode"][value="local"]')).toBeChecked();
    await expect(page.locator("#registration-toggle")).toBeVisible();
    await expect(page.locator("#auto-approve-toggle")).toBeVisible();
    await expect(page.locator('input[name="auth-mode"][value="dev"]')).toBeDisabled();
    await expect(page.locator("#dev-mode-hint")).toBeVisible();

    // Switch to Off (maintenance) and back; the admin stays signed in throughout.
    await page.locator('input[name="auth-mode"][value="disabled"]').click();
    await expect(page.locator("#local-users-panel")).toBeHidden();
    await page.locator('button:has-text("Save Authentication Settings")').click();
    await page.waitForTimeout(1000); // Wait for API call

    // Refresh to verify persistence.
    // The radios are populated by loadAuthConfig(), an async fetch that resolves
    // *after* #admin-dashboard is rendered, so reading isChecked() once races it.
    // toBeChecked() auto-retries until the fetch lands.
    await page.reload();
    await page.waitForSelector("#admin-dashboard");
    await expect(page.locator('input[name="auth-mode"][value="disabled"]')).toBeChecked();

    // Restore Local mode
    await page.locator('input[name="auth-mode"][value="local"]').click();
    await page.locator('button:has-text("Save Authentication Settings")').click();
    await page.waitForTimeout(500);
  });

  test("Audit nav link requires the audit permission", async ({ page }) => {
    // The audit timeline needs `view_audit` on the server. The nav link must not
    // advertise it to visitors who would be refused.
    if (!adminPassword) {
      throw new Error("INITIAL_ADMIN_PASSWORD environment variable is required for tests");
    }

    page.on("dialog", async (dialog) => await dialog.dismiss());

    // Signed out: hidden.
    await page.goto("/producers.html");
    await page.evaluate(() => {
      sessionStorage.clear();
      localStorage.clear();
    });
    await page.reload();
    await expect(page.locator("#nav-audit-link")).toBeHidden();

    // Signed in as an admin: visible.
    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });

    await page.goto("/producers.html");
    await expect(page.locator("#nav-audit-link")).toBeVisible();

    // Signed in WITHOUT the permission: hidden. This is the case the change
    // exists for, and the only one the other two cannot catch — a regression
    // that ignored permissions in the signed-in branch would still pass both.
    //
    // The banner state is driven directly rather than by registering a second
    // account, so the assertion does not depend on the registration or
    // auto-approve settings. That /auth/me reports permissions correctly is a
    // separate concern, covered server-side.
    await page.evaluate(() => window.updateBannerAuth({ username: "plain", permissions: [] }));
    await expect(page.locator("#nav-audit-link")).toBeHidden();

    // ...and the same call with the permission brings it back, so the assertion
    // above is about the permission and not about the call having no effect.
    await page.evaluate(() =>
      window.updateBannerAuth({ username: "root", permissions: ["view_audit"] }),
    );
    await expect(page.locator("#nav-audit-link")).toBeVisible();

    // A partial administrator — someone who administers users but may not read
    // the audit trail — still does not get the link. This is the arrangement the
    // permission model makes possible and the old all-or-nothing gate could not
    // express: holding *an* administrative permission is no longer the same as
    // holding this one.
    await page.evaluate(() =>
      window.updateBannerAuth({ username: "manager", permissions: ["manage_users"] }),
    );
    await expect(page.locator("#nav-audit-link")).toBeHidden();
  });
});

// The viewer renders an endpoint of one version-line entry. Immutability is
// absolute in 2.0: there is no editor, so the page must offer none.
test.describe("Endpoint viewer", () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

  const SERVICE = "viewer-fixture";
  const YAML_URL = `/yaml.html?service=${SERVICE}&api_type=openapi&version=1.0.0&path=%2Fhello&method=GET`;

  const SPEC = [
    "openapi: 3.0.0",
    "info:",
    "  title: Viewer Fixture",
    "  version: 1.0.0",
    "paths:",
    "  /hello:",
    "    get:",
    "      responses:",
    "        '200':",
    "          description: OK",
    "",
  ].join("\n");

  test.beforeAll(async ({ request }) => {
    if (!adminPassword) {
      throw new Error("INITIAL_ADMIN_PASSWORD environment variable is required for tests");
    }
    const login = await request.post("/auth/login", {
      data: { username: "root", password: adminPassword },
    });
    const { token } = await login.json();

    // Seed one version for the viewer to open.
    await request.post("/provide", {
      headers: { Authorization: `Bearer ${token}` },
      data: { producername: SERVICE, stability: "ga", openapi_yaml: SPEC },
    });
  });

  test("the viewer shows the version history of the endpoint", async ({ page }) => {
    page.on("dialog", async (d) => await d.dismiss());
    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });

    await page.goto(YAML_URL);
    const card = page.locator("#versions-list-container .version-card", { hasText: "1.0.0" });
    await expect(card).toBeVisible({ timeout: 10000 });
    await expect(card).toContainText("GA");
    // Immutability is absolute: no edit affordance exists anywhere.
    await expect(page.locator("#edit-btn")).toHaveCount(0);
  });
});

test.describe("Role, group and maintainer management", () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

  test.beforeEach(async ({ page }) => {
    if (!adminPassword) {
      throw new Error("INITIAL_ADMIN_PASSWORD environment variable is required for tests");
    }
    page.on("dialog", async (dialog) => await dialog.accept());
    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });
    await page.goto("/admin.html");
    await expect(page.locator("#admin-dashboard")).toBeVisible({ timeout: 10000 });
  });

  test("the management sections render", async ({ page }) => {
    await expect(page.locator("#users-list")).toBeVisible();
    await expect(page.locator("#groups-list")).toBeVisible();
    await expect(page.locator("#maintainers-list")).toBeVisible();
    // Root is configuration-held; the page must not offer to change that.
    await expect(page.locator("#root-note")).toContainText("cannot be granted or revoked here");
  });

  // The round trip that matters: a group created here can carry a role, and the
  // origin badge distinguishes it from one mirrored from the directory.
  test("a group can be created and given a role", async ({ page }) => {
    const name = "ui-test-group";
    await page.fill("#new-group-name", name);
    await page.click("#create-group-btn");

    const row = page.locator("#groups-list > div", { hasText: name });
    await expect(row).toBeVisible({ timeout: 10000 });
    await expect(row).toContainText("sanshain");
    await expect(row).toContainText("no roles attached");

    // The card carries two selects since 2.0 (role picker + member picker);
    // target the role one explicitly.
    await row.locator('select[id^="group-role-"]').selectOption("viewer");
    await row.locator('select[id^="group-role-"] ~ button:has-text("Add")').click();
    await expect(page.locator("#groups-list > div", { hasText: name })).toContainText("viewer", {
      timeout: 10000,
    });
  });

  test("the root account shows its admin role and offers no delete", async ({ page }) => {
    const row = page.locator("#users-list > div", { hasText: "root" });
    await expect(row).toBeVisible();
    await expect(row).toContainText("admin");
    await expect(row.locator('button:has-text("Delete")')).toHaveCount(0);
  });
});

test.describe("One-click promote", () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

  const SERVICE = "promote-fixture";
  const SPEC = [
    "openapi: 3.0.0",
    "info:",
    "  title: Promote Fixture",
    "  version: 1.0.0",
    "paths:",
    "  /release-me:",
    "    get:",
    "      responses:",
    "        '200':",
    "          description: OK",
    "",
  ].join("\n");

  test.beforeAll(async ({ request }) => {
    if (!adminPassword) {
      throw new Error("INITIAL_ADMIN_PASSWORD environment variable is required for tests");
    }
    const login = await request.post("/auth/login", {
      data: { username: "root", password: adminPassword },
    });
    const { token } = await login.json();
    await request.post("/provide", {
      headers: { Authorization: `Bearer ${token}` },
      data: {
        producername: SERVICE,
        stability: "snapshot",
        openapi_yaml: SPEC,
        change_url: "https://github.com/acme/promote/pull/12",
        tickets: ["PRM-12"],
        note: "Release-me endpoint for the promote flow",
      },
    });
    await request.post("/provide", {
      headers: { Authorization: `Bearer ${token}` },
      data: {
        producername: `${SERVICE}-admin`,
        stability: "snapshot",
        openapi_yaml: SPEC,
      },
    });
    await request.post("/admin/settings/ticket-url-template", {
      headers: { Authorization: `Bearer ${token}` },
      data: { template: "https://jira.acme.corp/browse/{id}" },
    });
  });

  test("a version row shows the origin of its build with the ticket linked", async ({ page }) => {
    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });
    await page.goto(`/producers.html?service=${SERVICE}`);
    const card = page.locator(".endpoint-card", { hasText: "1.0.0" });
    await expect(card).toBeVisible({ timeout: 10000 });
    const origin = card.locator("[data-origin]");
    await expect(origin.locator('[data-origin="note"]')).toHaveText(
      "Release-me endpoint for the promote flow",
    );
    await expect(origin.locator('[data-context-link="change"]')).toHaveAttribute(
      "href",
      "https://github.com/acme/promote/pull/12",
    );
    const ticket = origin.locator('[data-context-link="ticket"]');
    await expect(ticket).toHaveText("PRM-12");
    await expect(ticket).toHaveAttribute("href", "https://jira.acme.corp/browse/PRM-12");
    // The admin page shows and saves the template; the diff caption carries the origin.
    await page.goto("/admin.html");
    await expect(page.locator("#ticket-url-template")).toHaveValue(
      "https://jira.acme.corp/browse/{id}",
      { timeout: 10000 },
    );
  });

  // The happy path decided in #28: see the button on a snapshot row, confirm,
  // watch the badge flip to GA. Releasing runs through the same GA gate as any
  // release, so a green run here also proves the gate admits the privileged user.
  test("a releaser promotes a snapshot from the version timeline", async ({ page }) => {
    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });

    await page.goto(`/producers.html?service=${SERVICE}`);
    const card = page.locator(".endpoint-card", { hasText: "1.0.0" });
    await expect(card).toBeVisible({ timeout: 10000 });
    await expect(card).toContainText("Snapshot");

    await card.locator('button:has-text("Promote to GA")').click();
    await expect(page.locator("#confirm-modal")).toBeVisible();
    await expect(page.locator("#confirm-message")).toContainText("permanently claimed");
    await page.click("#confirm-yes");

    const refreshed = page.locator(".endpoint-card", { hasText: "1.0.0" });
    await expect(refreshed).toContainText("GA", { timeout: 10000 });
    await expect(refreshed.locator('button:has-text("Promote to GA")')).toHaveCount(0);
  });

  // The admin dashboard's promote path re-renders the versions panel in place
  // (no toggle-button driving) — this covers the refresh the producers-page
  // test cannot reach.
  test("the admin dashboard promotes and re-renders the versions panel", async ({ page }) => {
    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });

    await page.goto("/admin.html");
    await expect(page.locator("#admin-dashboard")).toBeVisible({ timeout: 10000 });

    const card = page.locator("#services-list > div", { hasText: `${SERVICE}-admin` });
    await expect(card).toBeVisible({ timeout: 10000 });
    await card.locator('button:has-text("▶")').click();
    const row = card.locator(".versions-container > div", { hasText: "1.0.0" });
    await expect(row).toContainText("snapshot", { timeout: 10000 });

    await row.locator('button:has-text("Promote to GA")').click();
    await expect(page.locator("#confirm-modal")).toBeVisible();
    await page.click("#confirm-yes");

    const refreshedRow = card.locator(".versions-container > div", { hasText: "1.0.0" });
    await expect(refreshedRow).toContainText("GA", { timeout: 10000 });
    await expect(refreshedRow.locator('button:has-text("Promote to GA")')).toHaveCount(0);
  });
});

test.describe("Snapshot cleanup settings", () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

  test("the snapshot max age setting is on the dashboard", async ({ page }) => {
    if (!adminPassword) {
      throw new Error("INITIAL_ADMIN_PASSWORD environment variable is required for tests");
    }
    page.on("dialog", async (dialog) => await dialog.accept());
    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });

    await page.goto("/admin.html");
    await expect(page.locator("#admin-dashboard")).toBeVisible({ timeout: 10000 });

    // Use-based snapshot expiry is the 2.0 cleanup model; the setting and the
    // manual trigger live where an administrator will encounter them.
    await expect(page.locator("#snapshot-max-age-days")).toBeVisible();
    await expect(page.locator('button:has-text("Run Cleanup")').first()).toBeVisible();
  });
});

test.describe("Main graph view (ADR-0004)", () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;
  const SERVICE = "trunk-graph-svc";
  const CONSUMER = "trunk-graph-consumer";
  const SPEC = [
    "openapi: 3.0.3",
    "info:",
    "  title: Trunk Graph Fixture",
    "  version: 1.0.0",
    "paths:",
    "  /hello:",
    "    get:",
    "      responses:",
    "        '200':",
    "          description: OK",
  ].join("\n");

  test.beforeAll(async ({ request }) => {
    if (!adminPassword) {
      throw new Error("INITIAL_ADMIN_PASSWORD environment variable is required for tests");
    }
    const login = await request.post("/auth/login", {
      data: { username: "root", password: adminPassword },
    });
    const { token } = await login.json();
    await request.post("/provide", {
      headers: { Authorization: `Bearer ${token}` },
      data: { producername: SERVICE, stability: "snapshot", openapi_yaml: SPEC, trunk: true },
    });
    await request.get(
      `/require?consumername=${CONSUMER}&producername=${SERVICE}&version=1.0.0&path=/hello&method=GET&trunk=true`,
      { headers: { Authorization: `Bearer ${token}` } },
    );
  });

  test("the Main toggle shows the trunk pin set with the producer trunk version", async ({
    page,
  }) => {
    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });

    await page.goto("/graph.html");
    await expect(page.locator("#custom-graph")).toBeVisible({ timeout: 10000 });

    await page.click("#stream-main");
    // The trunk edge and both nodes are drawn, the producer carries its
    // trunk version label, and the main-only legend entry appears.
    await expect(page.locator("#custom-graph")).toContainText(SERVICE, { timeout: 10000 });
    await expect(page.locator("#custom-graph")).toContainText(CONSUMER);
    await expect(page.locator("#custom-graph")).toContainText("v1.0.0");
    await expect(page.locator('[data-legend-highlight="edge-flag:conflict"]')).toBeVisible();
    await expect(page.locator('[data-legend-highlight="edge-type:missing"]')).toBeHidden();

    // Back to Dev: legend flips back.
    await page.click("#stream-dev");
    await expect(page.locator('[data-legend-highlight="edge-flag:conflict"]')).toBeHidden();
    await expect(page.locator('[data-legend-highlight="edge-type:missing"]')).toBeVisible();
  });

  test("a sanshain-branch can be selected as a graph view", async ({ page, request }) => {
    // Cut a branch off the current trunk graph, then draw it.
    const login = await request.post("/auth/login", {
      data: { username: "root", password: adminPassword },
    });
    const { token } = await login.json();
    await request.post("/admin/branches", {
      headers: { Authorization: `Bearer ${token}` },
      data: { name: "Smoke Graph Branch" },
    });

    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });

    await page.goto("/graph.html");
    await expect(page.locator("#custom-graph")).toBeVisible({ timeout: 10000 });
    await page.selectOption("#graph-branch-select", "Smoke Graph Branch");
    await expect(page.locator("#custom-graph")).toContainText(SERVICE, { timeout: 10000 });
    await expect(page.locator('[data-legend-highlight="edge-flag:dangling"]')).toBeVisible();

    // The timeline bar appears for branch (and main) views; root is a
    // releaser, so the retroactive-cut affordance shows.
    await expect(page.locator("#graph-timeline-row")).toBeVisible();
    await expect(page.locator("#graph-create-branch-here")).toBeVisible();
    await page.click("#stream-main");
    await expect(page.locator("#graph-timeline-row")).toBeVisible();
    await page.click("#stream-dev");
    await expect(page.locator("#graph-timeline-row")).toBeHidden();

    // The compare panel diffs any two selections; identical sides say so.
    await page.selectOption("#diff-left", "main");
    await page.selectOption("#diff-right", "main");
    await page.click('button:has-text("Diff")');
    await expect(page.locator("#graph-diff-output")).toContainText("No differences", {
      timeout: 10000,
    });
  });
});

test.describe("Sanshain-branches admin (ADR-0005)", () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

  test("a branch can be created, renamed and deleted from the dashboard", async ({ page }) => {
    if (!adminPassword) {
      throw new Error("INITIAL_ADMIN_PASSWORD environment variable is required for tests");
    }
    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });

    await page.goto("/admin.html");
    await expect(page.locator("#branches-list")).toBeVisible({ timeout: 10000 });

    await page.fill("#new-branch-name", "Smoke Release");
    await page.click("#create-branch-btn");
    const row = page.locator("#branches-list div", { hasText: "Smoke Release" }).first();
    await expect(row).toBeVisible({ timeout: 10000 });

    page.once("dialog", (dialog) => dialog.accept("Smoke Release LTS"));
    await row.locator('button:has-text("Rename")').click();
    const renamed = page.locator("#branches-list div", { hasText: "Smoke Release LTS" }).first();
    await expect(renamed).toBeVisible({ timeout: 10000 });

    await renamed.locator('button:has-text("Delete")').click();
    await expect(page.locator("#confirm-modal")).toBeVisible();
    await expect(page.locator("#confirm-message")).toContainText("name is freed");
    await page.click("#confirm-yes");
    await expect(page.locator("#branches-list")).not.toContainText("Smoke Release LTS", {
      timeout: 10000,
    });
  });
});

test.describe("Escaping", () => {
  // A branch name is user-chosen and unrestricted, and lands inside quoted
  // HTML attributes (producers.html title="…", reports.html option value="…").
  // escapeHtml must therefore escape quotes as well as angle brackets — a
  // textContent→innerHTML round-trip alone does not, which let a crafted name
  // close the attribute and add its own event handler.
  test("escapeHtml escapes quotes, not only angle brackets", async ({ page }) => {
    await page.goto("/producers.html");
    await page.waitForFunction(() => typeof window.escapeHtml === "function");
    const escaped = await page.evaluate(() => window.escapeHtml('x" onmouseover="steal()'));
    expect(escaped).not.toContain('"');
    expect(escaped).toContain("&quot;");

    // And the escaped value really is inert when placed in an attribute.
    const handlers = await page.evaluate((value) => {
      const host = document.createElement("div");
      host.innerHTML = `<span title="${value}">x</span>`;
      const span = host.firstElementChild;
      return {
        attrs: span.getAttributeNames(),
        title: span.getAttribute("title"),
      };
    }, escaped);
    expect(handlers.attrs).toEqual(["title"]);
    expect(handlers.title).toBe('x" onmouseover="steal()');
  });
});

test.describe("Release-graph regressions (ai/improvements.md #28)", () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;
  const LONER = "reg28-loner-svc"; // trunk-provided, never pinned
  const PINNED = "reg28-pinned-svc"; // pinned by the consumer
  const DOOMED = "reg28-doomed-svc"; // its version gets deleted -> dangling
  const CONSUMER = "reg28-consumer";
  const spec = (title) =>
    [
      "openapi: 3.0.3",
      "info:",
      `  title: ${title}`,
      "  version: 1.0.0",
      "paths:",
      "  /thing:",
      "    get:",
      "      responses:",
      "        '200':",
      "          description: OK",
    ].join("\n");

  test.beforeAll(async ({ request }) => {
    if (!adminPassword) {
      throw new Error("INITIAL_ADMIN_PASSWORD environment variable is required for tests");
    }
    const login = await request.post("/auth/login", {
      data: { username: "root", password: adminPassword },
    });
    const { token } = await login.json();
    const auth = { Authorization: `Bearer ${token}` };
    for (const name of [LONER, PINNED, DOOMED]) {
      await request.post("/provide", {
        headers: auth,
        data: { producername: name, stability: "snapshot", openapi_yaml: spec(name), trunk: true },
      });
    }
    for (const name of [PINNED, DOOMED]) {
      await request.get(
        `/require?consumername=${CONSUMER}&producername=${name}&version=1.0.0&path=/thing&method=GET&trunk=true`,
        { headers: auth },
      );
    }
    // The doomed version disappears out from under its pin.
    const del = await request.delete(`/admin/producers/${DOOMED}/versions/openapi/1.0.0`, {
      headers: auth,
    });
    if (!del.ok()) throw new Error(`delete-version failed: ${del.status()}`);
  });

  async function loginAndOpenMainView(page) {
    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });
    await page.goto("/graph.html");
    await expect(page.locator("#custom-graph")).toBeVisible({ timeout: 10000 });
    await page.click("#stream-main");
    await expect(page.locator("#custom-graph")).toContainText(PINNED, { timeout: 10000 });
  }

  // #28.2: with a narrowing filter active, the main view must not re-inject
  // every trunk-provided producer as an isolated node.
  test("a focus filter hides unrelated trunk producers in the main view", async ({ page }) => {
    await loginAndOpenMainView(page);
    // Unfiltered: the never-pinned producer is injected on purpose.
    await expect(page.locator("#custom-graph")).toContainText(LONER);

    await page.fill("#graph-service-filter", PINNED);
    await page.press("#graph-service-filter", "Enter");
    await expect(page.locator("#custom-graph")).toContainText(PINNED);
    await expect(page.locator("#custom-graph")).not.toContainText(LONER, { timeout: 10000 });
  });

  // #28.3: a dangling pin keeps its own colour and dash — the messaging/
  // missing styling pass used to overwrite the dasharray afterwards.
  test("a dangling pin renders dotted orange in the main view", async ({ page }) => {
    await loginAndOpenMainView(page);
    // Scope to this test's own edge (CONSUMER -> DOOMED) rather than counting
    // danglings across the whole shared-server graph — another suite's dangling
    // pin must not be able to fail this one.
    const dangling = page.locator(
      `#custom-graph path.graph-edge[data-dangling="1"][data-from="${CONSUMER}"][data-to="${DOOMED}"]`,
    );
    await expect(dangling).toHaveCount(1, { timeout: 10000 });
    await expect(dangling).toHaveAttribute("stroke-dasharray", "2 5");
    await expect(dangling).toHaveAttribute("stroke", "#ea580c");
  });

  // #28.6: only the newest timeline request may apply; a slower, older
  // response landing last must not win. Deterministic replay of the race via
  // a stubbed apiCall — no timing dependence.
  test("a stale timeline response cannot overwrite a newer one", async ({ page }) => {
    await loginAndOpenMainView(page);
    const result = await page.evaluate(async () => {
      const originalApi = window.apiCall;
      const originalRedraw = window.redrawGraph;
      window.redrawGraph = () => {};
      const resolvers = {};
      window.apiCall = (url) => {
        const key = url.includes("T01") ? "older" : "newer";
        const answer = { ok: true, json: async () => [{ marker: key }] };
        // The periphery of the instant (#46) is fetched after its pins; answer
        // it at once so only the pin responses are raced.
        // So is the scope data fetched beside it.
        if (url.includes("/admin/periphery") || url.includes("/admin/graph/scope"))
          return Promise.resolve(answer);
        return new Promise((resolve) => {
          resolvers[key] = () => resolve(answer);
        });
      };
      try {
        const older = window.setTimelinePosition("2026-01-01T01:00:00Z");
        const newer = window.setTimelinePosition("2026-01-01T02:00:00Z");
        resolvers.newer();
        await newer;
        resolvers.older(); // the stale response lands last…
        await older;
        return {
          at: window.graphTimelineAt,
          marker: window.graphTimelinePins?.[0]?.marker ?? null,
          peripheryMarker: window.graphScopePeriphery?.[0]?.marker ?? null,
        };
      } finally {
        window.apiCall = originalApi;
        window.redrawGraph = originalRedraw;
        window.graphTimelineAt = null;
        window.graphTimelinePins = null;
        window.graphScopePeriphery = null;
        window.graphScopeData = null;
      }
    });
    // …and must not win — neither its pins nor its periphery.
    expect(result.at).toBe("2026-01-01T02:00:00Z");
    expect(result.marker).toBe("newer");
    expect(result.peripheryMarker).toBe("newer");
  });

  // #28.7: an expired session mid-action redirects like every other admin
  // page, instead of leaving the Compare panel stuck on "Comparing…".
  test("an expired session during a graph diff redirects to sign-in", async ({ page }) => {
    await loginAndOpenMainView(page);
    await page.evaluate(() => {
      localStorage.setItem("sanshain_token", "expired-garbage");
      document.cookie = "sanshain_token=expired-garbage;path=/";
    });
    await page.click('button:has-text("Diff")');
    await page.waitForURL("**/account.html", { timeout: 10000 });
  });
});

// The endpoint card navigates by delegated action, and its arguments travel
// through a data-click-args attribute. When those args fail to parse the
// dispatcher logs and calls the handler with none at all, so the click still
// "works" — it just navigates to a URL of five `undefined`s. Nothing caught
// that: the handler resolved, no exception was thrown, and the page loaded.
test.describe("Endpoint card navigation", () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

  const SERVICE = "card-nav-fixture";
  const SPEC = [
    "openapi: 3.0.0",
    "info:",
    "  title: Card Nav Fixture",
    "  version: 1.0.0",
    "paths:",
    "  /widgets/{id}:",
    "    get:",
    "      summary: Reads one widget. By id.",
    "      responses:",
    "        '200':",
    "          description: OK",
    "",
  ].join("\n");

  test.beforeAll(async ({ request }) => {
    if (!adminPassword) {
      throw new Error("INITIAL_ADMIN_PASSWORD environment variable is required for tests");
    }
    const login = await request.post("/auth/login", {
      data: { username: "root", password: adminPassword },
    });
    const { token } = await login.json();
    await request.post("/provide", {
      headers: { Authorization: `Bearer ${token}` },
      data: { producername: SERVICE, stability: "ga", openapi_yaml: SPEC },
    });
  });

  test("an endpoint card shows the operation's summary from the spec", async ({ page }) => {
    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });
    await page.goto(`/producers.html?service=${SERVICE}&api_type=openapi&version=1.0.0`);
    const card = page.locator(".endpoint-card").first();
    await expect(card).toBeVisible({ timeout: 10000 });
    await expect(card.locator(".endpoint-summary")).toHaveText("Reads one widget.");
  });

  test("clicking an endpoint card opens that endpoint, not a URL of undefineds", async ({
    page,
  }) => {
    page.on("dialog", async (d) => await d.dismiss());
    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });

    await page.goto(`/producers.html?service=${SERVICE}&api_type=openapi&version=1.0.0`);
    const card = page.locator(".endpoint-card").first();
    await expect(card).toBeVisible({ timeout: 10000 });

    // The args must survive the round trip through the attribute — this is the
    // step that silently produced an empty list.
    const args = await card.evaluate((el) => JSON.parse(el.getAttribute("data-click-args")));
    expect(args, "the card's action args must parse and carry all five values").toEqual([
      SERVICE,
      "openapi",
      "1.0.0",
      "/widgets/{id}",
      "GET",
    ]);

    await card.click();
    await page.waitForURL(/yaml\.html/, { timeout: 10000 });
    const url = new URL(page.url());
    expect(url.pathname).toBe("/yaml.html");
    expect(page.url(), "no query parameter may be the string 'undefined'").not.toContain(
      "undefined",
    );
    expect(url.searchParams.get("service")).toBe(SERVICE);
    expect(url.searchParams.get("api_type")).toBe("openapi");
    expect(url.searchParams.get("version")).toBe("1.0.0");
    expect(url.searchParams.get("path")).toBe("/widgets/{id}");
    expect(url.searchParams.get("method")).toBe("GET");
  });
});

// The graph draws Producers and Consumers as the same kind of node, but they
// have different pages. Sending a consumer-only node to producers.html asks for
// a version line that does not exist, and the page renders "Fetch error: 404" —
// a dead end reachable by clicking any consumer in the graph.
test.describe("Graph node navigation", () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

  const PRODUCER = "nodelink-producer";
  const CONSUMER = "nodelink-consumer";
  const SPEC = [
    "openapi: 3.0.0",
    "info:",
    "  title: Node Link Fixture",
    "  version: 1.0.0",
    "paths:",
    "  /things:",
    "    get:",
    "      responses:",
    "        '200':",
    "          description: OK",
    "",
  ].join("\n");

  test.beforeAll(async ({ request }) => {
    if (!adminPassword) {
      throw new Error("INITIAL_ADMIN_PASSWORD environment variable is required for tests");
    }
    const login = await request.post("/auth/login", {
      data: { username: "root", password: adminPassword },
    });
    const { token } = await login.json();
    const auth = { Authorization: `Bearer ${token}` };
    await request.post("/provide", {
      headers: auth,
      data: { producername: PRODUCER, stability: "ga", openapi_yaml: SPEC },
    });
    // Give the producer a consumer, so the graph draws a node that provides
    // nothing.
    await request.get("/require", {
      headers: auth,
      params: {
        consumername: CONSUMER,
        producername: PRODUCER,
        version: "1.0.0",
        path: "/things",
        method: "GET",
      },
    });
  });

  test("a consumer node links to the consumers page, not a 404 on producers", async ({ page }) => {
    page.on("dialog", async (d) => await d.dismiss());
    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });

    await page.goto("/graph.html");
    await expect(page.locator(`[data-node="${CONSUMER}"]`).first()).toBeVisible({ timeout: 20000 });

    const hrefFor = async (node) => {
      await page.locator(`[data-node="${node}"]`).first().hover();
      await expect(page.locator("#graph-tooltip")).toBeVisible();
      return page.locator("#graph-tooltip a").first().getAttribute("href");
    };

    expect(await hrefFor(CONSUMER)).toBe(`/consumers.html?name=${CONSUMER}`);
    expect(await hrefFor(PRODUCER)).toBe(`/producers.html?service=${PRODUCER}`);

    // The link must also land somewhere that works: the consumers page focused
    // on that consumer, showing the pin recorded above.
    await page.goto(`/consumers.html?name=${CONSUMER}`);
    await expect(page.locator("#clients-list")).toContainText(PRODUCER, { timeout: 10000 });
    await expect(page.locator("#clients-list")).toContainText("1.0.0");
  });
});

// Both diff views (Producers page line diff, yaml.html) render the server's
// `similar` diff with an Ignore whitespace switch and a Unified / Side by side
// switch (side by side by default on wide screens, remembered per browser,
// always unified below 768px).
test.describe("Diff views", () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;
  const BIG = "diff-big-fixture";
  const WS = "diff-ws-fixture";

  // > 2500 lines; between the two versions only `info.version` differs.
  const bigSpec = (version) => {
    const props = [];
    for (let i = 0; i < 1300; i++) {
      props.push(`        p${i}:`, "          type: string");
    }
    return [
      "openapi: 3.0.0",
      "info:",
      "  title: Big",
      `  version: ${version}`,
      "paths:",
      "  /big:",
      "    get:",
      "      responses:",
      "        '200':",
      "          description: OK",
      "components:",
      "  schemas:",
      "    Big:",
      "      type: object",
      "      properties:",
      ...props,
      "",
    ].join("\n");
  };

  // 1.1.0 re-indents the tags list and adds trailing spaces — whitespace only,
  // apart from `info.version`.
  const wsSpec = (version, tagIndent, trailing) =>
    [
      "openapi: 3.0.0",
      "info:",
      "  title: Whitespace",
      `  version: ${version}`,
      "paths:",
      "  /ws:",
      "    get:",
      "      tags:",
      `${tagIndent}- alpha`,
      `${tagIndent}- beta`,
      `      responses:${trailing}`,
      "        '200':",
      `          description: OK${trailing}`,
      "",
    ].join("\n");

  const login = async (page) => {
    page.on("dialog", async (d) => await d.dismiss());
    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });
    await page.evaluate(() => localStorage.removeItem("sanshain.diffLayout"));
  };

  test.beforeAll(async ({ request }) => {
    if (!adminPassword) {
      throw new Error("INITIAL_ADMIN_PASSWORD environment variable is required for tests");
    }
    const loginRes = await request.post("/auth/login", {
      data: { username: "root", password: adminPassword },
    });
    const { token } = await loginRes.json();
    const provide = (producername, openapi_yaml) =>
      request.post("/provide", {
        headers: { Authorization: `Bearer ${token}` },
        data: { producername, stability: "snapshot", openapi_yaml },
      });
    await provide(BIG, bigSpec("1.0.0"));
    await provide(BIG, bigSpec("1.1.0"));
    await provide(WS, wsSpec("1.0.0", "      ", ""));
    await provide(WS, wsSpec("1.1.0", "        ", "  "));
  });

  test("a big spec with one changed line shows exactly one change, in both layouts", async ({
    page,
  }) => {
    await login(page);
    await page.goto(
      `/yaml.html?service=${BIG}&api_type=openapi&compare_version=1.0.0&version=1.1.0`,
    );
    const view = page.locator("#viewer-container .diff-view");

    // Wide screen: side by side by default, one changed row.
    await expect(view).toHaveAttribute("data-layout", "split", { timeout: 20000 });
    await expect(view.locator(".diff-row")).toHaveCount(2600 + 15, { timeout: 20000 });
    await expect(view.locator('.diff-row[data-diff="change"]')).toHaveCount(1);
    await expect(
      view.locator('.diff-row[data-diff="add"], .diff-row[data-diff="del"]'),
    ).toHaveCount(0);
    await expect(view.locator('.diff-row[data-diff="change"] [data-side="new"]')).toContainText(
      "version: 1.1.0",
    );

    // Unified: one deletion and one insertion.
    await view.locator('.diff-layout-btn[data-layout="unified"]').click();
    await expect(view).toHaveAttribute("data-layout", "unified");
    await expect(view.locator('.diff-line[data-diff="del"]')).toHaveCount(1);
    await expect(view.locator('.diff-line[data-diff="add"]')).toHaveCount(1);
    await expect(view.locator('.diff-line[data-diff="add"]')).toContainText("version: 1.1.0");

    // The layout choice is remembered across page loads.
    await page.reload();
    await expect(page.locator("#viewer-container .diff-view")).toHaveAttribute(
      "data-layout",
      "unified",
      { timeout: 20000 },
    );
  });

  test("yaml.html: Ignore whitespace hides re-indentation", async ({ page }) => {
    await login(page);
    await page.goto(
      `/yaml.html?service=${WS}&api_type=openapi&compare_version=1.0.0&version=1.1.0`,
    );
    const view = page.locator("#viewer-container .diff-view");
    await expect(view).toHaveAttribute("data-layout", "split", { timeout: 20000 });
    const changed = view.locator('.diff-row:not([data-diff="ctx"])');
    await expect(changed.first()).toBeVisible({ timeout: 20000 });

    await view.locator(".diff-ignore-ws").check();
    await expect(changed).toHaveCount(1);
    await expect(changed).toContainText("version: 1.1.0");

    await view.locator('.diff-layout-btn[data-layout="unified"]').click();
    await expect(view.locator('.diff-line[data-diff="add"]')).toHaveCount(1);
    await view.locator(".diff-ignore-ws").uncheck();
    await expect(view.locator('.diff-line[data-diff="add"]')).toHaveCount(5);
    await view.locator('.diff-layout-btn[data-layout="split"]').click();
  });

  // Endpoint snippets are re-serialized when a spec is split, so they never
  // differ in whitespace alone: the endpoint diff offers no dead switch.
  test("yaml.html endpoint diff has no Ignore whitespace switch", async ({ page }) => {
    await login(page);
    await page.goto(
      `/yaml.html?service=${WS}&api_type=openapi&path=%2Fws&method=GET&compare_version=1.0.0&version=1.1.0`,
    );
    const view = page.locator("#viewer-container .diff-view");
    await expect(view).toHaveAttribute("data-layout", "split", { timeout: 20000 });
    const changed = view.locator('.diff-row:not([data-diff="ctx"])');
    await expect(changed).toHaveCount(1, { timeout: 20000 });
    await expect(changed).toContainText("version: 1.1.0");
    await expect(view.locator(".diff-ignore-ws")).toHaveCount(0);
    await expect(view.locator(".diff-layout-switch")).toBeVisible();
  });

  // Copy writes the prefetched patch without fetching on click, so the
  // click's user activation is still valid when the clipboard is written.
  test("Copy in diff mode writes the patch without a fetch on click", async ({ page, context }) => {
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    await login(page);
    await page.goto(
      `/yaml.html?service=${WS}&api_type=openapi&compare_version=1.0.0&version=1.1.0`,
    );
    await expect(page.locator("#viewer-container .diff-row").first()).toBeVisible({
      timeout: 20000,
    });
    await page.waitForLoadState("networkidle");

    const diffRequests = [];
    page.on("request", (r) => {
      if (r.url().includes("/diff")) diffRequests.push(r.url());
    });
    await page.click("#copy-btn");
    await expect(page.getByText("Copied to clipboard!")).toBeVisible();
    const copied = await page.evaluate(() => navigator.clipboard.readText());
    expect(copied).toContain(`--- ${WS} 1.0.0`);
    expect(copied).toContain("+  version: 1.1.0");
    expect(copied).toContain("+        - alpha");
    expect(diffRequests).toEqual([]);
  });

  test("Copy falls back to selecting the text when the clipboard refuses", async ({ page }) => {
    await login(page);
    await page.goto(
      `/yaml.html?service=${WS}&api_type=openapi&compare_version=1.0.0&version=1.1.0`,
    );
    await expect(page.locator("#viewer-container .diff-row").first()).toBeVisible({
      timeout: 20000,
    });
    await page.waitForLoadState("networkidle");
    await page.evaluate(() => {
      window.__copied = null;
      navigator.clipboard.writeText = () => Promise.reject(new Error("denied"));
      document.execCommand = (cmd) => {
        const el = document.activeElement;
        window.__copied =
          cmd === "copy" && el ? el.value.slice(el.selectionStart, el.selectionEnd) : null;
        return true;
      };
    });
    await page.click("#copy-btn");
    await expect(page.getByText("Copied to clipboard!")).toBeVisible();
    const copied = await page.evaluate(() => window.__copied);
    expect(copied).toContain(`--- ${WS} 1.0.0`);
    expect(copied).toContain("+  version: 1.1.0");
  });

  test("Producers page: the line diff has both switches", async ({ page }) => {
    await login(page);
    await page.goto(`/producers.html?service=${WS}`);
    await page.getByRole("button", { name: "Diff", exact: true }).click();
    const view = page.locator(".diff-view");
    await expect(view).toHaveAttribute("data-layout", "split", { timeout: 20000 });
    const changed = view.locator('.diff-row:not([data-diff="ctx"])');
    await expect(changed.first()).toBeVisible({ timeout: 20000 });
    expect(await changed.count()).toBeGreaterThan(1);

    await view.locator(".diff-ignore-ws").check();
    await expect(changed).toHaveCount(1);
    await expect(changed).toContainText("version: 1.1.0");

    await view.locator('.diff-layout-btn[data-layout="unified"]').click();
    await expect(view).toHaveAttribute("data-layout", "unified");
    await expect(view.locator('.diff-line[data-diff="del"]')).toHaveCount(1);
    await expect(view.locator('.diff-line[data-diff="add"]')).toHaveCount(1);
  });

  test("narrow screens are always unified, without a layout switch", async ({ page }) => {
    await login(page);
    await page.evaluate(() => localStorage.setItem("sanshain.diffLayout", "split"));
    await page.setViewportSize({ width: 375, height: 800 });
    await page.goto(
      `/yaml.html?service=${WS}&api_type=openapi&compare_version=1.0.0&version=1.1.0`,
    );
    const view = page.locator("#viewer-container .diff-view");
    await expect(view).toHaveAttribute("data-layout", "unified", { timeout: 20000 });
    await expect(view.locator(".diff-layout-switch")).toBeHidden();
    await expect(view.locator('.diff-line[data-diff="add"]').first()).toBeVisible();

    // Widening the window brings back the remembered side-by-side layout.
    await page.setViewportSize({ width: 1280, height: 800 });
    await expect(view).toHaveAttribute("data-layout", "split");
    await expect(view.locator(".diff-layout-switch")).toBeVisible();
  });
});

// Blame groups consecutive lines last changed in the same version into one
// block: attribution once on the first line, a gutter bar along the block,
// alternating backgrounds between neighbours.
test.describe("Blame blocks", () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;
  const SERVICE = "blame-fixture";

  const spec = (version, description) =>
    [
      "openapi: 3.0.0",
      "info:",
      "  title: Blame",
      `  version: ${version}`,
      "paths:",
      "  /blame:",
      "    get:",
      "      responses:",
      "        '200':",
      `          description: ${description}`,
      "",
    ].join("\n");

  test.beforeAll(async ({ request }) => {
    if (!adminPassword) {
      throw new Error("INITIAL_ADMIN_PASSWORD environment variable is required for tests");
    }
    const loginRes = await request.post("/auth/login", {
      data: { username: "root", password: adminPassword },
    });
    const { token } = await loginRes.json();
    for (const [version, description] of [
      ["1.0.0", "OK"],
      ["1.1.0", "Fine"],
      ["1.2.0", "Fine"],
    ]) {
      await request.post("/provide", {
        headers: { Authorization: `Bearer ${token}` },
        data: {
          producername: SERVICE,
          stability: "snapshot",
          openapi_yaml: spec(version, description),
        },
      });
    }
  });

  const login = async (page) => {
    page.on("dialog", async (d) => await d.dismiss());
    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });
  };

  test("the full-spec blame shows one block per run of lines from the same version", async ({
    page,
  }) => {
    await login(page);
    await page.goto(`/yaml.html?service=${SERVICE}&api_type=openapi&version=1.2.0`);
    await expect(page.locator("#versions-list-container .version-card")).toHaveCount(3, {
      timeout: 20000,
    });
    await page.locator("#blame-toggle-container").click();

    const blocks = page.locator("#viewer-container .blame-block");
    await expect(blocks).toHaveCount(4, { timeout: 20000 });
    const versions = await blocks.evaluateAll((els) => els.map((e) => e.dataset.version));
    expect(versions).toEqual(["1.0.0", "1.2.0", "1.0.0", "1.1.0"]);

    // Lines per block, and the attribution only on each block's first line.
    const shape = await blocks.evaluateAll((els) =>
      els.map((e) =>
        [...e.querySelectorAll(".blame-meta")].map((m) => m.textContent.trim().length > 0),
      ),
    );
    expect(shape).toEqual([
      [true, false, false],
      [true],
      [true, false, false, false, false],
      [true],
    ]);
    await expect(blocks.nth(1).locator(".blame-meta").first()).toContainText("1.2.0");
    await expect(blocks.nth(1).locator(".blame-meta").first()).toContainText("root");
    await expect(blocks.nth(3)).toContainText("description: Fine");

    // A gutter bar along each block; neighbouring blocks alternate background.
    const styles = await blocks.evaluateAll((els) =>
      els.map((e) => {
        const cs = getComputedStyle(e);
        return { bar: cs.borderLeftWidth, bg: cs.backgroundColor };
      }),
    );
    for (const s of styles) expect(s.bar).toBe("2px");
    for (let i = 1; i < styles.length; i++) expect(styles[i].bg).not.toBe(styles[i - 1].bg);
    expect(styles[0].bg).toBe(styles[2].bg);
  });

  test("the endpoint blame uses the same blocks", async ({ page }) => {
    await login(page);
    await page.goto(
      `/yaml.html?service=${SERVICE}&api_type=openapi&path=%2Fblame&method=GET&version=1.2.0`,
    );
    await expect(page.locator("#versions-list-container .version-card")).toHaveCount(3, {
      timeout: 20000,
    });
    await page.locator("#blame-toggle-container").click();
    const blocks = page.locator("#viewer-container .blame-block");
    await expect(blocks.first()).toBeVisible({ timeout: 20000 });
    const versions = await blocks.evaluateAll((els) => els.map((e) => e.dataset.version));
    expect(versions).toContain("1.1.0");
    expect(versions).toContain("1.0.0");
    await expect(blocks.filter({ hasText: "description: Fine" })).toHaveAttribute(
      "data-version",
      "1.1.0",
    );
  });
});

test.describe("Graph without the BROKER node", () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;
  const SUBSCRIBER = "s6-subscriber";
  const CHANNEL = "s6.unowned.events";
  const MISSING = `${CHANNEL} ?`;
  const SPEC = [
    "asyncapi: 2.6.0",
    "info:",
    "  title: S6 Subscriber",
    "  version: 1.0.0",
    "channels:",
    `  ${CHANNEL}:`,
    "    subscribe:",
    "      message:",
    "        name: Unowned",
    "        payload:",
    "          type: object",
  ].join("\n");

  test.beforeAll(async ({ request }) => {
    if (!adminPassword) {
      throw new Error("INITIAL_ADMIN_PASSWORD environment variable is required for tests");
    }
    const login = await request.post("/auth/login", {
      data: { username: "root", password: adminPassword },
    });
    const { token } = await login.json();
    const res = await request.post("/provide/asyncapi", {
      headers: { Authorization: `Bearer ${token}` },
      data: { producername: SUBSCRIBER, stability: "snapshot", asyncapi_yaml: SPEC },
    });
    if (!res.ok()) throw new Error(`provide failed: ${res.status()} ${await res.text()}`);
  });

  const openGraph = async (page) => {
    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });
    await page.goto("/graph.html");
    await expect(page.locator("#custom-graph")).toBeVisible({ timeout: 10000 });
  };

  test("an unowned subscription points at a missing channel node, no BROKER", async ({ page }) => {
    await openGraph(page);
    const node = page.locator(`#custom-graph g.graph-node[data-node="${MISSING}"]`);
    await expect(node).toHaveCount(1, { timeout: 20000 });
    await expect(node).toHaveAttribute("data-role", "missing");
    await expect(node.locator("rect").first()).toHaveAttribute("stroke-dasharray", "6 3");
    const edge = page.locator(
      `#custom-graph path.graph-edge[data-from="${SUBSCRIBER}"][data-to="${MISSING}"]`,
    );
    await expect(edge).toHaveAttribute("data-edge-type", "missing");
    await expect(page.locator('#custom-graph [data-node="BROKER"]')).toHaveCount(0);
    await expect(
      page.locator('#custom-graph path.graph-edge[data-edge-type="messaging-register"]'),
    ).toHaveCount(0);
    await expect(
      page.locator('[data-legend-highlight="edge-type:messaging-register"]'),
    ).toHaveCount(0);
  });

  test("the mermaid renderer draws the same missing channel node", async ({ page }) => {
    await openGraph(page);
    await page.click("#graph-mode-mermaid");
    await expect(page.locator("#mermaid-graph svg")).toContainText(MISSING, { timeout: 20000 });
    await expect(page.locator("#mermaid-graph svg")).not.toContainText("BROKER");
    const code = await page.evaluate(() => lastMermaidCode);
    expect(code).toContain("class s6_unowned_events__ missing");
  });
});

test.describe("Graph provides/requires badges", () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;
  const REST = "s7-rest-svc"; // provides REST only
  const GRPC = "s7-grpc-svc"; // provides gRPC only
  const ASYNC_A = "s7-async-a"; // provides messaging, requires REST and gRPC
  const ASYNC_B = "s7-async-b"; // provides messaging, requires REST, subscribes
  const TRUNK = "s7-trunk-svc"; // trunk REST line, dev-only messaging line
  const TRUNK_CONSUMER = "s7-trunk-consumer";
  const SUB = "s7-subscriber"; // pins A's channel
  const openapi = (title) =>
    [
      "openapi: 3.0.3",
      "info:",
      `  title: ${title}`,
      "  version: 1.0.0",
      "paths:",
      "  /thing:",
      "    get:",
      "      responses:",
      "        '200':",
      "          description: OK",
    ].join("\n");
  const asyncapi = (title, channel, extra = []) =>
    [
      "asyncapi: 2.6.0",
      "info:",
      `  title: ${title}`,
      "  version: 1.0.0",
      "channels:",
      `  ${channel}:`,
      "    publish:",
      "      message:",
      `        name: ${title}Event`,
      "        payload:",
      "          type: object",
      ...extra,
    ].join("\n");
  const PROTO = [
    "// sanshain-version: 1.0.0",
    'syntax = "proto3";',
    "message Req {}",
    "message Res {}",
    "service S7Greeter { rpc Hello (Req) returns (Res); }",
  ].join("\n");

  test.beforeAll(async ({ request }) => {
    if (!adminPassword) {
      throw new Error("INITIAL_ADMIN_PASSWORD environment variable is required for tests");
    }
    const login = await request.post("/auth/login", {
      data: { username: "root", password: adminPassword },
    });
    const { token } = await login.json();
    const headers = { Authorization: `Bearer ${token}` };
    const ok = async (res, what) => {
      if (!res.ok()) throw new Error(`${what} failed: ${res.status()} ${await res.text()}`);
    };
    await ok(
      await request.post("/provide", {
        headers,
        data: { producername: REST, stability: "snapshot", openapi_yaml: openapi(REST) },
      }),
      "provide REST",
    );
    // The REST service once had a messaging line: the provide left a sticky
    // "messaging" row in service_tags that the delete never removes.
    await ok(
      await request.post("/provide/asyncapi", {
        headers,
        data: {
          producername: REST,
          stability: "snapshot",
          asyncapi_yaml: asyncapi("S7R", "s7.r.events"),
        },
      }),
      "provide REST's short-lived messaging line",
    );
    await ok(
      await request.delete(`/admin/producers/${REST}/versions/asyncapi/1.0.0`, { headers }),
      "delete REST's messaging line",
    );
    await ok(
      await request.post("/provide/grpc", {
        headers,
        data: { producername: GRPC, stability: "snapshot", proto_content: PROTO },
      }),
      "provide gRPC",
    );
    await ok(
      await request.post("/provide/asyncapi", {
        headers,
        data: {
          producername: ASYNC_A,
          stability: "snapshot",
          asyncapi_yaml: asyncapi("S7A", "s7.a.events"),
        },
      }),
      "provide A",
    );
    await ok(
      await request.post("/provide/asyncapi", {
        headers,
        data: {
          producername: ASYNC_B,
          stability: "snapshot",
          asyncapi_yaml: asyncapi("S7B", "s7.b.events", [
            "  s7.nobody.events:",
            "    subscribe:",
            "      message:",
            "        name: S7Nobody",
            "        payload:",
            "          type: object",
          ]),
        },
      }),
      "provide B",
    );
    for (const client of [ASYNC_A, ASYNC_B]) {
      await ok(
        await request.get(
          `/require?consumername=${client}&producername=${REST}&version=1.0.0&path=/thing&method=GET`,
          { headers },
        ),
        `${client} requires REST`,
      );
    }
    await ok(
      await request.get(
        `/require/grpc?consumername=${ASYNC_A}&producername=${GRPC}&version=1.0.0&path=S7Greeter&method=Hello`,
        { headers },
      ),
      "A requires gRPC",
    );
    // A pinned subscriber of A's channel, for the Topics switch.
    await ok(
      await request.get(
        `/require/asyncapi?consumername=${SUB}&producername=${ASYNC_A}&version=1.0.0&path=s7.a.events&method=PUB`,
        { headers },
      ),
      "SUB requires A's channel",
    );
    // Main view fixture: only the REST line is trunk-provided.
    await ok(
      await request.post("/provide", {
        headers,
        data: {
          producername: TRUNK,
          stability: "snapshot",
          openapi_yaml: openapi(TRUNK),
          trunk: true,
        },
      }),
      "provide trunk REST",
    );
    await ok(
      await request.post("/provide/asyncapi", {
        headers,
        data: {
          producername: TRUNK,
          stability: "snapshot",
          asyncapi_yaml: asyncapi("S7T", "s7.t.events"),
        },
      }),
      "provide dev-only messaging",
    );
    await ok(
      await request.get(
        `/require?consumername=${TRUNK_CONSUMER}&producername=${TRUNK}&version=1.0.0&path=/thing&method=GET&trunk=true`,
        { headers },
      ),
      "trunk require",
    );
    await ok(
      await request.put(`/catalog/${ASYNC_A}`, {
        headers,
        data: {
          catalog_info_yaml: [
            "apiVersion: backstage.io/v1alpha1",
            "kind: Component",
            "metadata:",
            `  name: ${ASYNC_A}`,
            "  tags:",
            "    - Kafka",
            "    - s7-tagged",
            "spec:",
            "  type: service",
            "  dependsOn:",
            "    - resource:s7-db",
            "    - component:s7-ghost",
          ].join("\n"),
        },
      }),
      "catalog push",
    );
  });

  const openGraph = async (page) => {
    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });
    await page.goto("/graph.html");
    await expect(page.locator("#custom-graph")).toBeVisible({ timeout: 10000 });
  };
  const node = (page, name) => page.locator(`#custom-graph g.graph-node[data-node="${name}"]`);

  test("the Topics switch draws channels as nodes between publishers and subscribers", async ({
    page,
  }) => {
    await openGraph(page);
    await expect(node(page, ASYNC_A)).toHaveCount(1, { timeout: 20000 });
    const edge = (from, to) =>
      page.locator(`#custom-graph path.graph-edge[data-from="${from}"][data-to="${to}"]`);
    const topicA = node(page, "topic:s7.a.events");
    const nobody = node(page, "topic:s7.nobody.events");
    await expect(edge(SUB, ASYNC_A)).toHaveCount(1);
    await expect(topicA).toHaveCount(0);
    await expect(node(page, "s7.nobody.events ?")).toHaveCount(1);

    await page.click("#graph-topics-toggle");
    await expect(page.locator("#custom-graph")).toHaveAttribute("data-topics", "1");
    await expect(topicA).toHaveAttribute("data-role", "topic");
    await expect(topicA).toContainText("s7.a.events");
    await expect(edge(ASYNC_A, "topic:s7.a.events")).toHaveCount(1);
    await expect(edge("topic:s7.a.events", SUB)).toHaveCount(1);
    await expect(edge(SUB, ASYNC_A)).toHaveCount(0);
    // A channel nobody publishes is a dashed topic; the old missing node is gone.
    await expect(nobody).toHaveAttribute("data-role", "topic");
    await expect(nobody.locator("rect").first()).toHaveAttribute("stroke-dasharray", "6 3");
    await expect(edge("topic:s7.nobody.events", ASYNC_B)).toHaveCount(1);
    await expect(node(page, "s7.nobody.events ?")).toHaveCount(0);
    // Roles stay what the recorded edges say: A still provides and requires.
    await expect(node(page, ASYNC_A)).toHaveAttribute("data-role", "both");
    await expect(node(page, SUB)).toHaveAttribute("data-requires", "msg");

    await topicA.hover();
    const tooltip = page.locator("#graph-tooltip");
    await expect(tooltip).toBeVisible({ timeout: 5000 });
    await expect(tooltip.locator('[data-panel="publishers"]')).toContainText(ASYNC_A);
    await expect(tooltip.locator('[data-panel="subscribers"]')).toContainText(SUB);

    // Remembered, and hidden in the Mermaid mode like the other switches.
    await page.goto("/graph.html");
    await expect(topicA).toHaveCount(1, { timeout: 20000 });
    await expect(page.locator("#graph-topics-toggle")).toHaveAttribute("aria-pressed", "true");
    await page.click("#graph-mode-mermaid");
    await expect(page.locator("#graph-topics-toggle")).toBeHidden();
  });

  test("catalog tags are chips in the node panel and a tag: focus narrows the graph", async ({
    page,
  }) => {
    await openGraph(page);
    await expect(node(page, ASYNC_A)).toHaveCount(1, { timeout: 20000 });
    await node(page, ASYNC_A).hover();
    const tooltip = page.locator("#graph-tooltip");
    await expect(tooltip).toBeVisible({ timeout: 5000 });
    await expect(tooltip.locator('[data-context="tags"] [data-tag]')).toHaveText([
      "kafka",
      "s7-tagged",
    ]);
    // The component ref names no Sanshain system: a dashed missing component
    // hangs off ASYNC_A, labelled by its name.
    const ghost = page.locator(
      '#custom-graph g.graph-node[data-node="component:default/s7-ghost"]',
    );
    await expect(ghost).toHaveAttribute("data-role", "missing-component");
    await expect(ghost.locator("rect").first()).toHaveAttribute("stroke-dasharray", "6 3");
    await expect(ghost).toContainText("s7-ghost");
    // Only ASYNC_A carries s7-tagged: the focus keeps it and its neighbours.
    await page.goto("/graph.html?focus=tag:s7-tagged");
    await expect(node(page, ASYNC_A)).toHaveCount(1, { timeout: 20000 });
    await expect(node(page, GRPC)).toHaveCount(1);
    await expect(node(page, ASYNC_B)).toHaveCount(0);
    await expect(node(page, TRUNK)).toHaveCount(0);
  });

  test("a REST service required by async providers shows no messaging badge", async ({ page }) => {
    await openGraph(page);
    const rest = node(page, REST);
    await expect(rest).toHaveCount(1, { timeout: 20000 });
    await expect(rest).toHaveAttribute("data-provides", "rest");
    await expect(rest.locator('[data-badge="provides-rest"]')).toHaveCount(1);
    await expect(rest.locator('[data-badge="provides-msg"]')).toHaveCount(0);
    await expect(rest).toHaveAttribute("data-role", "service");
    for (const client of [ASYNC_A, ASYNC_B]) {
      const n = node(page, client);
      await expect(n.locator('[data-badge="provides-msg"]')).toHaveCount(1);
      await expect(n.locator('[data-badge="requires-rest"]')).toHaveCount(1);
    }
    // Solid badge: dark circle with a white ring.
    const solid = rest.locator('[data-badge="provides-rest"] circle');
    await expect(solid).toHaveAttribute("fill", "#1e293b");
    await expect(solid).toHaveAttribute("stroke", "#fff");
    // Outlined badge: white circle with a dark ring.
    const outlined = node(page, ASYNC_A).locator('[data-badge="requires-rest"] circle');
    await expect(outlined).toHaveAttribute("fill", "#fff");
    await expect(outlined).toHaveAttribute("stroke", "#1e293b");
    // No node colour role comes from the tags table any more.
    await expect(page.locator('#custom-graph g.graph-node[data-role^="tag-"]')).toHaveCount(0);
  });

  test("badges sit on the top-right and bottom-left edges in a fixed order", async ({ page }) => {
    await openGraph(page);
    const a = node(page, ASYNC_A);
    await expect(a).toHaveCount(1, { timeout: 20000 });
    await expect(a).toHaveAttribute("data-requires", "rest,grpc");
    const geometry = await a.evaluate((g) => {
      const rect = g.querySelector("rect");
      const box = {
        left: +rect.getAttribute("x"),
        top: +rect.getAttribute("y"),
        right: +rect.getAttribute("x") + +rect.getAttribute("width"),
        bottom: +rect.getAttribute("y") + +rect.getAttribute("height"),
      };
      const at = (sel) => {
        const c = g.querySelector(`[data-badge="${sel}"] circle`);
        return { cx: +c.getAttribute("cx"), cy: +c.getAttribute("cy") };
      };
      return { box, msg: at("provides-msg"), rest: at("requires-rest"), grpc: at("requires-grpc") };
    });
    expect(geometry.msg.cx).toBeCloseTo(geometry.box.right - 4, 3);
    expect(geometry.msg.cy).toBeCloseTo(geometry.box.top + 2, 3);
    expect(geometry.rest.cx).toBeCloseTo(geometry.box.left + 4, 3);
    expect(geometry.grpc.cx).toBeCloseTo(geometry.box.left + 28, 3);
    expect(geometry.rest.cy).toBeCloseTo(geometry.box.bottom - 2, 3);
  });

  test("a Peripheral Resource provides the hexagon and adds no require", async ({ page }) => {
    await openGraph(page);
    const resource = page.locator(
      '#custom-graph g.graph-node[data-role="peripheral"][data-node*="s7-db"]',
    );
    await expect(resource).toHaveCount(1, { timeout: 20000 });
    await expect(resource.locator('[data-badge="provides-resource"]')).toHaveCount(1);
    await expect(resource.locator('[data-badge^="requires-"]')).toHaveCount(0);
    await expect(node(page, ASYNC_A)).toHaveAttribute("data-requires", "rest,grpc");
  });

  test("a gRPC-only service shows no REST badge", async ({ page }) => {
    await openGraph(page);
    const grpc = node(page, GRPC);
    await expect(grpc).toHaveCount(1, { timeout: 20000 });
    await expect(grpc).toHaveAttribute("data-provides", "grpc");
    await expect(grpc.locator('[data-badge="provides-rest"]')).toHaveCount(0);
  });

  test("a harvested subscription is a messaging require", async ({ page }) => {
    await openGraph(page);
    const b = node(page, ASYNC_B);
    await expect(b).toHaveCount(1, { timeout: 20000 });
    await expect(b).toHaveAttribute("data-requires", "rest,msg");
    await expect(b.locator('[data-badge="requires-msg"]')).toHaveCount(1);
  });

  test("the main view shows only trunk-provided lines", async ({ page }) => {
    await openGraph(page);
    await expect(node(page, TRUNK)).toHaveAttribute("data-provides", "rest,msg", {
      timeout: 20000,
    });
    await page.click("#stream-main");
    const trunk = node(page, TRUNK);
    await expect(trunk).toHaveAttribute("data-provides", "rest", { timeout: 20000 });
    await expect(trunk.locator('[data-badge="provides-msg"]')).toHaveCount(0);
    await expect(node(page, TRUNK_CONSUMER)).toHaveAttribute("data-requires", "rest");
  });

  test("the legend explains the badges instead of producer tags", async ({ page }) => {
    await openGraph(page);
    await expect(page.locator("#graph-legend-badges")).toBeVisible();
    await expect(page.locator("#graph-legend")).not.toContainText("Producer Tags");
    await expect(node(page, REST)).toHaveCount(1, { timeout: 20000 });
    await page.locator('[data-legend-highlight="node-badge:grpc"]').hover();
    await expect(node(page, GRPC)).toHaveCSS("opacity", "1");
    await expect(node(page, REST)).toHaveCSS("opacity", "0.12");
  });

  test("mermaid nodes carry a provides/requires line", async ({ page }) => {
    await openGraph(page);
    await expect(node(page, REST)).toHaveCount(1, { timeout: 20000 });
    await page.click("#graph-mode-mermaid");
    await expect(page.locator("#mermaid-graph svg")).toContainText(ASYNC_A, { timeout: 20000 });
    const code = await page.evaluate(() => lastMermaidCode);
    expect(code).toContain(`s7_rest_svc["${REST}<br/>▲ REST"]`);
    // A is required too now (the pinned subscriber): both roles, square shape.
    expect(code).toContain(`s7_async_a["${ASYNC_A}<br/>▲ MSG   ▼ REST · gRPC"]`);
    expect(code).toContain(`s7_grpc_svc["${GRPC}<br/>▲ gRPC"]`);
    const text = await page.locator("#mermaid-graph svg").textContent();
    expect(text.replace(/\s+/g, " ")).toContain("▲ MSG ▼ REST · gRPC");
  });
});

test.describe("Graph (unrequired) box", () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;
  const LONE = "s8-lone-svc"; // REST, trunk-provided, nobody requires it
  const ASYNC = "s8-async-svc"; // messaging only, nobody requires it
  const ISLAND = "s8-island-svc"; // nobody requires it, but has a resource
  const STALE = "s8-stale-svc"; // required; once had a messaging line
  const STALE_CONSUMER = "s8-stale-consumer";
  const DEV_SUB = "s8-dev-subscriber"; // dev-only subscription to nobody
  const DEV_CHANNEL = "s8.dev.only.events";
  const HOTFIX = "s8-hotfix-svc"; // only a tag-flagged member of BRANCH
  const BRANCH = "s8-rel";
  // A four-link chain: decisively tall when vertical, wide when horizontal.
  const DEEP = ["s8-deep-1", "s8-deep-2", "s8-deep-3", "s8-deep-4"];
  const openapi = (title, version = "1.0.0") =>
    [
      "openapi: 3.0.3",
      "info:",
      `  title: ${title}`,
      `  version: ${version}`,
      "paths:",
      "  /thing:",
      "    get:",
      "      responses:",
      "        '200':",
      "          description: OK",
    ].join("\n");
  const asyncapi = (title, channel, op = "publish") =>
    [
      "asyncapi: 2.6.0",
      "info:",
      `  title: ${title}`,
      "  version: 0.3.0",
      "channels:",
      `  ${channel}:`,
      `    ${op}:`,
      "      message:",
      `        name: ${title}Event`,
      "        payload:",
      "          type: object",
    ].join("\n");

  test.beforeAll(async ({ request }) => {
    if (!adminPassword) {
      throw new Error("INITIAL_ADMIN_PASSWORD environment variable is required for tests");
    }
    const login = await request.post("/auth/login", {
      data: { username: "root", password: adminPassword },
    });
    const { token } = await login.json();
    const headers = { Authorization: `Bearer ${token}` };
    const ok = async (res, what) => {
      if (!res.ok()) throw new Error(`${what} failed: ${res.status()} ${await res.text()}`);
    };
    await ok(
      await request.post("/provide", {
        headers,
        data: {
          producername: LONE,
          stability: "ga",
          openapi_yaml: openapi(LONE, "1.4.0"),
          trunk: true,
        },
      }),
      "provide LONE",
    );
    await ok(
      await request.post("/provide/asyncapi", {
        headers,
        data: {
          producername: ASYNC,
          stability: "snapshot",
          asyncapi_yaml: asyncapi("S8Async", "s8.async.events"),
        },
      }),
      "provide ASYNC",
    );
    await ok(
      await request.post("/provide", {
        headers,
        data: { producername: ISLAND, stability: "snapshot", openapi_yaml: openapi(ISLAND) },
      }),
      "provide ISLAND",
    );
    await ok(
      await request.put(`/catalog/${ISLAND}`, {
        headers,
        data: {
          catalog_info_yaml: [
            "apiVersion: backstage.io/v1alpha1",
            "kind: Component",
            "metadata:",
            `  name: ${ISLAND}`,
            "spec:",
            "  type: service",
            "  dependsOn:",
            "    - resource:s8-island-db",
          ].join("\n"),
        },
      }),
      "catalog push",
    );
    // STALE's messaging line is deleted: only the sticky service tag remains.
    await ok(
      await request.post("/provide", {
        headers,
        data: { producername: STALE, stability: "snapshot", openapi_yaml: openapi(STALE) },
      }),
      "provide STALE",
    );
    await ok(
      await request.post("/provide/asyncapi", {
        headers,
        data: {
          producername: STALE,
          stability: "snapshot",
          asyncapi_yaml: asyncapi("S8Stale", "s8.stale.events"),
        },
      }),
      "provide STALE messaging",
    );
    await ok(
      await request.delete(`/admin/producers/${STALE}/versions/asyncapi/0.3.0`, { headers }),
      "delete STALE messaging",
    );
    await ok(
      await request.get(
        `/require?consumername=${STALE_CONSUMER}&producername=${STALE}&version=1.0.0&path=/thing&method=GET&trunk=true`,
        { headers },
      ),
      "require STALE",
    );
    for (const name of DEEP) {
      await ok(
        await request.post("/provide", {
          headers,
          data: { producername: name, stability: "snapshot", openapi_yaml: openapi(name) },
        }),
        `provide ${name}`,
      );
    }
    for (let i = 1; i < DEEP.length; i++) {
      await ok(
        await request.get(
          `/require?consumername=${DEEP[i - 1]}&producername=${DEEP[i]}&version=1.0.0&path=/thing&method=GET`,
          { headers },
        ),
        `require ${DEEP[i]}`,
      );
    }
    // A dev-only (not trunk-flagged) subscription to a channel nobody owns.
    await ok(
      await request.post("/provide/asyncapi", {
        headers,
        data: {
          producername: DEV_SUB,
          stability: "snapshot",
          asyncapi_yaml: asyncapi("S8DevSub", DEV_CHANNEL, "subscribe"),
        },
      }),
      "provide DEV_SUB",
    );
    // A branch whose only own producer is a tag-flagged messaging hotfix.
    await ok(
      await request.post("/admin/branches", { headers, data: { name: BRANCH } }),
      "create branch",
    );
    await ok(
      await request.post("/provide/asyncapi", {
        headers,
        data: {
          producername: HOTFIX,
          stability: "ga",
          asyncapi_yaml: asyncapi("S8Hotfix", "s8.hotfix.events"),
          tag: BRANCH,
        },
      }),
      "tagged provide HOTFIX",
    );
  });

  const openGraph = async (page, query = "") => {
    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });
    await page.goto(`/graph.html${query}`);
    await expect(page.locator("#custom-graph")).toBeVisible({ timeout: 10000 });
  };
  const box = (page) => page.locator("#custom-graph #graph-unrequired-box");
  const chip = (page, name) =>
    page.locator(`#custom-graph g.graph-unrequired-chip[data-chip="${name}"]`);
  const node = (page, name) => page.locator(`#custom-graph g.graph-node[data-node="${name}"]`);

  test("unrequired producers are chips in one box, not loose nodes", async ({ page }) => {
    await openGraph(page);
    await expect(chip(page, LONE)).toHaveCount(1, { timeout: 20000 });
    await expect(box(page)).toHaveCount(1);
    await expect(box(page)).toContainText("(unrequired)");
    await expect(chip(page, ASYNC)).toHaveCount(1);
    await expect(node(page, LONE)).toHaveCount(0);
    // A producer with a resource edge stays a normal node.
    await expect(node(page, ISLAND)).toHaveCount(1);
    await expect(chip(page, ISLAND)).toHaveCount(0);
    await expect(page.locator("#graph-legend-unrequired")).toBeVisible();
  });

  test("a chip click opens the node panel with its version lines", async ({ page }) => {
    await openGraph(page);
    await chip(page, LONE).click();
    const tooltip = page.locator("#graph-tooltip");
    await expect(tooltip).toBeVisible();
    await expect(tooltip).toContainText(LONE);
    await expect(tooltip.locator('[data-panel="versions"]')).toHaveText("openapi 1.4.0 ga");
    await expect(tooltip.locator('[data-panel="last-provided"]')).toContainText("last provided 20");
    await expect(tooltip.locator('[data-panel="provides"]')).toContainText("REST");
  });

  test("a node panel opens after the hover delay, a neighbour's at once", async ({ page }) => {
    await openGraph(page);
    await expect(node(page, STALE)).toHaveCount(1, { timeout: 20000 });
    const delay = await page.evaluate(() => GRAPH_HOVER_DELAY_MS);
    expect(delay).toBe(2000);
    const tooltip = page.locator("#graph-tooltip");
    await node(page, STALE).hover();
    await page.waitForTimeout(delay / 2);
    await expect(tooltip).toBeHidden();
    await page.waitForTimeout(delay);
    await expect(tooltip).toBeVisible();
    await expect(tooltip).toContainText(STALE);
    // The panel is open: the neighbour's replaces it without waiting again.
    await node(page, STALE_CONSUMER).hover();
    await expect(tooltip).toContainText(STALE_CONSUMER, { timeout: delay / 2 });
    // Leaving before the delay never opens one.
    await page.mouse.move(5, 5);
    await expect(tooltip).toBeHidden();
    await node(page, STALE).hover();
    await page.mouse.move(5, 5);
    await page.waitForTimeout(delay + 300);
    await expect(tooltip).toBeHidden();
  });

  test("the node panel shows derived provides/requires, not stale tags", async ({ page }) => {
    await openGraph(page);
    await expect(node(page, STALE)).toHaveCount(1, { timeout: 20000 });
    await node(page, STALE).hover();
    const tooltip = page.locator("#graph-tooltip");
    await expect(tooltip.locator('[data-panel="provides"]')).toHaveText(/Provides\s*REST$/);
    await expect(tooltip).not.toContainText("messaging");
    await node(page, STALE_CONSUMER).hover();
    await expect(tooltip.locator('[data-panel="requires"]')).toContainText("REST");
  });

  test("the Unrequired switch hides the box and is remembered in the URL", async ({ page }) => {
    await openGraph(page);
    await expect(box(page)).toHaveCount(1, { timeout: 20000 });
    await page.click("#filter-unrequired");
    await expect(box(page)).toHaveCount(0);
    expect(page.url()).toContain("hide_protocols=unrequired");
    await page.goto("/graph.html?hide_protocols=unrequired");
    await expect(node(page, ISLAND)).toHaveCount(1, { timeout: 20000 });
    await expect(box(page)).toHaveCount(0);
    await expect(page.locator("#filter-unrequired")).toHaveClass(/bg-white/);
  });

  test("protocol, focus and circular filters narrow the box", async ({ page }) => {
    await openGraph(page);
    await expect(chip(page, ASYNC)).toHaveCount(1, { timeout: 20000 });
    await page.click("#filter-asyncapi");
    await expect(chip(page, ASYNC)).toHaveCount(0);
    await expect(chip(page, LONE)).toHaveCount(1);
    await page.click("#filter-asyncapi");
    await page.fill("#graph-service-filter", LONE);
    await page.press("#graph-service-filter", "Enter");
    await expect(chip(page, ASYNC)).toHaveCount(0);
    await expect(chip(page, LONE)).toHaveCount(1);
    await page.click("#graph-redraw-circular");
    await expect(box(page)).toHaveCount(0);
  });

  test("the box sits beside a tall graph and below a wide one", async ({ page }) => {
    // The chain and the box: the middle links focus all four.
    await openGraph(page, `?focus=${DEEP[1]},${DEEP[2]},${LONE}`);
    await expect(box(page)).toHaveCount(1, { timeout: 20000 });
    const measure = () =>
      page.evaluate(() => {
        const frame = document.querySelector("#graph-unrequired-box rect");
        const nodes = [...document.querySelectorAll("#custom-graph g.graph-node > rect")];
        return {
          boxX: +frame.getAttribute("x"),
          boxY: +frame.getAttribute("y"),
          maxRight: Math.max(...nodes.map((r) => +r.getAttribute("x") + +r.getAttribute("width"))),
          maxBottom: Math.max(
            ...nodes.map((r) => +r.getAttribute("y") + +r.getAttribute("height")),
          ),
          side: {
            tall: graphOverflowSide({ minX: 0, minY: 0, maxX: 300, maxY: 301 }, false),
            wide: graphOverflowSide({ minX: 0, minY: 0, maxX: 301, maxY: 300 }, true),
            square: graphOverflowSide({ minX: 0, minY: 0, maxX: 300, maxY: 300 }, false),
            noneTb: graphOverflowSide(null, false),
            noneLr: graphOverflowSide(null, true),
          },
        };
      });
    const tb = await measure();
    expect(tb.side).toEqual({
      tall: "right",
      wide: "below",
      square: "below",
      noneTb: "below",
      noneLr: "right",
    });
    expect(tb.boxX).toBeGreaterThan(tb.maxRight);
    await page.click("#graph-direction-toggle");
    await expect(page.locator("#graph-direction-label")).toHaveText("Horizontal");
    const lr = await measure();
    expect(lr.boxY).toBeGreaterThan(lr.maxBottom);
  });

  test("Compact tightens the layout; it and the direction are remembered", async ({ page }) => {
    await openGraph(page, `?focus=${DEEP[1]},${DEEP[2]},${LONE}`);
    await expect(box(page)).toHaveCount(1, { timeout: 20000 });
    const height = () =>
      page.evaluate(() => {
        const nodes = [...document.querySelectorAll("#custom-graph g.graph-node > rect")];
        const top = Math.min(...nodes.map((r) => +r.getAttribute("y")));
        const bottom = Math.max(
          ...nodes.map((r) => +r.getAttribute("y") + +r.getAttribute("height")),
        );
        return bottom - top;
      });
    const svg = page.locator("#custom-graph");
    await expect(svg).toHaveAttribute("data-compact", "0");
    const airy = await height();
    await page.click("#graph-compact-toggle");
    await expect(svg).toHaveAttribute("data-compact", "1");
    await expect(page.locator("#graph-compact-toggle")).toHaveAttribute("aria-pressed", "true");
    const compact = await height();
    expect(compact).toBeLessThan(airy * 0.6);
    await page.click("#graph-direction-toggle");
    await expect(page.locator("#graph-direction-label")).toHaveText("Horizontal");
    // Both survive a reload; the URL carries neither.
    expect(page.url()).not.toMatch(/compact|direction/);
    await page.goto(`/graph.html?focus=${DEEP[1]},${DEEP[2]},${LONE}`);
    await expect(box(page)).toHaveCount(1, { timeout: 20000 });
    await expect(svg).toHaveAttribute("data-compact", "1");
    await expect(page.locator("#graph-compact-toggle")).toHaveAttribute("aria-pressed", "true");
    await expect(page.locator("#graph-direction-label")).toHaveText("Horizontal");
    // Not a custom-renderer control: hidden in the other modes.
    await page.click("#graph-mode-mermaid");
    await expect(page.locator("#graph-compact-toggle")).toBeHidden();
    await expect(page.locator("#graph-direction-toggle")).toBeHidden();
  });

  test("Rainbow colours the plain edges per Consumer and leaves the warnings alone", async ({
    page,
  }) => {
    await openGraph(page);
    await expect(box(page)).toHaveCount(1, { timeout: 20000 });
    const strokes = () =>
      page.evaluate(() =>
        [...document.querySelectorAll("#custom-graph path.graph-edge")].map((p) => ({
          from: p.dataset.from,
          type: p.dataset.edgeType,
          stroke: p.getAttribute("stroke"),
          marker: p.getAttribute("marker-end"),
        })),
      );
    const svg = page.locator("#custom-graph");
    await expect(svg).toHaveAttribute("data-rainbow", "0");
    const plain = await strokes();
    expect(plain.filter((e) => e.type === "normal").every((e) => e.stroke === "#94a3b8")).toBe(
      true,
    );
    await page.click("#graph-rainbow-toggle");
    await expect(svg).toHaveAttribute("data-rainbow", "1");
    const coloured = await strokes();
    const normal = coloured.filter((e) => e.type === "normal");
    expect(normal.length).toBeGreaterThan(2);
    const byFrom = new Map();
    for (const e of normal) {
      expect(e.stroke).not.toBe("#94a3b8");
      expect(e.marker).toMatch(/^url\(#arrow-rainbow-\d+\)$/);
      if (byFrom.has(e.from)) expect(byFrom.get(e.from)).toBe(e.stroke);
      byFrom.set(e.from, e.stroke);
    }
    expect(new Set(byFrom.values()).size).toBeGreaterThan(1);
    // The missing dependency keeps its orange.
    const missing = coloured.filter((e) => e.type === "missing");
    expect(missing.length).toBeGreaterThan(0);
    expect(missing.every((e) => e.stroke === "#f97316")).toBe(true);
    // Remembered across a reload, stable per name.
    await page.goto("/graph.html");
    await expect(box(page)).toHaveCount(1, { timeout: 20000 });
    await expect(svg).toHaveAttribute("data-rainbow", "1");
    await expect(page.locator("#graph-rainbow-toggle")).toHaveAttribute("aria-pressed", "true");
    const again = await strokes();
    for (const e of again.filter((x) => x.type === "normal")) {
      expect(e.stroke).toBe(byFrom.get(e.from));
    }
  });

  test("the Periphery switch is remembered, and a link that hides protocols still wins", async ({
    page,
  }) => {
    await openGraph(page);
    const periphery = page.locator("#filter-periphery");
    await expect(periphery).toHaveClass(/bg-indigo-600/);
    await periphery.click();
    await expect(periphery).toHaveClass(/bg-white/);

    await page.goto("/graph.html");
    await expect(page.locator("#custom-graph")).toBeVisible({ timeout: 10000 });
    await expect(periphery).toHaveClass(/bg-white/);

    await page.goto("/graph.html?hide_protocols=proto");
    await expect(page.locator("#custom-graph")).toBeVisible({ timeout: 10000 });
    await expect(periphery).toHaveClass(/bg-indigo-600/);
  });

  test("islands pack into empty space without growing the graph", async ({ page }) => {
    await openGraph(page);
    await expect(node(page, ISLAND)).toHaveCount(1, { timeout: 20000 });
    const packed = await page.evaluate(() => {
      const box = { minX: 0, minY: 0, maxX: 1000, maxY: 600 };
      // The left half is taken; the right half is free.
      const obstacles = [{ x0: 0, y0: 0, x1: 450, y1: 600 }];
      const inside = graphPackIslands(box, obstacles, [{ w: 200, h: 100 }], "below");
      const many = graphPackIslands(
        box,
        [{ x0: 0, y0: 0, x1: 1000, y1: 600 }],
        [
          { w: 400, h: 100 },
          { w: 400, h: 100 },
          { w: 400, h: 100 },
        ],
        "below",
      );
      const lr = graphPackIslands(
        box,
        [{ x0: 0, y0: 0, x1: 1000, y1: 600 }],
        [
          { w: 100, h: 250 },
          { w: 100, h: 250 },
          { w: 100, h: 250 },
        ],
        "right",
      );
      return { inside, many, lr };
    });
    expect(packed.inside[0].x).toBeGreaterThanOrEqual(480);
    expect(packed.inside[0].x + 200).toBeLessThanOrEqual(1000);
    expect(packed.inside[0].y + 100).toBeLessThanOrEqual(600);
    // Overflow goes to the chosen side, never widening / heightening the frame.
    for (const p of packed.many) {
      expect(p.y).toBeGreaterThan(600);
      expect(p.x + 400).toBeLessThanOrEqual(1000);
    }
    for (const p of packed.lr) {
      expect(p.x).toBeGreaterThan(1000);
      expect(p.y + 250).toBeLessThanOrEqual(600);
    }
    // In the real graph the island overlaps no other node.
    const overlaps = await page.evaluate((island) => {
      const rectOf = (g) => {
        const r = g.querySelector("rect");
        const x = +r.getAttribute("x");
        const y = +r.getAttribute("y");
        return {
          x0: x,
          y0: y,
          x1: x + +r.getAttribute("width"),
          y1: y + +r.getAttribute("height"),
        };
      };
      const all = [...document.querySelectorAll("#custom-graph g.graph-node")];
      const own = rectOf(all.find((g) => g.dataset.node === island));
      return all
        .filter((g) => g.dataset.node !== island)
        .map(rectOf)
        .filter((o) => o.x0 < own.x1 && own.x0 < o.x1 && o.y0 < own.y1 && own.y0 < o.y1).length;
    }, ISLAND);
    expect(overlaps).toBe(0);
  });

  test("the mermaid renderer draws the box as a subgraph", async ({ page }) => {
    await openGraph(page);
    await expect(chip(page, LONE)).toHaveCount(1, { timeout: 20000 });
    await page.click("#graph-mode-mermaid");
    await expect(page.locator("#mermaid-graph svg")).toContainText("(unrequired)", {
      timeout: 20000,
    });
    const code = await page.evaluate(() => lastMermaidCode);
    expect(code).toContain('subgraph unrequired_box["(unrequired)"]');
    expect(code).toContain(`s8_lone_svc["${LONE}<br/>▲ REST"]`);
    expect(code).toContain("class s8_lone_svc unrequired");
  });

  test("main replaces the injected trunk producers and scopes subscriptions", async ({ page }) => {
    await openGraph(page);
    // Dev: the dev-only subscription points at its missing channel node.
    await expect(node(page, `${DEV_CHANNEL} ?`)).toHaveCount(1, { timeout: 20000 });
    await page.click("#stream-main");
    await expect(chip(page, LONE)).toHaveCount(1, { timeout: 20000 });
    await expect(node(page, LONE)).toHaveCount(0);
    // Not trunk-provided: not in main's box.
    await expect(chip(page, ASYNC)).toHaveCount(0);
    // Not trunk-flagged: not in main at all.
    await expect(node(page, `${DEV_CHANNEL} ?`)).toHaveCount(0);
    await expect(node(page, DEV_SUB)).toHaveCount(0);
  });

  test("a branch's own members are chips with their scoped lines", async ({ page }) => {
    await openGraph(page);
    await expect(box(page)).toHaveCount(1, { timeout: 20000 });
    await page.selectOption("#graph-branch-select", BRANCH);
    await expect(chip(page, HOTFIX)).toHaveCount(1, { timeout: 20000 });
    await expect(chip(page, LONE)).toHaveCount(0);
    await chip(page, HOTFIX).click();
    await expect(page.locator('#graph-tooltip [data-panel="versions"]')).toHaveText(
      "asyncapi 0.3.0 ga",
    );
    await expect(page.locator('#graph-tooltip [data-panel="provides"]')).toContainText("MSG");
  });
});

test.describe("Superseded snapshots", () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;
  const PRODUCER = "s9-producer"; // snapshot 1.0.0, GA 1.1.0, snapshot 1.2.0
  const OLD_PIN = "s9-old-pin"; // pins the superseded snapshot 1.0.0
  const NEW_PIN = "s9-new-pin"; // pins the snapshot 1.2.0, above the GA
  const openapi = (version) =>
    [
      "openapi: 3.0.3",
      "info:",
      "  title: S9",
      `  version: ${version}`,
      "paths:",
      "  /thing:",
      "    get:",
      "      responses:",
      "        '200':",
      "          description: OK",
    ].join("\n");

  test.beforeAll(async ({ request }) => {
    if (!adminPassword) {
      throw new Error("INITIAL_ADMIN_PASSWORD environment variable is required for tests");
    }
    const login = await request.post("/auth/login", {
      data: { username: "root", password: adminPassword },
    });
    const { token } = await login.json();
    const headers = { Authorization: `Bearer ${token}` };
    const ok = async (res, what) => {
      if (!res.ok()) throw new Error(`${what} failed: ${res.status()} ${await res.text()}`);
    };
    for (const [version, stability] of [
      ["1.0.0", "snapshot"],
      ["1.1.0", "ga"],
      ["1.2.0", "snapshot"],
    ]) {
      await ok(
        await request.post("/provide", {
          headers,
          data: { producername: PRODUCER, stability, openapi_yaml: openapi(version) },
        }),
        `provide ${version} ${stability}`,
      );
    }
    for (const [client, version] of [
      [OLD_PIN, "1.0.0"],
      [NEW_PIN, "1.2.0"],
    ]) {
      await ok(
        await request.get(
          `/require?consumername=${client}&producername=${PRODUCER}&version=${version}&path=/thing&method=GET`,
          { headers },
        ),
        `${client} requires ${version}`,
      );
    }
  });

  const login = async (page) => {
    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });
  };

  test("the Producers page marks a snapshot below the latest GA as superseded", async ({
    page,
  }) => {
    await login(page);
    await page.goto(`/producers.html?service=${PRODUCER}`);
    // Match the card's own version heading: the superseded text names a version too.
    const card = (version) =>
      page.locator(".endpoint-card", {
        has: page.getByRole("heading", { name: version, exact: true }),
      });
    await expect(card("1.0.0")).toBeVisible({ timeout: 10000 });
    await expect(card("1.0.0").locator("[data-superseded]")).toHaveText("superseded by 1.1.0 ga");
    await expect(card("1.1.0").locator("[data-superseded]")).toHaveCount(0);
    await expect(card("1.2.0").locator("[data-superseded]")).toHaveCount(0);
  });

  test("the graph shows a pin to a superseded snapshot as outdated with the text", async ({
    page,
  }) => {
    await login(page);
    await page.goto("/graph.html");
    await expect(page.locator("#custom-graph")).toBeVisible({ timeout: 10000 });
    const edge = (client) =>
      page.locator(`#custom-graph path.graph-edge[data-from="${client}"][data-to="${PRODUCER}"]`);
    await expect(edge(OLD_PIN)).toHaveAttribute("data-outdated", "1", { timeout: 20000 });
    await expect(edge(OLD_PIN)).toHaveAttribute("data-snapshot", "1");
    await expect(edge(NEW_PIN)).toHaveAttribute("data-outdated", "0");

    const hit = (client) =>
      page.locator(`#custom-graph path.edge-hit[data-from="${client}"][data-to="${PRODUCER}"]`);
    const tooltip = page.locator("#graph-tooltip");
    await hit(OLD_PIN).dispatchEvent("mouseenter");
    await expect(tooltip.locator("[data-superseded]")).toHaveText("superseded by 1.1.0 ga");
    await hit(NEW_PIN).dispatchEvent("mouseenter");
    await expect(tooltip).toContainText("1.2.0");
    await expect(tooltip.locator("[data-superseded]")).toHaveCount(0);

    // The Outdated highlight lights the superseded pin in the outdated colour.
    await page.click("#highlight-outdated");
    await expect(edge(OLD_PIN)).toHaveAttribute("stroke", "#f43f5e");
  });
});

test.describe("Graph toolbar layout", () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

  const openGraphAt = async (page, width) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });
    await page.goto("/graph.html");
    await expect(page.locator("#graph-download-png")).toBeVisible({ timeout: 10000 });
  };
  // The ids (or labels) of visible toolbar controls reaching outside the viewport.
  const controlsOutside = (page) =>
    page.evaluate(() =>
      [...document.querySelectorAll("#graph-toolbar-row :is(button, select, input)")]
        .filter((el) => el.offsetParent !== null)
        .map((el) => ({ id: el.id || el.textContent.trim(), box: el.getBoundingClientRect() }))
        .filter(({ box }) => box.left < 0 || box.right > window.innerWidth)
        .map(({ id }) => id),
    );
  const pageScrollsSideways = (page) =>
    page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth);

  test("every toolbar control fits the viewport at 1440px", async ({ page }) => {
    await openGraphAt(page, 1440);
    expect(await controlsOutside(page)).toEqual([]);
    expect(await pageScrollsSideways(page)).toBe(false);
  });

  test("the toolbar wraps at phone width without a sideways page scroll", async ({ page }) => {
    await openGraphAt(page, 375);
    expect(await controlsOutside(page)).toEqual([]);
    expect(await pageScrollsSideways(page)).toBe(false);
  });
});

test.describe("Observability log lines", () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

  test("a log line shows the local date and time and keeps the module on hover", async ({
    page,
  }) => {
    await page.goto("/account.html");
    await page.waitForSelector("#login-username", { state: "visible" });
    await page.fill('input[id="login-username"]', "root");
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator("#account-dashboard")).toBeVisible({ timeout: 10000 });
    await page.goto("/observability.html");
    const line = page.locator("#log-container .log-line").first();
    await expect(line).toBeVisible({ timeout: 20000 });
    await expect(line.locator(".log-time")).toHaveText(/^\[\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\]$/);
    await expect(line).toHaveAttribute("title", /^sanshain_service/);
    await expect(line).not.toContainText("sanshain_service");
    // The copied text keeps the exact UTC instant and the module.
    const copied = await page.evaluate(() => getLogsAsText().split("\n")[0]);
    expect(copied).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}.*Z? +[A-Z]+ +sanshain_service/);
  });
});
