import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, within } from '@testing-library/react';
import type { ComponentProps } from 'react';
import type { NotificationChannel, NotificationLog } from '../../../api/notifications';
import { NotificationsTab } from './NotificationsTab';
import { channelStatus, unresolvedFailures } from './notificationChannelStatus';

const channel = (channel_type: string, enabled: boolean, config: Record<string, unknown> = {}): NotificationChannel => ({
  channel_type,
  enabled,
  config,
  created_at: '',
  updated_at: '',
});

let logId = 1;
const log = (channel_type: string, status: 'sent' | 'failed', created_at: string, error_message: string | null = null): NotificationLog => ({
  id: logId++,
  user_id: 1,
  channel_type,
  event_type: 'app_update_failed',
  recipient: 'r***@example.com',
  status,
  error_message,
  created_at,
});

const logs = [
  log('telegram', 'sent', '2026-11-25T10:00:00Z'),
  log('telegram', 'failed', '2026-11-24T10:00:00Z', 'old failure'),
  log('messagebird', 'failed', '2026-11-25T09:00:00Z', 'invalid API key'),
  log('messagebird', 'sent', '2026-11-24T09:00:00Z'),
];

function renderTab(overrides: Partial<ComponentProps<typeof NotificationsTab>> = {}) {
  const props: ComponentProps<typeof NotificationsTab> = {
    notificationChannels: [channel('email', false), channel('telegram', true, { bot_token: '***' }), channel('messagebird', true, { api_key: '***' })],
    notificationEvents: [
      { event_type: 'login_failed', enabled: true, severity: 'warning' },
      { event_type: 'app_installed', enabled: false, severity: 'info' },
      { event_type: 'system_setting_changed', enabled: true, severity: 'info' },
    ],
    notificationLogs: logs,
    notificationLogsTotal: 4,
    notificationLoading: false,
    testingChannel: null,
    testDestination: {},
    editingChannel: null,
    channelConfig: {},
    setTestDestination: vi.fn(),
    setEditingChannel: vi.fn(),
    setChannelConfig: vi.fn(),
    onToggleChannel: vi.fn(),
    onToggleEvent: vi.fn(),
    onUpdateEventSeverity: vi.fn(),
    onTestChannel: vi.fn(),
    onSaveChannelConfig: vi.fn(),
    getChannelIcon: () => null,
    getChannelConfigFields: () => [{ key: 'smtp_host', label: 'SMTP Host', type: 'text', placeholder: 'smtp.example.com' }],
    formatAuditAction: (a: string) => a.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase()),
    canManage: true,
    ...overrides,
  };
  render(<NotificationsTab {...props} />);
  return props;
}

describe('channelStatus', () => {
  const tg = channel('telegram', true, { bot_token: '***' });
  const sms = channel('messagebird', true, { api_key: '***' });

  it('uses the latest delivery for the channel when logs are available', () => {
    expect(channelStatus(tg, logs, true).status).toBe('working');
    expect(channelStatus(sms, logs, true)).toEqual({ status: 'problem', detail: 'Last delivery failed: invalid API key' });
    expect(channelStatus(channel('email', true, { smtp_host: 'x' }), logs, true)).toEqual({ status: 'working', detail: 'No failed deliveries' });
  });

  it('shows a neutral Enabled without delivery logs (no audit.view)', () => {
    expect(channelStatus(sms, [], false)).toEqual({ status: 'enabled', detail: 'Delivery history needs audit access' });
    expect(channelStatus(tg, logs, false).status).toBe('enabled');
  });

  it('needs saved settings before anything else', () => {
    expect(channelStatus(channel('email', true), logs, true).status).toBe('unset');
    expect(channelStatus(channel('email', true), [], false).status).toBe('unset');
    expect(channelStatus(channel('email', false), logs, true).status).toBe('unset');
    expect(channelStatus(channel('email', false, { smtp_host: 'x' }), logs, true).status).toBe('off');
  });

  it('lets a newer test result decide', () => {
    expect(channelStatus(sms, logs, true, { success: true, error: null, at: '2026-11-25T11:00:00Z' }).status).toBe('working');
    expect(channelStatus(tg, logs, true, { success: false, error: 'chat not found', at: '2026-11-25T11:00:00Z' })).toEqual({
      status: 'problem',
      detail: 'Test failed: chat not found',
    });
    expect(channelStatus(sms, logs, true, { success: true, error: null, at: '2026-11-20T00:00:00Z' }).status).toBe('problem');
    expect(channelStatus(sms, [], false, { success: false, error: null, at: '2026-11-25T11:00:00Z' }).status).toBe('problem');
  });
});

