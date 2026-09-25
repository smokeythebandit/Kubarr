import { randomBytes } from 'node:crypto';
import type { Browser, Page, TestInfo } from '@playwright/test';
import { test, expect, getJson } from './fixtures';
import { redeemInvite } from './invite-redemption';

type User = { id: number; username: string; is_approved: boolean; roles: Array<{ name: string }> };
type Setting = { value: string };
type Invite = { id: number; code: string; is_used: boolean };
const name = (info: TestInfo, suffix: string) => `a${process.env.ACCEPTANCE_RUN_ID?.replace(/[^a-z0-9]/gi, '').slice(-14)}${info.workerIndex}${randomBytes(4).toString('hex')}${suffix}`;

async function setSwitch(page: Page, key: string, label: string, enabled: boolean) {
  await page.goto('/settings?section=general');
  const toggle = page.getByRole('switch', { name: label, exact: true });
  await expect(toggle).toBeVisible();
  // The switch first renders against empty client state. Wait for the API
  // baseline before deciding whether a click is needed.
  const current = (await getJson<Setting>(page, `/api/settings/${key}`)).value;
  await expect(toggle).toHaveAttribute('aria-checked', current);
  if (current !== String(enabled)) {
    const changed = page.waitForResponse(response =>
      response.request().method() === 'PUT' && new URL(response.url()).pathname === `/api/settings/${key}`);
    await toggle.click();
    expect((await changed).ok()).toBe(true);
  }
  await expect.poll(async () => (await getJson<Setting>(page, `/api/settings/${key}`)).value).toBe(String(enabled));
}

async function visitor(browser: Browser, admin: Page) {
  const context = await browser.newContext({ baseURL: new URL(admin.url()).origin, storageState: { cookies: [], origins: [] } });
  return { context, page: await context.newPage() };
}

async function register(page: Page, username: string, password: string) {
  const response = await page.goto('/login?register=1');
  expect(response?.status(), 'Registration document HTTP status').toBe(200);
  await expect(page, 'Registration must remain on its public route').toHaveURL(/\/login\?register=1/);
  await expect(page.getByRole('heading', { name: 'Create an account' })).toBeVisible({ timeout: 5_000 });
  await page.getByRole('textbox', { name: 'Username' }).fill(username);
  await page.getByRole('textbox', { name: 'Email' }).fill(`${username}@example.test`);
  await page.getByLabel('Password').fill(password);
  await page.getByRole('button', { name: 'Register', exact: true }).click();
}

async function lookup(page: Page, username: string): Promise<User | undefined> {
  return (await getJson<User[]>(page, '/api/users')).find(user => user.username === username);
}

test('registration toggle rejects strangers; pending approval and rejection control login', async ({ standardPage: admin, browser }) => {
  const approvalKey = 'registration_require_approval';
  const openKey = 'registration_enabled';
  const originalOpen = (await getJson<Setting>(admin, `/api/settings/${openKey}`)).value;
  const originalApproval = (await getJson<Setting>(admin, `/api/settings/${approvalKey}`)).value;
  const { context, page } = await visitor(browser, admin);
  const password = `Test-${randomBytes(18).toString('hex')}`;
  const blocked = name(test.info(), 'blocked');
  const approved = name(test.info(), 'approved');
  const rejected = name(test.info(), 'rejected');
  let approvedId: number | undefined;
  let rejectedId: number | undefined;
  try {
    await setSwitch(admin, openKey, 'Allow Open Registration', false);
    await register(page, blocked, password);
    await expect(page.getByRole('alert')).toContainText('disabled');
    expect(await lookup(admin, blocked)).toBeUndefined();
    await setSwitch(admin, openKey, 'Allow Open Registration', true);
    // The registration switch must be exercised through the UI; pin the
    // separate approval policy after page navigation has settled.
    expect((await admin.request.put(`/api/settings/${approvalKey}`, { data: { value: 'true' } })).ok()).toBe(true);
    for (const username of [approved, rejected]) {
      expect((await getJson<Setting>(admin, `/api/settings/${approvalKey}`)).value,
        'Approval must remain enabled before each public registration').toBe('true');
      await register(page, username, password);
      await expect(page.getByRole('status')).toContainText('Awaiting admin approval');
      await expect.poll(async () => (await lookup(admin, username))?.is_approved).toBe(false);
      if (username === approved) approvedId = (await lookup(admin, username))!.id;
      else rejectedId = (await lookup(admin, username))!.id;
      const denied = await context.request.post('/auth/login', { data: { username, password } });
      expect(denied.status()).toBe(401);
    }
    await admin.goto('/settings?section=pending');
    const row = (username: string) => admin.getByRole('row').filter({ hasText: username });
    await expect(row(approved)).toBeVisible();
    await row(approved).getByRole('button', { name: 'Approve' }).click();
    await expect.poll(async () => (await lookup(admin, approved))?.is_approved).toBe(true);
    expect((await context.request.post('/auth/login', { data: { username: approved, password } })).status()).toBe(200);
    await admin.goto('/settings?section=pending');
    admin.once('dialog', dialog => dialog.accept());
    await row(rejected).getByRole('button', { name: 'Reject' }).click();
    await expect(row(rejected)).toHaveCount(0);
    await expect.poll(async () => (await admin.request.get(`/api/users/${rejectedId}`)).status()).toBe(404);
  } finally {
    await context.close();
    if (approvedId) expect((await admin.request.delete(`/api/users/${approvedId}`)).ok()).toBe(true);
    for (const [key, value] of [[openKey, originalOpen], [approvalKey, originalApproval]]) {
      expect((await admin.request.put(`/api/settings/${key}`, { data: { value } })).ok()).toBe(true);
    }
  }
});

