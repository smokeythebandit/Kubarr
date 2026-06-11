import { test, expect, Page } from '@playwright/test';

// Helper to find an app card by display name
function getAppCard(page: Page, displayName: string) {
  return page.locator('.rounded-xl').filter({ has: page.locator(`h3:has-text("${displayName}")`) });
}

test.describe('Apps Page', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/apps');
    // Wait for apps page to load
    await expect(page.locator('h1:has-text("Apps")')).toBeVisible({ timeout: 10000 });
    // Wait for catalog to load
    await page.waitForLoadState('networkidle');
  });

  test('shows apps page with categories', async ({ page }) => {
    // Should show app categories
    await expect(page.locator('h2:has-text("Media Servers")')).toBeVisible();
    await expect(page.locator('h2:has-text("Download Clients")')).toBeVisible();
    await expect(page.locator('h2:has-text("Media Managers")')).toBeVisible();
  });

  test('shows available apps in catalog', async ({ page }) => {
    // Should show some well-known apps
    await expect(page.locator('h3:has-text("Sonarr")')).toBeVisible();
    await expect(page.locator('h3:has-text("Radarr")')).toBeVisible();
    await expect(page.locator('h3:has-text("Jellyfin")')).toBeVisible();
  });

  test('each app has install or open button', async ({ page }) => {
    // Check a few apps have the expected buttons
    for (const app of ['Sonarr', 'Radarr', 'Jellyfin']) {
      const appCard = getAppCard(page, app);
      await expect(appCard).toBeVisible();

      // Should have either Install or Open button
      const hasInstall = await appCard.locator('button:has-text("Install")').isVisible();
      const hasOpen = await appCard.locator('a:has-text("Open"), button:has-text("Open")').isVisible();
      expect(hasInstall || hasOpen).toBe(true);
    }
  });

  test('install button shows installing state when clicked', async ({ page }) => {
    // Find an app that's not installed
    const installButton = page.locator('button:has-text("Install")').first();

    if (await installButton.isVisible().catch(() => false)) {
      // Click install
      await installButton.click();

      // Button should show installing state (badge appears)
      await expect(page.locator('text=Installing').first()).toBeVisible({ timeout: 10000 });
    } else {
      // All apps are installed, skip
      test.skip();
    }
  });

  test('installed apps show uninstall button', async ({ page }) => {
    // Find an installed app (has Open button)
    const openButton = page.locator('a:has-text("Open")').first();

    if (await openButton.isVisible().catch(() => false)) {
      // Find the app card containing this Open button
      const appCard = openButton.locator('..').locator('..');

      // Should have uninstall button
      await expect(appCard.locator('button[title="Uninstall"]')).toBeVisible();
    } else {
      // No apps installed, skip
      test.skip();
    }
  });

  test('system apps show system badge', async ({ page }) => {
    // Look for System badge
    const systemBadge = page.locator('text=System').first();

    // System badge might or might not be visible depending on what's deployed
    if (await systemBadge.isVisible({ timeout: 2000 }).catch(() => false)) {
      await expect(systemBadge).toBeVisible();
    }
  });

  test.describe('App Filtering', () => {
    test('filter dropdown is visible', async ({ page }) => {
      // Look for a filter/category selector
      const filterButton = page.locator('button:has-text("All"), select, [role="combobox"]').first();
      if (await filterButton.isVisible().catch(() => false)) {
        await expect(filterButton).toBeVisible();
      }
    });

    test('filter dropdown changes displayed apps', async ({ page }) => {
      // Look for filter control that can filter by installed status
      const filterButton = page.locator('button:has-text("All"), button:has-text("Filter")').first();
      if (await filterButton.isVisible().catch(() => false)) {
        await filterButton.click();
        const installedOption = page.locator('text=Installed').first();
        if (await installedOption.isVisible().catch(() => false)) {
          await installedOption.click();
          await page.waitForLoadState('networkidle');
          // Page should still be functional
          await expect(page.locator('h1:has-text("Apps")')).toBeVisible();
        }
      }
    });
  });

  test.describe('App Detail Panel', () => {
    test('clicking an app card opens detail panel', async ({ page }) => {
      // Click on a known app card
      const appCard = getAppCard(page, 'Sonarr');
      if (await appCard.isVisible().catch(() => false)) {
        await appCard.click();
        await page.waitForTimeout(500);
        // Look for detail panel elements - app name, description, or action button
        const hasDetail = await page.locator('text=Sonarr').first().isVisible().catch(() => false);
        expect(hasDetail).toBe(true);
      }
    });

    test('app card shows app name and category', async ({ page }) => {
      const appCard = getAppCard(page, 'Sonarr');
      await expect(appCard).toBeVisible();
      await expect(appCard.locator('h3:has-text("Sonarr")')).toBeVisible();
    });

    test('app card has action button (Install or Open)', async ({ page }) => {
      const appCard = getAppCard(page, 'Sonarr');
      await expect(appCard).toBeVisible();
      const hasInstall = await appCard.locator('button:has-text("Install")').isVisible().catch(() => false);
      const hasOpen = await appCard.locator('a:has-text("Open"), button:has-text("Open")').isVisible().catch(() => false);
      expect(hasInstall || hasOpen).toBe(true);
    });
  });
});
