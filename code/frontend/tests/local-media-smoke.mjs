// Live, non-destructive smoke for the existing kubarr-local Immich and Frigate installs.
// Run from code/frontend: node tests/local-media-smoke.mjs
import assert from 'node:assert/strict';
import { readFileSync, statSync, writeFileSync } from 'node:fs';
import { chromium } from '@playwright/test';

const origin = 'http://127.0.0.1:18080';
const credentialsPath = '/home/benjamin/Projects/Kubarr/Kubarr/.local-cluster/app-browser-credentials.json';
assert.equal(statSync(credentialsPath).mode & 0o077, 0, 'local credentials must be private');
const kubarr = JSON.parse(readFileSync('/home/benjamin/Projects/Kubarr/Kubarr/.local-cluster/credentials.json', 'utf8'));
const credentials = JSON.parse(readFileSync(credentialsPath, 'utf8'));
const browser = await chromium.launch({ headless: true, args: ['--host-resolver-rules=MAP *.localhost 127.0.0.1'] });
const report = [];
// Screenshots are taken only after native authentication; hide any later-added media.
const screenshotOptions = page => ({ mask: [page.locator('img,video,canvas,[role="img"],[style*="background-image"]')] });

async function openApp(app) {
  const context = await browser.newContext({ locale: 'en-US', serviceWorkers: 'block' });
  const page = await context.newPage();
  page.setDefaultTimeout(15_000);
  await page.goto(`${origin}/login`);
  await page.getByLabel('Username').fill(kubarr.username);
  await page.getByLabel('Password').fill(kubarr.password);
  await Promise.all([
    page.waitForURL(url => url.pathname !== '/login'),
    page.getByRole('button', { name: 'Sign in', exact: true }).click(),
  ]);
  await page.goto(`${origin}/auth/host-transfer/start?app=${app}`, { waitUntil: 'domcontentloaded' });
  assert.equal(new URL(page.url()).hostname, `${app}.localhost`, `${app} host transfer failed`);
  return { context, page };
}

async function verify(app, login, home, secondary) {
  const { context, page } = await openApp(app);
  const result = { app, status: 'FAIL', screens: [], httpErrors: [], failedRequests: [], pageErrors: [] };
  try {
    await login(page);
    // Track the authenticated UI, excluding expected native pre-login 401 responses.
    page.on('response', response => {
      if (response.status() >= 400 && result.httpErrors.length < 20) {
        result.httpErrors.push({ status: response.status(), path: new URL(response.url()).pathname,
          type: response.request().resourceType() });
      }
    });
    page.on('requestfailed', request => {
      if (result.failedRequests.length < 20 && !/ERR_ABORTED/.test(request.failure()?.errorText || '')) {
        result.failedRequests.push(new URL(request.url()).pathname);
      }
    });
    page.on('pageerror', error => result.pageErrors.push(error.name));
    await home(page);
    assert.equal(await page.locator('input[type=password]').count(), 0, 'not past native login');
    result.screens.push(new URL(page.url()).pathname);
    await page.screenshot({ path: `/tmp/opencode/${app}-authenticated-home.png`, ...screenshotOptions(page) });
    await secondary(page);
    assert.equal(await page.locator('input[type=password]').count(), 0, 'secondary screen is a login');
    result.screens.push(new URL(page.url()).pathname);
    await page.screenshot({ path: `/tmp/opencode/${app}-authenticated-secondary.png`, ...screenshotOptions(page) });
    assert.equal(result.pageErrors.length, 0, 'uncaught app JavaScript error');
    assert.equal(result.failedRequests.length, 0, 'failed app network request');
    assert.equal(result.httpErrors.filter(x => x.type === 'script' || x.type === 'stylesheet' || x.status >= 500).length,
      0, 'asset or server HTTP error');
    result.status = 'PASS';
  } catch (error) {
    result.failure = error.name + ': ' + String(error.message).split('\n')[0].slice(0, 140);
  } finally {
    report.push(result);
    console.log(`${app}: ${result.status} ${result.screens.join(' -> ')}${result.failure ? ' (' + result.failure + ')' : ''}`);
    await context.close();
  }
}

try {
  await verify('immich', async page => {
    await page.getByLabel('Email').fill(credentials.immich.username);
    await page.getByLabel('Password').fill(credentials.immich.password);
    await page.getByRole('button', { name: 'Login', exact: true }).click();
    await page.waitForURL(url => url.pathname === '/photos');
  }, async page => {
    await page.getByRole('link', { name: 'Photos', exact: true }).waitFor();
    await page.getByText('Server Online').waitFor();
  }, async page => {
    await page.getByRole('link', { name: 'Explore', exact: true }).click();
    await page.waitForURL(url => url.pathname === '/explore');
    await page.getByRole('link', { name: 'Photos', exact: true }).waitFor();
  });

  await verify('frigate', async page => {
    await page.locator('input[name=user]').fill(credentials.frigate.username);
    await page.locator('input[name=password]').fill(credentials.frigate.initialPassword);
    await page.getByRole('button', { name: 'Login', exact: true }).click();
    await page.waitForURL(url => url.pathname === '/');
  }, async page => {
    await page.getByText('System is healthy').waitFor();
    await page.getByText('No Cameras Configured').waitFor();
  }, async page => {
    await page.goto('http://frigate.localhost:18080/system', { waitUntil: 'domcontentloaded' });
    await page.waitForURL(url => url.pathname === '/system');
    await page.getByText('System', { exact: true }).first().waitFor();
  });
} finally {
  await browser.close();
  writeFileSync('/tmp/opencode/local-media-browser-results.json', JSON.stringify(report, null, 2) + '\n', { mode: 0o600 });
}
if (report.some(item => item.status !== 'PASS')) process.exitCode = 1;
