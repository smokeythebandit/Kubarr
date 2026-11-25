import { describe, it, expect, vi, beforeEach } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import type { NotificationRoute } from '../api/notifications';

const { getRoutes, updateRoute } = vi.hoisted(() => ({ getRoutes: vi.fn(), updateRoute: vi.fn() }));
vi.mock('../api/notifications', () => ({ notificationsApi: { getRoutes, updateRoute } }));

import { useNotificationRoutes } from './useNotificationRoutes';

const route = (telegram: boolean, email = false): NotificationRoute => ({
  event_type: 'login_failed',
  channels: { telegram, email, messagebird: false },
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

beforeEach(() => {
  getRoutes.mockReset();
  updateRoute.mockReset();
});

describe('useNotificationRoutes', () => {
  it('sends only the toggled channel and shows it at once', async () => {
    getRoutes.mockResolvedValue([route(false)]);
    const pending = deferred<NotificationRoute>();
    updateRoute.mockReturnValue(pending.promise);
    const { result } = renderHook(() => useNotificationRoutes(vi.fn()));
    await act(() => result.current.load());

    let done!: Promise<void>;
    act(() => {
      done = result.current.toggle('login_failed', 'telegram', true);
    });
    expect(updateRoute).toHaveBeenCalledWith('login_failed', { telegram: true });
    expect(result.current.routes[0].channels.telegram).toBe(true);
    await act(async () => {
      pending.resolve(route(true));
      await done;
    });
    expect(result.current.routes).toEqual([route(true)]);
  });

  it('ignores a stale response that arrives after a newer toggle', async () => {
    getRoutes.mockResolvedValue([route(false)]);
    const first = deferred<NotificationRoute>();
    const second = deferred<NotificationRoute>();
    updateRoute.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const { result } = renderHook(() => useNotificationRoutes(vi.fn()));
    await act(() => result.current.load());

    let a!: Promise<void>;
    let b!: Promise<void>;
    act(() => {
      a = result.current.toggle('login_failed', 'telegram', true);
    });
    act(() => {
      b = result.current.toggle('login_failed', 'telegram', false);
    });
    await act(async () => {
      second.resolve(route(false));
      await b;
    });
    await act(async () => {
      first.resolve(route(true));
      await a;
    });
    expect(result.current.routes[0].channels.telegram).toBe(false);
  });

  it('reports a failed toggle and reloads the saved routes', async () => {
    getRoutes.mockResolvedValue([route(false)]);
    updateRoute.mockRejectedValue({ response: { data: { detail: 'Invalid channel type' } } });
    const onError = vi.fn();
    const { result } = renderHook(() => useNotificationRoutes(onError));
    await act(() => result.current.load());
    await act(() => result.current.toggle('login_failed', 'email', true));
    expect(onError).toHaveBeenCalledWith('Invalid channel type');
    expect(result.current.routes).toEqual([route(false)]);
  });
});
