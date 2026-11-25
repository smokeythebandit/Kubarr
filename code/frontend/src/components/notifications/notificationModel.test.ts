import { describe, expect, it } from 'vitest';
import type { Notification } from '../../api/notifications';
import {
  appOf,
  bellLabel,
  buildRows,
  categoryOf,
  dayGroupOf,
  formatCount,
  formatPartialCount,
  isUpdateEvent,
  matchesPageFilter,
  matchesPopoverFilter,
  needsAttention,
  nightKeyOf,
  toneOf,
  type ListRow,
} from './notificationModel';

// Local wall-clock times so day/night grouping is timezone independent.
const at = (day: number, hour: number, minute = 0) => new Date(2026, 10, day, hour, minute).toISOString();
const NOW = new Date(2026, 10, 25, 10, 0);

let nextId = 1;
function make(overrides: Partial<Notification> = {}): Notification {
  return {
    id: nextId++,
    title: 'Title',
    message: 'Message',
    event_type: 'login',
    severity: 'info',
    read: true,
    created_at: at(25, 9),
    ...overrides,
  };
}

const describeRows = (rows: ListRow[]) =>
  rows.map((r) => (r.kind === 'header' ? `# ${r.label} ${r.count}/${r.unread}` : r.kind === 'stack' ? `+ ${r.label}` : `- ${r.notification.id}`));

describe('categoryOf', () => {
  it.each([
    ['app_updated', 'updates'],
    ['app_update_failed', 'updates'],
    ['app_update_rollback_failed', 'updates'],
    ['app_installed', 'apps'],
    ['app_restarted', 'apps'],
    ['login', 'security'],
    ['login_failed', 'security'],
    ['logout', 'security'],
    ['2fa_enabled', 'security'],
    ['password_changed', 'security'],
    ['user_created', 'security'],
    ['role_assigned', 'security'],
    ['invite_used', 'security'],
    ['system_setting_changed', 'system'],
    ['vpn_provider_created', 'system'],
    ['something_new', 'system'],
    [null, 'system'],
  ] as const)('%s → %s', (eventType, category) => {
    expect(categoryOf(eventType)).toBe(category);
  });

  it('treats only app_update* and app_updated as updates', () => {
    expect(isUpdateEvent('app_updated')).toBe(true);
    expect(isUpdateEvent('app_update_failed')).toBe(true);
    expect(isUpdateEvent('app_uninstalled')).toBe(false);
    expect(isUpdateEvent(null)).toBe(false);
  });
});

describe('notification fields', () => {
  it('needs attention only when unread and warning or critical', () => {
    expect(needsAttention(make({ read: false, severity: 'critical' }))).toBe(true);
    expect(needsAttention(make({ read: false, severity: 'warning' }))).toBe(true);
    expect(needsAttention(make({ read: true, severity: 'critical' }))).toBe(false);
    expect(needsAttention(make({ read: false, severity: 'info' }))).toBe(false);
  });

  it('derives the app chip from update details', () => {
    expect(appOf(make({ message: 'App updated: app=sonarr from=4.0.1 to=4.0.2' }))).toBe('sonarr');
    expect(appOf(make({ message: 'App update failed: app=prowlarr to=2.7.0+10; rolled back' }))).toBe('prowlarr');
    expect(appOf(make({ message: 'User admin logged in successfully' }))).toBeNull();
  });

  it('shows healthy updates as success', () => {
    expect(toneOf(make({ event_type: 'app_updated' }))).toBe('success');
    expect(toneOf(make({ event_type: 'app_update_failed', severity: 'warning' }))).toBe('warning');
    expect(toneOf(make({ severity: 'critical' }))).toBe('critical');
    expect(toneOf(make())).toBe('info');
  });

  it('caps counts and labels the bell', () => {
    expect(formatCount(99)).toBe('99');
    expect(formatCount(100)).toBe('99+');
    expect(formatPartialCount(3, true)).toBe('3');
    expect(formatPartialCount(3, false)).toBe('3+');
    expect(bellLabel(3, 0)).toBe('Notifications, 3 unread');
    expect(bellLabel(3, 1)).toBe('Notifications, 3 unread, 1 critical');
  });
});

