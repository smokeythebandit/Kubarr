import { test as setup, expect } from '@playwright/test';

const authFile = 'tests/.auth/user.json';

/**
 * Authentication setup - logs in via UI and saves session state
 */
setup('authenticate', async ({ page }) => {
  const username = process.env.TEST_USERNAME || 'admin';
  const password = process.env.TEST_PASSWORD || 'adminadmin';

  // Go to login page
  await page.goto('/login');

  // Wait for login form
  await expect(page.locator('input[name="username"]')).toBeVisible();

  // Fill credentials
  await page.fill('input[name="username"]', username);
  await page.fill('input[name="password"]', password);

  // Submit
  await page.click('button[type="submit"]');

  // Wait for redirect to dashboard
  await expect(page).toHaveURL('/', { timeout: 10000 });

  // Save auth state
  await page.context().storageState({ path: authFile });
});
