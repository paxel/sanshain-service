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

// The endpoint editor is reachable only through an admin-gated button on
// yaml.html, and edit.html re-checks the same flag on entry. Both gates read the
// user handed to the checkDiscoveryAuth callback, so a callback invoked without
// that argument leaves `user` undefined, reads as not-an-admin, and disables the
// whole feature for everyone — root included — with no error anywhere.
test.describe('Endpoint editor access', () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

  const SERVICE = 'edit-access-fixture';
  const YAML_URL =
    `/yaml.html?service=${SERVICE}&branch=main&path=%2Fhello&method=GET&api_type=OpenApi`;
  const EDIT_URL =
    `/edit.html?service=${SERVICE}&branch=main&path=%2Fhello&method=GET&api_type=OpenApi`;

  const SPEC = [
    'openapi: 3.0.0',
    'info:',
    '  title: Edit Access Fixture',
    '  version: 1.0.0',
    'paths:',
    '  /hello:',
    '    get:',
    '      responses:',
    "        '200':",
    '          description: OK',
    '',
  ].join('\n');

  async function loginAs(page, username, password) {
    await page.goto('/account.html');
    await page.waitForSelector('#login-username', { state: 'visible' });
    await page.fill('input[id="login-username"]', username);
    await page.fill('input[id="login-password"]', password);
    await page.click('#login-panel button[type="submit"]');
    await expect(page.locator('#account-dashboard')).toBeVisible({ timeout: 10000 });
  }

  test.beforeAll(async ({ request }) => {
    if (!adminPassword) {
      throw new Error('INITIAL_ADMIN_PASSWORD environment variable is required for tests');
    }
    const login = await request.post('/auth/login', {
      data: { username: 'root', password: adminPassword },
    });
    const { token } = await login.json();

    // Seed one endpoint for the editor to open.
    await request.post('/provide', {
      headers: { Authorization: `Bearer ${token}` },
      data: { producername: SERVICE, branch: 'main', openapi_yaml: SPEC },
    });

    // A non-admin account, approved so it can sign in.
    await request.post('/auth/register', {
      data: { username: 'edit-access-plain', password: 'plain-pass-12345' },
    });
    const users = await (
      await request.get('/admin/users', {
        headers: { Authorization: `Bearer ${token}` },
      })
    ).json();
    const plain = users.find((u) => u.username === 'edit-access-plain');
    if (plain && !plain.approved) {
      await request.post(`/admin/users/${plain.id}/approve`, {
        headers: { Authorization: `Bearer ${token}` },
      });
    }
  });

  test('an admin can reach the editor', async ({ page }) => {
    page.on('dialog', async (d) => await d.dismiss());
    await loginAs(page, 'root', adminPassword);

    // The Edit button must actually be offered.
    await page.goto(YAML_URL);
    await expect(page.locator('#edit-btn')).toBeVisible();

    // ...and following it must land on the editor rather than bouncing back.
    await page.locator('#edit-btn').click();
    await page.waitForURL('**/edit.html**');
    await expect(page.locator('#yaml-editor')).toBeVisible();
  });

  test('a non-admin is offered no editor and cannot open it directly', async ({ page }) => {
    page.on('dialog', async (d) => await d.dismiss());
    await loginAs(page, 'edit-access-plain', 'plain-pass-12345');

    await page.goto(YAML_URL);
    // The page itself still loads; only the admin affordance is withheld.
    await expect(page.locator('#edit-btn')).toBeHidden();

    // Typing the editor URL directly is refused and returns to the viewer.
    await page.goto(EDIT_URL);
    await page.waitForURL('**/yaml.html**');
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

    await row.locator('select').selectOption('viewer');
    await row.locator('button:has-text("Add")').click();
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

test.describe('Held Provides inbox', () => {
  const adminPassword = process.env.INITIAL_ADMIN_PASSWORD;

  test('the inbox is visible on the dashboard with its count', async ({ page }) => {
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

    // Discoverability is the point: a held Provide means somebody's build is red,
    // so the count has to be where an administrator will encounter it.
    await expect(page.locator('#pending-list')).toBeVisible();
    await expect(page.locator('#pending-count')).toContainText('none');
  });
});
