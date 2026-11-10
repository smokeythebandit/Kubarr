import { test, expect, admin, timestamp } from './fixtures';
import type { AppConfig, AppState } from '../../src/types/app';

const app: AppConfig = {
  name: 'radarr', display_name: 'Radarr', description: 'Movie manager', icon: null, version: '1.0.0',
  container_image: 'radarr', default_port: 7878,
  resource_requirements: { cpu_request: '', cpu_limit: '', memory_request: '', memory_limit: '' },
  environment_variables: {}, volumes: [], category: 'media-manager', is_system: false, is_hidden: false, is_browseable: true,
};
const state: AppState = {
  app_name: 'radarr', namespace: 'radarr', desired_state: 'installed', observed_state: 'installed', healthy: true,
  message: null, installed_chart_version: '1.0.0', available_chart_version: '1.1.0', update_available: true,
  last_operation_id: null, last_checked_at: timestamp, updated_at: timestamp,
};

function installedRadarr(api: { get: (path: string, json: unknown) => void; on: (m: string, p: string, h: () => { json: unknown; status?: number }) => void }) {
  api.get('/api/apps/catalog', [app]);
  api.get('/api/apps/installed', ['radarr']);
  api.get('/api/apps/states', [state]);
  api.get('/api/apps/operations', []);
  api.get('/api/apps/catalog/icons', {});
  api.get('/api/monitoring/pods?namespaces=radarr', []);
  api.get('/api/apps/sync/status', { last_synced: timestamp });
  api.get('/api/monitoring/endpoints/radarr?namespace=radarr', []);
  api.on('GET', '/api/apps/catalog/radarr/install-options', () => ({ status: 404, json: { detail: 'not needed' } }));
  api.on('GET', '/api/apps/catalog/radarr/icon', () => ({ status: 404, json: { detail: 'No fixture icon' } }));
}

const asUser = (permissions: string[]) => ({
  ...admin, roles: [{ id: 2, name: 'viewer' }], permissions, allowed_apps: ['radarr'],
});

test('a user without apps.delete or apps.install sees no Uninstall or Update', async ({ page, api }) => {
  installedRadarr(api);
  api.get('/api/users/me', asUser(['apps.view']));
  await page.goto('/apps');
  await expect(page.getByRole('button', { name: 'Open', exact: true }).first()).toBeVisible();
  await expect(page.getByTitle('Uninstall')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Update', exact: true })).toHaveCount(0);
  await page.getByText('Radarr', { exact: true }).first().click();
  await expect(page.getByRole('button', { name: 'Uninstall', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Update', exact: true })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Restart', exact: true })).toHaveCount(0);
});

test('apps.delete and apps.install unlock Uninstall and Update', async ({ page, api }) => {
  installedRadarr(api);
  api.get('/api/users/me', asUser(['apps.view', 'apps.delete', 'apps.install']));
  await page.goto('/apps');
  await expect(page.getByTitle('Uninstall').first()).toBeVisible();
  await expect(page.getByRole('button', { name: 'Update', exact: true }).first()).toBeVisible();
});

const sonarr: AppConfig = { ...app, name: 'sonarr', display_name: 'Sonarr', description: 'TV manager', default_port: 8989 };

test('apps the user cannot access are hidden from the dashboard and have no Open button', async ({ page, api }) => {
  installedRadarr(api);
  api.get('/api/apps/catalog', [app, sonarr]);
  api.get('/api/apps/installed', ['radarr', 'sonarr']);
  api.get('/api/apps/states', [state, { ...state, app_name: 'sonarr', namespace: 'sonarr' }]);
  api.get('/api/users/me', asUser(['apps.view']));
  api.get('/api/monitoring/pods?namespaces=radarr,sonarr', []);
  api.on('GET', '/api/apps/catalog/sonarr/icon', () => ({ status: 404, json: { detail: 'No fixture icon' } }));
  api.on('GET', '/api/networking/flow-classes?window=15m', () => ({ status: 503, json: { detail: 'Hubble not in fixture' } }));

  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Installed Apps' })).toBeVisible();
  const grid = page.locator('h2:has-text("Installed Apps") + div');
  await expect(grid.getByText('Radarr', { exact: true })).toBeVisible();
  await expect(grid.getByText('Sonarr', { exact: true })).toHaveCount(0);

  await page.goto('/apps');
  await expect(page.getByLabel('Open Radarr').or(page.getByRole('button', { name: 'Open', exact: true })).first()).toBeVisible();
  await expect(page.getByLabel('Open Sonarr')).toHaveCount(0);
});

test('the dashboard shows a loading state, not "No apps installed yet", while installed apps load', async ({ page, api }) => {
  installedRadarr(api);
  api.get('/api/users/me', asUser(['apps.view']));
  api.on('GET', '/api/networking/flow-classes?window=15m', () => ({ status: 503, json: { detail: 'Hubble not in fixture' } }));
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  api.on('GET', '/api/apps/installed', async () => { await gate; return { json: ['radarr'] }; });

  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Installed Apps' })).toBeVisible();
  await expect(page.getByText('No apps installed yet.')).toHaveCount(0);
  release();
  await expect(page.locator('h2:has-text("Installed Apps") + div').getByText('Radarr', { exact: true })).toBeVisible();
  await expect(page.getByText('No apps installed yet.')).toHaveCount(0);
});
