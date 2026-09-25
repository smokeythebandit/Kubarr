import type { TestInfo } from '@playwright/test';
import { test, expect, getJson } from './fixtures';

function name(info: TestInfo, type: string) {
  return `acceptance-${process.env.ACCEPTANCE_RUN_ID}-${info.retry}-${type}`;
}

type Item = { id: number; name?: string; domain?: string };

test('manual DNS and staging certificate profiles persist through edit and delete without contacting providers', async ({ standardPage: page }, info) => {
  const dns = name(info, 'manual-dns');
  const cert = name(info, 'staging-cert');
  await page.goto('/settings?section=ddns');
  await page.getByRole('button', { name: 'Add Profile' }).click();
  await page.getByPlaceholder('Home DNS').fill(dns);
  await page.locator('form select').first().selectOption('manual');
  await page.getByRole('button', { name: 'Save Profile' }).click();
  await expect(page.getByText(dns, { exact: true })).toBeVisible();
  let dnsRecord = (await getJson<Item[]>(page, '/api/domains/ddns-profiles')).find(item => item.name === dns);
  expect(dnsRecord?.id).toBeTruthy();
  await page.reload();
  await expect(page.getByText(dns, { exact: true })).toBeVisible();
  await page.getByText(dns, { exact: true }).locator('xpath=ancestor::div[contains(@class,"border")][1]').locator('button').first().click();
  await page.getByRole('checkbox', { name: 'Enabled', exact: true }).uncheck();
  await page.getByRole('button', { name: 'Save Profile' }).click();
  await expect.poll(async () => (await getJson<Array<Item & { enabled: boolean }>>(page, '/api/domains/ddns-profiles'))
    .find(item => item.name === dns)?.enabled).toBe(false);

  await page.goto('/settings?section=letsencrypt');
  await page.getByRole('button', { name: 'Add Profile' }).click();
  await page.getByPlaceholder('Production DNS-01').fill(cert);
  await page.getByPlaceholder('admin@example.com').fill('acceptance@example.test');
  await page.getByRole('checkbox', { name: 'Automatic renewal' }).uncheck();
  await page.getByRole('button', { name: 'Save Profile' }).click();
  await expect(page.getByText(cert, { exact: true })).toBeVisible();
  const certRecord = (await getJson<Item[]>(page, '/api/domains/letsencrypt-profiles')).find(item => item.name === cert);
  expect(certRecord?.id).toBeTruthy();
  await page.reload();
  await expect(page.getByText(cert, { exact: true })).toBeVisible();
  await page.getByText(cert, { exact: true }).locator('xpath=ancestor::div[contains(@class,"border")][1]').locator('button').first().click();
  await page.getByRole('checkbox', { name: 'Enabled', exact: true }).uncheck();
  await page.getByRole('button', { name: 'Save Profile' }).click();
  await expect.poll(async () => (await getJson<Array<Item & { enabled: boolean; renewal_enabled: boolean }>>(page, '/api/domains/letsencrypt-profiles'))
    .find(item => item.id === certRecord!.id)).toMatchObject({ enabled: false, renewal_enabled: false });

  // Use the row containing the run-specific name, not a global delete selector.
  page.once('dialog', dialog => dialog.accept());
  await page.getByText(cert, { exact: true }).locator('xpath=ancestor::div[contains(@class,"border")][1]').locator('button').last().click();
  await expect.poll(async () => (await getJson<Item[]>(page, '/api/domains/letsencrypt-profiles')).some(item => item.id === certRecord!.id)).toBe(false);
  await page.goto('/settings?section=ddns');
  page.once('dialog', dialog => dialog.accept());
  await page.getByText(dns, { exact: true }).locator('xpath=ancestor::div[contains(@class,"border")][1]').locator('button').last().click();
  await expect.poll(async () => (await getJson<Item[]>(page, '/api/domains/ddns-profiles')).some(item => item.id === dnsRecord!.id)).toBe(false);
});

test('manual domain inventory CRUD does not request DNS or TLS provisioning', async ({ standardPage: page }, info) => {
  const domain = `${name(info, 'domain')}.example.test`;
  const renamed = `${name(info, 'renamed')}.example.test`;
  await page.goto('/settings?section=domains');
  await page.getByRole('button', { name: 'Add Domain' }).click();
  await page.getByPlaceholder('example.com', { exact: true }).fill(domain);
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByText(domain, { exact: true })).toBeVisible();
  const record = (await getJson<Item[]>(page, '/api/domains')).find(item => item.domain === domain);
  expect(record?.id).toBeTruthy();
  await page.reload();
  await page.getByText(domain, { exact: true }).locator('xpath=ancestor::div[contains(@class,"border")][1]').locator('button').first().click();
  await page.getByPlaceholder('example.com', { exact: true }).fill(renamed);
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect.poll(async () => (await getJson<Item[]>(page, '/api/domains')).find(item => item.id === record!.id)?.domain).toBe(renamed);
  await page.reload();
  page.once('dialog', dialog => dialog.accept());
  await page.getByText(renamed, { exact: true }).locator('xpath=ancestor::div[contains(@class,"border")][1]').locator('button').last().click();
  await expect.poll(async () => (await getJson<Item[]>(page, '/api/domains')).some(item => item.id === record!.id)).toBe(false);
});
