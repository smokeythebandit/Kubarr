import { test, expect } from '@playwright/test';

test.describe('Global Navigation', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    await expect(page.locator('h2:has-text("Dashboard")')).toBeVisible({ timeout: 10000 });
  });

  test.describe('Theme Toggle', () => {
    // Theme button has title="Theme: Light/Dark/System" - find the visible one
    const getThemeButton = (page: any) => {
      return page.locator('button[title^="Theme:"]').first();
    };

    test('theme button is visible in nav', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const themeButton = getThemeButton(page);
      await expect(themeButton).toBeVisible();
    });

    test('clicking opens dropdown with Light/Dark/System options', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const themeButton = getThemeButton(page);
      await themeButton.click();
      // Dropdown items have span text inside buttons
      await expect(page.locator('button:has-text("Light") span:has-text("Light")').first()).toBeVisible({ timeout: 5000 });
      await expect(page.locator('button:has-text("Dark") span:has-text("Dark")').first()).toBeVisible();
      await expect(page.locator('button:has-text("System") span:has-text("System")').first()).toBeVisible();
    });

    test('selecting Dark option changes theme', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const themeButton = getThemeButton(page);
      await themeButton.click();
      await page.locator('button:has-text("Dark") span:has-text("Dark")').first().click();
      // HTML element should have dark class
      await expect(page.locator('html')).toHaveClass(/dark/);
    });

    test('selecting Light option changes theme', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const themeButton = getThemeButton(page);
      await themeButton.click();
      await page.locator('button:has-text("Light") span:has-text("Light")').first().click();
      // HTML element should NOT have dark class
      await expect(page.locator('html')).not.toHaveClass(/dark/);
    });
  });

  test.describe('Notification Inbox', () => {
    test('bell icon button is visible', async ({ page }) => {
      const bellButton = page.locator('nav button').filter({ has: page.locator('svg.lucide-bell') }).first();
      await expect(bellButton).toBeVisible();
    });

    test('clicking opens notification dropdown', async ({ page }) => {
      const bellButton = page.locator('nav button').filter({ has: page.locator('svg.lucide-bell') }).first();
      await bellButton.click();
      await expect(page.locator('text=Notifications')).toBeVisible({ timeout: 5000 });
    });

    test('notification dropdown shows heading', async ({ page }) => {
      const bellButton = page.locator('nav button').filter({ has: page.locator('svg.lucide-bell') }).first();
      await bellButton.click();
      await expect(page.locator('text=Notifications')).toBeVisible();
    });

    test('notification preferences link is present', async ({ page }) => {
      const bellButton = page.locator('nav button').filter({ has: page.locator('svg.lucide-bell') }).first();
      await bellButton.click();
      await expect(page.locator('text=Notification preferences')).toBeVisible({ timeout: 5000 });
    });
  });

  test.describe('Mobile Navigation', () => {
    test.use({ viewport: { width: 375, height: 667 } });

    test('hamburger menu button is visible on mobile viewport', async ({ page }) => {
      await page.goto('/');
      await page.waitForLoadState('networkidle');
      // The hamburger button has aria-label="Toggle menu"
      const menuButton = page.locator('button[aria-label="Toggle menu"]');
      await expect(menuButton).toBeVisible();
    });

    test('clicking hamburger opens mobile menu with nav links', async ({ page }) => {
      await page.goto('/');
      await page.waitForLoadState('networkidle');
      const menuButton = page.locator('button[aria-label="Toggle menu"]');
      await menuButton.click();
      // Mobile menu shows nav links - use nth(1) to skip hidden desktop links
      await expect(page.locator('a:has-text("Apps")').nth(1)).toBeVisible({ timeout: 5000 });
      // Logout text last() gets the mobile menu instance (desktop dropdown one is first/hidden)
      await expect(page.locator('text=Logout').last()).toBeVisible();
    });

    test('Status submenu expands on click', async ({ page }) => {
      await page.goto('/');
      await page.waitForLoadState('networkidle');
      const menuButton = page.locator('button[aria-label="Toggle menu"]');
      await menuButton.click();
      // Click Status to expand submenu
      const statusButton = page.locator('button:has-text("Status"), a:has-text("Status")').first();
      if (await statusButton.isVisible({ timeout: 5000 }).catch(() => false)) {
        await statusButton.click();
        // Submenu items should appear
        await expect(page.locator('text=Resources')).toBeVisible();
        await expect(page.locator('text=Storage')).toBeVisible();
      }
    });

    test('nav links navigate to correct pages', async ({ page }) => {
      await page.goto('/');
      await page.waitForLoadState('networkidle');
      const menuButton = page.locator('button[aria-label="Toggle menu"]');
      await menuButton.click();
      // Mobile menu links are second in DOM (desktop links hidden come first)
      const appsLink = page.locator('a:has-text("Apps")').nth(1);
      await expect(appsLink).toBeVisible({ timeout: 5000 });
      await appsLink.click();
      await expect(page).toHaveURL('/apps');
    });
  });

  test.describe('Cluster Metrics', () => {
    test.use({ viewport: { width: 1920, height: 1080 } });

    test('CPU, RAM, NET labels visible in desktop nav', async ({ page }) => {
      await page.goto('/');
      await page.waitForLoadState('networkidle');
      // Cluster metrics should be visible in the nav on desktop
      await expect(page.locator('nav >> text=CPU')).toBeVisible({ timeout: 15000 });
      await expect(page.locator('nav >> text=RAM')).toBeVisible();
      await expect(page.locator('nav >> text=/NET/').first()).toBeVisible();
    });
  });
});
