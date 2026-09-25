import { test, expect, admin, timestamp } from './fixtures';
import type { User, Invite } from '../../src/api/users';
import type { Role } from '../../src/api/roles';

const pending = (id: number, username: string): User => ({ ...admin, id, username,
  email: `${username}@example.test`, roles: [{ id: 2, name: 'viewer', description: 'Read only' }], is_approved: false });
const viewer: Role = { id: 2, name: 'viewer', description: 'Read only', is_system: true,
  requires_2fa: false, created_at: timestamp, permissions: [], app_names: [] };
const role: Role = { ...viewer, id: 3, name: 'limited', is_system: false };

test('pending approval and reject only mutate selected accounts', async ({ page, api }) => {
  let people = [pending(201, 'awaiting-approval'), pending(202, 'awaiting-rejection')];
  api.on('GET', '/api/users/pending', () => ({ json: people }));
  api.on('GET', '/api/users', () => ({ json: [admin, ...people] }));
  api.get('/api/roles', [viewer]);
  api.on('POST', '/api/users/201/approve', () => {
    people = people.filter(user => user.id !== 201); return { json: { message: 'approved' } };
  });
  api.on('POST', '/api/users/202/reject', () => {
    people = people.filter(user => user.id !== 202); return { json: { message: 'rejected' } };
  });
  await page.goto('/settings?section=pending');
  const first = page.getByRole('row').filter({ hasText: 'awaiting-approval' });
  await first.getByRole('button', { name: 'Approve' }).click();
  await expect(first).toHaveCount(0);
  const second = page.getByRole('row').filter({ hasText: 'awaiting-rejection' });
  page.once('dialog', dialog => dialog.dismiss());
  await second.getByRole('button', { name: 'Reject' }).click();
  await expect(second).toBeVisible();
  page.once('dialog', dialog => dialog.accept());
  await second.getByRole('button', { name: 'Reject' }).click();
  await expect(second).toHaveCount(0);
  expect(api.calls.filter(call => call.startsWith('POST '))).toEqual([
    'POST /api/users/201/approve', 'POST /api/users/202/reject',
  ]);
});

test('invite create and delete never exposes code in inventory', async ({ page, api }) => {
  let invites: Invite[] = [];
  const invitation: Invite = { id: 301, code: 'fixture-invite-code', created_by_username: admin.username,
    used_by_username: null, is_used: false, expires_at: null, created_at: timestamp, used_at: null };
  api.on('GET', '/api/users/invites', () => ({ json: invites }));
  api.on('POST', '/api/users/invites', request => {
    expect(request.postDataJSON()).toEqual({ expires_in_days: 7 });
    invites = [invitation]; return { json: invitation };
  });
  api.on('DELETE', '/api/users/invites/301', () => { invites = []; return { json: { message: 'deleted' } }; });
  await page.goto('/settings?section=invites');
  await page.getByRole('button', { name: 'Create Invite' }).click();
  await expect(page.locator('input[readonly]')).toHaveValue(/\/login\?register=1&invite=fixture-invite-code$/);
  await page.getByRole('button', { name: 'Close', exact: true }).click();
  await expect(page.locator('input[readonly]')).toHaveCount(0);
  page.once('dialog', dialog => dialog.accept());
  await page.getByRole('button', { name: 'Delete' }).click();
  await expect(page.getByText('No invites created yet. Create one to get started.')).toBeVisible();
  expect(api.calls.filter(call => !call.startsWith('GET '))).toEqual([
    'POST /api/users/invites', 'DELETE /api/users/invites/301',
  ]);
});

