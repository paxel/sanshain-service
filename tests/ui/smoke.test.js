import { test, expect } from "@playwright/test";

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

    // Toggle Auth Mode (from Local to Dev)
    const devModeRadio = page.locator('input[name="auth-mode"][value="dev"]');
    await devModeRadio.click();

    // Click Save button
    await page.locator('button:has-text("Save Authentication Settings")').click();
    await page.waitForTimeout(1000); // Wait for API call

    // Refresh to verify persistence.
    // The radios are populated by loadAuthConfig(), an async fetch that resolves
    // *after* #admin-dashboard is rendered, so reading isChecked() once races it.
    // toBeChecked() auto-retries until the fetch lands.
    await page.reload();
    await page.waitForSelector("#admin-dashboard");
    await expect(page.locator('input[name="auth-mode"][value="dev"]')).toBeChecked();

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
      data: { producername: SERVICE, stability: "snapshot", openapi_yaml: SPEC },
    });
    await request.post("/provide", {
      headers: { Authorization: `Bearer ${token}` },
      data: {
        producername: `${SERVICE}-admin`,
        stability: "snapshot",
        openapi_yaml: SPEC,
      },
    });
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
      window.apiCall = (url) =>
        new Promise((resolve) => {
          const key = url.includes("T01") ? "older" : "newer";
          resolvers[key] = () => resolve({ ok: true, json: async () => [{ marker: key }] });
        });
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
        };
      } finally {
        window.apiCall = originalApi;
        window.redrawGraph = originalRedraw;
        window.graphTimelineAt = null;
        window.graphTimelinePins = null;
      }
    });
    // …and must not win.
    expect(result.at).toBe("2026-01-01T02:00:00Z");
    expect(result.marker).toBe("newer");
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
      await page.waitForTimeout(400);
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
