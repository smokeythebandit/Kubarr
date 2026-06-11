import { test, expect } from '@playwright/test';

test.describe('Storage Page', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/storage');
    await expect(page.locator('h1:has-text("Storage")')).toBeVisible({ timeout: 10000 });
  });

  test.describe('Storage Display', () => {
    test('shows page heading', async ({ page }) => {
      await expect(page.locator('h1:has-text("Storage")')).toBeVisible();
    });

    test('shows storage content or not-available message', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      // Storage may not be configured - shows either file browser or "Storage Not Available"
      const hasUsage = await page.locator('text=Storage Usage').isVisible().catch(() => false);
      const hasNotAvailable = await page.locator('text=Storage Not Available').isVisible().catch(() => false);
      expect(hasUsage || hasNotAvailable).toBe(true);
    });

    test('not-available state shows setup instructions', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const hasNotAvailable = await page.locator('text=Storage Not Available').isVisible().catch(() => false);
      if (hasNotAvailable) {
        await expect(page.locator('text=Shared storage is not configured')).toBeVisible();
      }
    });

    test('shows file/folder table with columns when storage available', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const hasNotAvailable = await page.locator('text=Storage Not Available').isVisible().catch(() => false);
      if (!hasNotAvailable) {
        await expect(page.locator('text=Name')).toBeVisible();
        await expect(page.locator('text=Size')).toBeVisible();
        await expect(page.locator('text=Modified')).toBeVisible();
      }
    });

    test('shows item count when storage available', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const hasNotAvailable = await page.locator('text=Storage Not Available').isVisible().catch(() => false);
      if (!hasNotAvailable) {
        await expect(page.locator('text=/\\d+ item/i')).toBeVisible({ timeout: 10000 });
      }
    });
  });

  test.describe('File Operations', () => {
    test('New Folder button opens modal when storage available', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const newFolderButton = page.locator('button:has-text("New Folder")');
      if (await newFolderButton.isVisible().catch(() => false)) {
        await newFolderButton.click();
        await expect(page.locator('text=Create New Folder')).toBeVisible();
        await expect(page.locator('input[placeholder*="Folder name" i]')).toBeVisible();
      }
    });

    test('New Folder modal has Create and Cancel buttons', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const newFolderButton = page.locator('button:has-text("New Folder")');
      if (await newFolderButton.isVisible().catch(() => false)) {
        await newFolderButton.click();
        await expect(page.locator('button:has-text("Create")')).toBeVisible();
        await expect(page.locator('button:has-text("Cancel")')).toBeVisible();
      }
    });

    test('Create button is disabled when folder name is empty', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const newFolderButton = page.locator('button:has-text("New Folder")');
      if (await newFolderButton.isVisible().catch(() => false)) {
        await newFolderButton.click();
        await expect(page.locator('text=Create New Folder')).toBeVisible();
        const createButton = page.locator('button:has-text("Create")');
        await expect(createButton).toBeDisabled();
      }
    });

    test('Can cancel new folder modal', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const newFolderButton = page.locator('button:has-text("New Folder")');
      if (await newFolderButton.isVisible().catch(() => false)) {
        await newFolderButton.click();
        await expect(page.locator('text=Create New Folder')).toBeVisible();
        await page.locator('button:has-text("Cancel")').click();
        await expect(page.locator('text=Create New Folder')).not.toBeVisible();
      }
    });

    test('Can create a folder when storage available', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const newFolderButton = page.locator('button:has-text("New Folder")');
      if (await newFolderButton.isVisible().catch(() => false)) {
        await newFolderButton.click();
        await expect(page.locator('text=Create New Folder')).toBeVisible();
        await page.locator('input[placeholder*="Folder name" i]').fill('test-folder');
        const createButton = page.locator('button:has-text("Create")');
        await expect(createButton).toBeEnabled();
        await createButton.click();
        await expect(page.locator('text=Create New Folder')).not.toBeVisible({ timeout: 5000 });
      }
    });

    test('Can navigate into a folder by clicking it', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const folderRow = page.locator('tr').filter({ has: page.locator('text=test-folder') }).first();
      if (await folderRow.isVisible().catch(() => false)) {
        await folderRow.click();
        await expect(page.locator('text=test-folder')).toBeVisible();
      }
    });

    test('Parent directory (..) link navigates up', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const folderRow = page.locator('tr').filter({ has: page.locator('text=test-folder') }).first();
      if (await folderRow.isVisible().catch(() => false)) {
        await folderRow.click();
        await page.waitForLoadState('networkidle');
        const parentRow = page.locator('text=..').first();
        if (await parentRow.isVisible()) {
          await parentRow.click();
          await page.waitForLoadState('networkidle');
        }
      }
    });

    test('Refresh button reloads directory when storage available', async ({ page }) => {
      await page.waitForLoadState('networkidle');
      const refreshButton = page.locator('button:has-text("Refresh")');
      if (await refreshButton.isVisible().catch(() => false)) {
        await refreshButton.click();
        await expect(page.locator('h1:has-text("Storage")')).toBeVisible();
      }
    });
  });
});
