// The graph's timeline range and shareable links: the range fields, the
// snapping slider, and the view, branch, range, instant and display switches
// carried in the address.
import { test, expect } from "./fixtures.js";

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;
const BRANCH = "Timeline Link Branch";

async function signIn(page) {
  if (!adminPassword) {
    throw new Error("INITIAL_ADMIN_PASSWORD environment variable is required for tests");
  }
  await page.goto("/account.html");
  await page.waitForSelector("#login-username", { state: "visible" });
  await page.fill('input[id="login-username"]', "root");
  await page.fill('input[id="login-password"]', adminPassword);
  await page.click('#login-panel button[type="submit"]');
  await expect(page.locator("#dashboard")).toBeVisible({ timeout: 10000 });
}

const param = (page, name) => new URL(page.url()).searchParams.get(name);

test.describe("Graph timeline and links", () => {
  test("the range helpers default to 3 months, check days and snap", async ({ page }) => {
    await signIn(page);
    await page.goto("/graph.html");
    const result = await page.evaluate(() => {
      const now = Date.parse("2026-11-05T10:00:00Z");
      const range = { from: "2026-11-01", to: "2026-11-05" };
      const { stops, max } = window.graphTimelineStops(
        range,
        ["2026-10-31T12:00:00Z", "2026-11-02T00:00:00Z", "2026-11-04T00:00:00Z"],
        now,
      );
      return {
        range: window.graphDefaultTimelineRange(new Date(now)),
        days: ["2026-02-28", "2026-02-30", "2026-2-1", ""].map(window.graphIsTimelineDay),
        past: window.graphTimelineEdges(range, new Date("2026-12-01T00:00:00Z")),
        today: window.graphTimelineEdges(range, new Date(now)),
        stops: stops.map((s) => s.at),
        max,
        near: window.graphNearestStop(stops, 200000).at,
      };
    });
    expect(result.range).toEqual({ from: "2026-08-05", to: "2026-11-05" });
    expect(result.days).toEqual([true, false, false, false]);
    expect(result.past).toEqual({ start: "2026-11-01T00:00:00Z", end: "2026-11-05T23:59:59Z" });
    expect(result.today).toEqual({ start: "2026-11-01T00:00:00Z", end: null });
    // Both edges and the change dates inside; the right edge is "now".
    expect(result.stops).toEqual([
      "2026-11-01T00:00:00Z",
      "2026-11-02T00:00:00Z",
      "2026-11-04T00:00:00Z",
      null,
    ]);
    expect(result.max).toBe(4 * 86400 + 10 * 3600);
    expect(result.near).toBe("2026-11-04T00:00:00Z");
  });

  test("a link opens its branch, range and switches, and keeps them", async ({ page, request }) => {
    const login = await request.post("/auth/login", {
      data: { username: "root", password: adminPassword },
    });
    const { token } = await login.json();
    await request.post("/admin/branches", {
      headers: { Authorization: `Bearer ${token}` },
      data: { name: BRANCH },
    });
    await signIn(page);

    const today = new Date().toISOString().slice(0, 10);
    const query = new URLSearchParams({
      view: "branch",
      branch: BRANCH,
      from: "2026-01-01",
      to: today,
      compact: "1",
    });
    await page.goto(`/graph.html?${query}`);
    await expect(page.locator("#graph-timeline-row")).toBeVisible({ timeout: 10000 });
    await expect(page.locator("#graph-branch-select")).toHaveValue(BRANCH);
    await expect(page.locator("#graph-timeline-from")).toHaveValue("2026-01-01");
    await expect(page.locator("#graph-timeline-to")).toHaveValue(today);
    await expect(page.locator("#graph-compact-toggle")).toHaveAttribute("aria-pressed", "true");
    await expect(page.locator("#graph-timeline-now")).toBeVisible();
    await expect(page.locator("#graph-url-notice")).toBeHidden();
    expect(param(page, "view")).toBe("branch");
    expect(param(page, "branch")).toBe(BRANCH);
    expect(param(page, "from")).toBe("2026-01-01");
    expect(param(page, "compact")).toBe("1");

    // Switching to main keeps the range and adds a history entry.
    await page.click("#stream-main");
    await expect.poll(() => param(page, "view")).toBe("main");
    expect(param(page, "branch")).toBeNull();
    expect(param(page, "from")).toBe("2026-01-01");
    await page.goBack();
    await expect.poll(() => param(page, "view")).toBe("branch");
    await expect(page.locator("#graph-branch-select")).toHaveValue(BRANCH);
  });

  test("a range ending in the past opens at the end of its last day", async ({ page }) => {
    await signIn(page);
    await page.goto("/graph.html?view=main&from=2026-01-01&to=2026-01-31");
    await expect(page.locator("#graph-timeline-row")).toBeVisible({ timeout: 10000 });
    await expect(page.locator("#graph-timeline-label")).toHaveText("2026-01-31 23:59");
    await expect(page.locator("#graph-timeline-now")).toBeHidden();
    await expect.poll(() => param(page, "at")).toBe("2026-01-31T23:59:59Z");
  });

  test("an instant outside the range widens it to the instant's day", async ({ page }) => {
    await signIn(page);
    await page.goto("/graph.html?view=main&at=2025-06-01T12:00:00Z");
    await expect(page.locator("#graph-timeline-row")).toBeVisible({ timeout: 10000 });
    await expect(page.locator("#graph-timeline-from")).toHaveValue("2025-06-01");
    await expect(page.locator("#graph-timeline-label")).toHaveText("2025-06-01 12:00");
    expect(param(page, "from")).toBe("2025-06-01");

    // An instant that is no RFC 3339 is dropped: the view opens at "now".
    await page.goto("/graph.html?view=main&at=June%201%202025");
    await expect(page.locator("#graph-timeline-row")).toBeVisible({ timeout: 10000 });
    await expect(page.locator("#graph-timeline-label")).toHaveText("now");
    expect(param(page, "at")).toBeNull();
  });

  test("an unknown branch opens main with a notice", async ({ page }) => {
    await signIn(page);
    await page.goto("/graph.html?view=branch&branch=No%20Such%20Branch&from=2026-01-01");
    await expect(page.locator("#graph-url-notice")).toHaveText(
      "There is no sanshain-branch 'No Such Branch': showing main instead.",
      { timeout: 10000 },
    );
    await expect.poll(() => param(page, "view")).toBe("main");
    expect(param(page, "from")).toBe("2026-01-01");
  });

  test("a start after the end is refused and the range stays", async ({ page }) => {
    await signIn(page);
    await page.goto("/graph.html?view=main&from=2026-01-01&to=2026-02-01");
    const from = page.locator("#graph-timeline-from");
    await expect(from).toHaveValue("2026-01-01", { timeout: 10000 });
    await from.fill("2026-03-01");
    await from.dispatchEvent("change");
    await expect(from).toHaveAttribute("aria-invalid", "true");
    expect(param(page, "from")).toBe("2026-01-01");

    // A valid start becomes the range, in the address.
    await from.fill("2026-01-15");
    await from.dispatchEvent("change");
    await expect(from).toHaveAttribute("aria-invalid", "false");
    await expect.poll(() => param(page, "from")).toBe("2026-01-15");

    // In a link, the same mistake opens the default range.
    await page.goto("/graph.html?view=main&from=2026-03-01&to=2026-02-01");
    await expect(page.locator("#graph-timeline-row")).toBeVisible({ timeout: 10000 });
    await expect(page.locator("#graph-timeline-to")).toHaveValue(
      new Date().toISOString().slice(0, 10),
    );
    expect(param(page, "from")).toBeNull();
  });
});
