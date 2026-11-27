// san-1021: the statistics page on /reports.html: its tabs, the range in the
// URL, the share link, and the exports of each panel. The tab tickets add
// their panels below.
import { test, expect, json } from "./fixtures.js";

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

/** @param {import("@playwright/test").Page} page */
async function signIn(page) {
  await page.route("**/api/sse/updates", (route) =>
    route.fulfill({ status: 200, contentType: "text/event-stream", body: "" }),
  );
  await page.goto("/account.html");
  await page.waitForSelector("#login-username", { state: "visible" });
  await page.fill("#login-username", "root");
  await page.fill("#login-password", /** @type {string} */ (adminPassword));
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#dashboard")).toBeVisible({ timeout: 10000 });
}

test("the page carries its tab and range in the URL", async ({ page }) => {
  await signIn(page);
  await page.goto("/reports.html");
  await expect(page.locator("#statistics-tab-landscape")).toHaveAttribute("aria-selected", "true");
  await expect(page).toHaveURL(/tab=landscape&from=\d{4}-\d{2}-\d{2}&to=\d{4}-\d{2}-\d{2}/);
  await expect(page.locator("#statistics-preset")).toHaveValue("90");

  await page.click("#statistics-tab-activity");
  await expect(page).toHaveURL(/tab=activity/);
  await expect(page.locator("#statistics-tab-activity")).toHaveAttribute("aria-selected", "true");

  await page.selectOption("#statistics-preset", "30");
  await expect(page.locator("#statistics-preset")).toHaveValue("30");
  const from = await page.locator("#statistics-from").inputValue();
  const to = await page.locator("#statistics-to").inputValue();
  const days = (Date.parse(to) - Date.parse(from)) / 86400000 + 1;
  expect(days).toBe(30);

  // A custom range is kept as it is, and a reload shows the same.
  await page.fill("#statistics-from", "2026-01-01");
  await page.locator("#statistics-from").dispatchEvent("change");
  await expect(page).toHaveURL(/from=2026-01-01/);
  await page.reload();
  await expect(page.locator("#statistics-preset")).toHaveValue("custom");
  await expect(page.locator("#statistics-from")).toHaveValue("2026-01-01");
  await expect(page.locator("#statistics-tab-activity")).toHaveAttribute("aria-selected", "true");
});

test("an unknown tab in the URL falls back to Landscape", async ({ page }) => {
  await signIn(page);
  await page.goto("/reports.html?tab=nope");
  await expect(page.locator("#statistics-tab-landscape")).toHaveAttribute("aria-selected", "true");
});

test("Landscape shows its panels and exports CSV, SVG and PNG", async ({ page }) => {
  await signIn(page);
  await page.goto("/reports.html?tab=landscape");
  const panel = page.locator("#statistics-panel-landscape-counts");
  await expect(panel).toContainText("What exists");
  await expect(panel).toContainText("For architects and management");
  await expect(panel.locator("svg").first()).toBeVisible();
  await expect(page.locator("#statistics-panel-landscape-active")).toBeVisible();
  await expect(page.locator("#statistics-panel-landscape-api-types")).toBeVisible();
  await expect(page.locator("#statistics-panel-landscape-groups")).toBeVisible();

  const csv = page.waitForEvent("download");
  await panel.getByRole("button", { name: "CSV" }).click();
  const csvFile = await csv;
  expect(csvFile.suggestedFilename()).toMatch(/^sanshain-landscape-counts-.*\.csv$/);
  const text = await (await csvFile.createReadStream()).toArray();
  expect(Buffer.concat(text).toString()).toMatch(/^Kind,Count\r\nProducers,\d+/);

  const svg = page.waitForEvent("download");
  await panel.getByRole("button", { name: "SVG" }).click();
  expect((await svg).suggestedFilename()).toMatch(/\.svg$/);

  const png = page.waitForEvent("download");
  await panel.getByRole("button", { name: "PNG" }).click();
  expect((await png).suggestedFilename()).toMatch(/\.png$/);
});

test("Activity shows calls over time, per owner, the busiest systems and refusals", async ({
  page,
}) => {
  await signIn(page);
  await page.goto("/reports.html?tab=activity");
  for (const id of ["activity-calls", "activity-owners", "activity-top", "activity-refusals"]) {
    await expect(page.locator(`#statistics-panel-${id}`)).toBeVisible();
  }
  await expect(page.locator("#statistics-panel-activity-calls")).toContainText("per day");
  await page.selectOption("#statistics-preset", "365");
  await expect(page.locator("#statistics-panel-activity-calls")).toContainText("per week starting");
});

