// The admin page's rarer paths: the typed confirmation of the destructive
// operations, the metadata dialog of a service and the directory settings the
// sign-in section fills in. Every request that would change the instance is
// stubbed, so the suites after this one find it as it was.
import { test, expect, json } from "./fixtures.js";

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

/**
 * Answers every request whose path passes the test, never the page itself.
 * @param {import("@playwright/test").Page} page
 * @param {(pathname: string) => boolean} matches
 * @param {(route: import("@playwright/test").Route) => Promise<void>} handler
 */
const onPaths = (page, matches, handler) => page.route((url) => matches(url.pathname), handler);

test.beforeEach(async ({ page }) => {
  if (!adminPassword) throw new Error("INITIAL_ADMIN_PASSWORD is required for the UI tests");
  await page.route("**/api/sse/updates", (route) =>
    route.fulfill({ status: 200, contentType: "text/event-stream", body: "" }),
  );
  page.on("dialog", (dialog) => dialog.accept());
  await page.goto("/account.html");
  await page.evaluate(() => {
    localStorage.clear();
  });
  await page.reload();
  await page.fill("#login-username", "root");
  await page.fill("#login-password", adminPassword);
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#dashboard")).toBeVisible({ timeout: 10000 });
});

/**
 * @param {import("@playwright/test").Page} page
 * @param {string} [section] The tab, as the address names it.
 * @param {string} [ready] What the tab shows once it is drawn.
 */
const openAdmin = async (page, section = "", ready = "h2:has-text('Danger Zone')") => {
  await page.goto(`/admin.html${section}`);
  // As in features.test.js: the loader may stay up while the live stream retries.
  try {
    await page.locator("#app-loader").waitFor({ state: "hidden", timeout: 2000 });
  } catch {
    await page.evaluate(() => {
      const loader = document.getElementById("app-loader");
      if (loader) loader.style.display = "none";
    });
  }
  await expect(page.locator(ready).first()).toBeVisible({ timeout: 10000 });
};

test("a destructive operation runs only once its phrase is typed", async ({ page }) => {
  /** @type {string[]} */
  const asked = [];
  await onPaths(
    page,
    (p) => p.startsWith("/admin/nuke/"),
    (route) => {
      asked.push(new URL(route.request().url()).pathname);
      // The services fail, to show the dialog's error; the rest succeed.
      const failed = route.request().url().endsWith("/producers");
      return route.fulfill({
        status: failed ? 500 : 200,
        contentType: "application/json",
        body: "{}",
      });
    },
  );
  await openAdmin(page);
  const modal = page.locator("#nuke-modal");
  const yes = page.locator("#nuke-yes");
  /**
   * @param {string} action
   * @param {string} phrase
   */
  const nuke = async (action, phrase) => {
    await page.evaluate((name) => {
      /** @type {HTMLElement} */ (document.querySelector(`[data-click="${name}"]`)).click();
    }, action);
    await expect(modal).toBeVisible();
    await expect(page.locator("#nuke-expected-text")).toHaveText(phrase);
    await page.fill("#nuke-input", "not it");
    await expect(yes).toBeDisabled();
    await page.fill("#nuke-input", phrase);
    await yes.click();
  };

  await nuke("confirmNukeUsers", "DELETE ALL USERS");
  await expect(modal).toBeHidden();
  await nuke("confirmNukeServices", "DELETE ALL SERVICES");
  await expect(page.locator("#nuke-error")).toHaveText("Operation failed: HTTP 500");
  await nuke("confirmNukeClients", "DELETE ALL CLIENTS");
  await expect(modal).toBeHidden();
  await nuke("confirmNukeDatabase", "NUKE DATABASE");
  await expect(modal).toBeHidden();
  expect(asked).toEqual([
    "/admin/nuke/users",
    "/admin/nuke/producers",
    "/admin/nuke/consumers",
    "/admin/nuke/database",
  ]);
});

test("a service's metadata dialog saves, and says when it could not", async ({ page }) => {
  /** @type {number[]} */
  const answers = [200, 500];
  await onPaths(
    page,
    (p) => p === "/admin/producers/metadata",
    (route) => route.fulfill({ status: answers.shift() ?? 500, body: "" }),
  );
  /** @type {string[]} */
  const alerts = [];
  page.on("dialog", (dialog) => {
    alerts.push(dialog.message());
  });
  await openAdmin(page);
  // The dialog a service row's edit button opens, opened for a name the stub accepts.
  const open = () =>
    page.evaluate(() => {
      openMetadataModal("ui-metadata-fixture", "", "");
    });
  await open();
  const modal = page.locator("#metadata-modal");
  await expect(modal).toBeVisible();
  await page.fill("#metadata-icon", "🛒");
  await page.fill("#metadata-domain", "shop");
  await page.click('#metadata-form button[type="submit"]');
  await expect(modal).toBeHidden();

  await open();
  await page.click('#metadata-form button[type="submit"]');
  await expect.poll(() => alerts).toContain("Failed to save metadata: 500");
});

