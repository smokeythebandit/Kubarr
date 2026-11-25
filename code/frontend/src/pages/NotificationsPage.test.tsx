import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';
import type { Notification } from '../api/notifications';
import NotificationsPage, { AUTO_PAGES, PAGE_SIZE } from './NotificationsPage';

const mocks = vi.hoisted(() => ({
  getUnreadCounts: vi.fn(),
  getInbox: vi.fn(),
  markAsRead: vi.fn(),
  markAllAsRead: vi.fn(),
  deleteNotification: vi.fn(),
}));

vi.mock('../api/notifications', () => ({ notificationsApi: mocks }));
vi.mock('../contexts/AuthContext', () => ({ useAuth: () => ({ isAuthenticated: true }) }));

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
    let server = items.map((n) => ({ ...n }));
    mocks.getInbox.mockImplementation(async () => ({ notifications: server, total: server.length, unread: server.filter((n) => !n.read).length }));
    mocks.getUnreadCounts.mockImplementation(async () => {
      const unread = server.filter((n) => !n.read);
      return {
        count: unread.length,
        critical: unread.filter((n) => n.severity === 'critical').length,
        attention: unread.filter((n) => n.severity !== 'info').length,
      };
    });
    mocks.markAllAsRead.mockImplementation(async () => {
      server = server.map((n) => ({ ...n, read: true }));
    });
  });
  afterEach(() => vi.useRealTimers());

  it('shows counters and facet counts', async () => {
    renderPage();
    await screen.findByText('Rollback failed for sonarr');
    expect(mocks.getInbox).toHaveBeenCalledWith(PAGE_SIZE, 0);
    await waitFor(() => expect(screen.getByTestId('facet-count-unread')).toHaveTextContent('3'));
    const counters = screen.getByRole('group', { name: 'Counters' });
    const counter = (label: string) => within(counters).getByRole('button', { name: new RegExp(label) });
    expect(counter('Unread')).toHaveTextContent('3Unread');
    expect(counter('Need you')).toHaveTextContent('2Need you');
    expect(counter('Critical unread')).toHaveTextContent('1Critical unread');
    expect(counter('Last 24 hours')).toHaveTextContent('2Last 24 hours');

    const facet = (id: string) => screen.getByTestId(`facet-count-${id}`).textContent;
    expect([facet('all'), facet('unread'), facet('attention')]).toEqual(['5', '3', '2']);
    expect([facet('updates'), facet('apps'), facet('security'), facet('system')]).toEqual(['2', '1', '1', '1']);
  });

  it('filters the list from counters, facets and search', async () => {
    renderPage();
    await screen.findByText('Rollback failed for sonarr');
    expect(titles()).toHaveLength(5);

    fireEvent.click(await screen.findByRole('button', { name: /^1\s*Critical unread/ }));
    expect(titles()).toEqual(['Rollback failed for sonarr']);

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
    await screen.findByText(new RegExp(`^Showing ${PAGE_SIZE} of ${PAGE_SIZE + 1}`));
    fireEvent.click(screen.getByRole('button', { name: 'Load more' }));
    await screen.findByText('Last item');
    expect(mocks.getInbox).toHaveBeenLastCalledWith(PAGE_SIZE, PAGE_SIZE);
    expect(screen.getByText(`Showing ${PAGE_SIZE + 1} of ${PAGE_SIZE + 1}`)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Load more' })).toBeNull();
  });

  it('marks loaded-only counts as partial while more pages exist', async () => {
    const page = [
      make({ title: 'Recent update', event_type: 'app_updated', created_at: at(25, 9) }),
      make({ title: 'Recent sign-in', created_at: at(25, 8) }),
    ];
    mocks.getInbox.mockResolvedValue({ notifications: page, total: 120, unread: 0 });
    mocks.getUnreadCounts.mockResolvedValue({ count: 7, critical: 2, attention: 4 });
    renderPage();
    await screen.findByText('Recent update');
    await waitFor(() => expect(screen.getByTestId('facet-count-unread')).toHaveTextContent('7'));
    expect(screen.getByTestId('facet-count-all')).toHaveTextContent('120');
    expect(screen.getByTestId('facet-count-attention')).toHaveTextContent('4');
    expect(screen.getByTestId('facet-count-updates')).toHaveTextContent('1+');
    expect(screen.getByTestId('facet-count-updates')).toHaveAttribute('title', 'Counted in the latest 2 of 120');
    const counters = screen.getByRole('group', { name: 'Counters' });
    // Everything loaded is from the last day, so older ones may be too.
    expect(within(counters).getByRole('button', { name: /Last 24 hours/ })).toHaveTextContent('2+Last 24 hours');
    expect(within(counters).getByRole('button', { name: /Critical unread/ })).toHaveTextContent('2Critical unread');
    expect(screen.getByText(/counts marked \+ include loaded notifications only/)).toBeInTheDocument();
  });

  it('knows Last 24 hours is exact once an older notification is loaded', async () => {
    const page = [make({ title: 'New', created_at: at(25, 9) }), make({ title: 'Old', created_at: at(20, 9) })];
    mocks.getInbox.mockResolvedValue({ notifications: page, total: 120, unread: 0 });
    renderPage();
    await screen.findByText('Old');
    expect(within(screen.getByRole('group', { name: 'Counters' })).getByRole('button', { name: /Last 24 hours/ })).toHaveTextContent('1Last 24 hours');
  });

  it('stops auto-loading after a few pages', async () => {
    type Entries = Array<{ isIntersecting: boolean }>;
    const live = new Set<FakeObserver>();
    class FakeObserver {
      constructor(public cb: (entries: Entries) => void) {}
      observe() {
        live.add(this);
      }
      disconnect() {
        live.delete(this);
      }
    }
    const scrollToBottom = () => live.forEach((o) => o.cb([{ isIntersecting: true }]));
    vi.stubGlobal('IntersectionObserver', FakeObserver);
    mocks.getInbox.mockImplementation(async (limit: number, offset: number) => ({
      notifications: Array.from({ length: limit }, (_, i) => make({ id: offset + i + 1, title: `Item ${offset + i}` })),
      total: 10_000,
      unread: 0,
    }));
    renderPage();
    for (let page = 1; page < AUTO_PAGES + 3; page++) {
      await screen.findByText(new RegExp(`^Showing ${Math.min(page, AUTO_PAGES) * PAGE_SIZE} of`));
      scrollToBottom();
    }
    await waitFor(() => expect(mocks.getInbox).toHaveBeenCalledTimes(AUTO_PAGES));
    expect(screen.getByRole('button', { name: 'Load more' })).toBeEnabled();
    vi.unstubAllGlobals();
  });
});