test("Versions and releases shows its panels", async ({ page }) => {
  await signIn(page);
  await page.goto("/reports.html?tab=versions");
  for (const id of [
    "versions-created",
    "versions-promotions",
    "versions-per-producer",
    "versions-pins",
    "versions-deprecated",
    "versions-retired",
  ]) {
    await expect(page.locator(`#statistics-panel-${id}`)).toBeVisible();
  }
});

test("Tags and branches shows its panels", async ({ page }) => {
  await signIn(page);
  await page.goto("/reports.html?tab=branches");
  for (const id of ["branches-builds", "branches-list", "branches-tags"]) {
    await expect(page.locator(`#statistics-panel-${id}`)).toBeVisible();
  }
});

test("Users and access shows counts only", async ({ page }) => {
  await signIn(page);
  await page.goto("/reports.html?tab=access");
  for (const id of ["access-overview", "access-roles", "access-groups", "access-tokens"]) {
    await expect(page.locator(`#statistics-panel-${id}`)).toBeVisible();
  }
  await expect(page.locator("#statistics-body")).not.toContainText("root");
});

test("the Catalog lists systems, filters them and opens a sheet from the URL", async ({
  page,
  request,
}) => {
  const stamp = Date.now().toString(36);
  const producer = `catalog-api-${stamp}`;
  const consumer = `catalog-app-${stamp}`;
  const login = await request.post("/auth/login", {
    data: { username: "root", password: adminPassword },
  });
  const { token } = /** @type {{ token: string }} */ (await json(login));
  const headers = { Authorization: `Bearer ${token}` };
  const spec = [
    "openapi: 3.0.0",
    "info:",
    `  title: ${producer}`,
    "  version: 1.0.0",
    "paths:",
    "  /items:",
    "    get:",
    "      responses:",
    "        '200': { description: ok }",
  ].join("\n");
  const provided = await request.post("/provide", {
    headers,
    data: { producername: producer, stability: "ga", openapi_yaml: spec },
  });
  expect(provided.ok(), await provided.text()).toBeTruthy();
  const required = await request.get("/require", {
    headers,
    params: {
      consumername: consumer,
      producername: producer,
      version: "1.0.0",
      path: "/items",
      method: "GET",
    },
  });
  expect(required.ok(), await required.text()).toBeTruthy();

  await signIn(page);
  await page.goto("/reports.html?tab=catalog");
  const panel = page.locator("#statistics-panel-catalog-systems");
  await expect(page.locator("#catalog-root")).toContainText("Pick a system");
  await page.selectOption("#catalog-filter-domain", "no domain");
  await expect(page).toHaveURL(/domain=no(\+|%20)domain/);
  await page
    .locator("#catalog-root button[data-click='catalogOpen']", { hasText: consumer })
    .click();
  await expect(page).toHaveURL(new RegExp(`system=${consumer}`));
  const sheet = page.locator("#catalog-sheet article");
  await expect(sheet).toContainText(consumer);
  await expect(sheet).toContainText(producer);
  await page.reload();
  await expect(page.locator("#catalog-sheet article")).toContainText(producer);
  await expect(page.locator("#catalog-filter-domain")).toHaveValue("no domain");

  // Both print paths go through window.print.
  await page.evaluate(() => {
    const counter = /** @type {Window & { printed: number }} */ (/** @type {unknown} */ (window));
    counter.printed = 0;
    window.print = () => {
      counter.printed += 1;
    };
  });
  await page.click("#catalog-print-all");
  expect(await page.locator("#catalog-print-sheets article").count()).toBeGreaterThan(0);
  await page.getByRole("button", { name: "Print sheet" }).click();
  const printed = await page.evaluate(
    () => /** @type {Window & { printed: number }} */ (/** @type {unknown} */ (window)).printed,
  );
  expect(printed).toBe(2);

  const csv = page.waitForEvent("download");
  await panel.getByRole("button", { name: "CSV" }).click();
  expect((await csv).suggestedFilename()).toMatch(/^sanshain-catalog-systems-.*\.csv$/);
});

