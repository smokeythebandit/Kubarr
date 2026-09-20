import { test, expect, admin, timestamp } from './fixtures';
import type { DirectoryListing, FileInfo, StorageUsage } from '../../src/api/storage';

const folder: FileInfo = { name: 'config', path: 'config', type: 'directory', size: 0, modified: timestamp, permissions: '755' };
const file: FileInfo = { name: 'settings.json', path: 'config/settings.json', type: 'file', size: 2, modified: timestamp, permissions: '644' };
const listing = (path: string, items: FileInfo[]): DirectoryListing => ({ path, parent: path ? '' : null, items, total_items: items.length });

test.beforeEach(async ({ api }) => {
  api.get('/api/storage/browse?path=', listing('', [folder]));
  api.get('/api/storage/browse?path=config', listing('config', [file]));
});

test('browses folders, parent rows, breadcrumbs, and thumbnail view', async ({ page, api }) => {
  await page.goto('/storage');
  await expect(page.getByText('1 folder · 0 files', { exact: true })).toBeVisible();
  await page.getByRole('row').filter({ hasText: 'config' }).click();
  await expect(page.getByText('0 folders · 1 file', { exact: true })).toBeVisible();
  await expect(page.getByRole('row').filter({ hasText: 'settings.json' })).toBeVisible();
  await page.getByRole('row').filter({ hasText: 'Parent directory' }).click();
  await expect(page.getByText('1 folder · 0 files', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Thumbnails', exact: true }).click();
  await page.getByRole('button', { name: 'config Folder', exact: true }).click();
  await expect(page.getByRole('button', { name: 'config', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Root', exact: true }).click();
  await expect(page.getByRole('button', { name: 'config Folder', exact: true })).toBeVisible();
  expect(api.calls).not.toContain('GET /api/storage/usage');
});

test('creates a folder in the current path and refreshes its listing', async ({ page, api }) => {
  let items = [file];
  api.on('GET', '/api/storage/browse?path=config', () => ({ json: listing('config', items) }));
  api.on('POST', '/api/storage/mkdir', request => {
    expect(request.postDataJSON()).toEqual({ path: 'config/new folder' });
    items = [...items, { ...folder, name: 'new folder', path: 'config/new folder' }];
    return { json: { success: true, message: 'Created' } };
  });
  await page.goto('/storage');
  await page.getByRole('row').filter({ hasText: 'config' }).click();
  await page.getByRole('button', { name: 'New Folder', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Create', exact: true })).toBeDisabled();
  await page.getByPlaceholder('Folder name').fill('new folder');
  await page.getByRole('button', { name: 'Create', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Create New Folder' })).toHaveCount(0);
  await expect(page.getByRole('row').filter({ hasText: 'new folder' })).toBeVisible();
  await expect(page.getByText('1 folder · 1 file', { exact: true })).toBeVisible();
  expect(api.calls.filter(call => call === 'POST /api/storage/mkdir')).toHaveLength(1);
});

test('cancel and rejected folder creation leave inventory unchanged', async ({ page, api }) => {
  api.on('POST', '/api/storage/mkdir', () => ({ status: 409, json: { detail: 'Already exists' } }));
  await page.goto('/storage');
  await page.getByRole('button', { name: 'New Folder', exact: true }).click();
  await page.getByPlaceholder('Folder name').fill('cancelled');
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  expect(api.calls).not.toContain('POST /api/storage/mkdir');
  await page.getByRole('button', { name: 'New Folder', exact: true }).click();
  await expect(page.getByPlaceholder('Folder name')).toHaveValue('');
  await page.getByPlaceholder('Folder name').fill('config');
  await page.getByRole('button', { name: 'Create', exact: true }).click();
  await expect(page.getByText('Request failed with status code 409', { exact: true })).toBeVisible();
  await expect(page.getByPlaceholder('Folder name')).toHaveValue('config');
  await expect(page.getByText('1 folder · 0 files', { exact: true })).toBeVisible();
});

test('rename and confirmed delete update only the selected file', async ({ page, api }) => {
  let items = [file];
  api.on('GET', '/api/storage/browse?path=config', () => ({ json: listing('config', items) }));
  api.on('POST', '/api/storage/rename', request => {
    expect(request.postDataJSON()).toEqual({ path: file.path, new_name: 'renamed.json' });
    items = [{ ...file, name: 'renamed.json', path: 'config/renamed.json' }];
    return { json: items[0] };
  });
  api.on('DELETE', '/api/storage/delete?path=config%2Frenamed.json', () => {
    items = [];
    return { json: { success: true, message: 'Deleted' } };
  });
  await page.goto('/storage');
  await page.getByRole('row').filter({ hasText: 'config' }).click();
  await page.getByRole('button', { name: 'Rename settings.json', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Rename', exact: true })).toBeDisabled();
  await page.getByRole('textbox').fill('renamed.json');
  await page.getByRole('button', { name: 'Rename', exact: true }).click();
  await expect(page.getByRole('row').filter({ hasText: 'settings.json' })).toHaveCount(0);
  await page.getByRole('button', { name: 'Delete renamed.json', exact: true }).click();
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  expect(api.calls.filter(call => call.startsWith('DELETE'))).toEqual([]);
  await page.getByRole('button', { name: 'Delete renamed.json', exact: true }).click();
  await page.getByRole('button', { name: 'Delete', exact: true }).click();
  await expect(page.getByText('This folder is empty', { exact: true })).toBeVisible();
  expect(api.calls.filter(call => call.startsWith('DELETE'))).toHaveLength(1);
});

for (const fails of [false, true]) {
  test(`text editor ${fails ? 'retains edits after save failure' : 'saves exact content and reloads it'}`, async ({ page, api }) => {
    let content = '{}';
    api.on('GET', '/api/storage/text?path=config%2Fsettings.json', () => ({ json: { path: file.path, content, size: content.length, modified: timestamp } }));
    api.on('PUT', '/api/storage/text', request => {
      expect(request.postDataJSON()).toEqual({ path: file.path, content: '{"enabled":true}' });
      if (fails) return { status: 500, json: { detail: 'Write failed' } };
      content = request.postDataJSON().content;
      return { json: { path: file.path, content, size: content.length, modified: timestamp } };
    });
    await page.goto('/storage');
    await page.getByRole('row').filter({ hasText: 'config' }).click();
    await page.getByRole('row').filter({ hasText: file.name }).click();
    await expect(page.getByRole('textbox')).toHaveValue('{}');
    await page.getByRole('textbox').fill('{"enabled":true}');
    await page.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(page.getByText(fails ? 'Request failed with status code 500' : 'Saved to NFS storage.', { exact: true })).toBeVisible();
    await expect(page.getByRole('textbox')).toHaveValue('{"enabled":true}');
    if (!fails) {
      await page.reload();
      await page.getByRole('row').filter({ hasText: 'config' }).click();
      await page.getByRole('row').filter({ hasText: file.name }).click();
      await expect(page.getByRole('textbox')).toHaveValue('{"enabled":true}');
    }
    expect(api.calls.filter(call => call === 'PUT /api/storage/text')).toHaveLength(1);
  });
}

test('viewer can read text but cannot create, rename, delete, or save', async ({ page, api }) => {
  api.get('/api/users/me', { ...admin, roles: [{ id: 2, name: 'viewer' }], permissions: ['storage.view'], allowed_apps: [] });
  api.get('/api/storage/text?path=config%2Fsettings.json', { path: file.path, content: '{}', size: 2, modified: timestamp });
  await page.goto('/storage');
  await page.getByRole('row').filter({ hasText: 'config' }).click();
  await page.getByRole('row').filter({ hasText: file.name }).click();
  await expect(page.getByRole('textbox')).toHaveValue('{}');
  await expect(page.getByRole('textbox')).toHaveAttribute('readonly', '');
  for (const name of ['New Folder', 'Save', 'Rename settings.json', 'Delete settings.json']) {
    await expect(page.getByRole('button', { name, exact: true })).toHaveCount(0);
  }
});

test('statistics is lazy-loaded and refresh replaces scan warnings', async ({ page, api }) => {
  const usage: StorageUsage = {
    root: { name: 'Root', path: '', type: 'directory', size: 2, children: [
      { name: 'config', path: 'config', type: 'directory', size: 2, children: [
        { name: file.name, path: file.path, type: 'file', size: 2, children: [] },
      ] },
    ] }, total_size: 2, total_files: 1, total_directories: 2, warnings: ['Fixture scan warning'],
  };
  api.get('/api/storage/usage', usage);
  await page.goto('/storage');
  await expect(page.getByText('1 folder · 0 files', { exact: true })).toBeVisible();
  expect(api.calls).not.toContain('GET /api/storage/usage');
  await page.getByRole('button', { name: 'Statistics', exact: true }).click();
  await expect(page.getByText('Scan warnings: Fixture scan warning', { exact: true })).toBeVisible();
  api.get('/api/storage/usage', { ...usage, warnings: ['Updated scan'] });
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByText('Scan warnings: Updated scan', { exact: true })).toBeVisible();
  await expect(page.getByText('Scan warnings: Fixture scan warning', { exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Browser', exact: true }).click();
  await expect(page.getByRole('row').filter({ hasText: 'config' })).toBeVisible();
});

test('browser refresh fetches changed inventory', async ({ page, api }) => {
  await page.goto('/storage');
  await expect(page.getByRole('row').filter({ hasText: 'config' })).toBeVisible();
  api.get('/api/storage/browse?path=', listing('', []));
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByText('This folder is empty', { exact: true })).toBeVisible();
  await expect(page.getByText('0 folders · 0 files', { exact: true })).toBeVisible();
  await expect(page.getByRole('row').filter({ hasText: 'config' })).toHaveCount(0);
});

test('failed browse displays offline instructions instead of an empty successful listing', async ({ page, api }) => {
  api.on('GET', '/api/storage/browse?path=', () => ({ status: 503, json: { detail: 'Storage unavailable' } }));
  await page.goto('/storage');
  await expect(page.getByRole('heading', { name: 'NFS Browser Offline' })).toBeVisible();
  await expect(page.getByText('Configure managed NFS or external NFS', { exact: true })).toBeVisible();
  await expect(page.getByText('This folder is empty', { exact: true })).toHaveCount(0);
});
