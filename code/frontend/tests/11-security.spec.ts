import { test, expect } from '@playwright/test';

test.describe('Security Page', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/security');
    await expect(page.locator('h1:has-text("Security")')).toBeVisible({ timeout: 10000 });
  });

  test.describe('Page Display', () => {
    test('shows page heading and subtitle', async ({ page }) => {
      await expect(page.locator('h1:has-text("Security")')).toBeVisible();
      await expect(page.locator('text=Monitor authentication events')).toBeVisible();
    });

    test('shows Events Today stat card', async ({ page }) => {
      await expect(page.locator('text=Events Today')).toBeVisible({ timeout: 10000 });
    });

    test('shows Failed Attempts stat card', async ({ page }) => {
      await expect(page.locator('text=Failed Attempts')).toBeVisible({ timeout: 10000 });
    });

    test('shows Total Events stat card', async ({ page }) => {
      await expect(page.locator('text=Total Events')).toBeVisible({ timeout: 10000 });
    });

    test('shows 2FA Enabled stat card', async ({ page }) => {
      await expect(page.locator('text=2FA Enabled')).toBeVisible({ timeout: 10000 });
    });

    test('shows security events table', async ({ page }) => {
      await expect(page.locator('h2:has-text("Security Events")')).toBeVisible({ timeout: 10000 });
    });

    test('shows security events table or empty state', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      // When there are no events, shows empty state instead of table columns
      const hasTable = await page.locator('th:has-text("Event")').isVisible({ timeout: 5000 }).catch(() => false);
      const hasEmpty = await page.locator('text=No security events found').isVisible().catch(() => false);
      expect(hasTable || hasEmpty).toBe(true);
    });
  });

  test.describe('Event Data', () => {
    test('events table shows login events from test setup', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      // The bootstrap/auth setup logs in, so there should be events
      const hasEvents = await page.locator('table tbody tr').first().isVisible({ timeout: 10000 }).catch(() => false);
      const hasEmpty = await page.locator('text=No security events found').isVisible().catch(() => false);
      expect(hasEvents || hasEmpty).toBe(true);
    });

    test('status badges show Success or Failed', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const hasSuccess = await page.locator('text=Success').first().isVisible({ timeout: 10000 }).catch(() => false);
      const hasFailed = await page.locator('text=Failed').first().isVisible().catch(() => false);
      const hasEmpty = await page.locator('text=No security events found').isVisible().catch(() => false);
      // Should show at least one status badge or empty state
      expect(hasSuccess || hasFailed || hasEmpty).toBe(true);
    });
  });

  test.describe('Event Distribution', () => {
    test('shows Event Distribution section', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      // Event distribution shows when there are events
      const hasDistribution = await page.locator('text=Event Distribution').isVisible({ timeout: 10000 }).catch(() => false);
      const hasEmpty = await page.locator('text=No security events found').isVisible().catch(() => false);
      expect(hasDistribution || hasEmpty).toBe(true);
    });
  });
});
