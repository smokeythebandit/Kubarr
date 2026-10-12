// Non-destructive real-browser check for the existing local development apps.
// Run from code/frontend: node tests/local-development-smoke.mjs
import assert from 'node:assert/strict';
import { readFileSync, statSync, writeFileSync } from 'node:fs';
import { chromium } from '@playwright/test';

const base = 'http://127.0.0.1:18080';
const privateFile = '/home/benjamin/Projects/Kubarr/Kubarr/.local-cluster/app-browser-credentials.json';
assert.equal(statSync(privateFile).mode & 0o077, 0, 'app credentials must remain private');
const kubarr = JSON.parse(readFileSync('/home/benjamin/Projects/Kubarr/Kubarr/.local-cluster/credentials.json', 'utf8'));
const accounts = JSON.parse(readFileSync(privateFile, 'utf8'));
const browser = await chromium.launch({ headless: true });
const report = [];

async function pageFor(app) {
  const context = await browser.newContext({ locale: 'en-US', serviceWorkers: 'block' });
  const page = await context.newPage();
  page.setDefaultTimeout(12_000);
  await page.goto(`${base}/login`);
  await page.getByLabel('Username').fill(kubarr.username);
  await page.getByLabel('Password').fill(kubarr.password);
  await Promise.all([
    page.waitForURL(url => url.pathname !== '/login'),
    page.getByRole('button', { name: 'Sign in', exact: true }).click(),
  ]);
  await page.goto(`${base}/${app}/login`, { waitUntil: 'domcontentloaded' });
  return { page, context };
}

async function run(app, verify) {
  const { page, context } = await pageFor(app);
  const result = { app, status: 'FAIL', screens: [], httpErrors: [], failedRequests: [], pageErrors: [] };
  try {
    await verify(page, result);
    assert.equal(result.pageErrors.length, 0, 'uncaught JavaScript exception');
    assert.equal(result.failedRequests.length, 0, 'failed browser requests');
    assert.equal(result.httpErrors.filter(e => e.type === 'script' || e.type === 'stylesheet' || e.status >= 500).length,
      0, 'broken asset or server request');
    result.status = 'PASS';
  } catch (error) {
    result.failure = error.name + ': ' + String(error.message).split('\n')[0].slice(0, 130);
  } finally {
    console.log(`${app}: ${result.status} ${result.screens.join(' -> ')}${result.failure ? ' (' + result.failure + ')' : ''}`);
    report.push(result);
    await context.close();
  }
}

function observe(page, result) {
  page.on('response', response => {
    if (response.status() >= 400 && result.httpErrors.length < 16) {
      result.httpErrors.push({ status: response.status(), path: new URL(response.url()).pathname,
        type: response.request().resourceType() });
    }
  });
  page.on('requestfailed', request => {
    if (!/ERR_ABORTED/.test(request.failure()?.errorText || '') && result.failedRequests.length < 12) {
      const url = new URL(request.url());
      result.failedRequests.push({ origin: url.origin, path: url.pathname });
    }
  });
  page.on('pageerror', error => result.pageErrors.push(error.name));
}

async function screenshot(page, app, name) {
  assert.equal(await page.locator('input[type=password]').count(), 0, 'refusing screenshot of login/password form');
  await page.screenshot({ path: `/tmp/opencode/${app}-${name}.png`,
    mask: [page.locator('img,video,canvas,[role="img"],[style*="background-image"]')] });
}

try {
  await run('redmine', async (page, result) => {
    await page.locator('input[name=username]').fill(accounts.redmine.username);
    await page.locator('input[name=password]').fill(accounts.redmine.password);
    await page.locator('input[name=login]').click();
    await page.getByRole('link', { name: 'My page' }).waitFor();
    assert.notEqual(new URL(page.url()).pathname, '/redmine/my/password', 'default password still requires rotation');
    observe(page, result);
    await page.goto(`${base}/redmine/my/page`);
    await page.getByRole('link', { name: 'Administration' }).waitFor();
    result.screens.push(new URL(page.url()).pathname);
    await screenshot(page, 'redmine', 'my-page');
    await page.getByRole('link', { name: 'Projects', exact: true }).first().click();
    await page.waitForURL(url => url.pathname === '/redmine/projects');
    await page.getByText('Projects', { exact: true }).first().waitFor();
    result.screens.push(new URL(page.url()).pathname);
    await screenshot(page, 'redmine', 'projects');
  });

  await run('jenkins', async (page, result) => {
    await page.locator('input[name=j_username]').fill(accounts.jenkins.username);
    await page.locator('input[name=j_password]').fill(accounts.jenkins.password);
    await page.getByRole('button', { name: 'Sign in', exact: true }).click();
    await page.getByRole('link', { name: 'Manage Jenkins' }).waitFor();
    observe(page, result);
    result.screens.push(new URL(page.url()).pathname);
    await screenshot(page, 'jenkins', 'dashboard');
    await page.getByRole('link', { name: 'Manage Jenkins' }).click();
    await page.waitForURL(url => url.pathname === '/jenkins/manage' || url.pathname === '/jenkins/manage/');
    await page.getByText('Manage Jenkins', { exact: true }).first().waitFor();
    result.screens.push(new URL(page.url()).pathname);
    await screenshot(page, 'jenkins', 'manage');
  });

  await run('scm-manager', async (page, result) => {
    await page.goto(`${base}/scm-manager/`, { waitUntil: 'domcontentloaded' });
    observe(page, result);
    await page.waitForTimeout(2500);
    result.screens.push(new URL(page.url()).pathname);
    assert.equal(result.failedRequests.length, 0, 'SCM-Manager requests the gateway without port 18080');
    await page.getByRole('textbox', { name: /user/i }).waitFor({ timeout: 5000 });
    throw new Error('SCM first-run admin workflow needs explicit browser selectors');
  });
} finally {
  await browser.close();
  writeFileSync('/tmp/opencode/local-development-browser-results.json', JSON.stringify(report, null, 2) + '\n',
    { mode: 0o600 });
}
if (report.some(item => item.status !== 'PASS')) process.exitCode = 1;
