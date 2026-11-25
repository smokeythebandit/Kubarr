import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useCallback } from 'react';
import { notificationsApi, type UnreadCounts } from '../../api/notifications';
import { useAuth } from '../../contexts/AuthContext';

export const NOTIFICATION_COUNTS_KEY = ['notifications', 'counts'] as const;
const POLL_MS = 30_000;

/** Unread + unread-critical counts, polled every 30 s and shared by the bell and nav. */
export function useNotificationCounts(): UnreadCounts {
  const { isAuthenticated } = useAuth();
  const { data } = useQuery({
    queryKey: NOTIFICATION_COUNTS_KEY,
    queryFn: notificationsApi.getUnreadCounts,
    enabled: isAuthenticated,
    refetchInterval: POLL_MS,
    retry: false,
  });
  return data ?? { count: 0, critical: 0 };
}

/** Refetch the shared counts after a read/dismiss so the bell and nav agree. */
export function useRefreshNotificationCounts(): () => void {
  const queryClient = useQueryClient();
  return useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: NOTIFICATION_COUNTS_KEY });
  }, [queryClient]);
}
