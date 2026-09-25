import { test, expect } from './fixtures';

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
