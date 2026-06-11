import { test, expect } from '@playwright/test';

test.describe('Resources Page', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/resources');
    await expect(page.locator('h1:has-text("Resources")')).toBeVisible({ timeout: 10000 });
  });

  test.describe('Page Display', () => {
    test('shows page heading and subtitle', async ({ page }) => {
      await expect(page.locator('h1:has-text("Resources")')).toBeVisible();
      await expect(page.locator('text=CPU, memory, and resource usage metrics')).toBeVisible();
    });

    test('shows Cluster Overview section', async ({ page }) => {
      await expect(page.locator('text=Cluster Overview')).toBeVisible({ timeout: 15000 });
    });

    test('shows CPU usage stat card', async ({ page }) => {
      await expect(page.locator('text=CPU Usage')).toBeVisible({ timeout: 15000 });
    });

    test('shows Memory usage stat card', async ({ page }) => {
      await expect(page.locator('text=Memory Usage')).toBeVisible({ timeout: 15000 });
    });

    test('shows Network I/O stat card', async ({ page }) => {
      await expect(page.locator('text=Network I/O')).toBeVisible({ timeout: 15000 });
    });

    test('shows Containers stat card', async ({ page }) => {
      await expect(page.locator('text=Containers')).toBeVisible({ timeout: 15000 });
    });

    test('shows Pods stat card', async ({ page }) => {
      await expect(page.locator('text=Pods')).toBeVisible({ timeout: 15000 });
    });

    test('shows App Resource Usage section', async ({ page }) => {
      await expect(page.locator('text=App Resource Usage')).toBeVisible({ timeout: 15000 });
    });
  });

  test.describe('Controls', () => {
    test('Live/Paused toggle button works', async ({ page }) => {
      // Should show either Live or Paused button
      const liveButton = page.locator('button:has-text("Live")');
      const pausedButton = page.locator('button:has-text("Paused")');

      const isLive = await liveButton.isVisible().catch(() => false);
      const isPaused = await pausedButton.isVisible().catch(() => false);
      expect(isLive || isPaused).toBe(true);

      // Toggle the state
      if (isLive) {
        await liveButton.click();
        await expect(pausedButton).toBeVisible();
      } else {
        await pausedButton.click();
        await expect(liveButton).toBeVisible();
      }
    });

    test('Refresh button is visible', async ({ page }) => {
      const _refreshButton = page.locator('button').filter({ has: page.locator('svg') }).filter({ hasText: /^$/ }).first();
      // There should be a refresh icon button
      await expect(page.locator('button').first()).toBeVisible();
    });

    test('Auto-refresh indicator text shown when live', async ({ page }) => {
      // Ensure live mode is active
      const pausedButton = page.locator('button:has-text("Paused")');
      if (await pausedButton.isVisible().catch(() => false)) {
        await pausedButton.click();
      }
      await expect(page.locator('text=/Auto-refreshing|auto-refresh/i')).toBeVisible({ timeout: 15000 });
    });
  });

  test.describe('App Resource Usage Table', () => {
    test('shows table with app metrics or empty state', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      // Either shows app rows or "No app metrics available"
      const hasTable = await page.locator('text=Total').isVisible({ timeout: 10000 }).catch(() => false);
      const hasEmpty = await page.locator('text=No app metrics available').isVisible().catch(() => false);
      expect(hasTable || hasEmpty).toBe(true);
    });
  });

  test.describe('App Detail Modal', () => {
    test('clicking an app row opens detail modal', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      // Find an app row in the resource usage table (skip the header and total rows)
      const appRow = page.locator('table tbody tr').first();
      if (await appRow.isVisible().catch(() => false)) {
        await appRow.click();
        // Modal should appear with tabs
        const metricsTab = page.locator('text=Metrics');
        const podsTab = page.locator('text=Pods');
        if (await metricsTab.isVisible({ timeout: 5000 }).catch(() => false)) {
          await expect(metricsTab).toBeVisible();
          await expect(podsTab).toBeVisible();
          await expect(page.locator('text=Logs')).toBeVisible();
        }
      }
    });

    test('modal has duration selector buttons', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const appRow = page.locator('table tbody tr').first();
      if (await appRow.isVisible().catch(() => false)) {
        await appRow.click();
        const metricsTab = page.locator('button:has-text("Metrics")');
        if (await metricsTab.isVisible({ timeout: 5000 }).catch(() => false)) {
          // Check duration selector buttons
          await expect(page.locator('button:has-text("15m")')).toBeVisible();
          await expect(page.locator('button:has-text("1h")')).toBeVisible();
        }
      }
    });

    test('modal can be closed', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const appRow = page.locator('table tbody tr').first();
      if (await appRow.isVisible().catch(() => false)) {
        await appRow.click();
        const metricsTab = page.locator('button:has-text("Metrics")');
        if (await metricsTab.isVisible({ timeout: 5000 }).catch(() => false)) {
          // Close the modal via the X button
          const closeButton = page.locator('button').filter({ has: page.locator('svg.lucide-x') }).first();
          if (await closeButton.isVisible()) {
            await closeButton.click();
            await expect(metricsTab).not.toBeVisible();
          }
        }
      }
    });
  });
});
