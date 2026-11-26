import { test, expect, timestamp, admin } from './fixtures';
import type { Notification, NotificationChannel, NotificationEvent, NotificationRoute } from '../../src/api/notifications';

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
  api.get('/api/notifications/routes', [{ event_type: 'user_login', channels: { telegram: false, email: false, messagebird: false } }]);
  api.get('/api/notifications/inbox?limit=30&offset=0', { notifications: [], total: 0, unread: 0 });
});

test('empty popover closes on outside click and Escape, and links to the notifications page', async ({ page }) => {
  await page.goto('/settings?section=notifications');
  const bell = page.getByRole('button', { name: 'Notifications, 0 unread', exact: true });
  await bell.click();
  const dialog = page.getByRole('dialog', { name: 'Notifications' });
  await expect(dialog.getByText('Nothing here. Kubarr will tell you when something needs you.')).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Mark all read' })).toHaveCount(0);
  await expect(dialog.getByText('All caught up')).toBeVisible();
  await expect(dialog.getByRole('link', { name: 'Open notifications' })).toHaveAttribute('href', '/notifications');
  await page.getByRole('heading', { name: 'Channels', exact: true }).click({ position: { x: 5, y: 5 } });
  await expect(dialog).toHaveCount(0);
  await bell.click();
  await expect(dialog).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(dialog).toHaveCount(0);
  await expect(bell).toBeFocused();
});

for (const action of ['single', 'all'] as const) {
  test(`marks ${action} notification read and preserves it after reopening`, async ({ page, api }) => {
    let item = { ...notification };
    api.on('GET', '/api/notifications/inbox/count', () => ({ json: { count: item.read ? 0 : 1, critical: 0 } }));
    api.on('GET', '/api/notifications/inbox?limit=30&offset=0', () => ({ json: { notifications: [item], total: 1, unread: item.read ? 0 : 1 } }));
    const path = action === 'single' ? '/api/notifications/inbox/71/read' : '/api/notifications/inbox/read-all';
    api.on('POST', path, () => { item = { ...item, read: true }; return { json: {} }; });
    await page.goto('/settings?section=notifications');
    const bell = page.getByRole('button', { name: /^Notifications, \d+ unread/ });
    await expect(bell).toHaveAccessibleName('Notifications, 1 unread');
    await expect(bell).toHaveText('1');
    await bell.click();
    const dialog = page.getByRole('dialog', { name: 'Notifications' });
    await expect(dialog.getByRole('heading', { name: notification.title })).toBeVisible();
    await expect(dialog.getByText(notification.message, { exact: true })).toBeVisible();
    await dialog.getByRole('button', { name: action === 'single' ? `Mark read: ${notification.title}` : 'Mark all read', exact: true }).click();
    await expect(bell).toHaveText('');
    await expect(dialog.getByRole('button', { name: `Mark read: ${notification.title}` })).toHaveCount(0);
    await expect(dialog.getByRole('button', { name: 'Mark all read' })).toHaveCount(0);
    await bell.click();
    await bell.click();
    await expect(dialog.getByRole('heading', { name: notification.title })).toBeVisible();
    expect(item.read).toBe(true);
    expect(api.calls.filter(call => call === `POST ${path}`)).toHaveLength(1);
  });
}

test('a critical unread turns the badge red; dismissing it clears the badge and list', async ({ page, api }) => {
  const critical = { ...notification, severity: 'critical' as const };
  let deleted = false;
  api.on('GET', '/api/notifications/inbox/count', () => ({ json: deleted ? { count: 0, critical: 0, attention: 0 } : { count: 1, critical: 1, attention: 1 } }));
  api.on('GET', '/api/notifications/inbox?limit=30&offset=0', () => ({ json: deleted
    ? { notifications: [], total: 0, unread: 0 } : { notifications: [critical], total: 1, unread: 1 } }));
  api.on('DELETE', '/api/notifications/inbox/71', () => { deleted = true; return { json: {} }; });
  await page.goto('/settings?section=notifications');
  const bell = page.getByRole('button', { name: /^Notifications, \d+ unread/ });
  await expect(bell).toHaveAccessibleName('Notifications, 1 unread, 1 critical');
  await expect(bell.getByTestId('bell-badge')).toHaveAttribute('data-kind', 'crit');
  await bell.click();
  const dialog = page.getByRole('dialog', { name: 'Notifications' });
  await expect(dialog.getByText('Needs you')).toBeVisible();
  await dialog.getByRole('button', { name: `Dismiss: ${notification.title}` }).click();
  await expect(dialog.getByRole('heading', { name: notification.title })).toHaveCount(0);
  await expect(dialog.getByText('All caught up')).toBeVisible();
  await expect(bell).toHaveText('');
  await expect(bell).toHaveAccessibleName('Notifications, 0 unread');
  expect(api.calls.filter(call => call === 'DELETE /api/notifications/inbox/71')).toHaveLength(1);
});

