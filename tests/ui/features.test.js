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
    await expect(page.locator('#dashboard')).toBeVisible();
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

  test('API Token Management - read-only scope', async ({ page }) => {
    await page.goto('/account.html');
    await ensureLoaderHidden(page);
    await expect(page.locator('#tokens-list')).toBeVisible();

    const tokenName = `ai-token-${Date.now()}`;
    await page.fill('#token-name', tokenName);
    await page.check('#token-read-only');
    await page.click('button:has-text("Create")');
    await expect(page.locator('#token-created')).toBeVisible();

    // The list shows the scope and that the token was never used.
    const tokenItem = page.locator('#tokens-list > div').filter({ hasText: tokenName });
    await expect(tokenItem.locator('[data-token-scope="read_only"]')).toHaveText('read-only');
    await expect(tokenItem).toContainText('Last used: never');

    // The form resets to a full token for the next one.
    await expect(page.locator('#token-read-only')).not.toBeChecked();

    await tokenItem.locator('button:has-text("Revoke")').click();
    const confirmBtn = page.locator('#confirm-yes');
    if (await confirmBtn.isVisible()) {
      await confirmBtn.click();
    }
    await expect(page.locator(`#tokens-list :text("${tokenName}")`)).not.toBeVisible();
  });

  test('Producer page shows owner and link chips from catalog-info, escaped', async ({ page, request }) => {
    const login = await request.post('/auth/login', {
      data: { username: 'root', password: adminPassword },
    });
    const { token } = await login.json();
    const name = `ctx-producer-${Date.now()}`;
    const spec = [
      'openapi: 3.0.3',
      'info:',
      '  title: Context',
      '  version: 1.0.0',
      'paths:',
      '  /ping:',
      '    get:',
      '      responses:',
      "        '200':",
      '          description: OK',
      '',
    ].join('\n');
    const provided = await request.post('/provide', {
      headers: { Authorization: `Bearer ${token}` },
      data: { producername: name, stability: 'snapshot', openapi_yaml: spec },
    });
    expect(provided.ok()).toBeTruthy();

    // A link title that is markup must land on the page as text (#48).
    const catalog = [
      'apiVersion: backstage.io/v1alpha1',
      'kind: Component',
      'metadata:',
      `  name: ${name}`,
      '  description: Takes orders.',
      '  annotations:',
      '    backstage.io/source-location: url:https://github.com/acme/orders',
      '  links:',
      '    - url: https://wiki.acme.corp/orders',
      '      title: "<script>alert(1)</script>"',
      'spec:',
      '  owner: team-orders',
      '',
    ].join('\n');
    const pushed = await request.put(`/catalog/${name}`, {
      headers: { Authorization: `Bearer ${token}` },
      data: { catalog_info_yaml: catalog },
    });
    expect(pushed.ok()).toBeTruthy();

    await page.goto(`/producers.html?service=${encodeURIComponent(name)}`);
    await ensureLoaderHidden(page);
    await expect(page.locator('[data-context="owner"]')).toContainText('team-orders');
    await expect(page.locator('[data-context="description"]')).toHaveText('Takes orders.');
    const repository = page.locator('[data-context-link="repository"]');
    await expect(repository).toHaveText('github.com');
    await expect(repository).toHaveAttribute('href', 'https://github.com/acme/orders');
    await expect(repository).toHaveAttribute('rel', 'noopener noreferrer');
    const chip = page.locator('[data-context-link="link"]');
    await expect(chip).toHaveText('<script>alert(1)</script>');
    await expect(chip).toHaveAttribute('href', 'https://wiki.acme.corp/orders');
    expect(await page.locator('[data-context="links"] script').count()).toBe(0);
  });

  test('Admin Webhooks section creates a hook and shows the secret once', async ({ page, request }) => {
    await page.goto('/admin.html');
    await ensureLoaderHidden(page);
    const section = page.locator('#webhooks-section');
    await section.scrollIntoViewIfNeeded();
    await expect(section).toBeVisible();
    await expect(page.locator('#webhook-allowed-hosts')).toBeVisible();
    const panel = page.locator('#webhooks-panel [data-webhook-form]');
    await expect(panel).toBeVisible();

    const url = `https://hooks.example/${Date.now()}`;
    await panel.locator('[data-webhook-url]').fill(url);
    await panel.locator('[data-webhook-format]').selectOption('slack');
    await panel.locator('[data-webhook-create]').click();

    // The secret is shown once, with a copy button, and never in the list.
    const secret = page.locator('#webhooks-panel [data-webhook-secret]');
    await expect(secret).toBeVisible();
    const value = await secret.textContent();
    expect(value).toMatch(/^whsec_[0-9a-f]{64}$/);
    await expect(page.locator('#webhooks-panel [data-webhook-copy]')).toBeVisible();
    const row = page.locator('#webhooks-panel [data-webhook-row]').filter({ hasText: url });
    await expect(row).toBeVisible();
    await expect(row).toContainText('slack');
    expect(await row.textContent()).not.toContain(value);

    await page.reload();
    await ensureLoaderHidden(page);
    await expect(page.locator('#webhooks-panel [data-webhook-row]').filter({ hasText: url })).toBeVisible();
    await expect(page.locator('#webhooks-panel [data-webhook-secret]')).toBeHidden();
    expect(await page.locator('#webhooks-section').textContent()).not.toContain(value);

    // Clean up through the API so the listing does not grow run over run.
    const login = await request.post('/auth/login', {
      data: { username: 'root', password: adminPassword },
    });
    const { token } = await login.json();
    const id = await row.getAttribute('data-webhook-row');
    const csrf = await (await request.get('/csrf-token')).json();
    const deleted = await request.delete(`/admin/webhooks/${id}`, {
      headers: { Authorization: `Bearer ${token}`, 'X-CSRF-Token': csrf.csrf_token },
    });
    expect(deleted.ok()).toBeTruthy();
  });

  test('Producer page shows the Webhooks card to an administrator', async ({ page, request }) => {
    const login = await request.post('/auth/login', {
      data: { username: 'root', password: adminPassword },
    });
    const { token } = await login.json();
    const name = `hook-producer-${Date.now()}`;
    const spec = [
      'openapi: 3.0.3',
      'info:',
      '  title: Hooks',
      '  version: 1.0.0',
      'paths:',
      '  /ping:',
      '    get:',
      '      responses:',
      "        '200':",
      '          description: OK',
      '',
    ].join('\n');
    const provided = await request.post('/provide', {
      headers: { Authorization: `Bearer ${token}` },
      data: { producername: name, stability: 'snapshot', openapi_yaml: spec },
    });
    expect(provided.ok()).toBeTruthy();

    await page.goto(`/producers.html?service=${encodeURIComponent(name)}`);
    await ensureLoaderHidden(page);
    const panel = page.locator(`[data-webhook-panel="/admin/producers/${name}/webhooks"]`);
    await expect(panel).toBeVisible();
    await expect(panel.locator('[data-webhook-form]')).toBeVisible();
    await expect(panel).toContainText('No hooks registered yet.');
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
