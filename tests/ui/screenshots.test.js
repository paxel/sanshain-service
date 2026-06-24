import { test, expect } from '@playwright/test';
import path from 'path';

test.describe('Sanshain Screenshot Capture', () => {
  const screenshotDir = 'docs/images';
  const token = process.env.SANSHAIN_TOKEN;

  test.beforeEach(async ({ page, context }) => {
    // Set viewport size for high-quality screenshots
    await page.setViewportSize({ width: 1280, height: 800 });

    // Inject token via cookie for more reliable auth initialization
    await context.addCookies([{
        name: 'sanshain_token',
        value: token,
        domain: 'localhost',
        path: '/'
    }]);

    // Also set in localStorage just in case some scripts only look there
    // Use the base domain URL
    await page.goto('http://localhost:3000/health'); 
    await page.evaluate((t) => {
      localStorage.setItem('sanshain_token', t);
      localStorage.setItem('sanshain_theme', 'light');
    }, token);
  });

  const ensureLoaderHidden = async (page) => {
    // Wait up to 2 seconds for loader to hide naturally
    try {
        await page.waitForSelector('#app-loader', { state: 'hidden', timeout: 2000 });
    } catch (e) {
        console.log('Loader still visible after 2s, forcing it to hide...');
        await page.evaluate(() => {
            const loader = document.getElementById('app-loader');
            if (loader) loader.style.display = 'none';
        });
    }
  };

  test('Capture landing page', async ({ page }) => {
    await page.goto('/');
    // Wait for content to settle
    await page.waitForSelector('img[alt="Sanshain Logo"]');
    await page.waitForTimeout(1000);
    await page.screenshot({ path: path.join(screenshotDir, 'landing.png') });
  });

  test('Capture services page', async ({ page }) => {
    await page.goto('/services.html');
    console.log('Services page URL:', page.url());
    
    await ensureLoaderHidden(page);
    
    // Wait for services to load
    try {
        await page.waitForSelector('.service-item', { state: 'visible', timeout: 5000 });
    } catch (e) {
        console.log('No services found or timeout. Content:', await page.textContent('body'));
    }
    
    await page.waitForTimeout(500);
    await page.screenshot({ path: path.join(screenshotDir, 'services.png') });
  });

  test('Capture dependency graph', async ({ page }) => {
    await page.goto('/graph.html');
    await ensureLoaderHidden(page);
    await page.waitForSelector('#custom-graph');
    // Give it time for the layout to stabilize
    await page.waitForTimeout(2000);
    await page.screenshot({ path: path.join(screenshotDir, 'graph.png') });

    // Click on a node if possible to show interactivity
    const node = page.locator('.node').first();
    if (await node.isVisible()) {
      await node.click();
      await page.waitForTimeout(500);
      await page.screenshot({ path: path.join(screenshotDir, 'graph_details.png') });
    }
  });

  test('Capture admin dashboard', async ({ page }) => {
    await page.goto('/admin.html');
    await ensureLoaderHidden(page);
    await page.waitForSelector('#admin-dashboard');
    await page.waitForTimeout(500);
    await page.screenshot({ path: path.join(screenshotDir, 'admin.png') });
  });

  test('Capture audit timeline', async ({ page }) => {
    await page.goto('/audit.html');
    console.log('Audit page URL:', page.url());
    await ensureLoaderHidden(page);
    
    // Wait for entries to load
    try {
        await page.waitForSelector('.timeline-item', { state: 'visible', timeout: 5000 });
    } catch (e) {
        console.log('No audit entries found or timeout. Content:', await page.textContent('body'));
    }
    
    await page.waitForTimeout(500);
    await page.screenshot({ path: path.join(screenshotDir, 'audit.png') });

    // Click on the first "View Changes" button
    const diffBtn = page.locator('button:has-text("View Changes")').first();
    if (await diffBtn.isVisible()) {
      await diffBtn.click();
      // Wait for diff content
      await page.waitForSelector('.d2h-wrapper', { state: 'visible' });
      await page.waitForTimeout(500);
      await page.screenshot({ path: path.join(screenshotDir, 'diff.png') });
    }
  });
});
