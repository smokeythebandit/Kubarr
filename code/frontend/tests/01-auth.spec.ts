import { test, expect } from '@playwright/test';

test.describe('Authentication', () => {
  test.describe('Login', () => {
    // These tests don't use the authenticated state
    test.use({ storageState: { cookies: [], origins: [] } });

    test('shows login form', async ({ page }) => {
      await page.goto('/login');

      await expect(page.locator('text=Kubarr Dashboard')).toBeVisible();
      await expect(page.locator('input[name="username"]')).toBeVisible();
      await expect(page.locator('input[name="password"]')).toBeVisible();
      await expect(page.locator('button[type="submit"]')).toBeVisible();
    });

    test('shows error for invalid credentials', async ({ page }) => {
      await page.goto('/login');

      await page.fill('input[name="username"]', 'invaliduser');
      await page.fill('input[name="password"]', 'wrongpassword');
      await page.click('button[type="submit"]');

      // Should show error message
      await expect(page.locator('text=Invalid credentials')).toBeVisible({ timeout: 5000 });
    });

    test('successful login redirects to dashboard', async ({ page }) => {
      const username = process.env.TEST_USERNAME || 'admin';
      const password = process.env.TEST_PASSWORD || 'adminadmin';

      await page.goto('/login');

      await page.fill('input[name="username"]', username);
      await page.fill('input[name="password"]', password);
      await page.click('button[type="submit"]');

      // Should redirect to dashboard
      await page.waitForURL('/', { timeout: 15000 });
      await expect(page.locator('h2:has-text("Dashboard")')).toBeVisible({ timeout: 10000 });
    });

    test('redirects authenticated user away from login', async ({ page }) => {
      // First login
      const username = process.env.TEST_USERNAME || 'admin';
      const password = process.env.TEST_PASSWORD || 'adminadmin';

      await page.goto('/login');
      await page.fill('input[name="username"]', username);
      await page.fill('input[name="password"]', password);
      await page.click('button[type="submit"]');
      await expect(page).toHaveURL('/');

      // Try to go back to login - should redirect to dashboard
      await page.goto('/login');
      await expect(page).toHaveURL('/');
    });
  });

  test.describe('Account Picker', () => {
    test.use({ storageState: { cookies: [], origins: [] } });

    test('shows account picker when add_account=true with existing sessions', async ({ page }) => {
      // First login to create a session
      const username = process.env.TEST_USERNAME || 'admin';
      const password = process.env.TEST_PASSWORD || 'adminadmin';

      await page.goto('/login');
      await page.fill('input[name="username"]', username);
      await page.fill('input[name="password"]', password);
      await page.click('button[type="submit"]');
      await expect(page).toHaveURL('/');

      // Now go to login with add_account flag
      await page.goto('/login?add_account=true');

      // Should show account picker
      await expect(page.locator('text=Choose an account')).toBeVisible();
      await expect(page.locator('text=Sign in with a different account')).toBeVisible();
    });

    test('sign in with different account button shows login form', async ({ page }) => {
      // First login to create a session
      const username = process.env.TEST_USERNAME || 'admin';
      const password = process.env.TEST_PASSWORD || 'adminadmin';

      await page.goto('/login');
      await page.fill('input[name="username"]', username);
      await page.fill('input[name="password"]', password);
      await page.click('button[type="submit"]');
      await expect(page).toHaveURL('/');

      // Go to login with add_account flag
      await page.goto('/login?add_account=true');
      await expect(page.locator('text=Choose an account')).toBeVisible();

      // Click "Sign in with a different account"
      await page.click('text=Sign in with a different account');

      // Should show login form
      await expect(page.locator('input[name="username"]')).toBeVisible();
      await expect(page.locator('input[name="password"]')).toBeVisible();
    });
  });

  test.describe('Logout', () => {
    // Use fresh context - don't share auth state since logout invalidates it
    test.use({ storageState: { cookies: [], origins: [] } });

    test('logout button logs out user', async ({ page }) => {
      // First login
      const username = process.env.TEST_USERNAME || 'admin';
      const password = process.env.TEST_PASSWORD || 'adminadmin';

      await page.goto('/login');
      await page.fill('input[name="username"]', username);
      await page.fill('input[name="password"]', password);
      await page.click('button[type="submit"]');
      await expect(page).toHaveURL('/');

      // Now test logout - click to open user dropdown menu
      await page.locator('nav').locator('button:has-text("admin")').click();
      await page.locator('text=Logout').click();

      // Should redirect to login
      await expect(page).toHaveURL('/login');
    });
  });
});
