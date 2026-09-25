import { test, expect } from './fixtures';

test('registration handles disabled public access without navigating to an admin page', async ({ page, api }) => {
  api.on('POST', '/auth/register', request => {
    expect(request.postDataJSON()).toEqual({ username: 'visitor', email: 'visitor@example.test', password: 'test-password', invite_code: null });
    return { status: 403, json: { detail: 'Open registration is disabled' } };
  });
  await page.goto('/login?register=1');
  await page.getByRole('textbox', { name: 'Username' }).fill('visitor');
  await page.getByRole('textbox', { name: 'Email' }).fill('visitor@example.test');
  await page.getByLabel('Password').fill('test-password');
  await page.getByRole('button', { name: 'Register' }).click();
  await expect(page.getByRole('alert')).toHaveText('Open registration is disabled; use an invite link.');
  expect(api.calls.filter(call => call.startsWith('POST '))).toEqual(['POST /auth/register']);
});

test('invited registration forwards only invite code and shows approved response', async ({ page, api }) => {
  api.on('POST', '/auth/register', request => {
    expect(request.postDataJSON()).toEqual({ username: 'visitor', email: 'visitor@example.test', password: 'test-password', invite_code: 'fixture-code' });
    return { json: { status: 'approved' } };
  });
  await page.goto('/login?register=1&invite=fixture-code');
  await page.getByRole('textbox', { name: 'Username' }).fill('visitor');
  await page.getByRole('textbox', { name: 'Email' }).fill('visitor@example.test');
  await page.getByLabel('Password').fill('test-password');
  await page.getByRole('button', { name: 'Register' }).click();
  await expect(page.getByRole('status')).toHaveText('Account created. You can sign in.');
  await expect(page.getByRole('link', { name: 'Sign in' })).toHaveAttribute('href', '/login');
});
