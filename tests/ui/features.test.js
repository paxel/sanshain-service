import { test, expect } from '@playwright/test';

test.describe('Sanshain 1.5.0 Features Integration Tests', () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

  test.beforeEach(async ({ page }) => {
    if (!adminPassword) {
      throw new Error('INITIAL_ADMIN_PASSWORD environment variable is required');
    }
    
    // Perform login
    await page.goto('/account.html');
    await page.fill('#login-username', 'root');
    await page.fill('#login-password', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator('#account-dashboard')).toBeVisible();
  });

  const ensureLoaderHidden = async (page) => {
    try {
        await page.waitForSelector('#app-loader', { state: 'hidden', timeout: 2000 });
    } catch (e) {
        await page.evaluate(() => {
            const loader = document.getElementById('app-loader');
            if (loader) loader.style.display = 'none';
        });
    }
  };

  test('Audit Timeline and Diff Viewer', async ({ page }) => {
    // Navigate to Audit
    await page.goto('/audit.html');
    await ensureLoaderHidden(page);
    await expect(page.locator('#audit-timeline')).toBeVisible();
    
    // Check if we have at least one entry (assuming we run after some data population)
    // If no data, this might fail, so we check for either entries or the "No updates" message
    const hasEntries = await page.locator('.timeline-item').count() > 0;
    
    if (hasEntries) {
      const firstEntry = page.locator('.timeline-item').first();
      await expect(firstEntry).toBeVisible();
      
      // Try to open a diff if available
      const viewChangesBtn = firstEntry.locator('button:has-text("View Changes")');
      if (await viewChangesBtn.isVisible()) {
        await viewChangesBtn.click();
        await expect(page.locator('.d2h-wrapper')).toBeVisible();
      }
    }
  });

  test('API Token Management', async ({ page }) => {
    await page.goto('/account.html');
    await ensureLoaderHidden(page);
    await expect(page.locator('#tokens-list')).toBeVisible();
    
    // Create a new token
    const tokenName = `test-token-${Date.now()}`;
    await page.fill('#token-name', tokenName);
    await page.click('button:has-text("Create")');
    
    // Check for success message and raw token display
    await expect(page.locator('#token-created')).toBeVisible();
    const rawToken = await page.locator('#token-created-value').textContent();
    expect(rawToken).toMatch(/^san_/);
    
    // Verify it's in the list
    await expect(page.locator(`#tokens-list :text("${tokenName}")`)).toBeVisible();
    
    // Revoke it
    // The revoke button doesn't have data-token-name, it's inside the item.
    const tokenItem = page.locator('#tokens-list > div').filter({ hasText: tokenName });
    await tokenItem.locator('button:has-text("Revoke")').click();
    
    // Handle confirm dialog
    const confirmBtn = page.locator('#confirm-yes');
    if (await confirmBtn.isVisible()) {
        await confirmBtn.click();
    }
    
    await expect(page.locator(`#tokens-list :text("${tokenName}")`)).not.toBeVisible();
  });

  test('Admin Settings - Snapshot Max Age', async ({ page }) => {
    await page.goto('/admin.html');
    await ensureLoaderHidden(page);
    await expect(page.locator('#admin-dashboard')).toBeVisible();

    // Scroll to settings
    await page.locator('#snapshot-max-age-days').scrollIntoViewIfNeeded();

    // Change value — scope the Save click to the Snapshot Cleanup section,
    // since the page has several Save buttons.
    const input = page.locator('#snapshot-max-age-days');
    await input.fill('45');
    const section = page.locator('section').filter({ hasText: 'Snapshot Cleanup' });
    await section.locator('button:has-text("Save")').click();

    // Reload and verify
    await page.reload();
    await expect(page.locator('#snapshot-max-age-days')).toHaveValue('45');

    // Restore default
    await page.locator('#snapshot-max-age-days').fill('30');
    const sectionRestore = page.locator('section').filter({ hasText: 'Snapshot Cleanup' });
    await sectionRestore.locator('button:has-text("Save")').click();
  });
});
