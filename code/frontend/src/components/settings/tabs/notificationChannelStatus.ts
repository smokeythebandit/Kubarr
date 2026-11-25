import type { NotificationChannel, NotificationLog } from '../../../api/notifications';

/** `enabled`: turned on, but delivery logs are not available to judge it. */
export type ChannelStatus = 'working' | 'enabled' | 'off' | 'unset' | 'problem';

export const CHANNEL_NAMES: Record<string, string> = {
  email: 'Email',
  telegram: 'Telegram',
  messagebird: 'SMS (MessageBird)',
};

/** Result of the last "Send test" in this session; newer than any loaded log. */
export type ChannelTestResult = { success: boolean; error: string | null; at: string };

function hasConfig(channel: NotificationChannel): boolean {
  return !!channel.config && Object.values(channel.config).some((v) => v !== '' && v !== null && v !== undefined);
}

function newer(a: NotificationLog, b: NotificationLog): boolean {
  return a.created_at > b.created_at || (a.created_at === b.created_at && a.id > b.id);
}

function latestFor(channelType: string, logs: NotificationLog[]): NotificationLog | null {
  return logs.filter((l) => l.channel_type === channelType).reduce<NotificationLog | null>((best, l) => (!best || newer(l, best) ? l : best), null);
}

/**
 * Card status for a channel.
 * - no saved settings → "Not set up" (even if switched on)
 * - saved but switched off → "Off"
 * - without delivery logs (no audit.view) → neutral "Enabled"
 * - latest delivery (or test) failed → "Problem", otherwise "Working"
 */
export function channelStatus(
  channel: NotificationChannel,
  logs: NotificationLog[],
  logsAvailable: boolean,
  test?: ChannelTestResult,
): { status: ChannelStatus; detail: string } {
  if (!hasConfig(channel)) {
    return { status: 'unset', detail: channel.enabled ? 'Switched on, but no settings saved yet' : 'No settings saved yet' };
  }
  if (!channel.enabled) return { status: 'off', detail: 'Configured, turned off' };

  const latest = logsAvailable ? latestFor(channel.channel_type, logs) : null;
  const testIsLatest = !!test && (!latest || test.at > latest.created_at);
  if (testIsLatest && test) {
    return test.success
      ? { status: 'working', detail: 'Test message sent' }
      : { status: 'problem', detail: `Test failed${test.error ? `: ${test.error}` : ''}` };
  }
  if (!logsAvailable) return { status: 'enabled', detail: 'Delivery history needs audit access' };
  if (latest?.status === 'failed') {
    return { status: 'problem', detail: `Last delivery failed${latest.error_message ? `: ${latest.error_message}` : ''}` };
  }
  if (latest) return { status: 'working', detail: `Last delivered ${new Date(latest.created_at).toLocaleString()}` };
  return { status: 'working', detail: 'No failed deliveries' };
}

/** Failed deliveries not followed by a later success on the same channel. */
export function unresolvedFailures(logs: NotificationLog[]): number {
  const lastSuccess = new Map<string, NotificationLog>();
  for (const l of logs) {
    if (l.status !== 'sent') continue;
    const prev = lastSuccess.get(l.channel_type);
    if (!prev || newer(l, prev)) lastSuccess.set(l.channel_type, l);
  }
  return logs.filter((l) => {
    if (l.status !== 'failed') return false;
    const ok = lastSuccess.get(l.channel_type);
    return !ok || newer(l, ok);
  }).length;
}
