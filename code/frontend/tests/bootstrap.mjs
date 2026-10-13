// Deploy the release E2E stack through the same browser wizard used by operators.
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { chromium, expect } from '@playwright/test';

const { KIND_CLUSTER, TEST_USERNAME, TEST_PASSWORD } = process.env;
if (!KIND_CLUSTER || !TEST_USERNAME || !TEST_PASSWORD) {
  throw new Error('KIND_CLUSTER, TEST_USERNAME and TEST_PASSWORD are required');
}

const env = { ...process.env };
// CI connects over loopback and does not need desktop browser auto-launch.
for (const key of ['DISPLAY', 'WAYLAND_DISPLAY', 'SSH_CONNECTION', 'SSH_CLIENT', 'SSH_TTY']) {
  delete env[key];
}
const cli = spawn(process.env.KUBARR_CLI || 'kubarr',
  ['bootstrap'], { env, stdio: ['ignore', 'pipe', 'inherit'] });
const exited = once(cli, 'exit');
// Register immediately so a spawn failure cannot become an unhandled rejection.
exited.catch(() => {});
let browser;
let page;

try {
  const invitation = await new Promise((resolve, reject) => {
    let output = '';
    const timer = setTimeout(() => onError(new Error('Wizard did not start within 30 seconds')), 30_000);
    const cleanup = () => {
      clearTimeout(timer);
      cli.stdout.off('data', onData);
      cli.off('error', onError);
      cli.off('exit', onExit);
    };
    const onError = error => { cleanup(); reject(error); };
    const onExit = () => onError(new Error('Wizard exited before publishing its invitation'));
    const onData = chunk => {
      output = (output + chunk).slice(-8192);
      const match = output.match(/(http:\/\/127\.0\.0\.1:\d+\/#[a-f0-9]+)\s/);
      if (match) { cleanup(); resolve(match[1]); }
    };
    cli.stdout.on('data', onData);
    cli.once('error', onError);
    cli.once('exit', onExit);
  });
  // Drain subsequent output without exposing the one-use invitation in CI logs.
  cli.stdout.resume();
  browser = await chromium.launch();
  page = await browser.newPage();
  page.setDefaultTimeout(120_000);
  await page.goto(invitation);
  await expect(page.locator('#wizard')).toBeVisible();
  const context = page.locator('#context-tiles label').filter({
    has: page.locator('strong', { hasText: ` · kind-${KIND_CLUSTER}` }),
  });
  await expect(context).toHaveCount(1);
  await context.click();
  await expect(context.locator('input')).toBeChecked();
  await page.locator('#next').click(); // Waits for required cluster checks.
  await page.locator('input[name="access_mode"][value="port-forward"]').check();
  await page.locator('#next').click();
  await page.locator('#storage_size').fill(process.env.KUBARR_BOOTSTRAP_STORAGE_SIZE || '5Gi');
  await page.locator('#storage_class').fill(process.env.KUBARR_BOOTSTRAP_STORAGE_CLASS || 'standard');
  await page.locator('#next').click();
  await page.locator('#admin_username').fill(TEST_USERNAME);
  await page.locator('#admin_email').fill(process.env.KUBARR_BOOTSTRAP_ADMIN_EMAIL || 'admin@example.com');
  await page.locator('#admin_password').fill(TEST_PASSWORD);
  await page.locator('#admin_confirm').fill(TEST_PASSWORD);
  await page.locator('#next').click();
  await page.locator('#start').click();
  await page.waitForFunction(() =>
    !document.getElementById('step-6').classList.contains('hidden') ||
    document.getElementById('phase').textContent === 'Installation failed',
  null, { timeout: 30 * 60_000 });
  await expect(page.locator('#step-6')).toBeVisible();
  console.log('Web wizard installation succeeded');
} catch (error) {
  if (page) {
    for (const selector of ['#error', '#checks', '#events', '#result']) {
      console.error(await page.locator(selector).textContent().catch(() => ''));
    }
  }
  throw error;
} finally {
  await browser?.close();
  if (cli.exitCode === null && cli.signalCode === null) {
    cli.kill('SIGINT');
    const timer = setTimeout(() => cli.kill('SIGKILL'), 30_000);
    try { await exited; } finally { clearTimeout(timer); }
  }
}
