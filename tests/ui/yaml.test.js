import { test, expect } from '@playwright/test';

test.describe('Sanshain YAML Viewer UI Tests', () => {
  test('YAML viewer page loaded with correct elements', async ({ page }) => {
    // Use a real service from demo.sh
    await page.goto('/yaml.html?service=user-service&branch=main&path=/users&method=GET');
    
    // Header should be visible
    await expect(page.locator('header#site-banner')).toBeVisible();
    
    // Breadcrumbs should contain service name and branch
    const breadcrumbService = page.locator('#breadcrumb-service');
    await expect(breadcrumbService).toBeVisible({ timeout: 10000 });
    await expect(breadcrumbService).toContainText('user-service');
    await expect(page.locator('#breadcrumb-branch')).toContainText('main');
    
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
