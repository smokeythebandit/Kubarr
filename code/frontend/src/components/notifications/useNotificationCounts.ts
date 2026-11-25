import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback } from 'react';
import { notificationsApi, type Notification, type UnreadCounts } from '../../api/notifications';
import { useAuth } from '../../contexts/AuthContext';

export const NOTIFICATION_COUNTS_KEY = ['notifications', 'counts'] as const;
const POLL_MS = 30_000;
const ZERO: UnreadCounts = { count: 0, critical: 0, attention: 0 };

/** Server-side unread / critical / needs-you counts, polled every 30 s and shared by the bell, nav and page. */
export function useNotificationCounts(): UnreadCounts {
  const { isAuthenticated } = useAuth();
  const { data } = useQuery({
    queryKey: NOTIFICATION_COUNTS_KEY,
    queryFn: notificationsApi.getUnreadCounts,
    enabled: isAuthenticated,
    refetchInterval: POLL_MS,
    retry: false,
  });
  return data ?? ZERO;
}

/**
 * After notifications are read or dismissed: adjust the shared counts at once
 * (so the bell and nav react immediately), then refetch the server's numbers.
 */
export function useSettleNotificationCounts(): (gone: Notification[] | 'all') => void {
  const queryClient = useQueryClient();
  return useCallback(
    (gone) => {
      queryClient.setQueryData<UnreadCounts>(NOTIFICATION_COUNTS_KEY, (prev) => {
        if (!prev) return prev;
        if (gone === 'all') return ZERO;
        const unread = gone.filter((n) => !n.read);
        const less = (value: number, pick: (n: Notification) => boolean) => Math.max(0, value - unread.filter(pick).length);
        return {
          count: less(prev.count, () => true),
          critical: less(prev.critical, (n) => n.severity === 'critical'),
          attention: less(prev.attention, (n) => n.severity === 'critical' || n.severity === 'warning'),
        };
      });
      void queryClient.invalidateQueries({ queryKey: NOTIFICATION_COUNTS_KEY });
    },
    [queryClient],
  );
}
