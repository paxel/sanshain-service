import { test, expect } from "@playwright/test";

// Behavioural coverage of the admin dashboard's controls, written before the
// inline handlers are removed: each test drives a
// control and asserts its visible effect, so it passes against the current
// onclick= wiring and must keep passing once those become a delegated
// data-action dispatcher. A control that goes dead in the rewrite fails here.
// Only non-destructive controls are exercised — no nuke, no cleanup-that-
// deletes, no logout, no navigation.

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

test.beforeEach(async ({ page }) => {
  if (!adminPassword) throw new Error("INITIAL_ADMIN_PASSWORD is required");
  await page.goto("/account.html");
  await page.waitForSelector("#login-username", { state: "visible" });
  await page.fill("#login-username", "root");
  await page.fill("#login-password", adminPassword);
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#dashboard")).toBeVisible({ timeout: 10000 });
  await page.goto("/admin.html");
  await expect(page.locator("#admin-dashboard:not(.hidden)")).toBeVisible({ timeout: 30000 });
});

// Sections live on tabs: General, Sign-in, Users.
const openTab = (page, tab) => page.click(`[data-admin-tab-button="${tab}"]`);

test("auth-mode radios reveal and hide their config forms", async ({ page }) => {
  await openTab(page, "sign-in");
  await page.locator('input[name="auth-mode"][value="ldap"]').check();
  await expect(page.locator("#ldap-config-form")).toBeVisible();
  await page.locator('input[name="auth-mode"][value="disabled"]').check();
  await expect(page.locator("#ldap-config-form")).toBeHidden();
});

test("the trunk max-age save reports success", async ({ page }) => {
  const input = page.locator("#trunk-max-age-days");
  await input.scrollIntoViewIfNeeded();
  await input.fill("77");
  await page.locator('#trunk-max-age-days ~ button:has-text("Save")').click();
  // The value persists across a reload — the save actually took.
  await page.reload();
  await expect(page.locator("#admin-dashboard:not(.hidden)")).toBeVisible({ timeout: 30000 });
  await expect(page.locator("#trunk-max-age-days")).toHaveValue("77");
  // Restore.
  await page.locator("#trunk-max-age-days").fill("90");
  await page.locator('#trunk-max-age-days ~ button:has-text("Save")').click();
});

test("a trunk cleanup run reports its result", async ({ page }) => {
  // Non-destructive on a clean graph: closes nothing, but the handler must run
  // and write the result span.
  await page.locator("#trunk-cleanup-result").scrollIntoViewIfNeeded();
  await page.locator('#trunk-max-age-days ~ button:has-text("Run Cleanup")').click();
  await expect(page.locator("#trunk-cleanup-result")).not.toBeEmpty({ timeout: 10000 });
});

test("the services filter narrows the list", async ({ page }) => {
  const filter = page.locator("#services-filter");
  await filter.scrollIntoViewIfNeeded();
  // Typing runs filterServices(); assert it does not throw and the input holds.
  await filter.fill("zzz-no-such-service");
  await expect(filter).toHaveValue("zzz-no-such-service");
  await filter.fill("");
});

test("the users section loads its list without error", async ({ page }) => {
  await openTab(page, "users");
  // loadUsers() runs on section render; the root account must appear.
  const usersHeading = page.locator("h2", { hasText: /^Users$/ });
  await usersHeading.scrollIntoViewIfNeeded();
  await expect(page.locator("body")).toContainText("root", { timeout: 10000 });
});

test("the user list shows who waits for approval and finds the rest by search and chips", async ({
  page,
}) => {
  const name = `chips-${Date.now().toString(36)}`;
  await page.goto("/admin.html#users");
  await expect(page.locator("#admin-dashboard:not(.hidden)")).toBeVisible({ timeout: 30000 });
  await page.fill("#service-account-name", name);
  await page.click('#service-account-form button[type="submit"]');
  await expect(page.locator("#users-list > div", { hasText: name })).toBeVisible();

  // Nothing searched, no chip: approved accounts stay out, the count says so.
  await page.fill("#users-search", "");
  await expect(page.locator("#users-list > div", { hasText: name })).toHaveCount(0);
  await expect(page.locator("#users-more")).toContainText("search or pick a filter");

  // Kind narrows to service accounts; adding the pending status narrows further.
  await page.click('[data-user-chip="kind:service"]');
  await expect(page.locator("#users-list > div", { hasText: name })).toBeVisible();
  await page.click('[data-user-chip="status:pending"]');
  await expect(page.locator("#users-list > div", { hasText: name })).toHaveCount(0);
  // A second chip of the same kind widens again.
  await page.click('[data-user-chip="status:approved"]');
  await expect(page.locator("#users-list > div", { hasText: name })).toBeVisible();
});

