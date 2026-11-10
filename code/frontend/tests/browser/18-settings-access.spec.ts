import { test, expect, admin, timestamp } from './fixtures';
import type { Invite } from '../../src/api/users';

test('registration and approval toggles send exact mutations and survive reload', async ({ page, api }) => {
  api.get('/api/oauth/providers', []);
  const settings: Record<string, { key: string; value: string; description: string }> = Object.fromEntries(
    ['registration_enabled', 'registration_require_approval'].map(key => [key, { key, value: 'true', description: key }]),
  );
  api.on('GET', '/api/settings', () => ({ json: { settings } }));
  for (const key of Object.keys(settings)) {
    api.on('PUT', `/api/settings/${key}`, request => {
      expect(request.postDataJSON()).toEqual({ value: 'false' });
      settings[key] = { ...settings[key], value: 'false' };
      return { json: settings[key] };
    });
  }
  await page.goto('/settings?section=general');
  for (const label of ['Allow Open Registration', 'Require Admin Approval']) {
    const toggle = page.getByRole('switch', { name: label, exact: true });
    await expect(toggle).toBeChecked();
    await toggle.click();
    await expect(toggle).not.toBeChecked();
  }
  await page.reload();
  await expect(page.getByRole('switch', { name: 'Allow Open Registration' })).not.toBeChecked();
  await expect(page.getByRole('switch', { name: 'Require Admin Approval' })).not.toBeChecked();
  expect(api.calls.filter(call => call.startsWith('PUT '))).toEqual([
    'PUT /api/settings/registration_enabled', 'PUT /api/settings/registration_require_approval',
  ]);
});

const asRole = (permissions: string[]) => ({
  ...admin, roles: [{ id: 5, name: 'helpdesk' }], permissions, allowed_apps: [],
});

test('a user with only users.view sees user sections read-only and nothing else', async ({ page, api }) => {
  api.get('/api/users/me', asRole(['users.view']));
  await page.goto('/settings?section=general');
  // general needs settings.view, so the page falls back to the first allowed section
  await expect(page.getByRole('heading', { name: 'Dashboard' }).first()).toBeVisible();
  const nav = page.locator('nav').filter({ hasText: 'Access Management' });
  for (const label of ['Dashboard', 'All Users', 'Pending Approval']) {
    await expect(nav.getByRole('button', { name: label })).toBeVisible();
  }
  for (const label of ['Notifications', 'Audit Logs', 'Domains', 'VPN', 'General', 'Invite Links', 'Permissions']) {
    await expect(nav.getByRole('button', { name: label, exact: true })).toHaveCount(0);
  }
  await expect(page.getByRole('button', { name: 'Create Invite' })).toHaveCount(0);
  await expect(page.getByText('Active Invites')).toHaveCount(0);
  await nav.getByRole('button', { name: 'All Users' }).click();
  await expect(page.getByRole('button', { name: 'Create New User' })).toHaveCount(0);
  await expect(page.getByRole('columnheader', { name: 'Actions' })).toHaveCount(0);
});

test('settings.view without settings.manage shows general settings but locks every control', async ({ page, api }) => {
  api.get('/api/users/me', asRole(['settings.view']));
  api.get('/api/oauth/providers', [{ id: 'google', name: 'Google', enabled: false, client_id: 'abc', has_secret: true }]);
  api.get('/api/settings', { settings: { registration_enabled: { key: 'registration_enabled', value: 'true', description: '' } } });
  await page.goto('/settings?section=general');
  await expect(page.getByRole('switch', { name: 'Allow Open Registration' })).toBeDisabled();
  await expect(page.getByRole('switch', { name: 'Google OAuth' })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Configure' })).toHaveCount(0);
  for (const path of ['/api/domains/assignments', '/api/domains/ddns-profiles', '/api/domains/letsencrypt-profiles']) api.get(path, []);
  api.get('/api/domains', [{ id: 1, domain: 'example.test', kind: 'root', scope: 'public', primary: true, enabled: true,
    dns_mode: 'manual', ddns_profile_id: null, dns_status: 'ok', tls_mode: 'none', letsencrypt_profile_id: null,
    tls_secret_name: null, certificate_status: 'none', certificate_expires_at: null, created_at: timestamp, updated_at: timestamp }]);
  await page.locator('nav').filter({ hasText: 'Networking' }).getByRole('button', { name: 'Domains' }).click();
  await expect(page.getByText('example.test')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Add Domain' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Assign App URL' })).toHaveCount(0);
});

test('an invite can grant a role and the list shows it', async ({ page, api }) => {
  let invites: Invite[] = [];
  api.get('/api/roles', [{ id: 2, name: 'viewer', description: null, is_system: true, requires_2fa: false, created_at: timestamp, app_names: [], permissions: [] }]);
  api.on('GET', '/api/users/invites', () => ({ json: invites }));
  api.on('POST', '/api/users/invites', request => {
    expect(request.postDataJSON()).toEqual({ expires_in_days: 7, role_id: 2 });
    const created: Invite = { id: 7, code: 'role-code', created_by_username: admin.username, used_by_username: null,
      role_id: 2, role_name: 'viewer', is_used: false, expires_at: null, created_at: timestamp, used_at: null };
    invites = [created];
    return { json: created };
  });
  await page.goto('/settings?section=invites');
  await page.getByRole('combobox', { name: 'Role for invited user' }).selectOption('2');
  await page.getByRole('button', { name: 'Create Invite' }).click();
  await expect(page.getByText('Role:')).toBeVisible();
  await page.getByRole('button', { name: 'Done' }).click();
  await expect(page.getByRole('row').filter({ hasText: 'viewer' })).toBeVisible();
});