test('invite is redeemable exactly once even with open registration disabled', async ({ standardPage: admin, browser }) => {
  const original = (await getJson<Setting>(admin, '/api/settings/registration_enabled')).value;
  const username = name(test.info(), 'invite');
  const reused = name(test.info(), 'reuse');
  const password = `Test-${randomBytes(18).toString('hex')}`;
  const { context, page } = await visitor(browser, admin);
  let invite: Invite | undefined;
  try {
    await setSwitch(admin, 'registration_enabled', 'Allow Open Registration', false);
    // The UI's creation modal displays the one-use code and Playwright writes
    // native DOM snapshots on failure. Create via the real admin API instead;
    // the strict mocked browser lane exercises the modal separately.
    const created = await admin.request.post('/api/users/invites', { data: { expires_in_days: 7 } });
    expect(created.status()).toBe(200);
    invite = await created.json() as Invite;
    expect(invite?.code).toBeTruthy();
    await admin.goto('/settings?section=invites');
    await expect(admin.getByRole('row').filter({ hasText: 'Active' })).toBeVisible();
    expect(await redeemInvite(page.request, username, password, invite!.code)).toBe(200);
    await expect.poll(async () => (await getJson<Invite[]>(admin, '/api/users/invites')).find(i => i.id === invite!.id)?.is_used).toBe(true);
    expect((await lookup(admin, username))?.is_approved).toBe(true);
    expect((await context.request.post('/auth/login', { data: { username, password } })).status()).toBe(200);
    expect(await redeemInvite(page.request, reused, password, invite!.code)).toBe(400);
    expect(await lookup(admin, reused)).toBeUndefined();
  } finally {
    await context.close();
    try {
      // Invite.used_by_id references the redeemed account; delete the invite first.
      if (invite) expect((await admin.request.delete(`/api/users/invites/${invite.id}`)).ok()).toBe(true);
      const user = await lookup(admin, username);
      if (user) expect((await admin.request.delete(`/api/users/${user.id}`)).ok()).toBe(true);
    } finally {
      expect((await admin.request.put('/api/settings/registration_enabled', { data: { value: original } })).ok()).toBe(true);
    }
  }
});

test('user CRUD role permissions enforce access and audit attributes admin mutations', async ({ standardPage: admin, browser }) => {
  const username = name(test.info(), 'role');
  const roleName = name(test.info(), 'limited');
  const password = `Test-${randomBytes(18).toString('hex')}`;
  const { context } = await visitor(browser, admin);
  let roleId: number | undefined;
  let userId: number | undefined;
  try {
    const createdRole = await admin.request.post('/api/roles', { data: { name: roleName, description: 'Disposable acceptance role' } });
    expect(createdRole.ok()).toBe(true);
    roleId = ((await createdRole.json()) as { id: number }).id;
    await admin.goto('/settings?section=users');
    await admin.getByRole('button', { name: 'Create New User' }).click();
    await admin.getByLabel('Username').fill(username);
    await admin.getByLabel('Email').fill(`${username}@example.test`);
    await admin.getByLabel('Password', { exact: true }).fill(password);
    await admin.getByLabel(roleName, { exact: false }).check();
    await admin.getByRole('button', { name: /Create User|Save User/ }).click();
    await expect.poll(async () => lookup(admin, username)).toMatchObject({ username });
    const id = (await lookup(admin, username))!.id;
    userId = id;
    await admin.goto('/settings?section=permissions');
    const permissionRow = (key: string) => admin.getByRole('row').filter({ has: admin.getByText(key, { exact: true }) });
    // Role columns follow the role inventory ordering. Locate the column by its header.
    const headers = admin.getByRole('columnheader');
    await expect(admin.getByRole('heading', { name: 'Permission Matrix' })).toBeVisible();
    const header = headers.filter({ hasText: roleName });
    await expect(header).toBeVisible();
    const column = await header.evaluate((element: HTMLTableCellElement) => element.cellIndex);
    expect(column).toBeGreaterThan(0);
    await permissionRow('audit.view').getByRole('cell').nth(column).getByRole('checkbox').check();
    await admin.getByRole('columnheader').nth(column).getByRole('button', { name: 'Save' }).click();
    await expect.poll(async () => (await getJson<Array<{ id: number; permissions: string[] }>>(admin, '/api/roles')).find(r => r.id === roleId)?.permissions).toContain('audit.view');
    const loggedIn = await context.request.post('/auth/login', { data: { username, password } });
    expect(loggedIn.ok()).toBe(true);
    expect((await context.request.get('/api/audit?per_page=10')).status()).toBe(200);
    expect((await context.request.get('/api/users')).status()).toBe(403);
    expect((await context.request.get('/api/settings')).status()).toBe(403);
    const audit = await getJson<{ logs: Array<{ action: string; username: string | null; resource_id: string | null }> }>(admin, '/api/audit?action=user_created&per_page=100');
    expect(audit.logs.some(entry => entry.action === 'user_created' && entry.username === process.env.TEST_USERNAME && entry.resource_id === String(id))).toBe(true);
    await admin.goto('/settings?section=users');
    admin.once('dialog', dialog => dialog.accept());
    await admin.getByRole('row').filter({ hasText: username }).getByRole('button', { name: 'Delete' }).click();
    await expect.poll(async () => (await admin.request.get(`/api/users/${id}`)).status()).toBe(404);
  } finally {
    await context.close();
    if (userId && (await admin.request.get(`/api/users/${userId}`)).ok()) {
      expect((await admin.request.delete(`/api/users/${userId}`)).ok()).toBe(true);
    }
    if (roleId) expect((await admin.request.delete(`/api/roles/${roleId}`)).ok()).toBe(true);
  }
});
