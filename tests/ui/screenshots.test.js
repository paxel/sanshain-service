import { test, expect } from '@playwright/test';
import path from 'path';

test.describe('Sanshain Screenshot Capture', () => {
  const screenshotDir = 'docs/images';
  const token = process.env.SANSHAIN_TOKEN;

  test.beforeEach(async ({ page, context }, testInfo) => {
    // Set viewport size for high-quality screenshots
    await page.setViewportSize({ width: 1440, height: 900 });

    // Enable console logging
    page.on('console', msg => console.log(`[BROWSER ${testInfo.title}]: ${msg.text()}`));

    // Public pages don't need auth
    const isPublic = testInfo.title.includes('landing') || 
                     testInfo.title.includes('login') || 
                     testInfo.title.includes('register');

    if (!isPublic) {
        // Inject token and flags via Init Script (runs before any other script)
        await page.addInitScript((t) => {
            if (t) {
                localStorage.setItem('sanshain_token', t);
                document.cookie = `sanshain_token=${t};path=/;max-age=3600`;
            }
            localStorage.setItem('sanshain_theme', 'light');
            window.SANSHAIN_FAST_SCREENSHOT = true;
        }, token);

        // Also inject cookie via context for API calls from common.js
        if (token) {
            await context.addCookies([{
                name: 'sanshain_token',
                value: token,
                domain: 'localhost',
                path: '/'
            }]);
        }
    }
  });

  const ensureLoaderHidden = async (page) => {
    // Fast check for loader hidden state
    try {
        await page.waitForSelector('#app-loader', { state: 'hidden', timeout: 5000 });
    } catch (e) {
        // Force hide if it's stuck
        await page.evaluate(() => {
            const loader = document.getElementById('app-loader');
            if (loader) {
                loader.classList.add('hidden');
                loader.style.display = 'none';
            }
        });
    }
  };


  const setTheme = async (page, theme) => {
    await page.evaluate((t) => {
      document.cookie = `sanshain_theme=${t};path=/;max-age=31536000;SameSite=Lax`;
      if (t === 'dark') {
          document.documentElement.classList.add('dark');
      } else {
          document.documentElement.classList.remove('dark');
      }
      // Re-trigger gimmicks if any
      if (window.applySockeGimmick) window.applySockeGimmick(t);
    }, theme);
    await page.waitForTimeout(500); // Wait for theme transition
  };

  test('Capture landing page', async ({ page }) => {
    await page.goto('/', { waitUntil: 'domcontentloaded' });
    await page.waitForSelector('img[alt="Sanshain Logo"]');
    await page.waitForTimeout(1000);
    await page.screenshot({ path: path.join(screenshotDir, 'landing.png') });

    // Dark mode landing
    await setTheme(page, 'dark');
    await page.waitForTimeout(1000);
    await page.screenshot({ path: path.join(screenshotDir, 'landing_dark.png') });
  });

  test('Capture login page', async ({ page, context }) => {
    // Clear cookies and localStorage for this test to show login
    await context.clearCookies();
    await page.goto('/account.html', { waitUntil: 'domcontentloaded' });
    await page.evaluate(() => localStorage.clear());
    await page.reload({ waitUntil: 'domcontentloaded' });

    await ensureLoaderHidden(page);
    await page.waitForSelector('#auth-screen:not(.hidden)');
    await page.screenshot({ path: path.join(screenshotDir, 'login.png') });
    
    // Switch to register
    await page.click('#tab-register');
    await page.waitForTimeout(500);
    await page.screenshot({ path: path.join(screenshotDir, 'register.png') });
  });

  test('Capture user dashboard', async ({ page }) => {
    await page.goto('/account.html', { waitUntil: 'domcontentloaded' });
    await ensureLoaderHidden(page);
    await page.waitForSelector('#account-dashboard:not(.hidden)');
    await page.screenshot({ path: path.join(screenshotDir, 'user_dashboard.png') });
  });

  test('Capture services and drill-down', async ({ page }) => {
    await page.goto('/services.html', { waitUntil: 'domcontentloaded' });
    await ensureLoaderHidden(page);
    await page.waitForSelector('.endpoint-card', { timeout: 30000 });
    await page.screenshot({ path: path.join(screenshotDir, 'services_list.png') });

    // Click on config-service (from demo2)
    const svcCard = page.locator('.endpoint-card', { hasText: 'config-service' }).first();
    await svcCard.click();
    await page.waitForFunction(() => document.body.innerText.includes('config-service'), { timeout: 20000 });
    await page.waitForTimeout(1000);
    await page.screenshot({ path: path.join(screenshotDir, 'service_branches.png') });

    // Click on main branch
    const branchCard = page.locator('.endpoint-card', { hasText: 'main' }).first();
    await branchCard.click();
    await page.waitForSelector('.endpoint-card', { timeout: 30000 });
    await page.waitForTimeout(1000);
    await page.screenshot({ path: path.join(screenshotDir, 'branch_endpoints.png') });
  });

  test('Capture YAML viewer', async ({ page }) => {
    test.setTimeout(300000); // 5 minutes
    // Navigate directly to a YAML view
    await page.goto('/yaml.html?service=ml-inference&branch=main&path=/predict&method=POST&api_type=openapi', { waitUntil: 'domcontentloaded' });
    await ensureLoaderHidden(page);
    
    // Optional network idle, don't fail if it doesn't happen
    await page.waitForLoadState('networkidle', { timeout: 15000 }).catch(() => {});
    
    // Wait for content that indicates YAML is loaded - use locator for more stability
    await page.locator('text=openapi:').first().waitFor({ timeout: 60000 });
    
    await page.waitForTimeout(3000);
    await page.screenshot({ path: path.join(screenshotDir, 'yaml_viewer.png') });
  });

  test('Capture dependency graph', async ({ page }) => {
    test.setTimeout(300000); // 5 minutes
    await page.goto('/graph.html', { waitUntil: 'domcontentloaded' });
    await ensureLoaderHidden(page);
    await page.waitForSelector('#custom-graph', { timeout: 30000 });
    
    // Wait for ANY graph node
    await page.waitForSelector('g.graph-node', { timeout: 120000 });
    
    // Give it plenty of time for the layout to stabilize and zoom-to-fit
    await page.waitForTimeout(15000);
    await page.screenshot({ path: path.join(screenshotDir, 'graph.png') });

    /*
    // Click on a node to show details - use dispatchEvent for more reliability on SVG
    const node = page.locator('g.graph-node').filter({ hasText: /api-middleware/i }).first();
    if (await node.isVisible()) {
        await node.evaluate(el => el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true })));
        await page.waitForSelector('.node-popup', { state: 'visible', timeout: 30000 });
        await page.waitForTimeout(2000);
        await page.screenshot({ path: path.join(screenshotDir, 'graph_details.png') });
        // Close popup by clicking outside
        await page.mouse.click(10, 10);
        await page.waitForTimeout(1000);
    }
    */

    // Horizontal mode
    await page.click('#graph-direction-toggle');
    await page.waitForTimeout(5000);
    await page.screenshot({ path: path.join(screenshotDir, 'graph_horizontal.png') });

    // Dark mode
    await setTheme(page, 'dark');
    await page.waitForTimeout(2000);
    await page.screenshot({ path: path.join(screenshotDir, 'graph_dark.png') });
  });

  test('Capture audit timeline', async ({ page }) => {
    await page.goto('/audit.html', { waitUntil: 'domcontentloaded' });
    await ensureLoaderHidden(page);
    
    // Wait for entries or error or empty message
    await page.waitForFunction(() => {
        return !!document.querySelector('.timeline-item') || 
               document.body.innerText.includes('No specification updates found') ||
               document.body.innerText.includes('Error loading timeline');
    }, { timeout: 15000 }).catch(() => console.log("Audit timeline timeout, proceeding anyway"));
    
    await page.waitForTimeout(2000);
    await page.screenshot({ path: path.join(screenshotDir, 'audit.png') });

    // Open a diff
    const diffBtn = page.locator('button:has-text("View Changes")').first();
    if (await diffBtn.isVisible()) {
      await diffBtn.click();
      await page.waitForSelector('.d2h-wrapper', { state: 'visible', timeout: 10000 }).catch(() => {});
      await page.waitForTimeout(1000);
      await page.screenshot({ path: path.join(screenshotDir, 'audit_diff.png') });
    }

    // Dark mode audit
    await setTheme(page, 'dark');
    await page.waitForTimeout(2000);
    await page.screenshot({ path: path.join(screenshotDir, 'audit_dark.png') });
  });

  test('Capture reports and observability', async ({ page }) => {
    await page.goto('/reports.html?branch=main', { waitUntil: 'domcontentloaded' });
    await ensureLoaderHidden(page);
    
    // Dashboard view
    await page.waitForSelector('h3:has-text("Full Dependency Report")', { timeout: 15000 }).catch(async () => {
        console.log("Reports dashboard timeout, forcing unhide");
        await page.evaluate(() => {
            const dash = document.getElementById('report-dashboard');
            if (dash) dash.classList.remove('hidden');
        });
    });
    
    await page.waitForTimeout(2000);
    await page.screenshot({ path: path.join(screenshotDir, 'reports_dashboard.png') });

    // Actual report view
    const viewReportBtn = page.locator('text=View Markdown Report');
    if (await viewReportBtn.isVisible()) {
        await viewReportBtn.click();
        await page.waitForSelector('#report-container table', { timeout: 20000 }).catch(() => {});
        await page.waitForTimeout(2000);
        await page.screenshot({ path: path.join(screenshotDir, 'reports.png') });
    }

    await page.goto('/observability.html', { waitUntil: 'domcontentloaded' });
    await ensureLoaderHidden(page);
    await page.waitForSelector('h3:has-text("Health Check")', { timeout: 15000 }).catch(() => {});
    await page.waitForTimeout(2000);
    await page.screenshot({ path: path.join(screenshotDir, 'observability.png') });
  });

  test('Capture clients list', async ({ page }) => {
    await page.goto('/clients.html', { waitUntil: 'domcontentloaded' });
    await ensureLoaderHidden(page);
    await page.waitForSelector('.endpoint-card', { timeout: 20000 });
    await page.screenshot({ path: path.join(screenshotDir, 'clients_list.png') });
    
    // Click on a client to show its requirements
    const clientCard = page.locator('.endpoint-card', { hasText: 'InventoryService' }).first();
    if (await clientCard.isVisible()) {
        await clientCard.click();
        await page.waitForSelector('span:has-text("InventoryService")');
        await page.waitForTimeout(1000);
        await page.screenshot({ path: path.join(screenshotDir, 'client_details.png') });
    }
  });

  test('Capture admin dashboard sections', async ({ page }) => {
    await page.goto('/admin.html', { waitUntil: 'domcontentloaded' });
    await ensureLoaderHidden(page);
    await page.waitForSelector('#admin-dashboard:not(.hidden)', { timeout: 30000 });
    
    // General overview
    await page.screenshot({ path: path.join(screenshotDir, 'admin_overview.png') });

    // Scroll to Users section - use exact match to avoid ambiguity
    const usersHeader = page.locator('h2', { hasText: /^Users$/ });
    await usersHeader.scrollIntoViewIfNeeded({ timeout: 10000 });
    await page.waitForTimeout(1000);
    await page.screenshot({ path: path.join(screenshotDir, 'admin_users.png') });

    // Scroll to Protected Branches
    const branchesHeader = page.locator('h2', { hasText: /^Protected Branches$/ });
    await branchesHeader.scrollIntoViewIfNeeded({ timeout: 10000 });
    await page.waitForTimeout(1000);
    await page.screenshot({ path: path.join(screenshotDir, 'admin_protected_branches.png') });

    // Scroll to Authentication
    const authHeader = page.locator('h2', { hasText: /^Authentication$/ });
    await authHeader.scrollIntoViewIfNeeded({ timeout: 10000 });
    // Switch to LDAP to show fields
    await page.click('input[value="ldap"]');
    await page.waitForSelector('#ldap-config-form:not(.hidden)', { timeout: 10000 });
    await page.waitForTimeout(1000);
    await page.screenshot({ path: path.join(screenshotDir, 'admin_auth_ldap.png') });
  });
});
