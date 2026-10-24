import { describe, it, expect, vi } from 'vitest';
import axios from 'axios';

// We need to test the module fresh each time
describe('apiClient', () => {
  it('keeps expected anonymous session probes quiet without swallowing failures', async () => {
    vi.resetModules();
    window.location.pathname = '/login';
    const logged = vi.spyOn(console, 'error').mockImplementation(() => {});
    try {
      const { default: apiClient } = await import('../client');
      const interceptor = (apiClient.interceptors.response as unknown as { handlers: Array<{ rejected: (err: unknown) => unknown }> }).handlers[0];
      for (const [url, status, shouldLog] of [
        ['/users/me', 401, false], ['/oauth/available', 401, true], ['/users/me', 500, true],
      ] as const) {
        logged.mockClear();
        const error = { config: { url }, response: { status, data: { detail: 'Request failed' } } };
        await expect(interceptor.rejected(error)).rejects.toBe(error);
        expect(logged.mock.calls.length > 0).toBe(shouldLog);
      }
    } finally {
      logged.mockRestore();
    }
  });
  it('creates an axios instance with correct defaults', async () => {
    const createSpy = vi.spyOn(axios, 'create');
    // Re-import to trigger module execution
    vi.resetModules();
    await import('../client');

    expect(createSpy).toHaveBeenCalledWith(
      expect.objectContaining({
        baseURL: '/api',
        timeout: 10000,
        withCredentials: true,
        headers: { 'Content-Type': 'application/json' },
      }),
    );
    createSpy.mockRestore();
  });

  it('redirects to /login on 401 when not already on /login', async () => {
    vi.resetModules();
    window.location.pathname = '/dashboard';

    const { default: apiClient } = await import('../client');

    // Simulate a 401 response through the interceptor
    const error = {
      response: { status: 401, data: { detail: 'Unauthorized' } },
      message: 'Request failed with status code 401',
    };

    // Find the error handler from the response interceptor
    const interceptor = (apiClient.interceptors.response as unknown as { handlers: Array<{ rejected: (err: unknown) => unknown }> }).handlers[0];
    await expect(interceptor.rejected(error)).rejects.toBe(error);

    expect(window.location.href).toBe('/login');
  });

  it('does not redirect on 401 when already on /login', async () => {
    vi.resetModules();
    window.location.pathname = '/login';

    const { default: apiClient } = await import('../client');

    const error = {
      response: { status: 401, data: { detail: 'Unauthorized' } },
      message: 'Request failed with status code 401',
    };

    const interceptor = (apiClient.interceptors.response as unknown as { handlers: Array<{ rejected: (err: unknown) => unknown }> }).handlers[0];
    await expect(interceptor.rejected(error)).rejects.toBe(error);

    // Should NOT have redirected
    expect(window.location.href).not.toBe('/login');
  });
});