test("the sign-in section shows the stored directory and OIDC settings", async ({ page }) => {
  await onPaths(
    page,
    (p) => p === "/admin/auth-config",
    async (route) => {
      if (route.request().method() !== "GET") return route.continue();
      const response = await route.fetch();
      const real = /** @type {Record<string, unknown>} */ (await json(response));
      return route.fulfill({
        response,
        json: {
          ...real,
          ldap_config: {
            server_url: "ldaps://ldap.example",
            bind_dn: "cn=reader,dc=example",
            bind_password: "",
            base_dn: "dc=example",
            user_filter: "(uid={username})",
            username_attribute: "uid",
            admin_group: "sanshain-admins",
          },
          oidc_config: {
            issuer_url: "https://id.example",
            client_id: "sanshain",
            redirect_url: "https://sanshain.example/auth/oidc/callback",
          },
        },
      });
    },
  );
  await openAdmin(page);
  await expect(page.locator("#ldap-server-url")).toHaveValue("ldaps://ldap.example");
  await expect(page.locator("#ldap-admin-group")).toHaveValue("sanshain-admins");
  await expect(page.locator("#oidc-issuer-url")).toHaveValue("https://id.example");
  // A field the stored settings leave out stays empty.
  await expect(page.locator("#oidc-client-secret")).toHaveValue("");
});

test("a pasted spec is checked and published, and says why when it is not", async ({ page }) => {
  /** @type {{ status: number, json?: object } | "abort"} */
  let next = { status: 422 };
  await onPaths(
    page,
    (p) => p === "/admin/producers/provide",
    (route) => {
      if (next === "abort") return route.abort();
      return route.fulfill(
        next.json
          ? { status: next.status, json: next.json }
          : { status: next.status, contentType: "application/json", body: '{"error":"no paths"}' },
      );
    },
  );
  await openAdmin(page);
  await page.check("#paste-spec-ga");
  await expect(page.locator("#paste-spec-ga-warning")).toBeVisible();
  await page.uncheck("#paste-spec-ga");
  await expect(page.locator("#paste-spec-ga-warning")).toBeHidden();

  await page.fill("#paste-spec-name", "ui-paste-fixture");
  await page.fill("#paste-spec-content", "openapi: 3.0.3\n");
  const dryRun = page.locator('[data-click="pasteSpecSubmit"][data-click-args="[true]"]');
  const publish = page.locator('[data-click="pasteSpecSubmit"][data-click-args="[false]"]');
  const result = page.locator("#paste-spec-result");

  await dryRun.click();
  await expect(result).toContainText("Rejected (dry run): no paths");
  await publish.click();
  await expect(result).toContainText("Rejected: no paths");

  next = {
    status: 200,
    json: { version: "1.0.0", stability: "snapshot", changes: { inserts: 2 } },
  };
  await dryRun.click();
  await expect(result).toContainText(
    "✓ Valid — would publish ui-paste-fixture 1.0.0 as snapshot (+2 ~0 -0)",
  );
  await publish.click();
  await expect(result).toContainText("✓ Published ui-paste-fixture 1.0.0");
  await expect(page.locator("#paste-spec-content")).toHaveValue("");

  next = "abort";
  await page.fill("#paste-spec-content", "openapi: 3.0.3\n");
  await dryRun.click();
  await expect(result).toContainText("Error:");
});

test("the sign-in settings are saved, and a refusal is said", async ({ page }) => {
  /** @type {number[]} */
  const answers = [200, 500];
  /** @type {unknown[]} */
  const sent = [];
  await onPaths(
    page,
    (p) => p === "/admin/auth-config",
    (route) => {
      if (route.request().method() !== "PUT") return route.continue();
      sent.push(route.request().postDataJSON());
      return route.fulfill({
        status: answers.shift() ?? 500,
        contentType: "application/json",
        body: "{}",
      });
    },
  );
  /** @type {string[]} */
  const alerts = [];
  page.on("dialog", (dialog) => {
    alerts.push(dialog.message());
  });
  await openAdmin(page, "#sign-in", '[data-click="saveAuthConfig"]');
  await page.check('input[name="auth-mode"][value="oidc"]', { force: true });
  await page.fill("#oidc-issuer-url", "https://id.example");
  await page.fill("#oidc-client-id", "sanshain");
  const save = page.locator('[data-click="saveAuthConfig"]');
  await save.click();
  await expect.poll(() => alerts).toContain("Authentication settings saved.");
  expect(sent[0]).toMatchObject({
    auth_mode: "oidc",
    oidc_config: { issuer_url: "https://id.example", client_id: "sanshain", client_secret: null },
  });

  await page.check('input[name="auth-mode"][value="ldap"]', { force: true });
  await save.click();
  await expect.poll(() => alerts.some((a) => a.startsWith("Failed to save"))).toBe(true);
  expect(sent[1]).toMatchObject({ auth_mode: "ldap" });
});

test("a service account the server refuses is not created", async ({ page }) => {
  await onPaths(
    page,
    (p) => p === "/admin/users",
    (route) =>
      route.request().method() === "POST"
        ? route.fulfill({
            status: 409,
            contentType: "application/json",
            body: '{"error":"taken"}',
          })
        : route.continue(),
  );
  /** @type {string[]} */
  const alerts = [];
  page.on("dialog", (dialog) => {
    alerts.push(dialog.message());
  });
  await openAdmin(page, "#users", "#service-account-name");
  await page.fill("#service-account-name", "ui-refused-account");
  await page.click('#service-account-form button[type="submit"]');
  await expect
    .poll(() => alerts.some((a) => a.startsWith("Could not create the service account")))
    .toBe(true);
  await expect(page.locator("#service-account-name")).toHaveValue("ui-refused-account");
});
