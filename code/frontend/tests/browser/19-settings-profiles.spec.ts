import { test, expect, timestamp } from './fixtures';
import type { DynamicDnsProfile, LetsEncryptProfile } from '../../src/api/domains';

test('manual DNS profile CRUD sends exact state and reloads without provider traffic', async ({ page, api }) => {
  let records: DynamicDnsProfile[] = [];
  api.on('GET', '/api/domains/ddns-profiles', () => ({ json: records }));
  api.on('POST', '/api/domains/ddns-profiles', request => {
    const data = request.postDataJSON();
    expect(data).toMatchObject({ name: 'Local DNS', provider: 'manual', enabled: true,
      capabilities: { txt_records: false, wildcard_records: false } });
    records = [{ ...data, id: 301, status: 'unknown', last_error: null, created_at: timestamp, updated_at: timestamp }];
    return { json: records[0] };
  });
  api.on('PUT', '/api/domains/ddns-profiles/301', request => {
    const data = request.postDataJSON();
    expect(data).toMatchObject({ name: 'Local DNS', provider: 'manual', enabled: false });
    records = [{ ...records[0], ...data }];
    return { json: records[0] };
  });
  api.on('DELETE', '/api/domains/ddns-profiles/301', () => { records = []; return { json: { success: true } }; });
  await page.goto('/settings?section=ddns');
  await page.getByRole('button', { name: 'Add Profile' }).click();
  await page.getByPlaceholder('Home DNS').fill('Local DNS');
  await page.locator('form select').first().selectOption('manual');
  await page.getByRole('button', { name: 'Save Profile' }).click();
  await expect(page.getByText('Local DNS', { exact: true })).toBeVisible();
  await page.reload();
  const row = page.getByText('Local DNS', { exact: true }).locator('xpath=ancestor::div[contains(@class,"border")][1]');
  await row.locator('button').first().click();
  await page.getByRole('checkbox', { name: 'Enabled', exact: true }).uncheck();
  await page.getByRole('button', { name: 'Save Profile' }).click();
  await expect(row.getByText('Disabled')).toBeVisible();
  page.once('dialog', dialog => dialog.accept());
  await row.locator('button').last().click();
  await expect(page.getByText('No Dynamic DNS profiles configured.')).toBeVisible();
  expect(api.calls.filter(call => !call.startsWith('GET'))).toEqual([
    'POST /api/domains/ddns-profiles', 'PUT /api/domains/ddns-profiles/301', 'DELETE /api/domains/ddns-profiles/301',
  ]);
});

test('certificate profile staging CRUD persists disabled renewal without ACME calls', async ({ page, api }) => {
  let records: LetsEncryptProfile[] = [];
  api.get('/api/domains/ddns-profiles', []);
  api.on('GET', '/api/domains/letsencrypt-profiles', () => ({ json: records }));
  api.on('POST', '/api/domains/letsencrypt-profiles', request => {
    const data = request.postDataJSON();
    expect(data).toEqual({ name: 'Local certificate', email: 'local@example.test', environment: 'staging',
      challenge_type: 'http01', dns_profile_id: null, renewal_enabled: false, enabled: true });
    records = [{ ...data, id: 302, status: 'unknown', last_error: null, created_at: timestamp, updated_at: timestamp }];
    return { json: records[0] };
  });
  api.on('DELETE', '/api/domains/letsencrypt-profiles/302', () => { records = []; return { json: { success: true } }; });
  await page.goto('/settings?section=letsencrypt');
  await page.getByRole('button', { name: 'Add Profile' }).click();
  await page.getByPlaceholder('Production DNS-01').fill('Local certificate');
  await page.getByPlaceholder('admin@example.com').fill('local@example.test');
  await page.getByRole('checkbox', { name: 'Automatic renewal' }).uncheck();
  await page.getByRole('button', { name: 'Save Profile' }).click();
  await page.reload();
  const row = page.getByText('Local certificate', { exact: true }).locator('xpath=ancestor::div[contains(@class,"border")][1]');
  await expect(row.getByText('staging', { exact: true })).toBeVisible();
  page.once('dialog', dialog => dialog.accept());
  await row.locator('button').last().click();
  await expect(page.getByText('No Let’s Encrypt profiles configured.')).toBeVisible();
  expect(api.calls.filter(call => !call.startsWith('GET'))).toEqual([
    'POST /api/domains/letsencrypt-profiles', 'DELETE /api/domains/letsencrypt-profiles/302',
  ]);
});
