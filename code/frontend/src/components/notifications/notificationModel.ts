import type { Notification } from '../../api/notifications';

// Pure helpers behind the bell popover, the notifications page and the
// settings tab. Everything here works on the fields the API returns today:
// id, title, message, event_type, severity, read, created_at.

export type NotificationCategory = 'updates' | 'apps' | 'system' | 'security';

export const CATEGORY_LABELS: Record<NotificationCategory, string> = {
  updates: 'Updates',
  apps: 'Apps',
  system: 'System',
  security: 'Security',
};

export const CATEGORY_ORDER: NotificationCategory[] = ['updates', 'apps', 'security', 'system'];

/** App update results: `app_updated`, `app_update_failed`, `app_update_rollback_failed`. */
export function isUpdateEvent(eventType: string | null | undefined): boolean {
  return !!eventType && (eventType.startsWith('app_update') || eventType === 'app_updated');
}

/**
 * Maps an event type (an audit action name) to the category shown in facets
 * and in Settings → What you get. Unknown or missing types are "system".
 */
export function categoryOf(eventType: string | null | undefined): NotificationCategory {
  if (!eventType) return 'system';
  if (isUpdateEvent(eventType)) return 'updates';
  if (eventType.startsWith('app_')) return 'apps';
  if (
    eventType.startsWith('login') ||
    eventType === 'logout' ||
    eventType === 'token_refresh' ||
    eventType.startsWith('2fa_') ||
    eventType === 'password_changed' ||
    eventType.startsWith('user_') ||
    eventType.startsWith('role_') ||
    eventType.startsWith('invite_') ||
    eventType === 'api_access'
  ) {
    return 'security';
  }
  return 'system';
}

/** Unread warnings and criticals: pinned as "Needs you" and counted as Attention. */
export function needsAttention(n: Notification): boolean {
  return !n.read && (n.severity === 'warning' || n.severity === 'critical');
}

/** A healthy update; shown with a green check and eligible for collapsing. */
export function isSuccessfulUpdate(n: Notification): boolean {
  return n.event_type === 'app_updated';
}

export type Tone = 'critical' | 'warning' | 'info' | 'success';

export function toneOf(n: Notification): Tone {
  if (n.severity === 'critical') return 'critical';
  if (n.severity === 'warning') return 'warning';
  if (isSuccessfulUpdate(n)) return 'success';
  return 'info';
}

