import { test, expect } from '@playwright/test';

test.describe('Sanshain UI Smoke Test', () => {
  test('Landing page loads and has correct title', async ({ page }) => {
    await page.goto('/');
    await expect(page).toHaveTitle(/Sanshain/);
    
    // Check for critical logo
    const logo = page.locator('img[alt="Sanshain Logo"]').first();
    await expect(logo).toBeVisible();
    
    // Verify logo dimensions are not broken (intrinsic vs display)
    const box = await logo.boundingBox();
    expect(box.height).toBeLessThan(100); // Should be h-9 (~36px)
  });

  test('Login and Admin settings persistence', async ({ page }) => {
    // Clear storage before starting to avoid stale session or banner issues
    await page.goto('/');
    await page.evaluate(() => {
      sessionStorage.clear();
      localStorage.clear();
    });

    // Handle ANY unexpected dialogs by dismissing them to prevent hangs
    page.on('dialog', async dialog => {
      console.log(`[UI Test] Auto-dismissing dialog: ${dialog.message()}`);
      await dialog.dismiss();
    });
    
    // Explicitly check for and dismiss the reload banner if it's a DOM element
    const dismissReloadBanner = async () => {
      const banner = page.locator('#sanshain-reload-banner');
      if (await banner.isVisible()) {
        await banner.locator('button:has-text("×")').click();
      }
    };

    await page.goto('/account.html');
    await dismissReloadBanner();
    
    // Login - Use specific selectors to avoid ambiguity with the "Sign In" tab
    await page.waitForSelector('#login-username', { state: 'visible' });
    await page.fill('input[id="login-username"]', 'root');
    await page.fill('input[id="login-password"]', 'root_password');
    await page.click('#login-panel button[type="submit"]');
    
    // Wait for redirect or UI change with better error info
    try {
      await expect(page.locator('#account-dashboard')).toBeVisible({ timeout: 10000 });
    } catch (e) {
      const errorText = await page.locator('#login-error').textContent();
      if (errorText && errorText.trim().length > 0) {
        throw new Error(`Login failed with error: ${errorText}`);
      }
      throw e;
    }
    await expect(page.locator('#banner-username')).toContainText('root');
    
    // Navigate to Admin
    await page.goto('/admin.html');
    await dismissReloadBanner();
    await page.waitForSelector('#admin-dashboard');
    
    // Toggle Local Users Registration
    const localUsersToggle = page.locator('#local-users-toggle');
    const initialState = await localUsersToggle.getAttribute('class');
    const isInitiallyOn = initialState.includes('bg-indigo-600');
    
    await localUsersToggle.click();
    await page.waitForTimeout(1000); // Wait for API call and UI update
    
    // Refresh to verify persistence
    await page.reload();
    await page.waitForSelector('#admin-dashboard');
    const newState = await page.locator('#local-users-toggle').getAttribute('class');
    const isNowOn = newState.includes('bg-indigo-600');
    
    expect(isNowOn).not.toBe(isInitiallyOn);
    
    // Restore state
    await page.locator('#local-users-toggle').click();
    await page.waitForTimeout(500);
  });
});
