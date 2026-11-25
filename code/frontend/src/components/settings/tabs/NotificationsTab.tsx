import { useRef, useState, type Dispatch, type KeyboardEvent, type ReactNode, type SetStateAction } from 'react';
import { Bell } from 'lucide-react';
import { ROUTE_CHANNELS, type NotificationChannel, type NotificationEvent, type NotificationLog, type NotificationRoute, type RouteChannel } from '../../../api/notifications';
import { CountBadge } from '../../notifications/NotificationParts';
import { CATEGORY_LABELS, CATEGORY_ORDER, categoryOf } from '../../notifications/notificationModel';
import { channelStatus, CHANNEL_NAMES, unresolvedFailures, type ChannelStatus, type ChannelTestResult } from './notificationChannelStatus';
import { SettingsTabLayout } from './SettingsTabLayout';

type ChannelConfigField = { key: string; label: string; type: string; placeholder: string };

type NotificationsTabProps = {
  notificationChannels: NotificationChannel[];
  notificationEvents: NotificationEvent[];
  notificationLogs: NotificationLog[];
  notificationLogsTotal: number;
  notificationLoading: boolean;
  testingChannel: string | null;
  testDestination: Record<string, string>;
  editingChannel: string | null;
  channelConfig: Record<string, Record<string, string>>;
  setTestDestination: Dispatch<SetStateAction<Record<string, string>>>;
  setEditingChannel: Dispatch<SetStateAction<string | null>>;
  setChannelConfig: Dispatch<SetStateAction<Record<string, Record<string, string>>>>;
  onToggleChannel: (channelType: string, enabled: boolean) => void;
  onToggleEvent: (eventType: string, enabled: boolean) => void;
  onUpdateEventSeverity: (eventType: string, severity: string) => void;
  /** Which external channels each event is sent to; a missing entry is off. */
  notificationRoutes?: NotificationRoute[];
  onToggleRoute?: (eventType: string, channel: RouteChannel, enabled: boolean) => void;
  onTestChannel: (channelType: string) => void;
  onSaveChannelConfig: (channelType: string) => void;
  getChannelIcon: (channelType: string) => ReactNode;
  getChannelConfigFields: (channelType: string) => ChannelConfigField[];
  formatAuditAction: (action: string) => string;
  /** settings.manage: configure channels and event triggers. */
  canManage: boolean;
  /** audit.view: delivery logs are only loaded with it. */
  canViewHistory?: boolean;
  /** Last "Send test" result per channel in this session. */
  testResults?: Record<string, ChannelTestResult>;
};

type TabId = 'channels' | 'rules' | 'history';

const STATUS_STYLE: Record<ChannelStatus, { label: string; className: string }> = {
  enabled: { label: 'Enabled', className: 'text-blue-700 bg-blue-50 dark:text-blue-300 dark:bg-blue-900/40' },
  working: { label: 'Working', className: 'text-emerald-700 bg-emerald-50 dark:text-emerald-300 dark:bg-emerald-900/40' },
  off: { label: 'Off', className: 'text-gray-600 bg-gray-100 dark:text-gray-300 dark:bg-gray-700' },
  unset: { label: 'Not set up', className: 'text-gray-600 bg-gray-100 dark:text-gray-300 dark:bg-gray-700' },
  problem: { label: 'Problem', className: 'text-red-700 bg-red-50 dark:text-red-300 dark:bg-red-900/40' },
};

const focusRing = 'focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500';
const btn = `rounded-lg border border-gray-200 dark:border-gray-600 bg-white dark:bg-gray-800 px-3 py-1.5 text-[0.82rem] font-semibold text-gray-700 dark:text-gray-200 hover:bg-gray-50 dark:hover:bg-gray-700 disabled:opacity-50 disabled:cursor-not-allowed ${focusRing}`;
const btnPrimary = `rounded-lg border border-blue-600 bg-blue-600 px-3 py-1.5 text-[0.82rem] font-semibold text-white hover:bg-blue-700 ${focusRing}`;
const input = `w-full rounded-md border border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-900 px-3 py-1.5 text-sm text-gray-900 dark:text-white placeholder-gray-500 ${focusRing}`;
/** Short column headers for the routing matrix. */
const ROUTE_COLUMN_NAMES: Record<RouteChannel, string> = { telegram: 'Telegram', email: 'Email', messagebird: 'SMS' };
const SET_UP_FIRST = 'Set up this channel first';
const BELL_ALWAYS_ON = 'The bell always shows every enabled event';
const BELL_EVENT_OFF = 'Turn the event on to show it in the bell';
const checkbox = `h-4 w-4 rounded border-gray-300 text-blue-600 disabled:cursor-not-allowed disabled:opacity-40 dark:border-gray-600 ${focusRing}`;