/** The app named in update notifications (`app=sonarr from=… to=…`), if any. */
export function appOf(n: Notification): string | null {
  const match = /(?:^|[\s:;(])app=([A-Za-z0-9][\w.-]*)/.exec(n.message);
  return match ? match[1] : null;
}

function dayKey(date: Date): string {
  return `${date.getFullYear()}-${date.getMonth() + 1}-${date.getDate()}`;
}

function shiftDays(date: Date, days: number): Date {
  const d = new Date(date);
  d.setDate(d.getDate() + days);
  return d;
}

export type DayGroup = 'Today' | 'Yesterday' | 'Earlier';

export function dayGroupOf(createdAt: string, now: Date): DayGroup {
  const key = dayKey(new Date(createdAt));
  if (key === dayKey(now)) return 'Today';
  if (key === dayKey(shiftDays(now, -1))) return 'Yesterday';
  return 'Earlier';
}

/**
 * Night window 18:00–11:59 local, keyed by the date it starts on. Afternoon
 * notifications (12:00–17:59) are not part of any night.
 */
export function nightKeyOf(createdAt: string): string | null {
  const t = new Date(createdAt);
  const hour = t.getHours();
  if (hour >= 12 && hour < 18) return null;
  return dayKey(hour < 12 ? shiftDays(t, -1) : t);
}

function nightLabel(key: string, now: Date): string {
  if (key === dayKey(shiftDays(now, -1))) return 'last night';
  if (key === dayKey(now)) return 'tonight';
  return 'overnight';
}

export type ListRow =
  | { kind: 'header'; key: string; label: string; count: number; unread: number }
  | { kind: 'item'; key: string; notification: Notification }
  | { kind: 'stack'; key: string; stackId: string; hidden: number; label: string };

/**
 * Turns a newest-first list into display rows: unread warnings/criticals are
 * pinned under "Needs you", the rest grouped by Today / Yesterday / Earlier.
 * Consecutive successful updates from the same night collapse behind a
 * "+N more updates last night" row unless their stack id is in `expanded`.
 */
export function buildRows(list: Notification[], now: Date, expanded: ReadonlySet<string> = new Set()): ListRow[] {
  const rows: ListRow[] = [];
  const pinned = list.filter(needsAttention);
  const rest = list.filter((n) => !needsAttention(n));

  if (pinned.length) {
    rows.push({ kind: 'header', key: 'h-needs', label: 'Needs you', count: pinned.length, unread: pinned.length });
    for (const n of pinned) rows.push({ kind: 'item', key: `n-${n.id}`, notification: n });
  }

  // Runs are keyed by night + type (+ ordinal within that night), so an expanded
  // run stays expanded when its first item is read, dismissed or replaced.
  const runsPerNight = new Map<string, number>();
  const groups: DayGroup[] = ['Today', 'Yesterday', 'Earlier'];
  for (const group of groups) {
    const inGroup = rest.filter((n) => dayGroupOf(n.created_at, now) === group);
    if (!inGroup.length) continue;
    rows.push({
      kind: 'header',
      key: `h-${group}`,
      label: group,
      count: inGroup.length,
      unread: inGroup.filter((n) => !n.read).length,
    });

    let i = 0;
    while (i < inGroup.length) {
      const first = inGroup[i];
      const night = isSuccessfulUpdate(first) ? nightKeyOf(first.created_at) : null;
      let end = i + 1;
      if (night) {
        while (
          end < inGroup.length &&
          isSuccessfulUpdate(inGroup[end]) &&
          nightKeyOf(inGroup[end].created_at) === night
        ) {
          end++;
        }
      }
      const run = inGroup.slice(i, end);
      rows.push({ kind: 'item', key: `n-${first.id}`, notification: first });
      if (run.length > 1 && night) {
        const ordinal = runsPerNight.get(night) ?? 0;
        runsPerNight.set(night, ordinal + 1);
        const stackId = `app_updated@${night}#${ordinal}`;
        if (expanded.has(stackId)) {
          for (const n of run.slice(1)) rows.push({ kind: 'item', key: `n-${n.id}`, notification: n });
        } else {
          const hidden = run.length - 1;
          rows.push({
            kind: 'stack',
            key: `s-${stackId}`,
            stackId,
            hidden,
            label: `+${hidden} more update${hidden > 1 ? 's' : ''} ${nightLabel(night, now)}`,
          });
        }
      }
      i = end;
    }
  }
  return rows;
}

export type PopoverFilter = 'all' | 'unread' | 'attention' | 'updates';

export function matchesPopoverFilter(n: Notification, filter: PopoverFilter): boolean {
  switch (filter) {
    case 'unread':
      return !n.read;
    case 'attention':
      return needsAttention(n);
    case 'updates':
      return isUpdateEvent(n.event_type);
    default:
      return true;
  }
}

export type PageFilter = 'all' | 'unread' | 'attention' | 'critical' | 'recent' | NotificationCategory;

const DAY_MS = 24 * 60 * 60 * 1000;

export function isWithinLastDay(n: Notification, now: Date): boolean {
  return now.getTime() - new Date(n.created_at).getTime() < DAY_MS;
}

export function matchesPageFilter(n: Notification, filter: PageFilter, now: Date): boolean {
  switch (filter) {
    case 'all':
      return true;
    case 'unread':
      return !n.read;
    case 'attention':
      return needsAttention(n);
    case 'critical':
      return !n.read && n.severity === 'critical';
    case 'recent':
      return isWithinLastDay(n, now);
    default:
      return categoryOf(n.event_type) === filter;
  }
}

export function matchesSearch(n: Notification, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return `${n.title} ${n.message} ${appOf(n) ?? ''}`.toLowerCase().includes(q);
}

export function bellLabel(unread: number, critical: number): string {
  return `Notifications, ${unread} unread${critical ? `, ${critical} critical` : ''}`;
}

/** A count from loaded pages only: "N+" ("at least N") unless it is known to be complete. */
export function formatPartialCount(n: number, complete: boolean): string {
  return complete ? String(n) : `${n}+`;
}

export function formatCount(n: number): string {
  return n > 99 ? '99+' : String(n);
}

export function formatWhen(createdAt: string, now: Date): string {
  const t = new Date(createdAt);
  const diff = now.getTime() - t.getTime();
  const minutes = Math.floor(diff / 60000);
  if (minutes < 1) return 'Just now';
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  return t.toLocaleString(undefined, { weekday: 'short', day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
}