test('permission matrix locks admin and persists limited role changes through reload', async ({ page, api }) => {
  const privileged = { ...role, permissions: [] as string[] };
  api.on('GET', '/api/roles', () => ({ json: [{ ...role, id: 1, name: 'admin', is_system: true }, privileged] }));
  api.get('/api/roles/permissions', [{ key: 'audit.view', category: 'Settings', description: 'Read audit' }]);
  api.get('/api/apps/installed', []);
  api.on('PUT', '/api/roles/3/permissions', request => {
    expect(request.postDataJSON()).toEqual({ permissions: ['audit.view'] });
    privileged.permissions = ['audit.view']; return { json: privileged };
  });
  await page.goto('/settings?section=permissions');
  const row = page.getByRole('row').filter({ has: page.getByText('audit.view', { exact: true }) });
  await expect(row.getByRole('checkbox').first()).toBeDisabled();
  await row.getByRole('checkbox').nth(1).check();
  await page.getByRole('columnheader').filter({ hasText: 'limited' }).getByRole('button', { name: 'Save' }).click();
  await page.reload();
  await expect(row.getByRole('checkbox').nth(1)).toBeChecked();
  expect(api.calls.filter(call => call.startsWith('PUT '))).toEqual(['PUT /api/roles/3/permissions']);
});

test('admin creates, assigns a role, edits and deletes only the selected user', async ({ page, api }) => {
  let users: User[] = [admin];
  api.on('GET', '/api/users', () => ({ json: users }));
  api.get('/api/roles', [viewer, role]);
  api.on('POST', '/api/users', request => {
    expect(request.postDataJSON()).toEqual({ username: 'fixture-member', email: 'member@example.test',
      password: 'test-password', role_ids: [3] });
    const created = { ...admin, id: 401, username: 'fixture-member', email: 'member@example.test', roles: [role] };
    users = [...users, created];
    return { json: created };
  });
  api.on('PATCH', '/api/users/401', request => {
    expect(request.postDataJSON()).toEqual({ role_ids: [2] });
    users = users.map(user => user.id === 401 ? { ...user, roles: [viewer] } : user);
    return { json: users[1] };
  });
  api.on('DELETE', '/api/users/401', () => { users = [admin]; return { json: { message: 'deleted' } }; });
  await page.goto('/settings?section=users');
  await page.getByRole('button', { name: 'Create New User' }).click();
  await page.getByLabel('Username').fill('fixture-member');
  await page.getByLabel('Email').fill('member@example.test');
  await page.getByLabel('Password', { exact: true }).fill('test-password');
  await page.getByLabel('limited', { exact: false }).check();
  await page.getByRole('button', { name: 'Create User' }).click();
  const row = page.getByRole('row').filter({ hasText: 'fixture-member' });
  await expect(row).toContainText('limited');
  await row.getByRole('button', { name: 'Edit' }).click();
  await page.getByLabel('limited', { exact: false }).uncheck();
  await page.getByLabel('viewer', { exact: false }).check();
  await page.getByRole('button', { name: 'Update User' }).click();
  await expect(row).toContainText('viewer');
  page.once('dialog', dialog => dialog.accept());
  await row.getByRole('button', { name: 'Delete' }).click();
  await expect(row).toHaveCount(0);
  await expect(page.getByRole('row').filter({ hasText: admin.username })).toBeVisible();
  expect(api.calls.filter(call => !call.startsWith('GET '))).toEqual([
    'POST /api/users', 'PATCH /api/users/401', 'DELETE /api/users/401',
  ]);
});

test('audit action filter requests attributed results and excludes other actions', async ({ page, api }) => {
  const item = { id: 91, timestamp, user_id: admin.id, username: admin.username, action: 'user_created',
    resource_type: 'user', resource_id: '401', details: null, ip_address: null, user_agent: null,
    success: true, error_message: null };
  const result = (logs: typeof item[]) => ({ logs, total: logs.length, page: 1, per_page: 20, total_pages: 1 });
  api.get('/api/audit?per_page=20&page=1', result([]));
  api.get('/api/audit/stats', { total_events: 1, successful_events: 1, failed_events: 0,
    events_today: 1, events_this_week: 1, top_actions: [], recent_failures: [] });
  api.get('/api/audit?per_page=20&action=user_created&page=1', result([item]));
  await page.goto('/settings?section=audit');
  await page.locator('select').filter({ has: page.getByRole('option', { name: 'User Created' }) }).selectOption('user_created');
  const row = page.getByRole('row').filter({ hasText: 'User Created' });
  await expect(row).toContainText(admin.username);
  await expect(row).toContainText('#401');
  expect(api.calls).toContain('GET /api/audit?per_page=20&action=user_created&page=1');
});
