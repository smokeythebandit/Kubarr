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
 * undo a later toggle. Reloads are sequenced the same way.
 */
export function useNotificationRoutes(onError: (message: string) => void) {
  const [routes, setRoutes] = useState<NotificationRoute[]>([]);
  const seq = useRef<Record<string, number>>({});

  const loads = useRef(0);

  // A load applies only if no newer load started, and never overwrites an event
  // toggled after the load began (its own response will settle it).
  const load = useCallback(async () => {
    const mine = ++loads.current;
    const started = { ...seq.current };
    try {
      const loaded = await notificationsApi.getRoutes();
      if (loads.current !== mine) return;
      setRoutes((prev) => {
        const toggled = (eventType: string) => seq.current[eventType] !== started[eventType];
        const kept = loaded.map((r) => (toggled(r.event_type) ? (prev.find((p) => p.event_type === r.event_type) ?? r) : r));
        const missing = prev.filter((p) => toggled(p.event_type) && !loaded.some((r) => r.event_type === p.event_type));
        return [...kept, ...missing];
      });
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
