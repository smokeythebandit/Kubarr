import { defineConfig, devices } from '@playwright/test';

// Intentionally independent of BASE_URL, live auth setup, and saved sessions.
export default defineConfig({
  testDir: './tests/browser',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: 0,
  workers: 2,
  reporter: [
    ['list'],
    ['json', { outputFile: 'test-results/browser-results.json' }],
    ['html', { outputFolder: 'playwright-report/browser', open: 'never' }],
  ],
  outputDir: 'test-results/browser',
  use: {
    baseURL: 'http://127.0.0.1:4174',
    storageState: { cookies: [], origins: [] },
    serviceWorkers: 'block',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  projects: [{ name: 'browser-chromium', use: { ...devices['Desktop Chrome'] } }],
  webServer: {
    command: 'node node_modules/vite/bin/vite.js --config vite.browser.config.ts',
    url: 'http://127.0.0.1:4174',
    reuseExistingServer: false,
    env: { VITE_API_URL: '/api' },
  },
});