const th = 'px-3 py-2 text-left font-mono text-[0.7rem] font-semibold uppercase tracking-[0.05em] text-gray-500 dark:text-gray-400';

function StatusPill({ className, children, wrap }: { className: string; children: ReactNode; wrap?: boolean }) {
  return (
    <span className={`${wrap ? 'inline-block break-words normal-case rounded-md' : 'whitespace-nowrap rounded-full'} px-2 py-1 font-mono text-[0.7rem] font-semibold uppercase leading-none tracking-[0.04em] ${className}`}>
      {children}
    </span>
  );
}

function Switch({ checked, label, disabled, onChange, small }: { checked: boolean; label: string; disabled?: boolean; onChange: () => void; small?: boolean }) {
  return (
    <button
      type="button"
      role="switch"
      aria-label={label}
      aria-checked={checked}
      onClick={onChange}
      disabled={disabled}
      className={`relative inline-flex ${small ? 'h-5 w-9' : 'h-6 w-11'} shrink-0 items-center rounded-full transition-colors disabled:cursor-not-allowed disabled:opacity-60 ${focusRing} ${
        checked ? 'bg-blue-600' : 'bg-gray-300 dark:bg-gray-600'
      }`}
    >
      <span
        className={`inline-block ${small ? 'h-3 w-3' : 'h-4 w-4'} transform rounded-full bg-white transition-transform ${
          checked ? (small ? 'translate-x-5' : 'translate-x-6') : 'translate-x-1'
        }`}
      />
    </button>
  );
}