describe('grouping', () => {
  it('uses calendar days for Today / Yesterday / Earlier', () => {
    expect(dayGroupOf(at(25, 0, 5), NOW)).toBe('Today');
    expect(dayGroupOf(at(24, 23, 55), NOW)).toBe('Yesterday');
    expect(dayGroupOf(at(23, 12), NOW)).toBe('Earlier');
  });

  it('keys nights from 18:00 to noon and skips afternoons', () => {
    expect(nightKeyOf(at(24, 23))).toBe(nightKeyOf(at(25, 4)));
    expect(nightKeyOf(at(25, 11))).toBe(nightKeyOf(at(25, 4)));
    expect(nightKeyOf(at(25, 15))).toBeNull();
    expect(nightKeyOf(at(25, 19))).not.toBe(nightKeyOf(at(25, 4)));
  });

  it('pins Needs you, then day groups with counts and unread', () => {
    const crit = make({ severity: 'critical', read: false, created_at: at(23, 8) });
    const today1 = make({ read: false, created_at: at(25, 9) });
    const today2 = make({ read: true, created_at: at(25, 8) });
    const yesterday = make({ read: true, created_at: at(24, 15) });
    const readWarning = make({ severity: 'warning', read: true, created_at: at(22, 9) });
    const rows = buildRows([today1, today2, yesterday, crit, readWarning], NOW);
    expect(describeRows(rows)).toEqual([
      '# Needs you 1/1',
      `- ${crit.id}`,
      '# Today 2/1',
      `- ${today1.id}`,
      `- ${today2.id}`,
      '# Yesterday 1/0',
      `- ${yesterday.id}`,
      '# Earlier 1/0',
      `- ${readWarning.id}`,
    ]);
  });

  it('collapses consecutive successful updates from the same night', () => {
    const u1 = make({ event_type: 'app_updated', read: false, created_at: at(25, 4, 30) });
    const u2 = make({ event_type: 'app_updated', read: false, created_at: at(25, 4, 20) });
    const u3 = make({ event_type: 'app_updated', read: false, created_at: at(25, 4, 10) });
    const other = make({ created_at: at(25, 3) });
    const rows = buildRows([u1, u2, u3, other], NOW);
    expect(describeRows(rows)).toEqual(['# Today 4/3', `- ${u1.id}`, '+ +2 more updates last night', `- ${other.id}`]);

    const stack = rows.find((r) => r.kind === 'stack');
    expect(stack?.kind === 'stack' && stack.stackId).toMatch(/^app_updated@/);
    const key = stack?.kind === 'stack' ? stack.stackId : '';
    const open = buildRows([u1, u2, u3, other], NOW, new Set([key]));
    expect(describeRows(open)).toEqual(['# Today 4/3', `- ${u1.id}`, `- ${u2.id}`, `- ${u3.id}`, `- ${other.id}`]);
    // The run stays expanded after its first item is dismissed.
    const afterDismiss = buildRows([u2, u3, other], NOW, new Set([key]));
    expect(describeRows(afterDismiss)).toEqual(['# Today 3/2', `- ${u2.id}`, `- ${u3.id}`, `- ${other.id}`]);
  });

  it('does not collapse across a failure, different nights or afternoons', () => {
    const u1 = make({ event_type: 'app_updated', created_at: at(25, 5) });
    const failed = make({ event_type: 'app_update_failed', severity: 'info', created_at: at(25, 4, 30) });
    const u2 = make({ event_type: 'app_updated', created_at: at(25, 4) });
    const afternoonA = make({ event_type: 'app_updated', created_at: at(24, 15) });
    const afternoonB = make({ event_type: 'app_updated', created_at: at(24, 14) });
    const rows = buildRows([u1, failed, u2, afternoonA, afternoonB], NOW);
    expect(rows.some((r) => r.kind === 'stack')).toBe(false);
  });
});

describe('filters', () => {
  const update = make({ event_type: 'app_update_failed', severity: 'warning', read: false, created_at: at(25, 9) });
  const login = make({ event_type: 'login', created_at: at(20, 9) });
  const crit = make({ event_type: 'system_setting_changed', severity: 'critical', read: false, created_at: at(25, 9) });
  const readCrit = make({ event_type: 'system_setting_changed', severity: 'critical', read: true, created_at: at(25, 9) });

  it('popover pills', () => {
    expect([update, login, crit].filter((n) => matchesPopoverFilter(n, 'unread'))).toEqual([update, crit]);
    expect([update, login, crit].filter((n) => matchesPopoverFilter(n, 'attention'))).toEqual([update, crit]);
    expect([update, login, crit].filter((n) => matchesPopoverFilter(n, 'updates'))).toEqual([update]);
    expect([update, login, crit].filter((n) => matchesPopoverFilter(n, 'all'))).toHaveLength(3);
  });

  it('page facets and counters', () => {
    const all = [update, login, crit, readCrit];
    expect(all.filter((n) => matchesPageFilter(n, 'critical', NOW))).toEqual([crit]);
    expect(all.filter((n) => matchesPageFilter(n, 'recent', NOW))).toEqual([update, crit, readCrit]);
    expect(all.filter((n) => matchesPageFilter(n, 'security', NOW))).toEqual([login]);
    expect(all.filter((n) => matchesPageFilter(n, 'system', NOW))).toEqual([crit, readCrit]);
    expect(all.filter((n) => matchesPageFilter(n, 'updates', NOW))).toEqual([update]);
  });
});
