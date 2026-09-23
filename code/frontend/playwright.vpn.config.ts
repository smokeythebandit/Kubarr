import { defineConfig, devices } from '@playwright/test';

const required = [
  'BASE_URL',
  'TEST_USERNAME',
  'TEST_PASSWORD',
  'ACCEPTANCE_RUN_ID',
  'VPN_LAB_CREDENTIALS_FILE',
  'ACCEPTANCE_RESULT_FILE',
] as const;

if (process.env.KUBARR_VPN_REAL_ACCEPTANCE !== '1') {
  throw new Error('Real VPN acceptance requires KUBARR_VPN_REAL_ACCEPTANCE=1');
}
for (const name of required) {
  if (!process.env[name]?.trim()) throw new Error(`Real VPN acceptance requires ${name}`);
}

const target = new URL(process.env.BASE_URL!);
if (
  target.protocol !== 'http:' ||
  !['localhost', '127.0.0.1'].includes(target.hostname) ||
  !target.port || target.pathname !== '/' || target.search || target.hash
) {
  throw new Error('BASE_URL must be an explicit disposable loopback HTTP gateway');
}

export default defineConfig({
  testDir: './tests/vpn',
  fullyParallel: false,
  forbidOnly: true,
  retries: 0,
  workers: 1,
  timeout: 12 * 60_000,
  expect: { timeout: 60_000 },
  outputDir: 'test-results/vpn',
  reporter: [['list']],
  use: {
    ...devices['Desktop Chrome'],
    baseURL: target.origin,
    actionTimeout: 30_000,
    navigationTimeout: 30_000,
    storageState: { cookies: [], origins: [] },
    serviceWorkers: 'block',
    trace: 'off',
    screenshot: 'off',
    video: 'off',
  },
  projects: [
    { name: 'vpn-configure', testMatch: /configure\.spec\.ts/ },
    { name: 'vpn-remove', testMatch: /remove\.spec\.ts/ },
  ],
});