test('the notifications page lists the inbox and filters by facet', async ({ page, api }) => {
  const update = { ...notification, id: 72, title: 'Sonarr update failed', event_type: 'app_update_failed', severity: 'warning' as const,
    message: 'App update failed: app=sonarr from=1 to=2' };
  api.get('/api/notifications/inbox/count', { count: 2, critical: 0, attention: 1 });
  api.get('/api/notifications/inbox?limit=50&offset=0', { notifications: [update, notification], total: 2, unread: 2 });
  await page.goto('/settings?section=notifications');
  await page.getByRole('button', { name: /^Notifications, \d+ unread/ }).click();
  await page.getByRole('dialog', { name: 'Notifications' }).getByRole('link', { name: 'Open notifications' }).click();
  await expect(page).toHaveURL(/\/notifications$/);
  await expect(page.getByRole('heading', { name: 'Everything Kubarr told you' })).toBeVisible();
  await expect(page.getByText('Needs you')).toBeVisible();
  await expect(page.getByText('sonarr', { exact: true })).toBeVisible();
  const filters = page.getByRole('navigation', { name: 'Filters' });
  await filters.getByRole('button', { name: /^Security/ }).click();
  await expect(page.getByRole('heading', { name: notification.title })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Sonarr update failed' })).toHaveCount(0);
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
  const card = page.getByTestId('channel-card-email');
  await expect(card.getByText('Not set up', { exact: true })).toBeVisible();
  await expect(card.getByRole('switch', { name: 'email notifications', exact: true })).not.toBeChecked();
  await expect(card.getByRole('button', { name: 'Send test', exact: true })).toHaveCount(0);
  await card.getByRole('button', { name: 'Set up', exact: true }).click();
  await card.getByPlaceholder('smtp.example.com', { exact: true }).fill('smtp.example.test');
  await card.getByPlaceholder('587', { exact: true }).fill('2525');
  await card.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(card.getByRole('button', { name: 'Edit', exact: true })).toBeVisible();
  await expect(card.getByText('Off', { exact: true })).toBeVisible();
  await card.getByRole('switch', { name: 'email notifications', exact: true }).click();
  await expect(card.getByRole('switch', { name: 'email notifications', exact: true })).toBeChecked();
  await expect(card.getByText('Working', { exact: true })).toBeVisible();
  await expect(card.getByRole('button', { name: 'Send test', exact: true })).toBeVisible();
  await card.getByRole('button', { name: 'Edit', exact: true }).click();
  await expect(card.getByPlaceholder('smtp.example.com', { exact: true })).toHaveValue('smtp.example.test');
  await expect(card.getByPlaceholder('587', { exact: true })).toHaveValue('2525');
  await card.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(card.getByPlaceholder('smtp.example.com', { exact: true })).toHaveCount(0);
  expect(updates).toEqual([{ config: { smtp_host: 'smtp.example.test', smtp_port: '2525' } }, { enabled: true }]);
});

test('a failed latest delivery marks the channel and both tab badges', async ({ page, api }) => {
  api.get('/api/notifications/channels', [{ channel_type: 'email', enabled: true, config: { smtp_host: 'smtp.example.test' }, created_at: timestamp, updated_at: timestamp }]);
  api.get('/api/notifications/logs?limit=50', { total: 1, logs: [{ id: 1, user_id: admin.id, channel_type: 'email', event_type: 'user_login',
    recipient: 'a***@example.test', status: 'failed', error_message: 'SMTP refused', created_at: timestamp }] });
  await page.goto('/settings?section=notifications');
  await expect(page.getByTestId('channel-card-email').getByText('Problem', { exact: true })).toBeVisible();
  await expect(page.getByRole('tab', { name: /^Channels\s*1$/ })).toBeVisible();
  await page.getByRole('tab', { name: /^Delivery history\s*1$/ }).click();
  await expect(page.getByText('Failed: SMTP refused')).toBeVisible();
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
  await page.getByRole('tab', { name: 'Delivery history' }).click();
  await expect(page.getByText('No notification logs yet.', { exact: true })).toBeVisible();
  await page.getByRole('tab', { name: 'What you get' }).click();
  await expect(page.getByTestId('category-row')).toHaveText('Security');
  const toggle = page.getByRole('switch', { name: 'User Login notifications', exact: true });
  await expect(toggle).toBeChecked();
  await toggle.click();
  await expect(toggle).not.toBeChecked();
  await page.getByRole('combobox', { name: 'User Login severity' }).selectOption('critical');
  await expect(page.getByRole('combobox', { name: 'User Login severity' })).toHaveValue('critical');
  await expect.poll(() => updates).toEqual([{ enabled: false }, { severity: 'critical' }]);
  await page.reload();
  await page.getByRole('tab', { name: 'What you get' }).click();
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
    await page.getByRole('button', { name: 'Send test', exact: true }).click();
    await expect(page.getByText('Please enter a test destination', { exact: true })).toBeVisible();
    expect(api.calls).not.toContain('POST /api/notifications/channels/email/test');
    await page.getByPlaceholder('test@example.com', { exact: true }).fill('recipient@example.test');
    if (success) {
      page.once('dialog', async alert => {
        expect(alert.message()).toBe('Test notification sent successfully!');
        await alert.accept();
      });
    }
    await page.getByRole('button', { name: 'Send test', exact: true }).click();
    if (!success) {
      await expect(page.getByText('Fixture SMTP rejected recipient', { exact: true })).toBeVisible();
    }
    await expect(page.getByRole('button', { name: 'Send test', exact: true })).toBeEnabled();
    expect(api.calls.filter(call => call === 'POST /api/notifications/channels/email/test')).toHaveLength(1);
  });
}

