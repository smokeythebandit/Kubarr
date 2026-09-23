import { chmodSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { basename, dirname, join } from 'node:path';
import { test as base, expect, type Page } from '@playwright/test';

export type Credentials = {
  provider_name: string;
  private_key: string;
  public_key: string;
  addresses: string[];
  endpoint_ip: string;
  endpoint_port: number;
  firewall_outbound_subnets: string;
};

function env(name: string): string {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`Missing ${name}`);
  return value;
}

export const test = base.extend<{ authenticatedPage: Page }>({
  authenticatedPage: async ({ page }, provide) => {
    await page.goto('/login');
    await page.getByLabel('Username').fill(env('TEST_USERNAME'));
    await page.getByLabel('Password').fill(env('TEST_PASSWORD'));
    await Promise.all([
      page.waitForURL(url => url.pathname !== '/login'),
      page.getByRole('button', { name: 'Sign in', exact: true }).click(),
    ]);
    await provide(page);
  },
});

export { expect };

export function credentials(): Credentials {
  const path = env('VPN_LAB_CREDENTIALS_FILE');
  const value = JSON.parse(readFileSync(path, 'utf8')) as Credentials;
  expect(value.addresses).toEqual(['10.66.0.2/32']);
  expect(value.endpoint_ip).toBe('203.0.113.2');
  expect(value.endpoint_port).toBe(51820);
  return value;
}

export function operationId(body: unknown): string {
  expect(body).toBeTruthy();
  const record = body as Record<string, unknown>;
  const config = record.config && typeof record.config === 'object'
    ? record.config as Record<string, unknown>
    : record;
  const id = config.operation_id ?? record.operation_id;
  expect(id).toMatch(/^[0-9a-fA-F-]{36}$/);
  return id as string;
}

export async function awaitOperation(page: Page, id: string): Promise<void> {
  await expect.poll(async () => {
    const response = await page.request.get(`/api/apps/operations/${encodeURIComponent(id)}`);
    expect(response.ok()).toBe(true);
    const operation = await response.json() as { status: string; error?: string };
    if (operation.status === 'failed') throw new Error(`VPN redeploy failed: ${operation.error ?? 'unknown error'}`);
    return operation.status;
  }, { timeout: 10 * 60_000, intervals: [1_000, 2_000, 5_000] }).toBe('succeeded');

  await expect.poll(async () => {
    const response = await page.request.get('/api/apps/sonarr/state');
    expect(response.ok()).toBe(true);
    const state = await response.json() as { healthy: boolean; last_operation_id: string | null };
    return state.healthy && state.last_operation_id === id;
  }, { timeout: 3 * 60_000, intervals: [1_000, 2_000, 5_000] }).toBe(true);
}

export function recordResult(values: Record<string, string>): void {
  const path = env('ACCEPTANCE_RESULT_FILE');
  const current = JSON.parse(readFileSync(path, 'utf8')) as Record<string, unknown>;
  const temporary = join(dirname(path), `.${basename(path)}.${process.pid}.tmp`);
  writeFileSync(temporary, `${JSON.stringify({ ...current, ...values }, null, 2)}\n`, { mode: 0o600 });
  chmodSync(temporary, 0o600);
  renameSync(temporary, path);
  chmodSync(path, 0o600);
}

export async function openSonarr(page: Page) {
  await page.goto('/apps');
  const heading = page.getByRole('heading', { name: 'Sonarr', exact: true }).first();
  await expect(heading).toBeVisible();
  await heading.click();
  const details = page.getByRole('region', { name: 'Sonarr details', exact: true });
  await expect(details).toBeVisible();
  return details;
}
