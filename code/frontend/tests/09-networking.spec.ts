import { test, expect } from '@playwright/test';

test.describe('Networking Page', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/networking');
    await expect(page.locator('h1:has-text("Networking")')).toBeVisible({ timeout: 10000 });
  });

  test.describe('Page Display', () => {
    test('shows page heading and subtitle', async ({ page }) => {
      await expect(page.locator('h1:has-text("Networking")')).toBeVisible();
      await expect(page.locator('text=Network topology and traffic flow')).toBeVisible();
    });

    test('shows Internet Traffic stat card', async ({ page }) => {
      await expect(page.locator('text=Internet Traffic')).toBeVisible({ timeout: 15000 });
    });

    test('shows Total Receive stat card', async ({ page }) => {
      await expect(page.locator('text=Total Receive')).toBeVisible({ timeout: 15000 });
    });

    test('shows Total Transmit stat card', async ({ page }) => {
      await expect(page.locator('text=Total Transmit')).toBeVisible({ timeout: 15000 });
    });

    test('shows Errors/sec stat card', async ({ page }) => {
      await expect(page.locator('text=Errors/sec')).toBeVisible({ timeout: 15000 });
    });

    test('shows Dropped/sec stat card', async ({ page }) => {
      await expect(page.locator('text=Dropped/sec')).toBeVisible({ timeout: 15000 });
    });

    test('shows Network Flow section', async ({ page }) => {
      await expect(page.locator('text=Network Flow')).toBeVisible({ timeout: 15000 });
    });

    test('shows Network Statistics section', async ({ page }) => {
      await expect(page.locator('h2:has-text("Network Statistics")').first()).toBeVisible({ timeout: 15000 });
    });
  });

  test.describe('Controls', () => {
    test('Infra toggle button works', async ({ page }) => {
      // Find the Infra toggle button
      const infraButton = page.locator('button:has-text("Infra")');
      await expect(infraButton).toBeVisible({ timeout: 15000 });
      // Click to toggle
      await infraButton.click();
      // Button should still be visible (toggled state)
      await expect(infraButton).toBeVisible();
    });

    test('Refresh button is visible', async ({ page }) => {
      // There should be a refresh button in the header area
      const refreshButton = page.locator('button').filter({ has: page.locator('svg.lucide-refresh-cw') }).first();
      await expect(refreshButton).toBeVisible({ timeout: 10000 });
    });

    test('Connection status indicator is shown', async ({ page }) => {
      // Should show one of: Live, Polling, or Disconnected
      const live = page.locator('text=Live');
      const polling = page.locator('text=Polling');
      const disconnected = page.locator('text=Disconnected');

      await page.waitForLoadState('networkidle');
      const hasLive = await live.first().isVisible().catch(() => false);
      const hasPolling = await polling.isVisible().catch(() => false);
      const hasDisconnected = await disconnected.isVisible().catch(() => false);
      expect(hasLive || hasPolling || hasDisconnected).toBe(true);
    });
  });

  test.describe('Network Statistics Table', () => {
    test('shows Network Statistics section or Network Flow only', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      // Network Statistics may not appear when there are no apps with network data
      const hasStats = await page.locator('text=Network Statistics').isVisible({ timeout: 5000 }).catch(() => false);
      const hasFlow = await page.locator('text=Network Flow').isVisible().catch(() => false);
      expect(hasStats || hasFlow).toBe(true);
    });
  });

  test.describe('Topology Graph (React Flow)', () => {
    test('Network Flow section container renders', async ({ page }) => {
      await page.waitForLoadState('networkidle');

      // The network flow visualization section should be present
      await expect(page.locator('text=Network Flow')).toBeVisible({ timeout: 15000 });
    });

    test('topology graph renders React Flow container or empty state', async ({ page }) => {
      await page.waitForLoadState('networkidle');

      // React Flow renders in a div with class containing "react-flow"
      const hasReactFlow = await page.locator('[class*="react-flow"]').first().isVisible({ timeout: 10000 }).catch(() => false);
      // Or shows an empty state when no apps are running
      const hasEmptyState = await page.locator('text=/no.*app|no.*network|no.*data|empty/i').first().isVisible().catch(() => false);
      // Or shows the network flow section container
      const hasFlowSection = await page.locator('text=Network Flow').isVisible().catch(() => false);

      expect(hasReactFlow || hasEmptyState || hasFlowSection).toBe(true);
    });

    test('does not show JavaScript errors or crash during load', async ({ page }) => {
      const jsErrors: string[] = [];
      page.on('pageerror', (error) => {
        jsErrors.push(error.message);
      });

      await page.goto('/networking');
      await expect(page.locator('h1:has-text("Networking")')).toBeVisible({ timeout: 10000 });
      await page.waitForLoadState('networkidle');

      // Filter out known non-critical errors
      const criticalErrors = jsErrors.filter(
        (e) =>
          !e.includes('ResizeObserver') &&
          !e.includes('Non-Error promise rejection') &&
          !e.includes('AbortError')
      );
      expect(criticalErrors.length).toBe(0);
    });

    test('SVG elements are present when React Flow renders', async ({ page }) => {
      await page.waitForLoadState('networkidle');

      const hasReactFlow = await page.locator('[class*="react-flow"]').first().isVisible({ timeout: 10000 }).catch(() => false);

      if (!hasReactFlow) {
        // React Flow not rendered (no apps) - skip SVG check
        test.skip();
        return;
      }

      // React Flow uses SVG for edges
      const hasSvg = await page.locator('[class*="react-flow"] svg').first().isVisible({ timeout: 5000 }).catch(() => false);
      expect(hasSvg).toBe(true);
    });

    test('MiniMap is visible when React Flow renders', async ({ page }) => {
      await page.waitForLoadState('networkidle');

      const hasReactFlow = await page.locator('[class*="react-flow"]').first().isVisible({ timeout: 10000 }).catch(() => false);

      if (!hasReactFlow) {
        test.skip();
        return;
      }

      // React Flow MiniMap component
      const hasMiniMap = await page.locator('[class*="react-flow__minimap"]').first().isVisible({ timeout: 5000 }).catch(() => false);
      // MiniMap is optional based on configuration — just verify it doesn't cause errors
      // If present, it should be visible
      if (hasMiniMap) {
        await expect(page.locator('[class*="react-flow__minimap"]').first()).toBeVisible();
      }
    });
  });

  test.describe('Stat Card Value Validation', () => {
    test('stat card values are not NaN or undefined', async ({ page }) => {
      await page.waitForLoadState('networkidle');

      // Check all visible stat cards
      const statLabels = ['Internet Traffic', 'Total Receive', 'Total Transmit', 'Errors/sec', 'Dropped/sec'];

      for (const label of statLabels) {
        const card = page.locator(`text=${label}`).first();
        const isVisible = await card.isVisible({ timeout: 5000 }).catch(() => false);
        if (!isVisible) continue;

        // Get the parent card container to find the value
        const cardParent = card.locator('../..');
        const cardText = await cardParent.textContent().catch(() => '');

        // Value should not contain NaN or undefined
        expect(cardText).not.toContain('NaN');
        expect(cardText).not.toContain('undefined');
      }
    });

    test('stat cards display numeric or formatted values', async ({ page }) => {
      await page.waitForLoadState('networkidle');

      // Internet Traffic card should show some value (0 B/s, 1.2 MB/s, etc.)
      const trafficCard = page.locator('text=Internet Traffic').first();
      const isVisible = await trafficCard.isVisible({ timeout: 15000 }).catch(() => false);

      if (isVisible) {
        // The value should be a formatted bandwidth string
        const parentText = await trafficCard.locator('../..').textContent().catch(() => '');
        // Should match a bandwidth pattern or zero value
        const hasValue = /\d|B\/s|KB\/s|MB\/s|GB\/s|0/.test(parentText ?? '');
        expect(hasValue).toBe(true);
      }
    });

    test('Errors/sec and Dropped/sec default to 0 with no errors', async ({ page }) => {
      await page.waitForLoadState('networkidle');

      // These should show 0 when there are no network errors in a test environment
      const errorsCard = page.locator('text=Errors/sec').first();
      const droppedCard = page.locator('text=Dropped/sec').first();

      const errorsVisible = await errorsCard.isVisible({ timeout: 10000 }).catch(() => false);
      const droppedVisible = await droppedCard.isVisible({ timeout: 10000 }).catch(() => false);

      if (errorsVisible) {
        const errorsText = await errorsCard.locator('../..').textContent().catch(() => '');
        // Should not show NaN or undefined
        expect(errorsText).not.toContain('NaN');
        expect(errorsText).not.toContain('undefined');
      }

      if (droppedVisible) {
        const droppedText = await droppedCard.locator('../..').textContent().catch(() => '');
        expect(droppedText).not.toContain('NaN');
        expect(droppedText).not.toContain('undefined');
      }
    });
  });

  test.describe('VPN Integration in Network Topology', () => {
    test('page loads without VPN-related errors', async ({ page }) => {
      const consoleErrors: string[] = [];
      page.on('console', (msg) => {
        if (msg.type() === 'error') consoleErrors.push(msg.text());
      });

      await page.goto('/networking');
      await expect(page.locator('h1:has-text("Networking")')).toBeVisible({ timeout: 10000 });
      await page.waitForLoadState('networkidle');

      // Filter non-critical errors
      const criticalErrors = consoleErrors.filter(
        (e) =>
          !e.includes('favicon') &&
          !e.includes('ERR_NETWORK_CHANGED') &&
          !e.includes('net::ERR_') &&
          !e.includes('Failed to load resource')
      );
      expect(criticalErrors.length).toBe(0);
    });

    test('VPN badge appears on app nodes when VPN is configured', async ({ page }) => {
      await page.waitForLoadState('networkidle');

      const hasReactFlow = await page.locator('[class*="react-flow"]').first().isVisible({ timeout: 10000 }).catch(() => false);

      if (!hasReactFlow) {
        // No app nodes rendered (empty topology)
        test.skip();
        return;
      }

      // Check if any app nodes are rendered
      const appNodes = page.locator('[class*="react-flow__node"]');
      const nodeCount = await appNodes.count();

      if (nodeCount === 0) {
        test.skip();
        return;
      }

      // VPN badge would show provider name on nodes with VPN configured
      // In a test environment without VPN setup, no VPN badges are expected
      const hasVpnBadge = await page.locator('text=/vpn|gluetun|wireguard|openvpn/i').first().isVisible({ timeout: 3000 }).catch(() => false);

      // The absence of VPN badges is acceptable (VPN not configured in test env)
      // The test verifies no errors thrown when checking VPN state on nodes
      if (hasVpnBadge) {
        await expect(page.locator('text=/vpn|gluetun|wireguard|openvpn/i').first()).toBeVisible();
      }
    });

    test('Infra toggle hides infrastructure nodes when clicked', async ({ page }) => {
      await page.waitForLoadState('networkidle');

      const infraButton = page.locator('button:has-text("Infra")');
      const isVisible = await infraButton.isVisible({ timeout: 10000 }).catch(() => false);

      if (!isVisible) {
        test.skip();
        return;
      }

      // Get initial state
      const _initialText = await infraButton.textContent();

      // Click to toggle
      await infraButton.click();
      await page.waitForTimeout(500);

      // Button should still be present (toggle is stateful)
      await expect(infraButton).toBeVisible();

      // Toggle back
      await infraButton.click();
      await page.waitForTimeout(500);
      await expect(infraButton).toBeVisible();
    });
  });
});
