import { randomBytes } from 'node:crypto';
import type { Page, TestInfo } from '@playwright/test';
import { test, expect, getJson } from './fixtures';

type Setting = { key: string; value: string };
type Preferences = { theme: 'system' | 'light' | 'dark' };
type NotificationEvent = { event_type: string; enabled: boolean; severity: string };
type Provider = {
  id: number;
  name: string;
  vpn_type: 'wireguard' | 'openvpn';
  enabled: boolean;
  kill_switch: boolean;
  firewall_outbound_subnets: string;
};

const runId = process.env.ACCEPTANCE_RUN_ID ?? 'local';

function uniqueName(testInfo: TestInfo, suffix: string) {
  const title = testInfo.title.replace(/[^a-z0-9]+/gi, '-').replace(/^-|-$/g, '').slice(0, 24);
  return `acceptance-${runId}-${title}-${suffix}`;
}

function wireGuardKey() {
  // A fresh, syntactically valid 32-byte WireGuard key used only as test data.
  return randomBytes(32).toString('base64');
}

async function providers(page: Page): Promise<Provider[]> {
  const body = await getJson(page, '/api/vpn/providers') as { providers: Provider[] };
  return body.providers;
}

async function openVpnSettings(page: Page) {
  await page.goto('/settings?section=vpn');
  await expect(page.getByRole('heading', { name: 'VPN Configuration' })).toBeVisible();
}

async function addWireGuard(page: Page, name: string, key = wireGuardKey()) {
  await openVpnSettings(page);
  await page.getByRole('button', { name: /Add VPN Provider|Add Provider/, exact: false }).first().click();
  await page.getByPlaceholder('My VPN').fill(name);
  await page.getByPlaceholder('Enter WireGuard private key').fill(key);
  await page.getByPlaceholder('10.2.0.2/32', { exact: true }).fill('10.250.0.2/32');
  const saved = page.waitForResponse(response =>
    response.request().method() === 'POST' && response.url().endsWith('/api/vpn/providers'));
  await page.getByRole('button', { name: 'Add Provider', exact: true }).click();
  expect((await saved).ok()).toBe(true);
  await expect(page.getByRole('heading', { name, exact: true })).toBeVisible();
}

function providerCard(page: Page, name: string) {
  return page.getByRole('heading', { name, exact: true })
    .locator('xpath=ancestor::div[contains(@class,"border")][1]');
}

async function deleteProvider(page: Page, name: string, accept = true) {
  page.once('dialog', async dialog => accept ? dialog.accept() : dialog.dismiss());
  const response = accept
    ? page.waitForResponse(item => item.request().method() === 'DELETE' && /\/api\/vpn\/providers\/\d+$/.test(item.url()))
    : undefined;
  await providerCard(page, name).getByTitle('Delete provider', { exact: true }).click();
  if (response) expect((await response).ok()).toBe(true);
}

function containsCredentialField(value: unknown): boolean {
  if (!value || typeof value !== 'object') return false;
  if (Array.isArray(value)) return value.some(containsCredentialField);
  const record = value as Record<string, unknown>;
  if (['credentials', 'private_key', 'privateKey', 'password'].some(key => Object.prototype.hasOwnProperty.call(record, key))) return true;
  return Object.values(record).some(containsCredentialField);
}

