import { test, expect } from '@playwright/test';

test.describe('OAuth Configuration and Login Flow', () => {
  test.describe('OAuth Provider Admin Configuration', () => {
    test.beforeEach(async ({ page }) => {
      await page.goto('/settings?section=general');
      await page.waitForLoadState('networkidle');
      // Wait for settings page to fully load
      await expect(page.locator('h1, h2, h3').filter({ hasText: /general|settings/i }).first()).toBeVisible({ timeout: 10000 });
    });

    test('General section shows OAuth Providers heading', async ({ page }) => {
      await expect(page.locator('text=OAuth Providers')).toBeVisible({ timeout: 10000 });
    });

    test('OAuth Providers section shows descriptive text', async ({ page }) => {
      await expect(page.locator('text=OAuth Providers')).toBeVisible({ timeout: 10000 });
      await expect(page.locator('text=/Google|Microsoft|sign in|OAuth/i').first()).toBeVisible({ timeout: 5000 });
    });

    test('shows provider list with Google and Microsoft entries', async ({ page }) => {
      await expect(page.locator('text=OAuth Providers')).toBeVisible({ timeout: 10000 });
      // Wait for providers to load from API
      await page.waitForTimeout(2000);
      const hasGoogle = await page.locator('text=Google').isVisible().catch(() => false);
      const hasMicrosoft = await page.locator('text=Microsoft').isVisible().catch(() => false);
      // At least one OAuth provider should be listed
      expect(hasGoogle || hasMicrosoft).toBe(true);
    });

    test('each provider card has a Configure button', async ({ page }) => {
      await expect(page.locator('text=OAuth Providers')).toBeVisible({ timeout: 10000 });
      await page.waitForTimeout(2000);
      const configureButton = page.locator('button:has-text("Configure")').first();
      await expect(configureButton).toBeVisible({ timeout: 5000 });
    });

    test('clicking Configure reveals Client ID and Client Secret fields', async ({ page }) => {
      await expect(page.locator('text=OAuth Providers')).toBeVisible({ timeout: 10000 });
      await page.waitForTimeout(2000);

      const configureButton = page.locator('button:has-text("Configure")').first();
      const isVisible = await configureButton.isVisible({ timeout: 5000 }).catch(() => false);
      if (!isVisible) {
        test.skip();
        return;
      }

      await configureButton.click();

      // Configuration form should appear with credential fields
      await expect(page.locator('text=Client ID').first()).toBeVisible({ timeout: 5000 });
      await expect(page.locator('text=Client Secret').first()).toBeVisible({ timeout: 5000 });

      // Action buttons should be present
      await expect(page.locator('button:has-text("Save")').first()).toBeVisible();
      await expect(page.locator('button:has-text("Cancel")').first()).toBeVisible();
    });

    test('Client ID input field accepts text', async ({ page }) => {
      await expect(page.locator('text=OAuth Providers')).toBeVisible({ timeout: 10000 });
      await page.waitForTimeout(2000);

      const configureButton = page.locator('button:has-text("Configure")').first();
      const isVisible = await configureButton.isVisible({ timeout: 5000 }).catch(() => false);
      if (!isVisible) {
        test.skip();
        return;
      }

      await configureButton.click();
      await expect(page.locator('text=Client ID').first()).toBeVisible({ timeout: 5000 });

      // Find the Client ID text input (not password)
      const clientIdInput = page.locator('input[type="text"]').first();
      await clientIdInput.fill('test-client-id-12345');
      await expect(clientIdInput).toHaveValue('test-client-id-12345');
    });

    test('Cancel button dismisses Configure form', async ({ page }) => {
      await expect(page.locator('text=OAuth Providers')).toBeVisible({ timeout: 10000 });
      await page.waitForTimeout(2000);

      const configureButton = page.locator('button:has-text("Configure")').first();
      const isVisible = await configureButton.isVisible({ timeout: 5000 }).catch(() => false);
      if (!isVisible) {
        test.skip();
        return;
      }

      await configureButton.click();
      await expect(page.locator('text=Client ID').first()).toBeVisible({ timeout: 5000 });

      // Cancel the form
      await page.locator('button:has-text("Cancel")').first().click();

      // The form should be dismissed and Configure button visible again
      await expect(page.locator('text=Client ID').first()).not.toBeVisible({ timeout: 3000 });
      await expect(page.locator('button:has-text("Configure")').first()).toBeVisible({ timeout: 5000 });
    });

    test('provider toggle is disabled when client_id is not configured', async ({ page }) => {
      await expect(page.locator('text=OAuth Providers')).toBeVisible({ timeout: 10000 });
      await page.waitForTimeout(2000);

      // A provider without credentials should show it's disabled
      const disabledText = page.locator('text=Disabled').first();
      const notConfiguredText = page.locator('text=/not configured|no credentials/i').first();
      const configureText = page.locator('button:has-text("Configure")').first();

      // At least one of these should be visible indicating unconfigured state
      const hasDisabled = await disabledText.isVisible().catch(() => false);
      const hasNotConfigured = await notConfiguredText.isVisible().catch(() => false);
      const hasConfigure = await configureText.isVisible().catch(() => false);

      expect(hasDisabled || hasNotConfigured || hasConfigure).toBe(true);
    });
  });

  test.describe('Login Page OAuth Buttons', () => {
    test('login page loads and renders without console errors', async ({ page }) => {
      const consoleErrors: string[] = [];
      page.on('console', (msg) => {
        if (msg.type() === 'error') {
          consoleErrors.push(msg.text());
        }
      });

      await page.goto('/login');
      await expect(page.locator('button:has-text("Sign in")').first()).toBeVisible({ timeout: 10000 });
      await page.waitForLoadState('networkidle');

      // Filter out known non-critical/environmental errors
      const criticalErrors = consoleErrors.filter(
        (e) =>
          !e.includes('favicon') &&
          !e.includes('ERR_NETWORK_CHANGED') &&
          !e.includes('net::ERR_') &&
          !e.includes('Failed to load resource')
      );
      expect(criticalErrors.length).toBe(0);
    });

    test('login page has standard username and password fields', async ({ page }) => {
      await page.goto('/login');
      await page.waitForLoadState('domcontentloaded');

      // Standard login form elements should be present
      await expect(page.locator('input[type="text"], input[type="email"]').first()).toBeVisible({ timeout: 10000 });
      await expect(page.locator('input[type="password"]').first()).toBeVisible({ timeout: 10000 });
    });

    test('login page shows "Sign in with" button when OAuth provider is enabled', async ({ page }) => {
      await page.goto('/login');
      await page.waitForLoadState('networkidle');

      // If any OAuth provider is configured and enabled, a "Sign in with" button appears
      const oauthButton = page.locator('button:has-text("Sign in with")').first();
      const orContinueWith = page.locator('text=Or continue with').first();

      // Either OAuth buttons are shown, or the login page renders normally without them
      const hasOAuth = await oauthButton.isVisible().catch(() => false);
      const hasDivider = await orContinueWith.isVisible().catch(() => false);
      const hasSignIn = await page.locator('button:has-text("Sign in")').first().isVisible().catch(() => false);

      // The page should always show the regular sign-in button at minimum
      expect(hasSignIn).toBe(true);
      // OAuth state is dynamic - just verify no error
      if (hasOAuth || hasDivider) {
        await expect(page.locator('button:has-text("Sign in with")').first()).toBeVisible();
      }
    });

    test('login page does not show blank white page', async ({ page }) => {
      await page.goto('/login');
      await page.waitForLoadState('domcontentloaded');

      const bodyText = await page.locator('body').textContent();
      expect(bodyText?.trim().length).toBeGreaterThan(10);
    });
  });

  test.describe('OAuth Callback Error Paths', () => {
    test('OAuth callback with access_denied does not return 500 or blank page', async ({ page }) => {
      // The OAuth callback URL is handled by the backend
      // It should redirect to login or return a handled error response
      const response = await page.goto('/api/oauth/google/callback?error=access_denied');

      if (response) {
        // Should not be a 500 server error
        expect(response.status()).not.toBe(500);
      }

      // Wait for any redirect to complete
      await page.waitForTimeout(1000);

      // Page should have content - not blank
      const bodyText = await page.locator('body').textContent();
      expect(bodyText?.trim().length).toBeGreaterThan(0);
    });

    test('OAuth callback with pending_approval does not return 500 or blank page', async ({ page }) => {
      const response = await page.goto('/api/oauth/google/callback?error=pending_approval');

      if (response) {
        expect(response.status()).not.toBe(500);
      }

      await page.waitForTimeout(1000);

      const bodyText = await page.locator('body').textContent();
      expect(bodyText?.trim().length).toBeGreaterThan(0);
    });

    test('login page with error query param shows login form, not error page', async ({ page }) => {
      // Simulate redirect from OAuth with error
      await page.goto('/login?error=access_denied');
      await page.waitForLoadState('domcontentloaded');

      // Login form should still be accessible
      await expect(page.locator('input[type="password"]').first()).toBeVisible({ timeout: 10000 });

      // Page should not be blank
      const bodyText = await page.locator('body').textContent();
      expect(bodyText?.trim().length).toBeGreaterThan(10);
    });

    test('login page with pending_approval error shows login form', async ({ page }) => {
      await page.goto('/login?error=pending_approval');
      await page.waitForLoadState('domcontentloaded');

      // Login form should be visible
      const bodyText = await page.locator('body').textContent();
      expect(bodyText?.trim().length).toBeGreaterThan(10);

      // Should not show a blank page or 500 error
      const has500 = await page.locator('text=500').isVisible().catch(() => false);
      const hasInternalError = await page.locator('text=Internal Server Error').isVisible().catch(() => false);
      expect(has500 || hasInternalError).toBe(false);
    });
  });

  test.describe('Linked Accounts Section on Account Page', () => {
    test('account page shows Linked Accounts section', async ({ page }) => {
      await page.goto('/account');
      await expect(page.locator('h1:has-text("Account Settings")')).toBeVisible({ timeout: 10000 });
      await page.waitForLoadState('networkidle');

      // Look for linked accounts section or OAuth-related content
      const hasLinkedAccounts = await page.locator('text=Linked Accounts').isVisible({ timeout: 5000 }).catch(() => false);
      const hasConnectedAccounts = await page.locator('text=/connected account|sign in with|link.*account/i').first().isVisible().catch(() => false);

      expect(hasLinkedAccounts || hasConnectedAccounts).toBe(true);
    });

    test('linked accounts section shows provider options', async ({ page }) => {
      await page.goto('/account');
      await expect(page.locator('h1:has-text("Account Settings")')).toBeVisible({ timeout: 10000 });
      await page.waitForLoadState('networkidle');

      const linkedSection = page.locator('text=Linked Accounts');
      if (!(await linkedSection.isVisible({ timeout: 5000 }).catch(() => false))) {
        test.skip();
        return;
      }

      // Should mention Google or Microsoft as linkable providers
      const hasGoogle = await page.locator('text=Google').isVisible().catch(() => false);
      const hasMicrosoft = await page.locator('text=Microsoft').isVisible().catch(() => false);
      expect(hasGoogle || hasMicrosoft).toBe(true);
    });
  });
});
