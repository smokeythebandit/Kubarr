import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';
import type { Notification } from '../api/notifications';
import NotificationsPage, { PAGE_SIZE } from './NotificationsPage';

const mocks = vi.hoisted(() => ({
  getInbox: vi.fn(),
  markAsRead: vi.fn(),
  markAllAsRead: vi.fn(),
  deleteNotification: vi.fn(),
}));

vi.mock('../api/notifications', () => ({ notificationsApi: mocks }));

const at = (day: number, hour: number) => new Date(2026, 10, day, hour).toISOString();

let nextId = 1;
const make = (o: Partial<Notification>): Notification => ({
  id: nextId++,
  title: 'T',
  message: 'M',
  event_type: 'login',
  severity: 'info',
  read: true,
  created_at: at(25, 9),
  ...o,
});

const items = [
  make({ title: 'Rollback failed for sonarr', severity: 'critical', read: false, event_type: 'app_update_rollback_failed', message: 'App update failed: app=sonarr from=1 to=2' }),
  make({ title: 'Update failed for radarr', severity: 'warning', read: false, event_type: 'app_update_failed', created_at: at(24, 9) }),
  make({ title: 'App installed', event_type: 'app_installed', created_at: at(23, 9) }),
  make({ title: 'Failed sign-in', event_type: 'login_failed', read: false, created_at: at(25, 8) }),
  make({ title: 'Setting changed', event_type: 'system_setting_changed', severity: 'critical', created_at: at(20, 9) }),
];

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <NotificationsPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

const titles = () => within(screen.getByTestId('page-list')).queryAllByRole('heading', { level: 4 }).map((h) => h.textContent);

describe('NotificationsPage', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(new Date(2026, 10, 25, 10, 0));
    Object.values(mocks).forEach((m) => m.mockReset());
    mocks.getInbox.mockResolvedValue({ notifications: items, total: items.length, unread: 3 });
    mocks.markAllAsRead.mockResolvedValue(undefined);
  });
  afterEach(() => vi.useRealTimers());

  it('shows counters and facet counts', async () => {
    renderPage();
    await screen.findByText('Rollback failed for sonarr');
    expect(mocks.getInbox).toHaveBeenCalledWith(PAGE_SIZE, 0);
    const counters = screen.getByRole('group', { name: 'Counters' });
    const counter = (label: string) => within(counters).getByRole('button', { name: new RegExp(label) });
    expect(counter('Unread')).toHaveTextContent('3Unread');
    expect(counter('Need you')).toHaveTextContent('2Need you');
    expect(counter('Critical')).toHaveTextContent('2Critical');
    expect(counter('Last 24 hours')).toHaveTextContent('2Last 24 hours');

    const facet = (id: string) => screen.getByTestId(`facet-count-${id}`).textContent;
    expect([facet('all'), facet('unread'), facet('attention')]).toEqual(['5', '3', '2']);
    expect([facet('updates'), facet('apps'), facet('security'), facet('system')]).toEqual(['2', '1', '1', '1']);
  });

  it('filters the list from counters, facets and search', async () => {
    renderPage();
    await screen.findByText('Rollback failed for sonarr');
    expect(titles()).toHaveLength(5);

    fireEvent.click(screen.getByRole('button', { name: /^2\s*Critical/ }));
    expect(titles()).toEqual(['Rollback failed for sonarr', 'Setting changed']);

    fireEvent.click(screen.getByRole('button', { name: /^2\s*Last 24 hours/ }));
    expect(titles()).toEqual(['Rollback failed for sonarr', 'Failed sign-in']);

    const filters = screen.getByRole('navigation', { name: 'Filters' });
    fireEvent.click(within(filters).getByRole('button', { name: /^Updates/ }));
    expect(within(filters).getByRole('button', { name: /^Updates/ })).toHaveAttribute('aria-pressed', 'true');
    expect(titles()).toEqual(['Rollback failed for sonarr', 'Update failed for radarr']);

    fireEvent.click(within(filters).getByRole('button', { name: /^Security/ }));
    expect(titles()).toEqual(['Failed sign-in']);

    fireEvent.click(within(filters).getByRole('button', { name: /^All/ }));
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search notifications' }), { target: { value: 'sonarr' } });
    expect(titles()).toEqual(['Rollback failed for sonarr']);
  });

  it('marks all read', async () => {
    renderPage();
    await screen.findByText('Rollback failed for sonarr');
    fireEvent.click(screen.getByRole('button', { name: 'Mark all read' }));
    await waitFor(() => expect(mocks.markAllAsRead).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.getByTestId('facet-count-unread')).toHaveTextContent('0'));
    expect(screen.queryAllByRole('button', { name: /^Mark read:/ })).toHaveLength(0);
  });

  it('loads more pages with limit/offset', async () => {
    const first = Array.from({ length: PAGE_SIZE }, (_, i) => make({ title: `Item ${i}` }));
    const second = [make({ title: 'Last item' })];
    mocks.getInbox.mockImplementation(async (_limit: number, offset: number) => ({
      notifications: offset === 0 ? first : second,
      total: PAGE_SIZE + 1,
      unread: 0,
    }));
    renderPage();
    await screen.findByText(`Showing ${PAGE_SIZE} of ${PAGE_SIZE + 1}`);
    fireEvent.click(screen.getByRole('button', { name: 'Load more' }));
    await screen.findByText('Last item');
    expect(mocks.getInbox).toHaveBeenLastCalledWith(PAGE_SIZE, PAGE_SIZE);
    expect(screen.getByText(`Showing ${PAGE_SIZE + 1} of ${PAGE_SIZE + 1}`)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Load more' })).toBeNull();
  });
});
