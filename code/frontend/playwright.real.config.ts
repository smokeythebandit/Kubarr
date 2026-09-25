import { defineConfig, devices } from '@playwright/test';

const required = [
  'BASE_URL',
  'TEST_USERNAME',
  'TEST_PASSWORD',
  'ACCEPTANCE_RUN_ID',
  'ACCEPTANCE_CHART_VERSION_A',
  'ACCEPTANCE_CHART_VERSION_B',
  'ACCEPTANCE_RESULT_FILE',
] as const;

if (process.env.KUBARR_REAL_ACCEPTANCE !== '1') {
  throw new Error('Real browser acceptance is destructive and requires KUBARR_REAL_ACCEPTANCE=1');
}

for (const name of required) {
  if (!process.env[name]?.trim()) throw new Error(`Real browser acceptance requires ${name}`);
}

const target = new URL(process.env.BASE_URL!);
const phase = process.env.ACCEPTANCE_PHASE || 'all';
if (!/^[a-z0-9-]+$/.test(phase)) throw new Error('Invalid acceptance report phase');
if (
  target.protocol !== 'http:' ||
  !['localhost', '127.0.0.1'].includes(target.hostname) ||
  !target.port ||
  target.username ||
  target.password ||
  target.pathname !== '/' ||
  target.search ||
  target.hash
) {
  throw new Error('BASE_URL must be an explicit disposable http://localhost:port or http://127.0.0.1:port gateway');
}

export default defineConfig({
  testDir: './tests/real',
  fullyParallel: false,
  forbidOnly: true,
  maxFailures: 1,
  retries: 0,
  workers: 1,
  timeout: 15 * 60_000,
  expect: { timeout: 60_000 },
  outputDir: `test-results/real/${phase}`,
  reporter: [
    ['list'],
    ['html', { outputFolder: `playwright-report/real/${phase}`, open: 'never' }],
    ['json', { outputFile: `test-results/real-${phase}.json` }],
  ],
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
    { name: 'real-settings-vpn', testMatch: /settings-vpn\.spec\.ts/ },
    { name: 'real-settings-profiles', testMatch: /settings-profiles\.spec\.ts/ },
    { name: 'real-settings-accounts', testMatch: /settings-accounts\.spec\.ts/ },
    { name: 'real-app-install', testMatch: /apps\.install\.spec\.ts/ },
    { name: 'real-app-upgrade', testMatch: /apps\.upgrade\.spec\.ts/ },
    { name: 'real-app-restart', testMatch: /apps\.restart\.spec\.ts/ },
    { name: 'real-app-uninstall', testMatch: /apps\.uninstall\.spec\.ts/ },
  ],
});