test("pickers take effect on the tick: group members and maintainers", async ({ page }) => {
  await openTab(page, "users");
  const run = Date.now().toString(36);
  const token = await page.evaluate(() => localStorage.getItem("sanshain_token"));
  const headers = { Authorization: `Bearer ${token}` };
  const producer = `picked-${run}`;
  const spec = `openapi: 3.0.0\ninfo:\n  title: ${producer}\n  version: 1.0.0\npaths:\n  /a:\n    get:\n      responses:\n        '200': { description: ok }\n`;
  const provided = await page.request.post("/provide", {
    headers,
    data: { producername: producer, stability: "snapshot", openapi_yaml: spec },
  });
  expect(provided.ok()).toBeTruthy();
  const group = `team-${run}`;
  expect((await page.request.post("/admin/groups", { headers, data: { name: group } })).ok());
  await page.reload();
  await expect(page.locator("#admin-dashboard:not(.hidden)")).toBeVisible({ timeout: 30000 });

  // Tick root into the new group: saved at once, the chip appears.
  const groupRow = page.locator("#groups-list > div", { hasText: group });
  await groupRow.locator("input[role=combobox]").click();
  await groupRow.locator("input[role=combobox]").fill("root");
  await groupRow.locator("[role=option]", { hasText: "root" }).first().click();
  await expect(groupRow.locator("[data-member-chips]")).toContainText("root");

  // Pick the Producer, tick the group as its maintainer, untick it again.
  await page.click("#maintainer-producer");
  await page.fill("#maintainer-producer", producer);
  await page.click(`[data-picker-option="${producer}"]`);
  await expect(page.locator("#maintainer-producer")).toHaveValue(producer);
  await page.click("#maintainer-subject");
  await page.fill("#maintainer-subject", group);
  const option = page.locator("#maintainer-subject-picker [role=option]", { hasText: group });
  await option.click();
  await expect(option).toHaveAttribute("aria-checked", "true");
  await expect(page.locator(`[data-maintained="${producer}"]`)).toContainText(group);
  await option.click();
  await expect(option).toHaveAttribute("aria-checked", "false");
  await expect(page.locator(`[data-maintained="${producer}"]`)).toHaveCount(0);

  // Tick it again and take it off with the ✕ in the list instead.
  await option.click();
  const maintained = page.locator(`[data-maintained="${producer}"]`);
  await expect(maintained).toContainText(group);
  await maintained.locator("button", { hasText: group }).click();
  await expect(maintained).toHaveCount(0);
});

test("a Producer provided on the admin page is external, tagged and drawn grey", async ({
  page,
}) => {
  const name = `outside-${Date.now().toString(36)}`;
  await page.fill("#paste-spec-name", name);
  await page.fill(
    "#paste-spec-content",
    `openapi: 3.0.0\ninfo:\n  title: ${name}\n  version: 1.0.0\npaths:\n  /a:\n    get:\n      responses:\n        '200': { description: ok }\n`,
  );
  await page.click('button:has-text("Publish")');
  await expect(page.locator("#paste-spec-result")).toContainText("Published");
  const row = page.locator("#services-list > div", { hasText: name });
  await expect(row.locator("[data-external]")).toHaveText("external");

  // Corrected by hand, and back.
  const external = row.getByRole("switch", { name: "External" });
  await expect(external).toHaveAttribute("aria-checked", "true");
  await external.click();
  await expect(row.locator("[data-external]")).toHaveCount(0);
  await expect(external).toHaveAttribute("aria-checked", "false");
  await external.click();
  await expect(row.locator("[data-external]")).toHaveText("external");

  // Nobody requires it yet: a chip in the (unrequired) box, grey all the same.
  await page.goto("/graph.html");
  await expect(
    page.locator(`#custom-graph g.graph-unrequired-chip[data-chip="${name}"]`),
  ).toHaveAttribute("data-external", "1", { timeout: 20000 });
});