test('routes an event to a set-up channel and keeps the others locked', async ({ page, api }) => {
  let route: NotificationRoute = { event_type: 'user_login', channels: { telegram: false, email: false, messagebird: false } };
  const updates: unknown[] = [];
  api.on('GET', '/api/notifications/channels', () => ({
    json: ['email', 'telegram', 'messagebird'].map(channel_type => ({
      channel_type, enabled: channel_type === 'email', config: channel_type === 'email' ? { smtp_host: 'smtp.example.com' } : {},
      created_at: timestamp, updated_at: timestamp,
    } satisfies NotificationChannel)),
  }));
  api.on('GET', '/api/notifications/routes', () => ({ json: [route] }));
  api.on('PUT', '/api/notifications/routes/user_login', request => {
    const update = request.postDataJSON();
    updates.push(update);
    route = { ...route, channels: { ...route.channels, ...update.channels } };
    return { json: route };
  });
  await page.goto('/settings?section=notifications');
  await page.getByRole('tab', { name: 'What you get' }).click();
  await expect(page.getByRole('checkbox', { name: 'User Login in the bell' })).toBeDisabled();
  await expect(page.getByRole('checkbox', { name: 'User Login via Telegram' })).toBeDisabled();
  await expect(page.getByRole('checkbox', { name: 'User Login via Telegram' })).toHaveAttribute('title', 'Set up this channel first');
  const email = page.getByRole('checkbox', { name: 'User Login via Email' });
  await email.click();
  await expect(email).toBeChecked();
  await expect.poll(() => updates).toEqual([{ channels: { email: true } }]);
  await expect(page.getByRole('tab', { name: /^What you get\s*1$/ })).toBeVisible();
});
