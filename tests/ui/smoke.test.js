import { test, expect } from '@playwright/test';

const BASE_URL = process.env.BASE_URL || 'http://localhost:3000';

test.describe('Sanshain UI Smoke Test', () => {
  test('Landing page loads and has correct title', async ({ page }) => {
    await page.goto(BASE_URL);
    await expect(page).toHaveTitle(/Sanshain/);
    
    // Check for critical logo
    const logo = page.locator('img[alt="Sanshain Logo"]').first();
    await expect(logo).toBeVisible();
    
    // Verify logo dimensions are not broken (intrinsic vs display)
    const box = await logo.boundingBox();
    expect(box.height).toBeLessThan(100); // Should be h-9 (~36px)
  });

  test('Login and Admin settings persistence', async ({ page }) => {
    await page.goto(`${BASE_URL}/account.html`);
    
    // Login
    await page.fill('input[id="login-username"]', 'root');
    await page.fill('input[id="login-password"]', 'root_password');
    await page.click('button:has-text("Sign In")');
    
    // Wait for redirect or UI change
    await expect(page.locator('#account-dashboard')).toBeVisible();
    await expect(page.locator('#banner-username')).toContainText('root');
    
    // Navigate to Admin
    await page.goto(`${BASE_URL}/admin.html`);
    await page.waitForSelector('#admin-panel');
    
    // Toggle Developer Mode
    const devModeToggle = page.locator('#dev-mode-toggle');
    const initialState = await devModeToggle.isChecked();
    
    await devModeToggle.click();
    await page.waitForTimeout(500); // Wait for API call
    
    // Refresh to verify persistence
    await page.reload();
    await page.waitForSelector('#admin-panel');
    const newState = await devModeToggle.isChecked();
    
    expect(newState).not.toBe(initialState);
    
    // Restore state
    await devModeToggle.click();
  });
});
