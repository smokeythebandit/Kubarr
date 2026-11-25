import { useCallback, useEffect, useRef, useState } from 'react';
import { Link } from 'react-router-dom';
import { Bell } from 'lucide-react';
import { notificationsApi, type Notification } from '../../api/notifications';
import { useAuth } from '../../contexts/AuthContext';
import { CountBadge, NotificationRowList } from './NotificationParts';
import { bellLabel, formatCount, isUpdateEvent, matchesPopoverFilter, needsAttention, type PopoverFilter } from './notificationModel';
import { useNotificationCounts, useRefreshNotificationCounts } from './useNotificationCounts';

const POPOVER_LIMIT = 30;

const FILTERS: { id: PopoverFilter; label: string; title?: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'unread', label: 'Unread' },
  { id: 'attention', label: 'Attention', title: 'Needs attention: unread warnings and critical notifications' },
  { id: 'updates', label: 'Updates' },
];

export function NotificationBell() {
  const { isAuthenticated } = useAuth();
  const counts = useNotificationCounts();
  const refreshCounts = useRefreshNotificationCounts();
  const [open, setOpen] = useState(false);
  const [filter, setFilter] = useState<PopoverFilter>('all');
  const [items, setItems] = useState<Notification[]>([]);
  const [total, setTotal] = useState(0);
  const [loaded, setLoaded] = useState(false);
  const [loading, setLoading] = useState(false);
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  const wrapRef = useRef<HTMLDivElement>(null);
  const bellRef = useRef<HTMLButtonElement>(null);

  // Loaded inbox totals, adjusted locally after read/dismiss so the badge
  // reacts instantly; the polled counts take over when the server changes.
  const [localUnread, setLocalUnread] = useState(0);
  const [localCritical, setLocalCritical] = useState(0);
  const unread = loaded ? localUnread : counts.count;
  const critical = loaded ? localCritical : counts.critical;

  const close = useCallback((returnFocus: boolean) => {
    setOpen(false);
    if (returnFocus) bellRef.current?.focus();
  }, []);

  useEffect(() => {
    if (!open) return;
    const onMouseDown = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) close(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close(true);
    };
    document.addEventListener('mousedown', onMouseDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onMouseDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [open, close]);

  // A new poll result means the server changed; reload next time it opens.
  useEffect(() => {
    if (!open) setLoaded(false);
  }, [counts.count, counts.critical, open]);

  const load = async () => {
    setLoading(true);
    try {
      const response = await notificationsApi.getInbox(POPOVER_LIMIT, 0);
      setItems(response.notifications);
      setTotal(response.total);
      setLocalUnread(response.unread);
      setLocalCritical(Math.max(counts.critical, response.notifications.filter((n) => !n.read && n.severity === 'critical').length));
      setLoaded(true);
    } catch {
      // The bell is best-effort; the list just stays as it was.
    } finally {
      setLoading(false);
    }
  };

  const toggle = () => {
    if (!open) void load();
    setOpen(!open);
  };

  const forget = (n: Notification) => {
    if (n.read) return;
    setLocalUnread((c) => Math.max(0, c - 1));
    if (n.severity === 'critical') setLocalCritical((c) => Math.max(0, c - 1));
  };

  const markRead = async (n: Notification) => {
    try {
      await notificationsApi.markAsRead(n.id);
      forget(n);
      setItems((prev) => prev.map((x) => (x.id === n.id ? { ...x, read: true } : x)));
      refreshCounts();
    } catch {
      // Leave it unread; the next open shows the server state.
    }
  };

  const markAll = async () => {
    try {
      await notificationsApi.markAllAsRead();
      setItems((prev) => prev.map((x) => ({ ...x, read: true })));
      setLocalUnread(0);
      setLocalCritical(0);
      refreshCounts();
    } catch {
      // Same as above.
    }
  };

  const dismiss = async (n: Notification) => {
    try {
      await notificationsApi.deleteNotification(n.id);
      forget(n);
      setItems((prev) => prev.filter((x) => x.id !== n.id));
      setTotal((t) => Math.max(0, t - 1));
      refreshCounts();
    } catch {
      // Same as above.
    }
  };

  if (!isAuthenticated) return null;

  const filterCounts: Record<PopoverFilter, number> = {
    all: total,
    unread: unread,
    attention: items.filter(needsAttention).length,
    updates: items.filter((n) => isUpdateEvent(n.event_type)).length,
  };
  const visible = items.filter((n) => matchesPopoverFilter(n, filter));
  const badgeKind = critical > 0 ? 'crit' : 'calm';

  return (
    <div ref={wrapRef} className="relative">
      <button
        ref={bellRef}
        type="button"
        onClick={toggle}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={bellLabel(unread, critical)}
        title="Notifications"
        className="relative flex items-center justify-center w-9 h-9 rounded-md text-gray-600 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-700 transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500"
      >
        <Bell size={18} strokeWidth={2} aria-hidden="true" />
        {unread > 0 && (
          <span
            data-testid="bell-badge"
            data-kind={badgeKind}
            className={`absolute -top-1 -right-1 flex items-center justify-center min-w-[18px] h-[18px] px-1 text-[0.68rem] font-bold text-white rounded-full ${
              badgeKind === 'crit' ? 'bg-red-600' : 'bg-blue-600'
            }`}
          >
            {formatCount(unread)}
          </span>
        )}
      </button>

      {open && (
        <div
          role="dialog"
          aria-label="Notifications"
          className="absolute right-0 top-[calc(100%+8px)] z-50 w-[min(420px,calc(100vw-32px))] grid overflow-hidden rounded-xl border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800 shadow-[0_18px_40px_rgba(15,23,42,0.18)] dark:shadow-[0_18px_40px_rgba(0,0,0,0.5)]"
        >
          <header className="flex items-center gap-2 px-4 pt-3 pb-2">
            <h3 className="flex-1 m-0 text-base font-semibold text-gray-900 dark:text-white">Notifications</h3>
            {unread > 0 && (
              <button
                type="button"
                onClick={markAll}
                className="text-[0.82rem] font-semibold text-blue-600 dark:text-blue-400 rounded px-1 py-0.5 hover:underline focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500"
              >
                Mark all read
              </button>
            )}
          </header>

          <div
            role="group"
            aria-label="Filter"
            data-testid="filter-pills"
            className="flex flex-nowrap gap-1.5 px-4 pb-2.5 overflow-x-auto overscroll-x-contain [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
          >
            {FILTERS.map((f) => {
              const pressed = filter === f.id;
              const value = filterCounts[f.id];
              return (
                <button
                  key={f.id}
                  type="button"
                  aria-pressed={pressed}
                  title={f.title}
                  onClick={() => setFilter(f.id)}
                  className={`flex-none whitespace-nowrap inline-flex items-center gap-1 rounded-full border pl-2.5 pr-1.5 py-0.5 text-[0.76rem] font-semibold focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 ${
                    pressed
                      ? 'bg-gray-900 text-white border-gray-900 dark:bg-gray-100 dark:text-gray-900 dark:border-gray-100'
                      : 'bg-white text-gray-600 border-gray-200 hover:bg-gray-50 dark:bg-gray-800 dark:text-gray-300 dark:border-gray-600 dark:hover:bg-gray-700'
                  }`}
                >
                  {f.label}
                  <CountBadge value={value} kind={f.id === 'attention' && value > 0 ? 'warn' : 'neutral'} />
                </button>
              );
            })}
          </div>

          <div className="max-h-[420px] overflow-y-auto border-t border-gray-200 dark:border-gray-700">
            {loading && !loaded ? (
              <p className="px-4 py-9 text-center text-sm text-gray-500 dark:text-gray-400">Loading…</p>
            ) : (
              <NotificationRowList
                notifications={visible}
                now={new Date()}
                expanded={expanded}
                onExpand={(id) => setExpanded((prev) => new Set(prev).add(id))}
                onMarkRead={markRead}
                onDismiss={dismiss}
              />
            )}
          </div>

          <footer className="flex items-center justify-between px-4 py-2 border-t border-gray-200 dark:border-gray-700 text-[0.82rem] text-gray-500 dark:text-gray-400">
            <span data-testid="popover-footer">{unread ? `${unread} unread of ${total}` : 'All caught up'}</span>
            <Link
              to="/notifications"
              onClick={() => close(false)}
              className="font-semibold text-blue-600 dark:text-blue-400 rounded px-1 py-0.5 hover:underline focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500"
            >
              Open notifications
            </Link>
          </footer>
        </div>
      )}
    </div>
  );
}
