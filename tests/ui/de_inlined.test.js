import { test, expect } from "@playwright/test";

// Runtime companion to the static tests/no_inline_handlers.rs guard for the CSP
// de-inlining (ai/improvements.md #11). The static test proves no source ships
// an inline on* handler; this proves the replacements are wired correctly:
//
//  1. every data-<event> attribute present in the live DOM names a function
//     that actually resolves on window (a typo like data-click="loadUsres" is
//     a silent dead button the dispatcher only logs about on click), and
//  2. no element in the live DOM carries an inline on* attribute — a second
//     line of defence over the static scan, catching anything injected at
//     runtime.

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

const EVENTS = ["click", "change", "input", "submit", "keydown"];
const ON_ATTRS = [
  "onclick",
  "onchange",
  "oninput",
  "onsubmit",
  "onkeydown",
  "onkeyup",
  "onmouseover",
  "onmouseout",
];

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

const AUTHED_PAGES = [
  "/account.html",
  "/admin.html",
  "/producers.html",
  "/consumers.html",
  "/graph.html",
  "/reports.html",
  "/audit.html",
  "/observability.html",
  "/dashboard",
];

async function auditPage(page) {
  return page.evaluate(
    ({ events, onAttrs }) => {
      const unresolved = [];
      for (const type of events) {
        for (const el of document.querySelectorAll(`[data-${type}]`)) {
          const name = el.getAttribute(`data-${type}`);
          if (typeof window[name] !== "function") {
            unresolved.push(`data-${type}="${name}" (${el.tagName.toLowerCase()})`);
          }
        }
      }
      const inline = [];
      for (const el of document.querySelectorAll("*")) {
        for (const a of onAttrs) {
          if (el.hasAttribute(a)) inline.push(`${a} on <${el.tagName.toLowerCase()}>`);
        }
      }
      return { unresolved, inline };
    },
    { events: EVENTS, onAttrs: ON_ATTRS },
  );
}

for (const path of AUTHED_PAGES) {
  test(`${path} data-action handlers all resolve and no inline handlers remain`, async ({
    page,
  }) => {
    await login(page);
    await page.goto(path);
    await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
    await page.waitForTimeout(1500);
    const { unresolved, inline } = await auditPage(page);
    expect(unresolved, `${path} has dead data-action handlers:\n${unresolved.join("\n")}`).toEqual(
      [],
    );
    expect(inline, `${path} still has inline handlers:\n${inline.join("\n")}`).toEqual([]);
  });
}

test("the public landing page data-action handlers resolve", async ({ page }) => {
  await page.goto("/");
  await page.waitForLoadState("networkidle", { timeout: 15000 }).catch(() => {});
  await page.waitForTimeout(1000);
  const { unresolved, inline } = await auditPage(page);
  expect(unresolved, `landing has dead data-action handlers:\n${unresolved.join("\n")}`).toEqual(
    [],
  );
  expect(inline, `landing still has inline handlers:\n${inline.join("\n")}`).toEqual([]);

  // The landing page (and /dashboard) switched CSS delivery from the Tailwind
  // play-CDN to the committed static app.css; nothing else asserts they are
  // styled. Confirm a Tailwind utility actually applied — text-4xl is ~36px,
  // well above the browser-default h2 (~24px) — so a missing class in the
  // build (e.g. templates dropped from the content globs) can't ship an
  // unstyled front door.
  const h2FontSize = await page
    .locator("h2.text-4xl")
    .first()
    .evaluate((el) => parseFloat(getComputedStyle(el).fontSize));
  expect(
    h2FontSize,
    "landing h2 should render at Tailwind text-4xl size — app.css did not apply",
  ).toBeGreaterThan(30);
});
