// The Products page: a Product is created from the list, gets its members from a
// pasted Chart.yaml, loses one by hand and is deleted again.
import { test, expect } from "./fixtures.js";

const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;
const PRODUCT = `ui-product-${Date.now().toString(36)}`;
// A system the graph tests registered; a dependency nobody registered is skipped.
const MEMBER = "g10-a2";

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

test("a Product is created, filled from a Chart.yaml, trimmed and deleted", async ({ page }) => {
  await page.goto("/products.html");
  await page.fill("#product-create-name", PRODUCT);
  await page.click("#product-create-button");
  await expect(page.locator("#product-name")).toHaveText(PRODUCT, { timeout: 10000 });
  await expect(page.locator("#product-maintainers")).toContainText("Nobody maintains");

  // A chart that is no YAML is refused with the server's message.
  await page.fill("#product-chart-yaml", "dependencies: [");
  await page.click("#product-chart-import");
  await expect(page.locator("[data-import-error]")).toBeVisible();

  await page.fill(
    "#product-chart-yaml",
    `apiVersion: v2\nname: shop\ndependencies:\n  - name: ${MEMBER}\n  - name: nobody-registered-this\n`,
  );
  await page.click("#product-chart-import");
  await expect(page.locator('[data-import="added"]')).toContainText(MEMBER);
  await expect(page.locator('[data-import="skipped"]')).toContainText("nobody-registered-this");
  const row = page.locator(`[data-member="${MEMBER}"]`);
  await expect(row.locator("[data-origin]")).toHaveText("Helm");

  await row.locator('button:has-text("Remove")').click();
  await expect(row).toHaveCount(0);

  // Back to the list, where the Product has its card, and in again by the address.
  await page.goto("/products.html");
  await expect(page.locator("#products-list")).toContainText(PRODUCT);
  await page.goto(`/products.html?name=${encodeURIComponent(PRODUCT)}`);
  await expect(page.locator("#product-name")).toHaveText(PRODUCT);
  await page.click("#product-delete-button");
  await expect(page.locator("#products-list-view")).toBeVisible();
  await expect(page.locator("#products-list")).not.toContainText(PRODUCT);
});
