// The landing page's free validator: it suggests the format of what is pasted
// and says what the validator answered, whatever that was. The answers are
// stubbed, so each kind of verdict is shown once.
import { test, expect } from "./fixtures.js";

/**
 * Answers the validator's request, never the page.
 * @param {import("@playwright/test").Page} page
 * @param {(route: import("@playwright/test").Route) => Promise<void>} handler
 */
const onValidate = (page, handler) => page.route((url) => url.pathname === "/validate", handler);

/**
 * @param {import("@playwright/test").Page} page
 * @param {object} verdict
 */
const answer = (page, verdict) =>
  onValidate(page, (route) => route.fulfill({ status: 200, json: verdict }));

test.beforeEach(async ({ page }) => {
  await page.route("**/api/sse/updates", (route) =>
    route.fulfill({ status: 200, contentType: "text/event-stream", body: "" }),
  );
  await page.goto("/");
});

test("the validator suggests the format of what is pasted", async ({ page }) => {
  const input = page.locator("#validator-input");
  await input.fill("asyncapi: 3.0.0\ninfo:\n  title: Events\n");
  await expect(page.locator('[data-validate="asyncapi"]')).toHaveClass(/ring-2/);
  await expect(page.locator('[data-validate="openapi"]')).not.toHaveClass(/ring-2/);
  await input.fill('syntax = "proto3";\npackage shop;\n');
  await expect(page.locator('[data-validate="proto"]')).toHaveClass(/ring-2/);
  await input.fill("nothing it knows");
  await expect(page.locator(".validator-btn.ring-2")).toHaveCount(0);
});

test("the validator shows every kind of answer", async ({ page }) => {
  const result = page.locator("#validator-result");
  const validate = () => page.click('[data-validate="openapi"]');

  await validate();
  await expect(result).toHaveText("Paste a specification first.");

  await page.locator("#validator-input").fill("openapi: 3.0.3\n");
  await onValidate(page, (route) => route.fulfill({ status: 503, body: "" }));
  await validate();
  await expect(result).toHaveText("The validator answered 503.");

  await page.unrouteAll();
  await answer(page, {
    valid: true,
    version: "1.2.0",
    endpoint_count: 3,
    endpoints: ["GET /orders"],
  });
  await validate();
  await expect(result).toContainText("✓ Valid openapi — version 1.2.0, 3 endpoints");
  await expect(result).toContainText("GET /orders");
  await expect(result).toContainText("… and 2 more");

  await page.unrouteAll();
  await answer(page, { valid: true, version: "1.0.0", endpoint_count: 1, endpoints: [] });
  await validate();
  await expect(result).toContainText("1 endpoint");
  await expect(result).not.toContainText("would store");

  await page.unrouteAll();
  await answer(page, {
    valid: false,
    error: "paths: <missing>",
    endpoints: ["GET /half"],
  });
  await validate();
  await expect(result).toContainText("✗ Not valid as openapi");
  // The message is text, never markup.
  await expect(result).toContainText("paths: <missing>");
  await expect(result).toContainText("GET /half");

  await page.unrouteAll();
  await answer(page, { valid: false, endpoints: [] });
  await validate();
  await expect(result).toContainText("Unknown error");

  await page.unrouteAll();
  await onValidate(page, (route) => route.abort());
  await validate();
  await expect(result).toContainText("Error:");
});
