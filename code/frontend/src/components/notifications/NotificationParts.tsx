import { AlertCircle, AlertTriangle, Check, CheckCheck, Info, X } from 'lucide-react';
import type { Notification } from '../../api/notifications';
import { AskAssistantButton } from '../assistant/AssistantPanel';
import { failedOperationContext, isFailedUpdateEvent, appOf, buildRows, formatCount, formatWhen, toneOf, type Tone } from './notificationModel';

export type BadgeKind = 'neutral' | 'accent' | 'crit' | 'warn';

const BADGE_CLASSES: Record<BadgeKind, string> = {
  neutral: 'bg-gray-200 text-gray-800 dark:bg-gray-700 dark:text-gray-100',
  accent: 'bg-blue-600 text-white dark:bg-blue-500',
  crit: 'bg-red-600 text-white dark:bg-red-500',
  warn: 'bg-amber-100 text-amber-800 dark:bg-amber-900/60 dark:text-amber-200',
};

/** Small pill counter; dimmed at zero, capped at 99+. */
export function CountBadge({
  value,
  display,
  kind = 'neutral',
  className = '',
  label,
}: {
  value: number;
  /** Overrides the rendered text, e.g. "3+" for a partial count. */
  display?: string;
  kind?: BadgeKind;
  className?: string;
  label?: string;
}) {
  return (
    <span
      data-testid="count-badge"
      data-kind={kind}
      title={label}
      className={`inline-block min-w-[1.35em] px-1.5 rounded-full text-[0.68rem] font-bold leading-[1.35rem] text-center tabular-nums ${BADGE_CLASSES[kind]} ${value === 0 ? 'opacity-45' : ''} ${className}`}
    >
      {display ?? formatCount(value)}
    </span>
  );
}

const TILE: Record<Tone, { className: string; Icon: typeof Info; label: string }> = {
  critical: { className: 'bg-red-100 text-red-600 dark:bg-red-900/40 dark:text-red-400', Icon: AlertCircle, label: 'Critical' },
  warning: { className: 'bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-300', Icon: AlertTriangle, label: 'Warning' },
  info: { className: 'bg-blue-100 text-blue-600 dark:bg-blue-900/40 dark:text-blue-400', Icon: Info, label: 'Info' },
  success: { className: 'bg-emerald-100 text-emerald-700 dark:bg-emerald-900/40 dark:text-emerald-400', Icon: Check, label: 'Success' },
};

export function SeverityTile({ tone }: { tone: Tone }) {
  const { className, Icon, label } = TILE[tone];
  return (
    <span data-testid="severity-tile" data-tone={tone} title={label} className={`grid place-items-center w-8 h-8 rounded-lg shrink-0 ${className}`}>
      <Icon size={16} strokeWidth={2.25} aria-hidden="true" />
      <span className="sr-only">{label}</span>
    </span>
  );
}

const iconButton =
  'p-1 rounded-md text-gray-400 hover:text-gray-900 hover:bg-gray-100 dark:hover:text-white dark:hover:bg-gray-700 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500';

export function NotificationItem({
  notification: n,
  now,
  onMarkRead,
  onDismiss,
}: {
  notification: Notification;
  now: Date;
  onMarkRead: (n: Notification) => void;
  onDismiss: (n: Notification) => void;
}) {
  const app = appOf(n);
  return (
    <li
      data-testid="notification-item"
      data-unread={!n.read}
      className={`relative grid grid-cols-[auto_1fr_auto] gap-3 px-4 py-3 border-t border-gray-100 dark:border-gray-700/70 ${
        n.read ? '' : 'bg-blue-50/70 dark:bg-blue-900/15'
      }`}
    >
      {!n.read && <span aria-hidden="true" className="absolute left-0 top-3.5 bottom-3.5 w-[3px] rounded-r bg-blue-600 dark:bg-blue-400" />}
      <SeverityTile tone={toneOf(n)} />
      <div className="min-w-0">
        <h4 className="text-sm font-semibold leading-snug text-gray-900 dark:text-white">{n.title}</h4>
        <p className="mt-0.5 text-[0.84rem] text-gray-500 dark:text-gray-400 break-words">{n.message}</p>
        {app && (
          <div className="mt-1.5 flex flex-wrap items-center gap-2">
            <span data-testid="app-chip" className="font-mono text-[0.72rem] font-semibold rounded-md px-1.5 py-0.5 bg-gray-100 text-gray-700 dark:bg-gray-700 dark:text-gray-200">
              {app}
            </span>
            {isFailedUpdateEvent(n.event_type) && (
              <AskAssistantButton
                label="Explain this"
                context={{ kind: 'app', app }}
                draft="Why did this update fail, and what should I do?"
                resolveContext={() => failedOperationContext(app, n.created_at)}
                className="text-xs font-semibold text-blue-600 dark:text-blue-400 hover:underline rounded"
              />
            )}
          </div>
        )}
      </div>
      <div className="grid justify-items-end content-start gap-1.5">
        <time dateTime={n.created_at} className="text-xs text-gray-500 dark:text-gray-400 whitespace-nowrap">
          {formatWhen(n.created_at, now)}
        </time>
        <div className="flex items-center gap-0.5">
          {!n.read && (
            <button type="button" className={iconButton} aria-label={`Mark read: ${n.title}`} title="Mark read" onClick={() => onMarkRead(n)}>
              <CheckCheck size={15} aria-hidden="true" />
            </button>
          )}
          <button type="button" className={iconButton} aria-label={`Dismiss: ${n.title}`} title="Dismiss" onClick={() => onDismiss(n)}>
            <X size={15} aria-hidden="true" />
          </button>
        </div>
      </div>
    </li>
  );
}

/** Grouped list: "Needs you" pinned, day groups with counts, collapsed update runs. */
export function NotificationRowList({
  notifications,
  now,
  expanded,
  onExpand,
  onMarkRead,
  onDismiss,
  emptyText = 'Nothing here. Kubarr will tell you when something needs you.',
}: {
  notifications: Notification[];
  now: Date;
  expanded: ReadonlySet<string>;
  onExpand: (stackId: string) => void;
  onMarkRead: (n: Notification) => void;
  onDismiss: (n: Notification) => void;
  emptyText?: string;
}) {
  if (!notifications.length) {
    return <p className="px-4 py-9 text-center text-sm text-gray-500 dark:text-gray-400">{emptyText}</p>;
  }
  const rows = buildRows(notifications, now, expanded);
  return (
    <ul className="list-none m-0 p-0">
      {rows.map((row) => {
        if (row.kind === 'header') {
          return (
            <li
              key={row.key}
              data-testid="group-header"
              className="px-4 pt-3 pb-1.5 font-mono text-[0.7rem] font-semibold uppercase tracking-[0.07em] text-gray-500 dark:text-gray-400"
            >
              {row.label}
              <span className="ml-1.5 tracking-normal">
                {row.count}
                {row.unread > 0 && row.label !== 'Needs you' ? ` · ${row.unread} unread` : ''}
              </span>
            </li>
          );
        }
        if (row.kind === 'stack') {
          return (
            <li key={row.key} className="grid grid-cols-[2rem_1fr] gap-3 px-4 pb-2.5 -mt-1">
              <span />
              <button
                type="button"
                onClick={() => onExpand(row.stackId)}
                className="justify-self-start text-xs font-semibold text-blue-600 dark:text-blue-400 hover:underline rounded focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500"
              >
                {row.label}
              </button>
            </li>
          );
        }
        return <NotificationItem key={row.key} notification={row.notification} now={now} onMarkRead={onMarkRead} onDismiss={onDismiss} />;
      })}
    </ul>
  );
}