export function NotificationsTab({
  notificationChannels,
  notificationEvents,
  notificationLogs,
  notificationLogsTotal,
  notificationLoading,
  testingChannel,
  testDestination,
  editingChannel,
  channelConfig,
  setTestDestination,
  setEditingChannel,
  setChannelConfig,
  onToggleChannel,
  onToggleEvent,
  onUpdateEventSeverity,
  notificationRoutes = [],
  onToggleRoute,
  onTestChannel,
  onSaveChannelConfig,
  getChannelIcon,
  getChannelConfigFields,
  formatAuditAction,
  canManage,
  canViewHistory = true,
  testResults = {},
}: NotificationsTabProps) {
  const [tab, setTab] = useState<TabId>('channels');
  const tabRefs = useRef<Record<TabId, HTMLButtonElement | null>>({ channels: null, rules: null, history: null });

  const statuses = notificationChannels.map((c) => ({
    channel: c,
    ...channelStatus(c, notificationLogs, canViewHistory, testResults[c.channel_type]),
  }));
  const problems = statuses.filter((s) => s.status === 'problem').length;
  // Failures that a later success on the same channel has not cleared.
  const failed = unresolvedFailures(notificationLogs);

  // A channel can be routed to once it has saved settings.
  const available = (channel: RouteChannel) => statuses.some((s) => s.channel.channel_type === channel && s.status !== 'unset');
  const routeOn = (eventType: string, channel: RouteChannel) =>
    available(channel) && !!notificationRoutes.find((r) => r.event_type === eventType)?.channels[channel];
  // Only routes that can fire: the event is on and the channel is set up.
  const routeCount = notificationEvents
    .filter((e) => e.enabled)
    .reduce((n, e) => n + ROUTE_CHANNELS.filter((c) => routeOn(e.event_type, c)).length, 0);

  const tabs: { id: TabId; label: string; badge: number; badgeTitle: string }[] = [
    { id: 'channels', label: 'Channels', badge: problems, badgeTitle: `${problems} with a problem` },
    { id: 'rules', label: 'What you get', badge: routeCount, badgeTitle: `${routeCount} routes to Telegram, email or SMS` },
    ...(canViewHistory ? [{ id: 'history' as TabId, label: 'Delivery history', badge: failed, badgeTitle: `${failed} failed since the last successful delivery` }] : []),
  ];

  const onTabKey = (e: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const delta = e.key === 'ArrowRight' ? 1 : e.key === 'ArrowLeft' ? -1 : 0;
    let next = index;
    if (delta) next = (index + delta + tabs.length) % tabs.length;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = tabs.length - 1;
    else return;
    e.preventDefault();
    setTab(tabs[next].id);
    tabRefs.current[tabs[next].id]?.focus();
  };

  const eventsByCategory = CATEGORY_ORDER.map((cat) => ({
    cat,
    events: notificationEvents.filter((e) => categoryOf(e.event_type) === cat),
  })).filter((g) => g.events.length > 0);

  return (
    <SettingsTabLayout title="Notifications" description="Where notifications go and which events send them. Everything also appears in the bell.">
      {notificationLoading ? (
        <div className="flex justify-center items-center py-12">
          <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500"></div>
        </div>
      ) : (
        <>
          <div role="tablist" aria-label="Notification settings" className="flex flex-wrap gap-1 border-b border-gray-200 dark:border-gray-700">
            {tabs.map((t, i) => (
              <button
                key={t.id}
                ref={(el) => {
                  tabRefs.current[t.id] = el;
                }}
                type="button"
                role="tab"
                id={`notif-tab-${t.id}`}
                aria-selected={tab === t.id}
                aria-controls={`notif-panel-${t.id}`}
                tabIndex={tab === t.id ? 0 : -1}
                onClick={() => setTab(t.id)}
                onKeyDown={(e) => onTabKey(e, i)}
                className={`-mb-px inline-flex items-center gap-1.5 border-b-2 px-3 py-2.5 text-sm font-semibold ${focusRing} ${
                  tab === t.id
                    ? 'border-blue-600 text-gray-900 dark:border-blue-400 dark:text-white'
                    : 'border-transparent text-gray-500 hover:text-gray-800 dark:text-gray-400 dark:hover:text-gray-200'
                }`}
              >
                {t.label}
                {t.badge > 0 && <CountBadge value={t.badge} kind={t.id === 'rules' ? 'neutral' : 'crit'} label={t.badgeTitle} />}
              </button>
            ))}
          </div>

          {tab === 'channels' && (
            <section
              role="tabpanel"
              id="notif-panel-channels"
              aria-labelledby="notif-tab-channels"
              className="grid gap-4 rounded-xl border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800 p-5"
            >
              <div>
                <h4 className="text-lg font-semibold text-gray-900 dark:text-white">Channels</h4>
                <p className="text-sm text-gray-500 dark:text-gray-400">
                  Where notifications can be sent. Test a channel to check it before you rely on it. Users pick their own destinations in Account.
                </p>
              </div>
              <div className="grid gap-3 [grid-template-columns:repeat(auto-fit,minmax(240px,1fr))]">
                <div data-testid="channel-card-bell" className="grid content-start gap-2 rounded-xl border border-gray-200 dark:border-gray-700 p-4">
                  <div className="flex items-center gap-2">
                    <span className="text-gray-500 dark:text-gray-400">
                      <Bell className="w-5 h-5" aria-hidden="true" />
                    </span>
                    <span className="flex-1 font-bold text-gray-900 dark:text-white">Bell (in Kubarr)</span>
                    <StatusPill className={STATUS_STYLE.working.className}>{STATUS_STYLE.working.label}</StatusPill>
                  </div>
                  <p className="font-mono text-[0.8rem] text-gray-500 dark:text-gray-400">Always on</p>
                </div>

                {statuses.map(({ channel, status, detail }) => {
                  const type = channel.channel_type;
                  const name = CHANNEL_NAMES[type] ?? type;
                  const style = STATUS_STYLE[status];
                  const editing = editingChannel === type;
                  return (
                    <div key={type} data-testid={`channel-card-${type}`} className="grid content-start gap-2.5 rounded-xl border border-gray-200 dark:border-gray-700 p-4">
                      <div className="flex items-center gap-2">
                        <span className={channel.enabled ? 'text-blue-600 dark:text-blue-400' : 'text-gray-500 dark:text-gray-400'}>{getChannelIcon(type)}</span>
                        <span className="flex-1 font-bold text-gray-900 dark:text-white">{name}</span>
                        <StatusPill className={style.className}>{style.label}</StatusPill>
                      </div>
                      <p className="font-mono text-[0.8rem] text-gray-500 dark:text-gray-400 break-words">{detail}</p>
                      <div className="flex items-center gap-2 text-sm text-gray-700 dark:text-gray-300">
                        <Switch
                          checked={channel.enabled}
                          label={`${type} notifications`}
                          disabled={!canManage}
                          onChange={() => onToggleChannel(type, !channel.enabled)}
                        />
                        <span>Send through this channel</span>
                      </div>

                      {canManage && editing && (
                        <div className="grid gap-2.5 border-t border-gray-200 dark:border-gray-700 pt-3">
                          {getChannelConfigFields(type).map((field) => (
                            <label key={field.key} className="grid gap-1 text-sm font-medium text-gray-700 dark:text-gray-300">
                              {field.label}
                              <input
                                type={field.type}
                                placeholder={field.placeholder}
                                value={channelConfig[type]?.[field.key] || ''}
                                onChange={(e) =>
                                  setChannelConfig((prev) => ({
                                    ...prev,
                                    [type]: { ...prev[type], [field.key]: e.target.value },
                                  }))
                                }
                                className={input}
                              />
                            </label>
                          ))}
                          <div className="flex gap-2">
                            <button type="button" onClick={() => onSaveChannelConfig(type)} className={btnPrimary}>
                              Save
                            </button>
                            <button type="button" onClick={() => setEditingChannel(null)} className={btn}>
                              Cancel
                            </button>
                          </div>
                        </div>
                      )}

                      {canManage && !editing && (
                        <div className="grid gap-2">
                          <div className="flex flex-wrap gap-2">
                            <button
                              type="button"
                              className={btn}
                              onClick={() => {
                                setEditingChannel(type);
                                setChannelConfig((prev) => ({ ...prev, [type]: (channel.config as Record<string, string>) || {} }));
                              }}
                            >
                              {status === 'unset' ? 'Set up' : 'Edit'}
                            </button>
                          </div>
                          {channel.enabled && (
                            <div className="flex flex-wrap items-center gap-2">
                              <input
                                type="text"
                                aria-label={`${name} test destination`}
                                placeholder={type === 'email' ? 'test@example.com' : type === 'telegram' ? 'Chat ID' : 'Phone number'}
                                value={testDestination[type] || ''}
                                onChange={(e) => setTestDestination((prev) => ({ ...prev, [type]: e.target.value }))}
                                className={`${input} flex-1 min-w-[140px] w-auto`}
                              />
                              <button type="button" className={btn} onClick={() => onTestChannel(type)} disabled={testingChannel === type}>
                                {testingChannel === type ? 'Sending…' : 'Send test'}
                              </button>
                            </div>
                          )}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            </section>
          )}

          {tab === 'rules' && (
            <section
              role="tabpanel"
              id="notif-panel-rules"
              aria-labelledby="notif-tab-rules"
              className="grid gap-4 rounded-xl border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800 p-5"
            >
              <div>
                <h4 className="text-lg font-semibold text-gray-900 dark:text-white">What you get</h4>
                <p className="text-sm text-gray-500 dark:text-gray-400">
                  Turn an event on to show it in the bell, then tick a channel to also send it there. Severity sets the icon and whether it counts as needing attention.
                </p>
              </div>
              <div className="overflow-x-auto">
                <table className="w-full min-w-[640px] border-collapse text-sm">
                  <thead>
                    <tr className="border-b border-gray-200 dark:border-gray-700">
                      <th className={th}>Event</th>
                      <th className={th}>Severity</th>
                      <th className={`${th} text-center`}>On</th>
                      <th className={`${th} text-center`}>Bell</th>
                      {ROUTE_CHANNELS.map((c) => (
                        <th key={c} className={`${th} text-center`}>
                          {ROUTE_COLUMN_NAMES[c]}
                          {!available(c) && (
                            <span title={SET_UP_FIRST} aria-label={SET_UP_FIRST}>
                              {' ·'}
                            </span>
                          )}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {eventsByCategory.map(({ cat, events }) => (
                      <CategoryRows key={cat} label={CATEGORY_LABELS[cat]}>
                        {events.map((event) => {
                          const label = formatAuditAction(event.event_type);
                          return (
                            <tr key={event.event_type} className="border-b border-gray-200 dark:border-gray-700">
                              <td className="px-3 py-2 font-semibold text-gray-900 dark:text-white">{label}</td>
                              <td className="px-3 py-2">
                                <select
                                  aria-label={`${label} severity`}
                                  value={event.severity}
                                  onChange={(e) => onUpdateEventSeverity(event.event_type, e.target.value)}
                                  disabled={!canManage}
                                  className={`rounded-lg border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-900 px-2 py-1 text-[0.8rem] text-gray-900 dark:text-white ${focusRing}`}
                                >
                                  <option value="info">Info</option>
                                  <option value="warning">Warning</option>
                                  <option value="critical">Critical</option>
                                </select>
                              </td>
                              <td className="px-3 py-2 text-center">
                                <Switch
                                  small
                                  checked={event.enabled}
                                  label={`${label} notifications`}
                                  disabled={!canManage}
                                  onChange={() => onToggleEvent(event.event_type, !event.enabled)}
                                />
                              </td>
                              <td className="px-3 py-2 text-center">
                                <input
                                  type="checkbox"
                                  className={checkbox}
                                  checked={event.enabled}
                                  readOnly
                                  disabled
                                  aria-label={`${label} in the bell`}
                                  title={event.enabled ? BELL_ALWAYS_ON : BELL_EVENT_OFF}
                                />
                              </td>
                              {ROUTE_CHANNELS.map((c) => {
                                const on = routeOn(event.event_type, c);
                                return (
                                  <td key={c} className="px-3 py-2 text-center">
                                    <input
                                      type="checkbox"
                                      className={checkbox}
                                      checked={on}
                                      disabled={!canManage || !available(c) || !onToggleRoute}
                                      title={available(c) ? undefined : SET_UP_FIRST}
                                      aria-label={`${label} via ${ROUTE_COLUMN_NAMES[c]}`}
                                      onChange={() => onToggleRoute?.(event.event_type, c, !on)}
                                    />
                                  </td>
                                );
                              })}
                            </tr>
                          );
                        })}
                      </CategoryRows>
                    ))}
                  </tbody>
                </table>
              </div>
            </section>
          )}

          {tab === 'history' && canViewHistory && (
            <section
              role="tabpanel"
              id="notif-panel-history"
              aria-labelledby="notif-tab-history"
              className="grid gap-4 rounded-xl border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800 p-5"
            >
              <div className="flex flex-wrap items-baseline justify-between gap-2">
                <div>
                  <h4 className="text-lg font-semibold text-gray-900 dark:text-white">Delivery history</h4>
                  <p className="text-sm text-gray-500 dark:text-gray-400">Every message Kubarr tried to send outside the bell.</p>
                </div>
                <span className="text-sm text-gray-500 dark:text-gray-400">
                  {notificationLogs.length < notificationLogsTotal ? `Latest ${notificationLogs.length} of ${notificationLogsTotal}` : `${notificationLogsTotal} total`}
                </span>
              </div>
              {notificationLogs.length === 0 ? (
                <p className="py-8 text-center text-gray-500 dark:text-gray-400">No notification logs yet.</p>
              ) : (
                <div className="overflow-x-auto">
                  <table className="w-full min-w-[620px] border-collapse text-sm">
                    <thead>
                      <tr className="border-b border-gray-200 dark:border-gray-700">
                        <th className={th}>When</th>
                        <th className={th}>Event</th>
                        <th className={th}>Channel</th>
                        <th className={th}>Recipient</th>
                        <th className={th}>Result</th>
                      </tr>
                    </thead>
                    <tbody>
                      {notificationLogs.map((log) => (
                        <tr key={log.id} className="border-b border-gray-200 dark:border-gray-700 align-top">
                          <td className="px-3 py-2 whitespace-nowrap font-mono text-[0.78rem] tabular-nums text-gray-600 dark:text-gray-300">
                            {new Date(log.created_at).toLocaleString()}
                          </td>
                          <td className="px-3 py-2 text-gray-900 dark:text-white">{formatAuditAction(log.event_type)}</td>
                          <td className="px-3 py-2 text-gray-700 dark:text-gray-300">{CHANNEL_NAMES[log.channel_type] ?? log.channel_type}</td>
                          <td className="px-3 py-2 font-mono text-[0.78rem] text-gray-600 dark:text-gray-300">{log.recipient ?? '—'}</td>
                          <td className="px-3 py-2">
                            {log.status === 'sent' ? (
                              <StatusPill className={STATUS_STYLE.working.className}>Delivered</StatusPill>
                            ) : (
                              <StatusPill wrap className={STATUS_STYLE.problem.className}>
                                <span title={log.error_message || ''}>Failed{log.error_message ? `: ${log.error_message}` : ''}</span>
                              </StatusPill>
                            )}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </section>
          )}
        </>
      )}
    </SettingsTabLayout>
  );
}

function CategoryRows({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <tr>
        <td
          data-testid="category-row"
          colSpan={3 + 1 + ROUTE_CHANNELS.length}
          className="bg-gray-50 dark:bg-gray-900/40 px-3 py-2 text-left font-mono text-[0.72rem] font-bold uppercase tracking-[0.07em] text-gray-500 dark:text-gray-400"
        >
          {label}
        </td>
      </tr>
      {children}
    </>
  );
}
