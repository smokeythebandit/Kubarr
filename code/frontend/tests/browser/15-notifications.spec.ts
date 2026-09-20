import { test, expect, timestamp } from './fixtures';
import type { Notification, NotificationChannel, NotificationEvent } from '../../src/api/notifications';

const notification: Notification = {
  id: 71, title: 'Fixture login', message: 'Signed in from the browser fixture',
  event_type: 'user_login', severity: 'info', read: false, created_at: timestamp,
};

test.beforeEach(async ({ api }) => {
  api.get('/api/notifications/channels', ['email', 'telegram', 'messagebird'].map(channel_type => ({
    channel_type, enabled: channel_type === 'email', config: {}, created_at: timestamp, updated_at: timestamp,
  } satisfies NotificationChannel)));
  api.get('/api/notifications/events', [{ event_type: 'user_login', severity: 'info', enabled: true }]);
  api.get('/api/notifications/logs?limit=50', { logs: [], total: 0 });
  api.get('/api/notifications/inbox?limit=10&offset=0', { notifications: [], total: 0, unread: 0 });
});

test('empty inbox opens and closes on outside click, with a preferences link', async ({ page }) => {
  await page.goto('/settings?section=notifications');
  const bell = page.getByTitle('Notifications', { exact: true });
  await bell.click();
  const dropdown = bell.locator('..').getByRole('heading', { name: 'Notifications', exact: true }).locator('..').locator('..');
  await expect(dropdown).toHaveCSS('opacity', '1');
  await expect(dropdown.getByText('No notifications', { exact: true })).toBeVisible();
  await expect(dropdown.getByRole('button', { name: 'Mark all as read' })).toHaveCount(0);
  await expect(dropdown.getByRole('link', { name: 'Notification preferences' })).toHaveAttribute('href', '/account');
  await page.getByRole('heading', { name: 'Notification Channels', exact: true }).click({ position: { x: 5, y: 5 } });
  await expect(dropdown).toHaveCSS('opacity', '0');
  await expect(dropdown).toHaveCSS('pointer-events', 'none');
});

for (const action of ['single', 'all'] as const) {
  test(`marks ${action} notification read and preserves it after reopening`, async ({ page, api }) => {
    let item = { ...notification };
    api.on('GET', '/api/notifications/inbox/count', () => ({ json: { count: item.read ? 0 : 1 } }));
    api.on('GET', '/api/notifications/inbox?limit=10&offset=0', () => ({ json: { notifications: [item], total: 1, unread: item.read ? 0 : 1 } }));
    const path = action === 'single' ? '/api/notifications/inbox/71/read' : '/api/notifications/inbox/read-all';
    api.on('POST', path, () => { item = { ...item, read: true }; return { json: {} }; });
    await page.goto('/settings?section=notifications');
    const bell = page.getByTitle('Notifications', { exact: true });
    await expect(bell).toHaveText('1');
    await bell.click();
    await expect(page.getByRole('heading', { name: notification.title })).toBeVisible();
    await expect(page.getByText(notification.message, { exact: true })).toBeVisible();
    await page.getByRole('button', { name: action === 'single' ? 'Mark as read' : 'Mark all as read', exact: true }).click();
    await expect(bell).toHaveText('');
    await expect(page.getByTitle('Mark as read', { exact: true })).toHaveCount(0);
    await expect(page.getByRole('button', { name: 'Mark all as read' })).toHaveCount(0);
    await bell.click();
    await bell.click();
    await expect(page.getByRole('heading', { name: notification.title })).toBeVisible();
    expect(item.read).toBe(true);
    expect(api.calls.filter(call => call === `POST ${path}`)).toHaveLength(1);
  });
}

test('deleting the last unread notification clears the badge and inbox', async ({ page, api }) => {
  api.get('/api/notifications/inbox/count', { count: 1 });
  api.get('/api/notifications/inbox?limit=10&offset=0', { notifications: [notification], total: 1, unread: 1 });
  api.on('DELETE', '/api/notifications/inbox/71', () => ({ json: {} }));
  await page.goto('/settings?section=notifications');
  const bell = page.getByTitle('Notifications', { exact: true });
  await bell.click();
  await page.getByTitle('Delete', { exact: true }).click();
  await expect(page.getByRole('heading', { name: notification.title })).toHaveCount(0);
  await expect(page.getByText('No notifications', { exact: true })).toBeVisible();
  await expect(bell).toHaveText('');
  expect(api.calls.filter(call => call === 'DELETE /api/notifications/inbox/71')).toHaveLength(1);
});

