import { test, expect } from '@playwright/test';

test.describe('Error Pages', () => {
  test.describe('404 Page', () => {
    test('navigating to invalid route shows 404', async ({ page }) => {
      await page.goto('/this-page-does-not-exist-at-all');
      await expect(page.locator('text=404')).toBeVisible({ timeout: 10000 });
      await expect(page.locator('text=Page Not Found')).toBeVisible();
    });

    test('shows descriptive message', async ({ page }) => {
      await page.goto('/nonexistent-route');
      await expect(page.locator('text=404')).toBeVisible({ timeout: 10000 });
      await expect(page.locator("text=doesn't exist")).toBeVisible();
    });

    test('Go Back button is present', async ({ page }) => {
      await page.goto('/nonexistent-route');
      await expect(page.locator('text=404')).toBeVisible({ timeout: 10000 });
      await expect(page.locator('button:has-text("Go Back"), a:has-text("Go Back")')).toBeVisible();
    });

    test('Home link navigates to dashboard', async ({ page }) => {
      await page.goto('/nonexistent-route');
      await expect(page.locator('text=404')).toBeVisible({ timeout: 10000 });
      const homeLink = page.locator('a:has-text("Home")');
      await expect(homeLink).toBeVisible();
      await homeLink.click();
      await expect(page).toHaveURL('/');
    });
  });

  test.describe('App Error Page', () => {
    test('connection_failed reason shows Connection Failed', async ({ page }) => {
      await page.goto('/app-error?app=testapp&reason=connection_failed');
      await expect(page.locator('text=Connection Failed')).toBeVisible({ timeout: 10000 });
    });

    test('not_found reason shows App Not Found', async ({ page }) => {
      await page.goto('/app-error?app=testapp&reason=not_found');
      await expect(page.locator('text=App Not Found')).toBeVisible({ timeout: 10000 });
    });

    test('Try Again button is present', async ({ page }) => {
      await page.goto('/app-error?app=testapp&reason=connection_failed');
      await expect(page.locator('text=Connection Failed')).toBeVisible({ timeout: 10000 });
      await expect(page.locator('button:has-text("Try Again"), a:has-text("Try Again")')).toBeVisible();
    });

    test('Go Back button is present', async ({ page }) => {
      await page.goto('/app-error?app=testapp&reason=connection_failed');
      await expect(page.locator('text=Connection Failed')).toBeVisible({ timeout: 10000 });
      await expect(page.locator('button:has-text("Go Back"), a:has-text("Go Back")')).toBeVisible();
    });

    test('Dashboard button is present', async ({ page }) => {
      await page.goto('/app-error?app=testapp&reason=connection_failed');
      await expect(page.locator('text=Connection Failed')).toBeVisible({ timeout: 10000 });
      // Dashboard is a button on the error page (not just the nav link)
      const dashboardButton = page.locator('button:has-text("Dashboard"), a:has-text("Dashboard")');
      await expect(dashboardButton.first()).toBeVisible();
    });

    test('generic error shows App Error', async ({ page }) => {
      await page.goto('/app-error?app=testapp&reason=unknown');
      await expect(page.locator('text=App Error')).toBeVisible({ timeout: 10000 });
    });

    test('shows link to apps page', async ({ page }) => {
      await page.goto('/app-error?app=testapp&reason=connection_failed');
      await expect(page.locator('text=Connection Failed')).toBeVisible({ timeout: 10000 });
      await expect(page.locator('text=Apps page')).toBeVisible();
    });
  });
});
