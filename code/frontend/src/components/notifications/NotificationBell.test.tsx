import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';
import type { Notification } from '../../api/notifications';
import { NotificationBell } from './NotificationBell';

const mocks = vi.hoisted(() => ({
  getUnreadCounts: vi.fn(),
  getInbox: vi.fn(),
  markAsRead: vi.fn(),
  markAllAsRead: vi.fn(),
  deleteNotification: vi.fn(),
}));

vi.mock('../../api/notifications', () => ({ notificationsApi: mocks }));
vi.mock('../../contexts/AuthContext', () => ({ useAuth: () => ({ isAuthenticated: true }) }));

const at = (day: number, hour: number, minute = 0) => new Date(2026, 10, day, hour, minute).toISOString();

let nextId = 1;
const make = (o: Partial<Notification> = {}): Notification => ({
  id: nextId++,
  title: `Notification ${nextId}`,
  message: 'Something happened',
  event_type: 'login',
  severity: 'info',
  read: true,
  created_at: at(25, 9),
  ...o,
});

function renderBell() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <NotificationBell />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

function inbox(notifications: Notification[]) {
  mocks.getInbox.mockResolvedValue({
    notifications,
    total: notifications.length,
    unread: notifications.filter((n) => !n.read).length,
  });
}

describe('NotificationBell', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(new Date(2026, 10, 25, 10, 0));
    Object.values(mocks).forEach((m) => m.mockReset());
    mocks.markAsRead.mockResolvedValue(undefined);
    mocks.markAllAsRead.mockResolvedValue(undefined);
    mocks.deleteNotification.mockResolvedValue(undefined);
    inbox([]);
  });
  afterEach(() => vi.useRealTimers());

  it('shows an accent badge and counts in the label when nothing critical is unread', async () => {
    mocks.getUnreadCounts.mockResolvedValue({ count: 3, critical: 0 });
    renderBell();
    const bell = await screen.findByRole('button', { name: 'Notifications, 3 unread' });
    const badge = within(bell).getByTestId('bell-badge');
    expect(badge).toHaveTextContent('3');
    expect(badge).toHaveAttribute('data-kind', 'calm');
    expect(badge).toHaveClass('bg-blue-600');
  });

  it('turns the badge red when a critical notification is unread and caps at 99+', async () => {
    mocks.getUnreadCounts.mockResolvedValue({ count: 120, critical: 2 });
    renderBell();
    const bell = await screen.findByRole('button', { name: 'Notifications, 120 unread, 2 critical' });
    const badge = within(bell).getByTestId('bell-badge');
    expect(badge).toHaveTextContent('99+');
    expect(badge).toHaveAttribute('data-kind', 'crit');
    expect(badge).toHaveClass('bg-red-600');
  });

  it('hides the badge when everything is read', async () => {
    mocks.getUnreadCounts.mockResolvedValue({ count: 0, critical: 0 });
    renderBell();
    const bell = await screen.findByRole('button', { name: 'Notifications, 0 unread' });
    expect(within(bell).queryByTestId('bell-badge')).toBeNull();
  });

  it('shows filter pills with counts on one non-wrapping row', async () => {
    mocks.getUnreadCounts.mockResolvedValue({ count: 3, critical: 1 });
    inbox([
      make({ severity: 'critical', read: false, event_type: 'app_update_rollback_failed' }),
      make({ severity: 'warning', read: false, event_type: 'login_failed' }),
      make({ read: false, event_type: 'app_updated' }),
      make({ read: true }),
    ]);
    renderBell();
    fireEvent.click(await screen.findByRole('button', { name: /Notifications, 3 unread/ }));
    const dialog = await screen.findByRole('dialog', { name: 'Notifications' });
    const pills = within(dialog).getByRole('group', { name: 'Filter' });
    expect(pills).toHaveClass('flex-nowrap', 'overflow-x-auto');
    expect(pills.className).not.toMatch(/(^|\s)flex-wrap(\s|$)/);
    const pill = (name: string) => within(pills).getByRole('button', { name: new RegExp(`^${name}`) });
    for (const name of ['All', 'Unread', 'Attention', 'Updates']) expect(pill(name)).toHaveClass('whitespace-nowrap', 'flex-none');
    await waitFor(() => expect(within(pill('All')).getByTestId('count-badge')).toHaveTextContent('4'));
    expect(within(pill('Unread')).getByTestId('count-badge')).toHaveTextContent('3');
    expect(within(pill('Attention')).getByTestId('count-badge')).toHaveTextContent('2');
    expect(within(pill('Attention')).getByTestId('count-badge')).toHaveAttribute('data-kind', 'warn');
    expect(within(pill('Updates')).getByTestId('count-badge')).toHaveTextContent('2');
    expect(within(dialog).getByTestId('popover-footer')).toHaveTextContent('3 unread of 4');
    expect(within(dialog).getByRole('link', { name: 'Open notifications' })).toHaveAttribute('href', '/notifications');

    fireEvent.click(pill('Updates'));
    expect(pill('Updates')).toHaveAttribute('aria-pressed', 'true');
    expect(within(dialog).getAllByTestId('notification-item')).toHaveLength(2);
  });

  it('groups Needs you, day groups, app chips and collapsed night updates', async () => {
    mocks.getUnreadCounts.mockResolvedValue({ count: 4, critical: 1 });
    inbox([
      make({ title: 'Update of Prowlarr failed', severity: 'critical', read: false, event_type: 'app_update_rollback_failed', message: 'App update failed: app=prowlarr from=1 to=2', created_at: at(25, 9, 40) }),
      make({ title: 'Jackett updated', read: false, event_type: 'app_updated', message: 'App updated: app=jackett from=1 to=2', created_at: at(25, 4, 30) }),
      make({ title: 'Navidrome updated', read: false, event_type: 'app_updated', message: 'App updated: app=navidrome from=1 to=2', created_at: at(25, 4, 20) }),
      make({ title: 'Sonarr updated', read: false, event_type: 'app_updated', message: 'App updated: app=sonarr from=1 to=2', created_at: at(25, 4, 10) }),
      make({ title: 'Old sign-in', created_at: at(24, 13) }),
    ]);
    renderBell();
    fireEvent.click(await screen.findByRole('button', { name: /Notifications, 4 unread/ }));
    const dialog = await screen.findByRole('dialog');
    await within(dialog).findByText('Update of Prowlarr failed');
    expect(within(dialog).getAllByTestId('group-header').map((h) => h.textContent)).toEqual(['Needs you1', 'Today3 · 3 unread', 'Yesterday1']);
    expect(within(dialog).getAllByTestId('app-chip').map((c) => c.textContent)).toEqual(['prowlarr', 'jackett']);
    expect(within(dialog).queryByText('Navidrome updated')).toBeNull();
    fireEvent.click(within(dialog).getByRole('button', { name: '+2 more updates last night' }));
    expect(within(dialog).getByText('Navidrome updated')).toBeInTheDocument();
    expect(within(dialog).getByText('Sonarr updated')).toBeInTheDocument();
  });

  it('marks read and dismisses, updating the badge; Esc closes and returns focus', async () => {
    const crit = make({ title: 'Rollback failed', severity: 'critical', read: false });
    const info = make({ title: 'Signed in', read: false });
    mocks.getUnreadCounts.mockResolvedValue({ count: 2, critical: 1 });
    inbox([crit, info]);
    renderBell();
    const bell = await screen.findByRole('button', { name: 'Notifications, 2 unread, 1 critical' });
    fireEvent.click(bell);
    const dialog = await screen.findByRole('dialog');
    await within(dialog).findByText('Rollback failed');

    fireEvent.click(within(dialog).getByRole('button', { name: 'Mark read: Rollback failed' }));
    await waitFor(() => expect(mocks.markAsRead).toHaveBeenCalledWith(crit.id));
    await waitFor(() => expect(bell).toHaveAccessibleName('Notifications, 1 unread'));
    expect(within(bell).getByTestId('bell-badge')).toHaveAttribute('data-kind', 'calm');
    expect(within(dialog).queryByRole('button', { name: 'Mark read: Rollback failed' })).toBeNull();

    fireEvent.click(within(dialog).getByRole('button', { name: 'Dismiss: Signed in' }));
    await waitFor(() => expect(mocks.deleteNotification).toHaveBeenCalledWith(info.id));
    await waitFor(() => expect(within(dialog).queryByText('Signed in')).toBeNull());
    expect(bell).toHaveAccessibleName('Notifications, 0 unread');

    act(() => {
      fireEvent.keyDown(document, { key: 'Escape' });
    });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(bell).toHaveFocus();
    expect(bell).toHaveAttribute('aria-expanded', 'false');
  });

  it('marks everything read', async () => {
    mocks.getUnreadCounts.mockResolvedValue({ count: 1, critical: 0 });
    inbox([make({ read: false })]);
    renderBell();
    fireEvent.click(await screen.findByRole('button', { name: 'Notifications, 1 unread' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Mark all read' }));
    await waitFor(() => expect(mocks.markAllAsRead).toHaveBeenCalledTimes(1));
    expect(await screen.findByText('All caught up')).toBeInTheDocument();
  });
});