test('channel configuration persists and enabled state controls test delivery controls', async ({ page, api }) => {
  let channel: NotificationChannel = { channel_type: 'email', enabled: false, config: {}, created_at: timestamp, updated_at: timestamp };
  api.on('GET', '/api/notifications/channels', () => ({ json: [channel] }));
  const updates: unknown[] = [];
  api.on('PUT', '/api/notifications/channels/email', request => {
    const update = request.postDataJSON();
    updates.push(update);
    channel = { ...channel, ...update };
    return { json: channel };
  });
  await page.goto('/settings?section=notifications');
  await expect(page.getByRole('switch', { name: 'email notifications', exact: true })).not.toBeChecked();
  await expect(page.getByRole('button', { name: 'Test', exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Configure', exact: true }).click();
  await page.getByPlaceholder('smtp.example.com', { exact: true }).fill('smtp.example.test');
  await page.getByPlaceholder('587', { exact: true }).fill('2525');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Configure', exact: true })).toBeVisible();
  await page.getByRole('switch', { name: 'email notifications', exact: true }).click();
  await expect(page.getByRole('switch', { name: 'email notifications', exact: true })).toBeChecked();
  await expect(page.getByRole('button', { name: 'Test', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Configure', exact: true }).click();
  await expect(page.getByPlaceholder('smtp.example.com', { exact: true })).toHaveValue('smtp.example.test');
  await expect(page.getByPlaceholder('587', { exact: true })).toHaveValue('2525');
  await page.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(page.getByPlaceholder('smtp.example.com', { exact: true })).toHaveCount(0);
  expect(updates).toEqual([{ config: { smtp_host: 'smtp.example.test', smtp_port: '2525' } }, { enabled: true }]);
});

test('supported channels and event changes have exact state and payloads', async ({ page, api }) => {
  let event: NotificationEvent = { event_type: 'user_login', enabled: true, severity: 'info' };
  const updates: unknown[] = [];
  api.on('GET', '/api/notifications/events', () => ({ json: [event] }));
  api.on('PUT', '/api/notifications/events/user_login', request => {
    const update = request.postDataJSON();
    updates.push(update);
    event = { ...event, ...update };
    return { json: event };
  });
  await page.goto('/settings?section=notifications');
  for (const name of ['email', 'telegram', 'messagebird']) {
    await expect(page.getByRole('switch', { name: `${name} notifications`, exact: true })).toBeVisible();
  }
  await expect(page.getByText('signal', { exact: true })).toHaveCount(0);
  await expect(page.getByText('No notification logs yet.', { exact: true })).toBeVisible();
  const toggle = page.getByRole('switch', { name: 'User Login notifications', exact: true });
  await expect(toggle).toBeChecked();
  await toggle.click();
  await expect(toggle).not.toBeChecked();
  await page.getByRole('combobox', { name: 'User Login severity' }).selectOption('critical');
  await expect(page.getByRole('combobox', { name: 'User Login severity' })).toHaveValue('critical');
  await expect.poll(() => updates).toEqual([{ enabled: false }, { severity: 'critical' }]);
  await page.reload();
  await expect(toggle).not.toBeChecked();
  await expect(page.getByRole('combobox', { name: 'User Login severity' })).toHaveValue('critical');
});

for (const success of [true, false]) {
  test(`test delivery validates destination and reports ${success ? 'success' : 'failure'}`, async ({ page, api }) => {
    api.on('POST', '/api/notifications/channels/email/test', request => {
      expect(request.postDataJSON()).toEqual({ destination: 'recipient@example.test' });
      return { json: { success, error: success ? null : 'Fixture SMTP rejected recipient' } };
    });
    await page.goto('/settings?section=notifications');
    await page.getByRole('button', { name: 'Test', exact: true }).click();
    await expect(page.getByText('Please enter a test destination', { exact: true })).toBeVisible();
    expect(api.calls).not.toContain('POST /api/notifications/channels/email/test');
    await page.getByPlaceholder('test@example.com', { exact: true }).fill('recipient@example.test');
    const dialog = success ? page.waitForEvent('dialog') : undefined;
    await page.getByRole('button', { name: 'Test', exact: true }).click();
    if (dialog) {
      const alert = await dialog;
      expect(alert.message()).toBe('Test notification sent successfully!');
      await alert.accept();
    } else {
      await expect(page.getByText('Fixture SMTP rejected recipient', { exact: true })).toBeVisible();
    }
    await expect(page.getByRole('button', { name: 'Test', exact: true })).toBeEnabled();
    expect(api.calls.filter(call => call === 'POST /api/notifications/channels/email/test')).toHaveLength(1);
  });
}