test("the former reports are Landscape panels: isolation sorts, dependencies filter", async ({
  page,
}) => {
  await signIn(page);
  await page.goto("/reports.html?tab=landscape");
  await expect(page.locator("#reports-grid")).toHaveCount(0);
  const isolation = page.locator("#statistics-panel-landscape-isolation");
  await expect(isolation).toContainText("Producer isolation");
  await isolation.getByRole("button", { name: /System/ }).click();
  const deps = page.locator("#statistics-panel-landscape-dependencies");
  await expect(deps).toContainText("Every dependency");
  await page.fill("#statistics-filter-landscape-dependencies", "no-such-system-anywhere");
  await expect(deps).toContainText("Nothing in this range.");
  const csv = page.waitForEvent("download");
  await deps.getByRole("button", { name: "CSV" }).click();
  const file = await csv;
  const text = Buffer.concat(await (await file.createReadStream()).toArray()).toString();
  expect(text).toBe("Consumer,Producer,API type,Pin,Stability,Method,Path\r\n");
});

// san-1036: each domain keeps its graph colour on both sides of the sankey,
// and the systems without a domain are grey.
test("the domain sankey colours each domain like the graph does", async ({ page }) => {
  await signIn(page);
  await page.route("**/admin/statistics/landscape?*", async (route) => {
    const response = await route.fetch();
    const answer = /** @type {{ data: { flows: unknown[] } }} */ (await json(response));
    answer.data.flows = [
      { from: "payments", to: "billing", pairs: 2 },
      { from: "no domain", to: "payments", pairs: 1 },
    ];
    await route.fulfill({ response, json: answer });
  });
  await page.goto("/reports.html?tab=landscape");
  const chart = page.locator("#statistics-chart-landscape-flows svg");
  await expect(chart).toBeVisible();
  const colours = await page.evaluate(() => ({
    payments: window.graphDomainColor("payments").border,
    billing: window.graphDomainColor("billing").border,
  }));
  const svg = await chart.evaluate((el) => el.outerHTML.toLowerCase());
  expect(svg).toContain(colours.payments);
  expect(svg).toContain(colours.billing);
  expect(svg).toContain("#94a3b8");
});

// san-1035: the admin's first day of the week starts the weeks; a daily chart
// bands every second one, a weekly chart none.
test("the first day of the week is set on the admin page and bands the daily charts", async ({
  page,
}) => {
  await signIn(page);
  await page.goto("/admin.html");
  await page.click('[data-admin-tab-button="general"]');
  await expect(page.locator("#week-start")).toHaveValue("monday");
  await page.selectOption("#week-start", "sunday");
  await page.locator('[data-click="saveWeekStart"]').click();
  await expect(page.locator("#week-start-result")).toHaveText("✓ Saved");
  await page.reload();
  await page.click('[data-admin-tab-button="general"]');
  await expect(page.locator("#week-start")).toHaveValue("sunday");

  const answer = page.waitForResponse((r) => r.url().includes("/admin/statistics/activity?"));
  await page.goto("/reports.html?tab=activity&from=2026-11-01&to=2026-11-21");
  /** @type {unknown} */
  const body = await (await answer).json();
  const periods = /** @type {{ data: { periods: { week_start: string } } }} */ (body).data.periods;
  expect(periods.week_start).toBe("sunday");
  await expect(page.locator("#statistics-chart-activity-calls svg")).toBeVisible();

  // 2026-11-01 is a Sunday: the second week, the 8th to the 14th, is banded.
  const bands = await page.evaluate(() => {
    const starts = Array.from(
      { length: 21 },
      (_, i) => `2026-11-${String(i + 1).padStart(2, "0")}`,
    );
    return {
      sunday: statisticsWeekBands({ bucket: "day", week_start: "sunday", starts }),
      monday: statisticsWeekBands({ bucket: "day", week_start: "monday", starts }),
      weekly: statisticsWeekBands({ bucket: "week", week_start: "sunday", starts }),
    };
  });
  expect(bands.sunday).toEqual([["2026-11-08", "2026-11-14"]]);
  expect(bands.monday).toEqual([
    ["2026-11-02", "2026-11-08"],
    ["2026-11-16", "2026-11-21"],
  ]);
  expect(bands.weekly).toEqual([]);

  await page.goto("/admin.html");
  await page.click('[data-admin-tab-button="general"]');
  await page.selectOption("#week-start", "monday");
  await page.locator('[data-click="saveWeekStart"]').click();
  await expect(page.locator("#week-start-result")).toHaveText("✓ Saved");
});
