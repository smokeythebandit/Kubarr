import { useCallback, useEffect, useRef, useState } from 'react';
import { notificationsApi, type Notification } from '../api/notifications';
import { NotificationRowList } from '../components/notifications/NotificationParts';
import {
  CATEGORY_LABELS,
  CATEGORY_ORDER,
  isWithinLastDay,
  matchesPageFilter,
  matchesSearch,
  needsAttention,
  type PageFilter,
} from '../components/notifications/notificationModel';
import { useRefreshNotificationCounts } from '../components/notifications/useNotificationCounts';

export const PAGE_SIZE = 50;

const focusRing = 'focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500';

export default function NotificationsPage() {
  const refreshCounts = useRefreshNotificationCounts();
  const [items, setItems] = useState<Notification[]>([]);
  const [total, setTotal] = useState(0);
  const [serverUnread, setServerUnread] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<PageFilter>('all');
  const [query, setQuery] = useState('');
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  const sentinelRef = useRef<HTMLDivElement>(null);
  const loadingRef = useRef(false);

  const loadPage = useCallback(async (offset: number) => {
    if (loadingRef.current) return;
    loadingRef.current = true;
    setLoading(true);
    try {
      const response = await notificationsApi.getInbox(PAGE_SIZE, offset);
      setItems((prev) => {
        const base = offset === 0 ? [] : prev;
        const seen = new Set(base.map((n) => n.id));
        return [...base, ...response.notifications.filter((n) => !seen.has(n.id))];
      });
      setTotal(response.total);
      setServerUnread(response.unread);
      setError(null);
    } catch {
      setError('Could not load notifications.');
    } finally {
      loadingRef.current = false;
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadPage(0);
  }, [loadPage]);

  const hasMore = items.length < total;

  // Infinite scroll; the "Load more" button covers browsers without IntersectionObserver.
  useEffect(() => {
    const el = sentinelRef.current;
    if (!el || !hasMore || typeof IntersectionObserver === 'undefined') return;
    const observer = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) void loadPage(items.length);
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [hasMore, items.length, loadPage]);

  const markRead = async (n: Notification) => {
    try {
      await notificationsApi.markAsRead(n.id);
      setItems((prev) => prev.map((x) => (x.id === n.id ? { ...x, read: true } : x)));
      if (!n.read) setServerUnread((c) => Math.max(0, c - 1));
      refreshCounts();
    } catch {
      setError('Could not mark the notification read.');
    }
  };

  const dismiss = async (n: Notification) => {
    try {
      await notificationsApi.deleteNotification(n.id);
      setItems((prev) => prev.filter((x) => x.id !== n.id));
      setTotal((t) => Math.max(0, t - 1));
      if (!n.read) setServerUnread((c) => Math.max(0, c - 1));
      refreshCounts();
    } catch {
      setError('Could not dismiss the notification.');
    }
  };

  const markAll = async () => {
    try {
      await notificationsApi.markAllAsRead();
      setItems((prev) => prev.map((x) => ({ ...x, read: true })));
      setServerUnread(0);
      refreshCounts();
    } catch {
      setError('Could not mark all read.');
    }
  };

  const now = new Date();
  const count = (f: PageFilter) => (f === 'all' ? total : f === 'unread' ? serverUnread : items.filter((n) => matchesPageFilter(n, f, now)).length);
  const attention = items.filter(needsAttention).length;
  const critical = items.filter((n) => n.severity === 'critical').length;
  const recent = items.filter((n) => isWithinLastDay(n, now)).length;

  const kpis: { id: PageFilter; value: number; label: string; tone: string }[] = [
    { id: 'unread', value: serverUnread, label: 'Unread', tone: serverUnread ? 'text-blue-600 dark:text-blue-400' : '' },
    { id: 'attention', value: attention, label: 'Need you', tone: attention ? 'text-amber-700 dark:text-amber-300' : '' },
    { id: 'critical', value: critical, label: 'Critical', tone: critical ? 'text-red-600 dark:text-red-400' : '' },
    { id: 'recent', value: recent, label: 'Last 24 hours', tone: '' },
  ];

  const facets: { id: PageFilter; label: string }[] = [
    { id: 'all', label: 'All' },
    { id: 'unread', label: 'Unread' },
    { id: 'attention', label: 'Needs attention' },
  ];

  const visible = items.filter((n) => matchesPageFilter(n, filter, now) && matchesSearch(n, query));

  const facetButton = (f: { id: PageFilter; label: string }) => {
    const pressed = filter === f.id;
    return (
      <button
        key={f.id}
        type="button"
        aria-pressed={pressed}
        onClick={() => setFilter(f.id)}
        className={`flex flex-none justify-between gap-3 rounded-lg px-2.5 py-1.5 text-left text-sm ${focusRing} ${
          pressed
            ? 'bg-white dark:bg-gray-800 font-semibold text-gray-900 dark:text-white ring-1 ring-inset ring-gray-200 dark:ring-gray-700'
            : 'font-medium text-gray-500 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-800/60'
        }`}
      >
        <span>{f.label}</span>
        <span className="font-mono text-xs tabular-nums" data-testid={`facet-count-${f.id}`}>
          {count(f.id)}
        </span>
      </button>
    );
  };

  return (
    <div className="mx-auto max-w-6xl grid gap-5">
      <div>
        <div className="font-mono text-[0.72rem] font-semibold uppercase tracking-[0.08em] text-gray-500 dark:text-gray-400">Notifications</div>
        <h1 className="mt-1.5 text-2xl font-bold text-gray-900 dark:text-white">Everything Kubarr told you</h1>
        <p className="mt-1.5 max-w-[70ch] text-gray-500 dark:text-gray-400">
          Newest first. Unread warnings and critical notifications stay at the top until you deal with them.
        </p>
      </div>

      <div className="grid grid-cols-2 sm:grid-cols-4 gap-3" aria-label="Counters" role="group">
        {kpis.map((k) => (
          <button
            key={k.id}
            type="button"
            aria-pressed={filter === k.id}
            onClick={() => setFilter(k.id)}
            className={`grid gap-0.5 rounded-xl border bg-white dark:bg-gray-800 px-3.5 py-3 text-left hover:bg-gray-50 dark:hover:bg-gray-700/60 ${focusRing} ${
              filter === k.id ? 'border-blue-500 dark:border-blue-400' : 'border-gray-200 dark:border-gray-700'
            }`}
          >
            <span className={`text-2xl font-bold leading-tight tabular-nums ${k.tone || 'text-gray-900 dark:text-white'}`}>{k.value}</span>
            <span className="text-[0.8rem] font-semibold text-gray-500 dark:text-gray-400">{k.label}</span>
          </button>
        ))}
      </div>

      <div className="grid gap-4 md:grid-cols-[220px_1fr] items-start">
        <nav aria-label="Filters" className="flex md:grid gap-1 overflow-x-auto md:overflow-visible">
          {facets.map(facetButton)}
          <div className="hidden md:block px-2.5 pt-3 pb-1 font-mono text-[0.7rem] font-semibold uppercase tracking-[0.07em] text-gray-500 dark:text-gray-400">
            By type
          </div>
          {CATEGORY_ORDER.map((c) => facetButton({ id: c, label: CATEGORY_LABELS[c] }))}
        </nav>

        <section className="min-w-0 overflow-hidden rounded-xl border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800">
          <div className="flex flex-wrap items-center gap-2 border-b border-gray-200 dark:border-gray-700 px-4 py-3">
            <input
              type="search"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search notifications"
              aria-label="Search notifications"
              className={`flex-1 min-w-[160px] rounded-lg border border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-900 px-2.5 py-1.5 text-sm text-gray-900 dark:text-white placeholder-gray-500 ${focusRing}`}
            />
            <button
              type="button"
              onClick={markAll}
              disabled={serverUnread === 0}
              className={`rounded-lg border border-gray-200 dark:border-gray-600 bg-white dark:bg-gray-800 px-3 py-1.5 text-[0.82rem] font-semibold text-gray-700 dark:text-gray-200 hover:bg-gray-50 dark:hover:bg-gray-700 disabled:opacity-50 disabled:cursor-not-allowed ${focusRing}`}
            >
              Mark all read
            </button>
          </div>

          {error && (
            <p role="alert" className="px-4 py-2 text-sm text-red-600 dark:text-red-400 border-b border-gray-200 dark:border-gray-700">
              {error}
            </p>
          )}

          {loading && items.length === 0 ? (
            <p className="px-4 py-9 text-center text-sm text-gray-500 dark:text-gray-400">Loading…</p>
          ) : (
            <div data-testid="page-list">
              <NotificationRowList
                notifications={visible}
                now={now}
                expanded={expanded}
                onExpand={(id) => setExpanded((prev) => new Set(prev).add(id))}
                onMarkRead={markRead}
                onDismiss={dismiss}
                emptyText={query ? 'No notifications match your search.' : undefined}
              />
            </div>
          )}

          <div ref={sentinelRef} className="flex items-center justify-between gap-2 border-t border-gray-200 dark:border-gray-700 px-4 py-2.5 text-[0.82rem] text-gray-500 dark:text-gray-400">
            <span>
              Showing {items.length} of {total}
            </span>
            {hasMore && (
              <button
                type="button"
                onClick={() => void loadPage(items.length)}
                disabled={loading}
                className={`font-semibold text-blue-600 dark:text-blue-400 rounded px-1 hover:underline disabled:opacity-50 ${focusRing}`}
              >
                {loading ? 'Loading…' : 'Load more'}
              </button>
            )}
          </div>
        </section>
      </div>
    </div>
  );
}
