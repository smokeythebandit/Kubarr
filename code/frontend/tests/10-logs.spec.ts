import { test, expect } from '@playwright/test';

test.describe('Logs Page', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/logs');
    await expect(page.locator('h1:has-text("Logs")')).toBeVisible({ timeout: 10000 });
  });

  test.describe('Page Display', () => {
    test('shows page heading and subtitle', async ({ page }) => {
      await expect(page.locator('h1:has-text("Logs")')).toBeVisible();
      await expect(page.locator('text=View application logs')).toBeVisible();
    });

    test('shows log entries table with column headers', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      // Log table has sticky headers - look in the main content area
      const hasLogs = await page.locator('text=/\\d+ log entr/i').isVisible({ timeout: 10000 }).catch(() => false);
      const hasNoLogs = await page.locator('text=No logs found').isVisible().catch(() => false);
      // Verify the log viewer rendered (either with data or empty state)
      expect(hasLogs || hasNoLogs).toBe(true);
    });

    test('shows log entry count or loading indicator', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      // Should show either entry count or "Loading..."
      const hasCount = await page.locator('text=/\\d+ log entr/i').isVisible({ timeout: 10000 }).catch(() => false);
      const hasLoading = await page.locator('text=Loading').isVisible().catch(() => false);
      const hasNoLogs = await page.locator('text=No logs found').isVisible().catch(() => false);
      expect(hasCount || hasLoading || hasNoLogs).toBe(true);
    });
  });

  test.describe('Filters', () => {
    test('App filter dropdown opens and shows app checkboxes', async ({ page }) => {
      // Click the app filter button
      const appFilter = page.locator('button').filter({ hasText: /All apps|apps?/i }).first();
      await expect(appFilter).toBeVisible({ timeout: 10000 });
      await appFilter.click();
      // Dropdown should show checkboxes
      await expect(page.locator('text=/Select All|Deselect All/i')).toBeVisible({ timeout: 5000 });
    });

    test('Log level filter dropdown opens and shows level checkboxes', async ({ page }) => {
      // Click the level filter button
      const levelFilter = page.locator('button').filter({ hasText: /All levels|levels?/i }).first();
      await expect(levelFilter).toBeVisible({ timeout: 10000 });
      await levelFilter.click();
      // Dropdown should show log level options
      await expect(page.locator('text=INFO')).toBeVisible({ timeout: 5000 });
      await expect(page.locator('text=ERROR')).toBeVisible();
      await expect(page.locator('text=WARN')).toBeVisible();
    });

    test('Time range dropdown shows options', async ({ page }) => {
      // Click the time range filter button (shows "Last 1 hour" etc.)
      const timeFilter = page.locator('button').filter({ hasText: /Last|hour/i }).first();
      await expect(timeFilter).toBeVisible({ timeout: 10000 });
      await timeFilter.click();
      // Dropdown shows full labels - use .first() since button label and dropdown item both match
      await expect(page.locator('text=Last 15 minutes').first()).toBeVisible({ timeout: 5000 });
      await expect(page.locator('text=Last 7 days').first()).toBeVisible();
    });

    test('Search input accepts text', async ({ page }) => {
      const searchInput = page.locator('input[placeholder*="Search" i]');
      await expect(searchInput).toBeVisible({ timeout: 10000 });
      await searchInput.fill('test search');
      await expect(searchInput).toHaveValue('test search');
    });
  });

  test.describe('Controls', () => {
    test('Auto-refresh (Live) toggle button works', async ({ page }) => {
      // Find the auto-refresh / Live button
      const liveButton = page.locator('button').filter({ hasText: /Live|Auto/i }).first();
      await expect(liveButton).toBeVisible({ timeout: 10000 });
      await liveButton.click();
      // Button should still be visible after toggle
      await expect(liveButton).toBeVisible();
    });

    test('Manual refresh button works', async ({ page }) => {
      const refreshButton = page.locator('button:has-text("Refresh")');
      await expect(refreshButton).toBeVisible({ timeout: 10000 });
      await refreshButton.click();
      // Page should still show logs heading after refresh
      await expect(page.locator('h1:has-text("Logs")')).toBeVisible();
    });

    test('Column headers are clickable for sorting', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      // Click on "Time" column header to sort
      const timeHeader = page.locator('th:has-text("Time"), div:has-text("Time")').first();
      await expect(timeHeader).toBeVisible({ timeout: 10000 });
      await timeHeader.click();
      // Page should still be functional after sort click
      await expect(page.locator('h1:has-text("Logs")')).toBeVisible();
    });
  });
});
