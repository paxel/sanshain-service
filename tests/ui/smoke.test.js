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

  test('Audit nav link requires the audit permission', async ({ page }) => {
    // The audit timeline needs `view_audit` on the server. The nav link must not
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

    // Signed in WITHOUT the permission: hidden. This is the case the change
    // exists for, and the only one the other two cannot catch — a regression
    // that ignored permissions in the signed-in branch would still pass both.
    //
    // The banner state is driven directly rather than by registering a second
    // account, so the assertion does not depend on the registration or
    // auto-approve settings. That /auth/me reports permissions correctly is a
    // separate concern, covered server-side.
    await page.evaluate(() =>
      window.updateBannerAuth({ username: 'plain', permissions: [] })
    );
    await expect(page.locator('#nav-audit-link')).toBeHidden();

    // ...and the same call with the permission brings it back, so the assertion
    // above is about the permission and not about the call having no effect.
    await page.evaluate(() =>
      window.updateBannerAuth({ username: 'root', permissions: ['view_audit'] })
    );
    await expect(page.locator('#nav-audit-link')).toBeVisible();

    // A partial administrator — someone who administers users but may not read
    // the audit trail — still does not get the link. This is the arrangement the
    // permission model makes possible and the old all-or-nothing gate could not
    // express: holding *an* administrative permission is no longer the same as
    // holding this one.
    await page.evaluate(() =>
      window.updateBannerAuth({ username: 'manager', permissions: ['manage_users'] })
    );
    await expect(page.locator('#nav-audit-link')).toBeHidden();
  });
});

// The viewer renders an endpoint of one version-line entry. Immutability is
// absolute in 2.0: there is no editor, so the page must offer none.
test.describe('Endpoint viewer', () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

  const SERVICE = 'viewer-fixture';
  const YAML_URL =
    `/yaml.html?service=${SERVICE}&api_type=openapi&version=1.0.0&path=%2Fhello&method=GET`;

  const SPEC = [
    'openapi: 3.0.0',
    'info:',
    '  title: Viewer Fixture',
    '  version: 1.0.0',
    'paths:',
    '  /hello:',
    '    get:',
    '      responses:',
    "        '200':",
    '          description: OK',
    '',
  ].join('\n');

  test.beforeAll(async ({ request }) => {
    if (!adminPassword) {
      throw new Error('INITIAL_ADMIN_PASSWORD environment variable is required for tests');
    }
    const login = await request.post('/auth/login', {
      data: { username: 'root', password: adminPassword },
    });
    const { token } = await login.json();

    // Seed one version for the viewer to open.
    await request.post('/provide', {
      headers: { Authorization: `Bearer ${token}` },
      data: { producername: SERVICE, stability: 'ga', openapi_yaml: SPEC },
    });
  });

  test('the viewer shows the version history of the endpoint', async ({ page }) => {
    page.on('dialog', async (d) => await d.dismiss());
    await page.goto('/account.html');
    await page.waitForSelector('#login-username', { state: 'visible' });
    await page.fill('input[id="login-username"]', 'root');
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator('#account-dashboard')).toBeVisible({ timeout: 10000 });

    await page.goto(YAML_URL);
    const card = page.locator('#versions-list-container .version-card', { hasText: '1.0.0' });
    await expect(card).toBeVisible({ timeout: 10000 });
    await expect(card).toContainText('GA');
    // Immutability is absolute: no edit affordance exists anywhere.
    await expect(page.locator('#edit-btn')).toHaveCount(0);
  });
});

test.describe('Role, group and maintainer management', () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

  test.beforeEach(async ({ page }) => {
    if (!adminPassword) {
      throw new Error('INITIAL_ADMIN_PASSWORD environment variable is required for tests');
    }
    page.on('dialog', async (dialog) => await dialog.accept());
    await page.goto('/account.html');
    await page.waitForSelector('#login-username', { state: 'visible' });
    await page.fill('input[id="login-username"]', 'root');
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator('#account-dashboard')).toBeVisible({ timeout: 10000 });
    await page.goto('/admin.html');
    await expect(page.locator('#admin-dashboard')).toBeVisible({ timeout: 10000 });
  });

  test('the management sections render', async ({ page }) => {
    await expect(page.locator('#users-list')).toBeVisible();
    await expect(page.locator('#groups-list')).toBeVisible();
    await expect(page.locator('#maintainers-list')).toBeVisible();
    // Root is configuration-held; the page must not offer to change that.
    await expect(page.locator('#root-note')).toContainText('cannot be granted or revoked here');
  });

  // The round trip that matters: a group created here can carry a role, and the
  // origin badge distinguishes it from one mirrored from the directory.
  test('a group can be created and given a role', async ({ page }) => {
    const name = 'ui-test-group';
    await page.fill('#new-group-name', name);
    await page.click('button:has-text("Create")');

    const row = page.locator('#groups-list > div', { hasText: name });
    await expect(row).toBeVisible({ timeout: 10000 });
    await expect(row).toContainText('sanshain');
    await expect(row).toContainText('no roles attached');

    // The card carries two selects since 2.0 (role picker + member picker);
    // target the role one explicitly.
    await row.locator('select[id^="group-role-"]').selectOption('viewer');
    await row.locator('select[id^="group-role-"] ~ button:has-text("Add")').click();
    await expect(page.locator('#groups-list > div', { hasText: name })).toContainText('viewer', {
      timeout: 10000,
    });
  });

  test('the root account shows its admin role and offers no delete', async ({ page }) => {
    const row = page.locator('#users-list > div', { hasText: 'root' });
    await expect(row).toBeVisible();
    await expect(row).toContainText('admin');
    await expect(row.locator('button:has-text("Delete")')).toHaveCount(0);
  });
});

test.describe('Snapshot cleanup settings', () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

  test('the snapshot max age setting is on the dashboard', async ({ page }) => {
    if (!adminPassword) {
      throw new Error('INITIAL_ADMIN_PASSWORD environment variable is required for tests');
    }
    page.on('dialog', async (dialog) => await dialog.accept());
    await page.goto('/account.html');
    await page.waitForSelector('#login-username', { state: 'visible' });
    await page.fill('input[id="login-username"]', 'root');
    await page.fill('input[id="login-password"]', adminPassword);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator('#account-dashboard')).toBeVisible({ timeout: 10000 });

    await page.goto('/admin.html');
    await expect(page.locator('#admin-dashboard')).toBeVisible({ timeout: 10000 });

    // Use-based snapshot expiry is the 2.0 cleanup model; the setting and the
    // manual trigger live where an administrator will encounter them.
    await expect(page.locator('#snapshot-max-age-days')).toBeVisible();
    await expect(page.locator('button:has-text("Run Cleanup")').first()).toBeVisible();
  });
});
