import { test, expect } from '@playwright/test';

test.describe('Sanshain YAML Viewer UI Tests', () => {
  test('YAML viewer page loaded with correct elements', async ({ page }) => {
    // Use a real service from demo.sh; without a version param the viewer
    // loads the endpoint's full blame trail and selects the newest version
    // that carries the endpoint.
    await page.goto('/yaml.html?service=user-service&api_type=openapi&path=/users&method=GET');

    // Header should be visible
    await expect(page.locator('header#site-banner')).toBeVisible();

    // Breadcrumbs should contain service name and the version line
    const breadcrumbService = page.locator('#breadcrumb-service');
    await expect(breadcrumbService).toBeVisible({ timeout: 10000 });
    await expect(breadcrumbService).toContainText('user-service');
    await expect(page.locator('#breadcrumb-version')).toContainText('openapi');

    // Endpoint header details should be present
    await expect(page.locator('#endpoint-method')).toContainText('GET');
    await expect(page.locator('#endpoint-path')).toContainText('/users');

    // Right panel action buttons should be visible
    await expect(page.locator('#view-yaml-btn')).toBeVisible();
    await expect(page.locator('#view-diff-btn')).toBeVisible();
    await expect(page.locator('#blame-toggle-container')).toBeVisible();
    await expect(page.locator('#copy-btn')).toBeVisible();
    await expect(page.locator('#download-btn')).toBeVisible();
  });
});
