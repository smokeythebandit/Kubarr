import { defineConfig, devices } from '@playwright/test';

const baseURL = process.env.BASE_URL || 'http://localhost:8000';

export default defineConfig({
  testDir: './tests',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: [['html'], ['list']],

  use: {
    baseURL,
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
    video: process.env.CI ? 'off' : 'retain-on-failure',
    launchOptions: {
      slowMo: process.env.SLOW_MO ? parseInt(process.env.SLOW_MO) : 0,
      args: process.env.CI ? ['--disable-dev-shm-usage', '--disable-gpu'] : [],
    },
    viewport: { width: 1920, height: 1080 },
  },

  projects: [
    {
      name: 'bootstrap',
      testMatch: /bootstrap\.setup\.ts/,
    },
    {
      name: 'auth',
      testMatch: /auth\.setup\.ts/,
      dependencies: ['bootstrap'],
    },
    {
      name: 'chromium',
      use: {
        ...devices['Desktop Chrome'],
        storageState: 'tests/.auth/user.json',
        viewport: { width: 1920, height: 1080 },
      },
      dependencies: ['auth'],
    },
  ],

  timeout: process.env.CI ? 600000 : 30000,
  expect: {
    timeout: process.env.CI ? 30000 : 5000,
  },
});