test.describe('real settings and VPN (configuration only)', () => {
  test('registration and approval switches persist, then restore their baseline', async ({ standardPage: page }) => {
    await page.goto('/settings?section=general');
    const registration = page.getByRole('switch', { name: 'Allow Open Registration', exact: true });
    const approval = page.getByRole('switch', { name: 'Require Admin Approval', exact: true });
    const originalRegistration = await registration.getAttribute('aria-checked') === 'true';
    const originalApproval = await approval.getAttribute('aria-checked') === 'true';

    for (const [toggle, key, original] of [
      [registration, 'registration_enabled', originalRegistration],
      [approval, 'registration_require_approval', originalApproval],
    ] as const) {
      await toggle.click();
      await expect(toggle).toHaveAttribute('aria-checked', String(!original));
      const changed = await getJson(page, `/api/settings/${key}`) as Setting;
      expect(changed.value).toBe(String(!original));
      await page.reload();
      await expect(toggle).toHaveAttribute('aria-checked', String(!original));
      await toggle.click();
      await expect(toggle).toHaveAttribute('aria-checked', String(original));
      const restored = await getJson(page, `/api/settings/${key}`) as Setting;
      expect(restored.value).toBe(String(original));
    }
  });

  for (const selected of ['dark', 'light'] as const) {
    test(`${selected} theme updates the document, API preference, and reload`, async ({ standardPage: page }) => {
      const original = (await getJson(page, '/api/users/me/preferences') as Preferences).theme;
      await page.goto('/account');
      const appearance = page.getByRole('heading', { name: 'Appearance', exact: true }).locator('..').locator('..');
      await appearance.getByRole('button', { name: new RegExp(`^${selected}`, 'i') }).click();
      await expect.poll(async () => (await getJson(page, '/api/users/me/preferences') as Preferences).theme).toBe(selected);
      await expect(page.locator('html')).toHaveClass(selected === 'dark' ? /(^|\s)dark(\s|$)/ : /^(?!.*(?:^|\s)dark(?:\s|$))/);
      await page.reload();
      await expect.poll(async () => (await getJson(page, '/api/users/me/preferences') as Preferences).theme).toBe(selected);
      await expect(page.locator('html')).toHaveClass(selected === 'dark' ? /(^|\s)dark(\s|$)/ : /^(?!.*(?:^|\s)dark(?:\s|$))/);
      await appearance.getByRole('button', { name: new RegExp(`^${original}`, 'i') }).click();
      await expect.poll(async () => (await getJson(page, '/api/users/me/preferences') as Preferences).theme).toBe(original);
    });
  }

  test('notification event enabled state and severity survive reload, then restore', async ({ standardPage: page }) => {
    const baseline = await getJson(page, '/api/notifications/events') as NotificationEvent[];
    const original = baseline.find(event => event.event_type === 'user_login') ?? baseline[0];
    expect(original).toBeTruthy();
    await page.goto('/settings?section=notifications');
    const label = original.event_type.split('_').map(word => word[0].toUpperCase() + word.slice(1)).join(' ');
    const toggle = page.getByRole('switch', { name: `${label} notifications`, exact: true });
    const severity = page.getByRole('combobox', { name: `${label} severity`, exact: true });
    const changedSeverity = original.severity === 'critical' ? 'warning' : 'critical';

    const toggled = page.waitForResponse(response =>
      response.request().method() === 'PUT' && response.url().endsWith(`/api/notifications/events/${original.event_type}`));
    await toggle.click();
    expect((await toggled).ok()).toBe(true);
    const severityChanged = page.waitForResponse(response =>
      response.request().method() === 'PUT' && response.url().endsWith(`/api/notifications/events/${original.event_type}`));
    await severity.selectOption(changedSeverity);
    expect((await severityChanged).ok()).toBe(true);
    await expect.poll(async () => {
      const events = await getJson(page, '/api/notifications/events') as NotificationEvent[];
      return events.find(event => event.event_type === original.event_type);
    }).toMatchObject({ enabled: !original.enabled, severity: changedSeverity });
    await page.reload();
    await expect(toggle).toHaveAttribute('aria-checked', String(!original.enabled));
    await expect(severity).toHaveValue(changedSeverity);

    const toggleRestored = page.waitForResponse(response =>
      response.request().method() === 'PUT' && response.url().endsWith(`/api/notifications/events/${original.event_type}`));
    await toggle.click();
    expect((await toggleRestored).ok()).toBe(true);
    const severityRestored = page.waitForResponse(response =>
      response.request().method() === 'PUT' && response.url().endsWith(`/api/notifications/events/${original.event_type}`));
    await severity.selectOption(original.severity);
    expect((await severityRestored).ok()).toBe(true);
    await expect.poll(async () => {
      const events = await getJson(page, '/api/notifications/events') as NotificationEvent[];
      return events.find(event => event.event_type === original.event_type);
    }).toMatchObject({ enabled: original.enabled, severity: original.severity });
  });

  test('creates a WireGuard provider without exposing its key in UI or read APIs', async ({ standardPage: page }, testInfo) => {
    const name = uniqueName(testInfo, 'wg-create');
    const key = wireGuardKey();
    await addWireGuard(page, name, key);
    const listed = await providers(page);
    const created = listed.find(provider => provider.name === name && provider.vpn_type === 'wireguard');
    expect(Boolean(created)).toBe(true);
    expect(containsCredentialField(listed)).toBe(false);
    const detail = await getJson(page, `/api/vpn/providers/${created!.id}`);
    expect(containsCredentialField(detail)).toBe(false);
    expect(JSON.stringify([listed, detail]).includes(key), 'Read APIs must not return the private key').toBe(false);
    expect((await page.locator('body').innerText()).includes(key), 'Closed provider UI must not expose the private key').toBe(false);
    await expect(page.getByText(/private key/i)).toHaveCount(0);
    await deleteProvider(page, name);
    await expect.poll(async () => (await providers(page)).some(provider => provider.name === name)).toBe(false);
  });

  test('edits WireGuard name, enabled, kill switch, and allowed subnets', async ({ standardPage: page }, testInfo) => {
    const name = uniqueName(testInfo, 'wg-edit');
    const renamed = `${name}-renamed`;
    await addWireGuard(page, name);
    await providerCard(page, name).getByTitle('Edit provider', { exact: true }).click();
    await page.getByPlaceholder('My VPN').fill(renamed);
    await page.getByRole('switch', { name: 'Enabled', exact: true }).click();
    await page.getByRole('switch', { name: 'Kill Switch', exact: true }).click();
    await page.getByPlaceholder('10.0.0.0/8,172.16.0.0/12,192.168.0.0/16').fill('10.42.0.0/16');
    // The current form asks for the key again while editing; use fresh disposable test data.
    await page.getByPlaceholder('(unchanged)').fill(wireGuardKey());
    const saved = page.waitForResponse(response => response.request().method() === 'PUT' && /\/api\/vpn\/providers\/\d+$/.test(response.url()));
    await page.getByRole('button', { name: 'Save Changes', exact: true }).click();
    expect((await saved).ok()).toBe(true);
    await page.reload();
    const result = (await providers(page)).find(provider => provider.name === renamed);
    expect(result?.enabled).toBe(false);
    expect(result?.kill_switch).toBe(false);
    expect(result?.firewall_outbound_subnets).toBe('10.42.0.0/16');
    await expect(providerCard(page, renamed).getByText('Disabled', { exact: true })).toBeVisible();
    await expect(providerCard(page, renamed).getByText('Kill switch OFF', { exact: true })).toBeVisible();
    await deleteProvider(page, renamed);
  });

  test('WireGuard deletion can be cancelled before confirmation removes it', async ({ standardPage: page }, testInfo) => {
    const name = uniqueName(testInfo, 'wg-delete');
    await addWireGuard(page, name);
    await deleteProvider(page, name, false);
    expect((await providers(page)).some(provider => provider.name === name)).toBe(true);
    await expect(page.getByRole('heading', { name, exact: true })).toBeVisible();
    await deleteProvider(page, name);
    await expect.poll(async () => (await providers(page)).some(provider => provider.name === name)).toBe(false);
  });

  for (const vpnType of ['WireGuard', 'OpenVPN'] as const) {
    test(`${vpnType} required credential validation sends no mutation`, async ({ standardPage: page }) => {
      const before = (await providers(page)).length;
      let mutationResponses = 0;
      page.on('response', response => {
        if (response.request().method() === 'POST' && response.url().endsWith('/api/vpn/providers')) mutationResponses++;
      });
      await openVpnSettings(page);
      await page.getByRole('button', { name: /Add VPN Provider|Add Provider/, exact: false }).first().click();
      await page.getByPlaceholder('My VPN').fill(`invalid-${runId}`);
      if (vpnType === 'OpenVPN') await page.getByRole('radio', { name: 'OpenVPN', exact: true }).check();
      await page.getByRole('button', { name: 'Add Provider', exact: true }).click();
      const required = vpnType === 'WireGuard'
        ? page.getByPlaceholder('Enter WireGuard private key')
        : page.getByPlaceholder('VPN username');
      expect(await required.evaluate((input: HTMLInputElement) => input.validity.valueMissing)).toBe(true);
      expect(mutationResponses).toBe(0);
      expect((await providers(page)).length).toBe(before);
      await page.getByRole('button', { name: 'Cancel', exact: true }).click();
    });
  }

  test('OpenVPN create, edit, reload, and delete remain configuration-only', async ({ standardPage: page }, testInfo) => {
    const name = uniqueName(testInfo, 'ovpn');
    const renamed = `${name}-renamed`;
    await openVpnSettings(page);
    await page.getByRole('button', { name: /Add VPN Provider|Add Provider/, exact: false }).first().click();
    await page.getByPlaceholder('My VPN').fill(name);
    await page.getByRole('radio', { name: 'OpenVPN', exact: true }).check();
    await page.getByPlaceholder('VPN username').fill(`user-${runId}`);
    await page.getByPlaceholder('VPN password').fill(randomBytes(18).toString('base64url'));
    const created = page.waitForResponse(response => response.request().method() === 'POST' && response.url().endsWith('/api/vpn/providers'));
    await page.getByRole('button', { name: 'Add Provider', exact: true }).click();
    expect((await created).ok()).toBe(true);
    await providerCard(page, name).getByTitle('Edit provider', { exact: true }).click();
    await page.getByPlaceholder('My VPN').fill(renamed);
    await page.getByPlaceholder('(unchanged)').nth(0).fill(`user2-${runId}`);
    await page.getByPlaceholder('(unchanged)').nth(1).fill(randomBytes(18).toString('base64url'));
    const updated = page.waitForResponse(response => response.request().method() === 'PUT' && /\/api\/vpn\/providers\/\d+$/.test(response.url()));
    await page.getByRole('button', { name: 'Save Changes', exact: true }).click();
    expect((await updated).ok()).toBe(true);
    await page.reload();
    expect((await providers(page)).some(provider => provider.name === renamed && provider.vpn_type === 'openvpn')).toBe(true);
    await deleteProvider(page, renamed);
    await expect.poll(async () => (await providers(page)).some(provider => provider.name === renamed)).toBe(false);
  });
});
