import type { NotificationChannel, NotificationLog } from '../../../api/notifications';

export type ChannelStatus = 'working' | 'off' | 'unset' | 'problem';

export const CHANNEL_NAMES: Record<string, string> = {
  email: 'Email',
  telegram: 'Telegram',
  messagebird: 'SMS (MessageBird)',
};

function hasConfig(channel: NotificationChannel): boolean {
  return !!channel.config && Object.values(channel.config).some((v) => v !== '' && v !== null && v !== undefined);
}

/**
 * Card status for a channel. "Problem" means the channel is enabled and its
 * most recent delivery (in the loaded logs) failed.
 */
export function channelStatus(channel: NotificationChannel, logs: NotificationLog[]): { status: ChannelStatus; detail: string } {
  const latest = logs
    .filter((l) => l.channel_type === channel.channel_type)
    .reduce<NotificationLog | null>((best, l) => (!best || l.created_at > best.created_at || (l.created_at === best.created_at && l.id > best.id) ? l : best), null);

  if (!channel.enabled) {
    return hasConfig(channel) ? { status: 'off', detail: 'Configured, turned off' } : { status: 'unset', detail: 'No settings saved yet' };
  }
  if (latest?.status === 'failed') {
    return { status: 'problem', detail: `Last delivery failed${latest.error_message ? `: ${latest.error_message}` : ''}` };
  }
  if (latest) {
    return { status: 'working', detail: `Last delivered ${new Date(latest.created_at).toLocaleString()}` };
  }
  return { status: 'working', detail: 'Enabled, nothing sent yet' };
}
