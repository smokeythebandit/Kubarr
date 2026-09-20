import { test, expect, timestamp } from './fixtures';
import type { AppVpnConfig, VpnProvider } from '../../src/api/vpn';

const provider: VpnProvider = {
  id: 41, name: 'Fixture VPN', vpn_type: 'wireguard', service_provider: 'custom',
  enabled: true, kill_switch: true, app_count: 0,
  firewall_outbound_subnets: '10.0.0.0/8,172.16.0.0/12,192.168.0.0/16',
  created_at: timestamp, updated_at: timestamp,
};

test.beforeEach(async ({ api }) => {
  api.get('/api/vpn/providers', { providers: [] });
  api.get('/api/vpn/apps', { configs: [] });
  api.get('/api/vpn/supported-providers', { providers: [{
    id: 'custom', name: 'Custom', vpn_types: ['wireguard', 'openvpn'],
    description: 'Fixture service', supports_port_forwarding: false,
  }] });
});

test('empty state, sidebar navigation, required name, and cancel', async ({ page, api }) => {
  await page.goto('/settings?section=vpn');
  await page.getByRole('button', { name: 'VPN', exact: true }).click();
  await expect(page).toHaveURL(/section=vpn$/);
  await expect(page.getByRole('heading', { name: 'VPN Configuration' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'No VPN Providers' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'No Apps Using VPN' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Assign VPN to App' })).toHaveCount(0);
  await page.getByRole('button', { name: 'Add VPN Provider' }).click();
  await expect(page.getByRole('radio', { name: 'WireGuard' })).toBeChecked();
  await expect(page.getByRole('heading', { name: 'WireGuard Configuration' })).toBeVisible();
  await page.getByRole('button', { name: 'Add Provider', exact: true }).click();
  expect(await page.getByPlaceholder('My VPN').evaluate((input: HTMLInputElement) => input.validity.valueMissing)).toBe(true);
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Add VPN Provider' })).toHaveCount(0);
  expect(api.calls.filter(call => call.startsWith('POST'))).toEqual([]);
});

for (const vpnType of ['wireguard', 'openvpn'] as const) {
  test(`creates ${vpnType} with exact credentials and refreshes the provider list`, async ({ page, api }) => {
    let providers: VpnProvider[] = [];
    api.on('GET', '/api/vpn/providers', () => ({ json: { providers } }));
    api.on('POST', '/api/vpn/providers', request => {
      expect(request.postDataJSON()).toEqual({
        name: 'New VPN', vpn_type: vpnType, service_provider: 'custom',
        enabled: true, kill_switch: true, firewall_outbound_subnets: provider.firewall_outbound_subnets,
        credentials: vpnType === 'wireguard'
          ? { private_key: 'fixture-key', addresses: ['10.2.0.2/32', '10.2.0.3/32'] }
          : { username: 'fixture-user', password: 'fixture-password' },
      });
      providers = [{ ...provider, name: 'New VPN', vpn_type: vpnType }];
      return { json: providers[0] };
    });
    await page.goto('/settings?section=vpn');
    await page.getByRole('button', { name: 'Add VPN Provider' }).click();
    await page.getByPlaceholder('My VPN').fill('New VPN');
    if (vpnType === 'wireguard') {
      await page.getByPlaceholder('Enter WireGuard private key').fill('fixture-key');
      await page.getByPlaceholder('10.2.0.2/32', { exact: true }).fill('10.2.0.2/32, 10.2.0.3/32');
    } else {
      await page.getByRole('radio', { name: 'OpenVPN', exact: true }).check();
      await expect(page.getByRole('heading', { name: 'WireGuard Configuration' })).toHaveCount(0);
      await page.getByPlaceholder('VPN username').fill('fixture-user');
      await page.getByPlaceholder('VPN password').fill('fixture-password');
    }
    await page.getByRole('button', { name: 'Add Provider', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Add VPN Provider' })).toHaveCount(0);
    await expect(page.getByRole('heading', { name: 'New VPN', exact: true })).toBeVisible();
    await expect(page.getByText('Kill switch ON', { exact: true })).toBeVisible();
    expect(api.calls.filter(call => call === 'POST /api/vpn/providers')).toHaveLength(1);
  });
}

test('rejected creation retains the form and shows the error, not a provider', async ({ page, api }) => {
  api.on('POST', '/api/vpn/providers', () => ({ status: 422, json: { detail: 'Invalid credentials' } }));
  await page.goto('/settings?section=vpn');
  await page.getByRole('button', { name: 'Add VPN Provider' }).click();
  await page.getByPlaceholder('My VPN').fill('Rejected VPN');
  await page.getByPlaceholder('Enter WireGuard private key').fill('invalid');
  await page.getByRole('button', { name: 'Add Provider', exact: true }).click();
  await expect(page.getByText('Request failed with status code 422', { exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Add VPN Provider' })).toBeVisible();
  await expect(page.getByPlaceholder('My VPN')).toHaveValue('Rejected VPN');
  await expect(page.getByRole('heading', { name: 'Rejected VPN' })).toHaveCount(0);
});

test('edits a seeded provider and confirms deletion before removing it', async ({ page, api }) => {
  let providers = [{ ...provider }];
  api.on('GET', '/api/vpn/providers', () => ({ json: { providers } }));
  api.on('PUT', '/api/vpn/providers/41', request => {
    expect(request.postDataJSON()).toEqual({
      name: 'Renamed VPN', service_provider: 'custom', enabled: true, kill_switch: true,
      firewall_outbound_subnets: provider.firewall_outbound_subnets,
      credentials: { private_key: 'replacement-key', addresses: [] },
    });
    providers = [{ ...provider, name: 'Renamed VPN' }];
    return { json: providers[0] };
  });
  api.on('DELETE', '/api/vpn/providers/41', () => { providers = []; return { json: {} }; });
  await page.goto('/settings?section=vpn');
  await page.getByTitle('Edit provider', { exact: true }).click();
  await expect(page.getByPlaceholder('My VPN')).toHaveValue('Fixture VPN');
  await expect(page.getByRole('radio', { name: 'WireGuard' })).toBeDisabled();
  await page.getByPlaceholder('My VPN').fill('Renamed VPN');
  await page.getByPlaceholder('(unchanged)').fill('replacement-key');
  await page.getByRole('button', { name: 'Save Changes' }).click();
  await expect(page.getByRole('heading', { name: 'Renamed VPN' })).toBeVisible();
  page.once('dialog', async dialog => {
    expect(dialog.message()).toBe('Delete this VPN provider? Apps using it will lose VPN connectivity.');
    await dialog.dismiss();
  });
  await page.getByTitle('Delete provider', { exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Renamed VPN' })).toBeVisible();
  expect(api.calls).not.toContain('DELETE /api/vpn/providers/41');
  page.once('dialog', dialog => dialog.accept());
  await page.getByTitle('Delete provider', { exact: true }).click();
  await expect(page.getByRole('heading', { name: 'No VPN Providers' })).toBeVisible();
  expect(api.calls.filter(call => call === 'DELETE /api/vpn/providers/41')).toHaveLength(1);
});

for (const success of [true, false]) {
  test(`connection test displays the ${success ? 'success' : 'failure'} result`, async ({ page, api }) => {
    api.get('/api/vpn/providers', { providers: [provider] });
    const message = success ? 'Connected to fixture endpoint' : 'Fixture handshake failed';
    api.on('POST', '/api/vpn/providers/41/test', () => ({ json: { success, message } }));
    await page.goto('/settings?section=vpn');
    await page.getByTitle('Test connection', { exact: true }).click();
    await expect(page.getByText(message, { exact: true })).toBeVisible();
    await expect(page.getByText(message, { exact: true }).locator('..')).toHaveClass(success ? /bg-green-50/ : /bg-red-50/);
    await expect(page.getByTitle('Test connection', { exact: true })).toBeEnabled();
    expect(api.calls.filter(call => call === 'POST /api/vpn/providers/41/test')).toHaveLength(1);
  });
}

test('assigns an installed app with a kill switch override and removes the assignment', async ({ page, api }) => {
  let configs: AppVpnConfig[] = [];
  api.get('/api/vpn/providers', { providers: [provider] });
  api.get('/api/apps/installed', ['radarr']);
  api.on('GET', '/api/apps/catalog/radarr/icon', () => ({ status: 404, json: { detail: 'No fixture icon' } }));
  api.on('GET', '/api/vpn/apps', () => ({ json: { configs } }));
  api.on('PUT', '/api/vpn/apps/radarr', request => {
    expect(request.postDataJSON()).toEqual({ vpn_provider_id: 41, kill_switch_override: false });
    configs = [{ app_name: 'radarr', vpn_provider_id: 41, vpn_provider_name: provider.name,
      kill_switch_override: false, effective_kill_switch: false, port_forwarding: false,
      created_at: timestamp, updated_at: timestamp }];
    return { json: configs[0] };
  });
  api.on('DELETE', '/api/vpn/apps/radarr', () => { configs = []; return { json: {} }; });
  await page.goto('/settings?section=vpn');
  await page.getByRole('button', { name: 'Assign VPN to App', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Assign VPN', exact: true })).toBeDisabled();
  await page.getByRole('combobox').nth(0).selectOption('radarr');
  await page.getByRole('combobox').nth(1).selectOption('41');
  await page.getByRole('combobox').nth(2).selectOption('false');
  await page.getByRole('button', { name: 'Assign VPN', exact: true }).click();
  const row = page.getByRole('row').filter({ hasText: 'radarr' });
  await expect(row.getByRole('cell')).toHaveText(['radarr', 'Fixture VPN', 'OFF(override)', '']);
  await expect(page.getByText('Apps are automatically redeployed when VPN settings change.')).toBeVisible();
  page.once('dialog', async dialog => {
    expect(dialog.message()).toBe('Remove VPN from radarr? The app will be redeployed without the VPN sidecar.');
    await dialog.accept();
  });
  await row.getByTitle('Remove VPN').click();
  await expect(page.getByRole('heading', { name: 'No Apps Using VPN' })).toBeVisible();
  expect(api.calls).toContain('DELETE /api/vpn/apps/radarr');
});

test('load failure retries and refresh fetches changed state', async ({ page, api }) => {
  api.on('GET', '/api/vpn/providers', () => ({ status: 503, json: { detail: 'Unavailable' } }));
  await page.goto('/settings?section=vpn');
  await expect(page.getByRole('heading', { name: 'Error Loading VPN Data' })).toBeVisible();
  api.get('/api/vpn/providers', { providers: [] });
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'No VPN Providers' })).toBeVisible();
  api.get('/api/vpn/providers', { providers: [{ ...provider, enabled: false }] });
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('heading', { name: provider.name })).toBeVisible();
  await expect(page.getByText('All VPN providers are disabled. Enable a provider to assign VPN to apps.')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Assign VPN to App' })).toHaveCount(0);
});
