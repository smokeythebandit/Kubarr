import { useCallback, useRef, useState } from 'react';
import { notificationsApi, type NotificationRoute, type RouteChannel } from '../api/notifications';

const OFF: Record<RouteChannel, boolean> = { telegram: false, email: false, messagebird: false };

function withRoute(routes: NotificationRoute[], route: NotificationRoute): NotificationRoute[] {
  return routes.some((r) => r.event_type === route.event_type)
    ? routes.map((r) => (r.event_type === route.event_type ? route : r))
    : [...routes, route];
}

/**
 * Routes for "What you get". A toggle shows at once; only the response to the
 * newest request for that event is applied, so a slow earlier response cannot
 * undo a later toggle.
 */
export function useNotificationRoutes(onError: (message: string) => void) {
  const [routes, setRoutes] = useState<NotificationRoute[]>([]);
  const seq = useRef<Record<string, number>>({});

  const load = useCallback(async () => {
    try {
      setRoutes(await notificationsApi.getRoutes());
    } catch (err: unknown) {
      console.error('Failed to load notification routes:', err);
    }
  }, []);

  const toggle = useCallback(
    async (eventType: string, channel: RouteChannel, enabled: boolean) => {
      const mine = (seq.current[eventType] ?? 0) + 1;
      seq.current[eventType] = mine;
      setRoutes((prev) => {
        const current = prev.find((r) => r.event_type === eventType)?.channels ?? OFF;
        return withRoute(prev, { event_type: eventType, channels: { ...current, [channel]: enabled } });
      });
      try {
        const updated = await notificationsApi.updateRoute(eventType, { [channel]: enabled });
        if (seq.current[eventType] === mine) setRoutes((prev) => withRoute(prev, updated));
      } catch (err: unknown) {
        if (seq.current[eventType] !== mine) return;
        const error = err as { response?: { data?: { detail?: string } } };
        onError(error.response?.data?.detail || 'Failed to update where this event is sent');
        await load();
      }
    },
    [load, onError],
  );

  return { routes, load, toggle };
}
