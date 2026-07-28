import { test, expect } from '@playwright/test';

test.describe('Sanshain UI Smoke Test', () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

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
    if (!adminPassword) {
      throw new Error('INITIAL_ADMIN_PASSWORD environment variable is required for tests');
    }
    await page.waitForSelector('#login-username', { state: 'visible' });
    await page.fill('input[id="login-username"]', 'root');
    await page.fill('input[id="login-password"]', adminPassword);
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
    
    // Toggle Auth Mode (from Local to Dev)
    const devModeRadio = page.locator('input[name="auth-mode"][value="dev"]');
    await devModeRadio.click();
    
    // Click Save button
    await page.locator('button:has-text("Save Authentication Settings")').click();
    await page.waitForTimeout(1000); // Wait for API call
    
    // Refresh to verify persistence.
    // The radios are populated by loadAuthConfig(), an async fetch that resolves
    // *after* #admin-dashboard is rendered, so reading isChecked() once races it.
    // toBeChecked() auto-retries until the fetch lands.
    await page.reload();
    await page.waitForSelector('#admin-dashboard');
    await expect(page.locator('input[name="auth-mode"][value="dev"]')).toBeChecked();

    // Restore Local mode
    await page.locator('input[name="auth-mode"][value="local"]').click();
    await page.locator('button:has-text("Save Authentication Settings")').click();
    await page.waitForTimeout(500);
  });

  test('Audit nav link is admin-only', async ({ page }) => {
    // The audit timeline is admin-only on the server. The nav link must not
    // advertise it to visitors who would be refused.
    if (!adminPassword) {
      throw new Error('INITIAL_ADMIN_PASSWORD environment variable is required for tests');
    }

    page.on('dialog', async (dialog) => await dialog.dismiss());

    // Signed out: hidden.
    await page.goto('/producers.html');
    await page.evaluate(() => {
      sessionStorage.clear();
      localStorage.clear();
    });
    await page.reload();
    await expect(page.locator('#nav-audit-link')).toBeHidden();

    // Signed in as an admin: visible.
    await page.goto('/account.html');
    await page.waitForSelector('#login-username', { state: 'visible' });
    await page.fill('input[id="login-username"]', 'root');
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator('#account-dashboard')).toBeVisible({ timeout: 10000 });

    await page.goto('/producers.html');
    await expect(page.locator('#nav-audit-link')).toBeVisible();

    // Signed in as a NON-admin: hidden. This is the case the change exists for,
    // and the only one the other two cannot catch — a regression that ignored
    // is_admin in the signed-in branch would still pass both of them.
    //
    // The banner state is driven directly rather than by registering a second
    // account, so the assertion does not depend on the registration or
    // auto-approve settings. That /auth/me reports is_admin correctly is a
    // separate concern, covered server-side.
    await page.evaluate(() => window.updateBannerAuth({ username: 'plain', is_admin: false }));
    await expect(page.locator('#nav-audit-link')).toBeHidden();

    // ...and the same call with is_admin true brings it back, so the assertion
    // above is about the flag and not about the call having any effect at all.
    await page.evaluate(() => window.updateBannerAuth({ username: 'root', is_admin: true }));
    await expect(page.locator('#nav-audit-link')).toBeVisible();
  });
});