describe('unresolvedFailures', () => {
  it('counts only failures newer than the latest success on that channel', () => {
    expect(unresolvedFailures(logs)).toBe(1);
    expect(unresolvedFailures([...logs, log('messagebird', 'sent', '2026-11-25T12:00:00Z')])).toBe(0);
    expect(unresolvedFailures([log('email', 'failed', '2026-11-25T12:00:00Z'), log('email', 'failed', '2026-11-25T13:00:00Z')])).toBe(2);
  });
});

describe('NotificationsTab', () => {
  it('shows tab badges for channel problems and failed deliveries', () => {
    renderTab();
    const tabs = screen.getByRole('tablist', { name: 'Notification settings' });
    const channels = within(tabs).getByRole('tab', { name: /Channels/ });
    const history = within(tabs).getByRole('tab', { name: /Delivery history/ });
    expect(within(channels).getByTestId('count-badge')).toHaveTextContent('1');
    expect(within(channels).getByTestId('count-badge')).toHaveAttribute('data-kind', 'crit');
    expect(within(history).getByTestId('count-badge')).toHaveTextContent('1');
    expect(within(history).getByTestId('count-badge')).toHaveAttribute('data-kind', 'crit');
    expect(within(within(tabs).getByRole('tab', { name: 'What you get' })).queryByTestId('count-badge')).toBeNull();
    expect(within(tabs).queryByRole('tab', { name: /Quiet hours/ })).toBeNull();
  });

  it('omits badges when nothing failed and hides history without audit.view', () => {
    renderTab({ notificationLogs: [], canViewHistory: false });
    expect(screen.queryAllByTestId('count-badge')).toHaveLength(0);
    expect(screen.queryByRole('tab', { name: /Delivery history/ })).toBeNull();
    // Without logs nothing can be called Working.
    expect(within(screen.getByTestId('channel-card-telegram')).getByText('Enabled')).toBeInTheDocument();
    expect(within(screen.getByTestId('channel-card-telegram')).queryByText('Working')).toBeNull();
  });

  it('clears the history badge once a later delivery succeeds and shows test results', () => {
    renderTab({
      notificationLogs: [...logs, log('messagebird', 'sent', '2026-11-25T12:00:00Z')],
      testResults: { telegram: { success: false, error: 'chat not found', at: '2026-11-26T00:00:00Z' } },
    });
    expect(within(screen.getByRole('tab', { name: /Delivery history/ })).queryByTestId('count-badge')).toBeNull();
    expect(within(screen.getByTestId('channel-card-messagebird')).getByText('Working')).toBeInTheDocument();
    expect(within(screen.getByTestId('channel-card-telegram')).getByText('Test failed: chat not found')).toBeInTheDocument();
  });

  it('renders channel cards with status, edit and send test', () => {
    const props = renderTab();
    expect(within(screen.getByTestId('channel-card-bell')).getByText('Working')).toBeInTheDocument();
    expect(within(screen.getByTestId('channel-card-email')).getByText('Not set up')).toBeInTheDocument();
    expect(within(screen.getByTestId('channel-card-telegram')).getByText('Working')).toBeInTheDocument();
    const sms = screen.getByTestId('channel-card-messagebird');
    expect(within(sms).getByText('Problem')).toBeInTheDocument();
    expect(within(sms).getByText('Last delivery failed: invalid API key')).toBeInTheDocument();

    // Disabled channels can be set up but not tested.
    const email = screen.getByTestId('channel-card-email');
    expect(within(email).queryByRole('button', { name: 'Send test' })).toBeNull();
    fireEvent.click(within(email).getByRole('button', { name: 'Set up' }));
    expect(props.setEditingChannel).toHaveBeenCalledWith('email');

    fireEvent.click(within(sms).getByRole('button', { name: 'Send test' }));
    expect(props.onTestChannel).toHaveBeenCalledWith('messagebird');
    fireEvent.click(within(sms).getByRole('switch', { name: 'messagebird notifications' }));
    expect(props.onToggleChannel).toHaveBeenCalledWith('messagebird', false);
  });

  it('opens the existing config form when editing', () => {
    const props = renderTab({ editingChannel: 'telegram' });
    const card = screen.getByTestId('channel-card-telegram');
    expect(within(card).getByPlaceholderText('smtp.example.com')).toBeInTheDocument();
    fireEvent.click(within(card).getByRole('button', { name: 'Save' }));
    expect(props.onSaveChannelConfig).toHaveBeenCalledWith('telegram');
  });

  it('groups events by category with severity and an On toggle', () => {
    const props = renderTab();
    const tab = screen.getByRole('tab', { name: 'What you get' });
    fireEvent.click(tab);
    expect(tab).toHaveAttribute('aria-selected', 'true');
    const panel = screen.getByRole('tabpanel');
    expect(within(panel).getAllByRole('columnheader').map((h) => h.textContent)).toEqual(['Event', 'Severity', 'On', 'Bell', 'Telegram', 'Email ·', 'SMS']);
    expect(within(panel).getAllByTestId('category-row').map((h) => h.textContent)).toEqual(['Apps', 'Security', 'System']);
    fireEvent.change(within(panel).getByRole('combobox', { name: 'Login Failed severity' }), { target: { value: 'critical' } });
    expect(props.onUpdateEventSeverity).toHaveBeenCalledWith('login_failed', 'critical');
    const toggle = within(panel).getByRole('switch', { name: 'App Installed notifications' });
    expect(toggle).toHaveAttribute('aria-checked', 'false');
    fireEvent.click(toggle);
    expect(props.onToggleEvent).toHaveBeenCalledWith('app_installed', true);
  });

  const routes = [
    { event_type: 'login_failed', channels: { telegram: true, email: true, messagebird: true } },
    { event_type: 'app_installed', channels: { telegram: false, email: false, messagebird: false } },
  ];

  it('routes each event to channels and badges the number of routes', () => {
    const onToggleRoute = vi.fn();
    renderTab({ notificationRoutes: routes, onToggleRoute });
    // Email is not set up, so its route does not count: Telegram + SMS for login_failed.
    const tab = screen.getByRole('tab', { name: /What you get/ });
    expect(within(tab).getByTestId('count-badge')).toHaveTextContent('2');
    expect(within(tab).getByTestId('count-badge')).toHaveAttribute('data-kind', 'neutral');
    fireEvent.click(tab);
    const panel = screen.getByRole('tabpanel');

    const telegram = within(panel).getByRole('checkbox', { name: 'Login Failed via Telegram' });
    expect(telegram).toBeChecked();
    fireEvent.click(telegram);
    expect(onToggleRoute).toHaveBeenCalledWith('login_failed', 'telegram', false);

    const sms = within(panel).getByRole('checkbox', { name: 'App Installed via SMS' });
    expect(sms).not.toBeChecked();
    fireEvent.click(sms);
    expect(onToggleRoute).toHaveBeenCalledWith('app_installed', 'messagebird', true);

    // An event with no route entry is off everywhere but the bell.
    expect(within(panel).getByRole('checkbox', { name: 'System Setting Changed via Telegram' })).not.toBeChecked();
  });

  it('keeps the bell always on and channels without settings disabled', () => {
    const onToggleRoute = vi.fn();
    renderTab({ notificationRoutes: routes, onToggleRoute });
    fireEvent.click(screen.getByRole('tab', { name: /What you get/ }));
    const panel = screen.getByRole('tabpanel');

    const bell = within(panel).getByRole('checkbox', { name: 'App Installed in the bell' });
    expect(bell).toBeChecked();
    expect(bell).toBeDisabled();
    expect(bell).toHaveAttribute('title', 'The bell always shows every enabled event');

    const email = within(panel).getByRole('checkbox', { name: 'Login Failed via Email' });
    expect(email).toBeDisabled();
    expect(email).not.toBeChecked();
    expect(email).toHaveAttribute('title', 'Set up this channel first');
    fireEvent.click(email);
    expect(onToggleRoute).not.toHaveBeenCalled();
  });

  it('makes routes read-only without settings.manage', () => {
    renderTab({ notificationRoutes: routes, onToggleRoute: vi.fn(), canManage: false });
    fireEvent.click(screen.getByRole('tab', { name: /What you get/ }));
    expect(screen.getByRole('checkbox', { name: 'Login Failed via Telegram' })).toBeDisabled();
  });

  it('shows delivery history with status pills', () => {
    renderTab();
    fireEvent.click(screen.getByRole('tab', { name: /Delivery history/ }));
    const panel = screen.getByRole('tabpanel');
    expect(within(panel).getAllByText('Delivered')).toHaveLength(2);
    expect(within(panel).getByText('Failed: invalid API key')).toBeInTheDocument();
  });

  it('moves between tabs with the arrow keys', () => {
    renderTab();
    const channels = screen.getByRole('tab', { name: /Channels/ });
    channels.focus();
    fireEvent.keyDown(channels, { key: 'ArrowRight' });
    expect(screen.getByRole('tab', { name: 'What you get' })).toHaveFocus();
    expect(screen.getByRole('tab', { name: 'What you get' })).toHaveAttribute('aria-selected', 'true');
    fireEvent.keyDown(screen.getByRole('tab', { name: 'What you get' }), { key: 'End' });
    expect(screen.getByRole('tab', { name: /Delivery history/ })).toHaveFocus();
  });

  it('is read-only without settings.manage', () => {
    renderTab({ canManage: false });
    expect(screen.queryByRole('button', { name: 'Edit' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Send test' })).toBeNull();
    expect(screen.getByRole('switch', { name: 'telegram notifications' })).toBeDisabled();
  });
});
