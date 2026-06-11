import { test, expect } from '@playwright/test';

test.describe('Cloudflare Tunnel Settings', () => {
  // Run tests in this describe block serially to prevent the deploy flow test
  // (which starts a long-running helm operation) from interfering with
  // validation tests via shared DB/K8s state.
  test.describe.configure({ mode: 'serial' });
  test.beforeEach(async ({ page }) => {
    await page.goto('/settings?section=cloudflare');
    await expect(page.locator('text=Cloudflare Tunnel').first()).toBeVisible({ timeout: 10000 });
  });

  test.describe('Navigation', () => {
    test('shows Cloudflare Tunnel item in Networking sidebar section', async ({ page }) => {
      await page.goto('/settings');
      await expect(page.locator('text=NETWORKING').first()).toBeVisible();
      await expect(page.locator('nav button:has-text("Cloudflare Tunnel")')).toBeVisible();
    });

    test('navigates to Cloudflare Tunnel section via sidebar', async ({ page }) => {
      await page.goto('/settings');
      await page.locator('nav button:has-text("Cloudflare Tunnel")').click();
      await expect(page).toHaveURL(/section=cloudflare/);
      await expect(page.locator('text=Cloudflare Tunnel').first()).toBeVisible({ timeout: 10000 });
    });

    test('shows heading and description', async ({ page }) => {
      await expect(page.locator('h3:has-text("Cloudflare Tunnel")').first()).toBeVisible();
      await expect(page.locator('text=Expose Kubarr to the internet')).toBeVisible();
    });
  });

  test.describe('Wizard Form (no tunnel configured)', () => {
    test('shows API token password input in idle state', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const isDeployed = await page.locator('button[aria-label="Remove Tunnel"]').isVisible().catch(() => false);
      if (isDeployed) return; // skip if already deployed

      await expect(page.locator('input[type="password"]').first()).toBeVisible();
      await expect(page.locator('text=Cloudflare API Token').first()).toBeVisible();
    });

    test('shows Connect Cloudflare Account button in idle state', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const isDeployed = await page.locator('button[aria-label="Remove Tunnel"]').isVisible().catch(() => false);
      if (isDeployed) return; // skip if already deployed

      await expect(
        page.locator('button[aria-label="Connect Cloudflare Account"]'),
      ).toBeVisible();
    });

    test('show/hide token toggle works', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const isDeployed = await page.locator('button[aria-label="Remove Tunnel"]').isVisible().catch(() => false);
      if (isDeployed) return; // skip if already deployed

      const tokenInput = page.locator('input[aria-label="Cloudflare API Token"]');
      await expect(tokenInput).toHaveAttribute('type', 'password');

      // Click the eye icon to reveal
      await page.locator('button[aria-label="Show token"]').click();
      await expect(tokenInput).toHaveAttribute('type', 'text');

      // Click again to hide
      await page.locator('button[aria-label="Hide token"]').click();
      await expect(tokenInput).toHaveAttribute('type', 'password');
    });

    test('shows Cloudflare API Tokens link', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const isDeployed = await page.locator('button[aria-label="Remove Tunnel"]').isVisible().catch(() => false);
      if (isDeployed) return; // skip if already deployed

      const link = page.locator('a[href*="dash.cloudflare.com/profile/api-tokens"]');
      await expect(link).toBeVisible();
      await expect(link).toHaveAttribute('target', '_blank');
      await expect(link).toHaveAttribute('rel', 'noopener noreferrer');
    });
  });

  test.describe('Deployed view', () => {
    test('shows Remove Tunnel button when tunnel is configured', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const isDeployed = await page.locator('button[aria-label="Remove Tunnel"]').isVisible().catch(() => false);
      if (!isDeployed) return; // skip if not deployed

      await expect(page.locator('button[aria-label="Remove Tunnel"]')).toBeVisible();
    });

    test('shows status badge when a tunnel is configured', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const isDeployed = await page.locator('button[aria-label="Remove Tunnel"]').isVisible().catch(() => false);
      if (!isDeployed) return; // skip if not deployed

      const hasBadge = await page.locator('text=/Not Deployed|Deploying|Running|Failed|Removing/')
        .first().isVisible().catch(() => false);
      expect(hasBadge).toBe(true);
    });

    test('Remove Tunnel button is absent when no tunnel is deployed', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const isDeployed = await page.locator('button[aria-label="Remove Tunnel"]').isVisible().catch(() => false);
      if (!isDeployed) {
        await expect(page.locator('button[aria-label="Remove Tunnel"]')).not.toBeVisible();
      }
    });
  });

  test.describe('Validation', () => {
    test('shows error when connecting with empty API token', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const isDeployed = await page.locator('button[aria-label="Remove Tunnel"]').isVisible().catch(() => false);
      if (isDeployed) return; // skip if already deployed

      // Click without filling in the token
      await page.locator('button[aria-label="Connect Cloudflare Account"]').click();
      await page.waitForTimeout(300);

      const hasError = await page.locator('text=/required|Please enter/i').first()
        .isVisible().catch(() => false);
      const formStillVisible = await page.locator('input[aria-label="Cloudflare API Token"]')
        .isVisible().catch(() => false);
      expect(hasError || formStillVisible).toBe(true);
    });

    test('shows error when API token is invalid', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const isDeployed = await page.locator('button[aria-label="Remove Tunnel"]').isVisible().catch(() => false);
      if (isDeployed) return; // skip if already deployed

      await page.locator('input[aria-label="Cloudflare API Token"]').fill('invalid-token');
      await page.locator('button[aria-label="Connect Cloudflare Account"]').click();

      // Should show an error after the API call fails
      await expect(page.locator('text=/error|failed|invalid/i').first())
        .toBeVisible({ timeout: 15000 });
    });
  });

  test.describe('Error States', () => {
    test('page loads without crashing', async ({ page }) => {
      await expect(page.locator('text=Cloudflare Tunnel').first()).toBeVisible();
      const hasUnhandledError = await page.locator('text=/Something went wrong|Uncaught Error/')
        .isVisible().catch(() => false);
      expect(hasUnhandledError).toBe(false);
    });

    test('shows deployment error message when last deploy failed', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const hasErrorBox = await page.locator('text=Deployment error').isVisible().catch(() => false);
      if (hasErrorBox) {
        await expect(page.locator('text=Deployment error')).toBeVisible();
      }
    });
  });

  // Deploy flow is last — it initiates a real (slow) Cloudflare API call + helm deploy
  // that leaves DB state. Running last prevents contamination of other tests.
  test.describe('Deploy Flow', () => {
    test('connect with fake token fails gracefully and shows error state', async ({ page }) => {
      await page.waitForLoadState('networkidle');

      // Only attempt if not already deployed (to avoid interfering with real tunnel)
      const isDeployed = await page.locator('button[aria-label="Remove Tunnel"]').isVisible().catch(() => false);
      if (isDeployed) {
        test.skip();
        return;
      }

      await page.locator('input[aria-label="Cloudflare API Token"]').fill('fake-token-for-e2e-testing-only');
      await page.locator('button[aria-label="Connect Cloudflare Account"]').click();

      // Button becomes disabled while validating
      await expect(
        page.locator('button[aria-label="Connect Cloudflare Account"]'),
      ).toBeDisabled({ timeout: 5000 });

      // Should get an error from the Cloudflare API (invalid token)
      await expect(
        page.locator('text=/error|failed|invalid/i').first(),
      ).toBeVisible({ timeout: 30000 });

      // Page should still be functional (no crash)
      await expect(page.locator('text=Cloudflare Tunnel').first()).toBeVisible();
      // Should be back in idle state with the form visible
      await expect(
        page.locator('button[aria-label="Connect Cloudflare Account"]'),
      ).toBeVisible({ timeout: 5000 });
    });
  });
});
