import { test, expect, timestamp } from './fixtures';
import type { AppDomainAssignment, DomainConfig, DomainRequest } from '../../src/api/domains';
import type { AppConfig } from '../../src/types';

const domainRequest: DomainRequest = {
  domain: 'example.test', kind: 'root', scope: 'public', primary: false, enabled: true,
  dns_mode: 'manual', ddns_profile_id: null, tls_mode: 'none', letsencrypt_profile_id: null, tls_secret_name: null,
};
const domain: DomainConfig = {
  ...domainRequest, id: 51, dns_status: 'manual', certificate_status: 'none',
  certificate_expires_at: null, created_at: timestamp, updated_at: timestamp,
};

test.beforeEach(async ({ api }) => {
  api.get('/api/domains', []);
  api.get('/api/domains/assignments', []);
  api.get('/api/domains/ddns-profiles', []);
  api.get('/api/domains/letsencrypt-profiles', []);
});

test('Domains navigation shows empty inventory and disables assignments until a domain exists', async ({ page }) => {
  await page.goto('/settings?section=domains');
  await page.getByRole('button', { name: 'Domains', exact: true }).click();
  await expect(page).toHaveURL(/section=domains$/);
  await expect(page.getByRole('heading', { name: 'Domains', exact: true })).toBeVisible();
  await expect(page.getByText('No domains configured yet.', { exact: true })).toBeVisible();
  await expect(page.getByText('No app URLs assigned yet.', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Assign App URL' })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Cloudflare Tunnel', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Add Domain', exact: true }).click();
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Add Domain', exact: true })).toHaveCount(0);
});

test('domain form enforces required name and wildcard syntax without sending a mutation', async ({ page, api }) => {
  await page.goto('/settings?section=domains');
  await page.getByRole('button', { name: 'Add Domain', exact: true }).click();
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  expect(await page.getByPlaceholder('example.com', { exact: true }).evaluate((input: HTMLInputElement) => input.validity.valueMissing)).toBe(true);
  await page.getByRole('combobox').nth(0).selectOption('wildcard');
  await page.getByPlaceholder('*.example.com', { exact: true }).fill('example.test');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByText('Wildcard domains must start with *.', { exact: true })).toBeVisible();
  expect(api.calls.filter(call => call.startsWith('POST'))).toEqual([]);
});

test('creates, edits, and deletes a domain using only isolated inventory', async ({ page, api }) => {
  let domains: DomainConfig[] = [];
  api.on('GET', '/api/domains', () => ({ json: domains }));
  api.on('POST', '/api/domains', request => {
    expect(request.postDataJSON()).toEqual(domainRequest);
    domains = [{ ...domain }];
    return { json: domains[0] };
  });
  api.on('PUT', '/api/domains/51', request => {
    expect(request.postDataJSON()).toEqual({ ...domainRequest, domain: 'renamed.example.test' });
    domains = [{ ...domain, domain: 'renamed.example.test' }];
    return { json: domains[0] };
  });
  api.on('DELETE', '/api/domains/51', () => { domains = []; return { json: {} }; });
  await page.goto('/settings?section=domains');
  await page.getByRole('button', { name: 'Add Domain', exact: true }).click();
  await page.getByPlaceholder('example.com', { exact: true }).fill('example.test');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Add Domain', exact: true })).toHaveCount(0);
  await expect(page.getByText('example.test', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Assign App URL' })).toBeEnabled();
  // Inventory icon buttons currently have no accessible names.
  await page.locator('button').filter({ has: page.locator('svg.lucide-pencil') }).click();
  await expect(page.getByRole('heading', { name: 'Edit Domain', exact: true })).toBeVisible();
  await expect(page.getByPlaceholder('example.com', { exact: true })).toHaveValue('example.test');
  await page.getByPlaceholder('example.com', { exact: true }).fill('renamed.example.test');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByText('renamed.example.test', { exact: true })).toBeVisible();
  page.once('dialog', async dialog => {
    expect(dialog.message()).toBe('Delete domain "renamed.example.test"? App URL assignments using it will be removed.');
    await dialog.dismiss();
  });
  await page.locator('button').filter({ has: page.locator('svg.lucide-trash-2') }).click();
  expect(api.calls).not.toContain('DELETE /api/domains/51');
  await expect(page.getByText('renamed.example.test', { exact: true })).toBeVisible();
  page.once('dialog', dialog => dialog.accept());
  await page.locator('button').filter({ has: page.locator('svg.lucide-trash-2') }).click();
  await expect(page.getByText('No domains configured yet.', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Assign App URL' })).toBeDisabled();
  expect(api.calls.filter(call => !call.startsWith('GET'))).toEqual([
    'POST /api/domains', 'PUT /api/domains/51', 'DELETE /api/domains/51',
  ]);
});

test('rejected domain save shows the API detail and retains entered values', async ({ page, api }) => {
  api.on('POST', '/api/domains', request => {
    expect(request.postDataJSON()).toEqual(domainRequest);
    return { status: 409, json: { detail: 'Domain already exists' } };
  });
  await page.goto('/settings?section=domains');
  await page.getByRole('button', { name: 'Add Domain', exact: true }).click();
  await page.getByPlaceholder('example.com', { exact: true }).fill('example.test');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByText('Domain already exists', { exact: true })).toBeVisible();
  await expect(page.getByPlaceholder('example.com', { exact: true })).toHaveValue('example.test');
  await expect(page.getByRole('button', { name: 'Save', exact: true })).toBeEnabled();
  await expect(page.getByText('No domains configured yet.', { exact: true })).toBeVisible();
});

for (const mode of ['path', 'subdomain', 'exact_host'] as const) {
  test(`assigns a ${mode} URL and displays the exact public address`, async ({ page, api }) => {
    const app: AppConfig = {
      name: 'radarr', display_name: 'Radarr', description: 'Fixture app', icon: null,
      version: '1.0', container_image: 'fixture', default_port: 7878,
      resource_requirements: { cpu_request: '100m', cpu_limit: '1', memory_request: '128Mi', memory_limit: '1Gi' },
      environment_variables: {}, volumes: [], category: 'media',
      is_system: false, is_hidden: false, is_browseable: true,
    };
    api.get('/api/apps/catalog', [app]);
    api.on('GET', '/api/apps/catalog/radarr/icon', () => ({ status: 404, json: { detail: 'No fixture icon' } }));
    api.get('/api/domains', [domain]);
    let assignments: AppDomainAssignment[] = [];
    api.on('GET', '/api/domains/assignments', () => ({ json: assignments }));
    api.on('POST', '/api/domains/assignments', request => {
      const expected = {
        app_name: 'radarr', domain_id: 51, route_mode: mode, primary: false, enabled: true,
        hostname: mode === 'path' ? '' : mode === 'subdomain' ? 'movies' : 'movies.example.test',
        path_prefix: mode === 'path' ? '/movies' : '',
      };
      expect(request.postDataJSON()).toEqual(expected);
      assignments = [{ ...expected, id: 61, created_at: timestamp, updated_at: timestamp }];
      return { json: assignments[0] };
    });
    await page.goto('/settings?section=domains');
    await page.getByRole('button', { name: 'Assign App URL', exact: true }).click();
    await expect(page.getByRole('combobox').nth(0)).toHaveValue('radarr');
    await page.getByRole('combobox').nth(2).selectOption(mode);
    await page.locator('form input').first().fill(mode === 'path' ? '/movies' : mode === 'subdomain' ? 'movies' : 'movies.example.test');
    await page.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(page.getByRole('heading', { name: 'Assign App URL', exact: true })).toHaveCount(0);
    await expect(page.getByText(mode === 'path' ? 'https://example.test/movies' : 'https://movies.example.test', { exact: true })).toBeVisible();
    await expect(page.getByText('1 app URL', { exact: true })).toBeVisible();
    expect(api.calls.filter(call => call === 'POST /api/domains/assignments')).toHaveLength(1);
  });
}
