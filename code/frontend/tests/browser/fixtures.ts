import { test as base, expect, type Request } from '@playwright/test';
import type { User } from '../../src/api/users';

export const timestamp = '2026-01-01T00:00:00Z';
export const admin: User = {
  id: 101, username: 'browser-admin', email: 'admin@example.test',
  is_active: true, is_approved: true, created_at: timestamp, updated_at: timestamp,
  roles: [{ id: 1, name: 'admin', description: 'Browser fixture administrator' }], preferences: { theme: 'light' },
  permissions: [], allowed_apps: [],
};

type Reply = { json: unknown; status?: number };
type Handler = (request: Request) => Reply | Promise<Reply>;
type MockApi = {
  on: (method: string, path: string, handler: Handler) => void;
  get: (path: string, json: unknown) => void;
  calls: string[];
};

export const test = base.extend<{ api: MockApi }>({
  api: [async ({ context, baseURL }, use) => {
    const handlers = new Map<string, Handler>();
    const unexpected: string[] = [];
    const errors: string[] = [];
    const api: MockApi = {
      on: (method, path, handler) => { handlers.set(`${method} ${path}`, handler); },
      get: (path, json) => { handlers.set(`GET ${path}`, () => ({ json })); },
      calls: [],
    };

    // Authentication is explicit API state, not a real login or a reused cookie.
    api.get('/api/users/me', admin);
    api.get('/auth/accounts', [{ slot: 0, user_id: admin.id, username: admin.username, email: admin.email, is_active: true }]);
    api.get('/api/users/me/preferences', admin.preferences);
    api.get('/api/users', [admin]);
    api.get('/api/users/pending', []);
    api.get('/api/users/invites', []);
    api.get('/api/roles', []);
    api.get('/api/settings', { settings: {} });
    api.get('/api/apps/catalog', []);
    api.get('/api/apps/installed', []);
    api.get('/api/apps/states', []);
    api.get('/api/monitoring/vm/available', { available: false });
    api.get('/api/notifications/inbox/count', { count: 0 });

    context.on('page', page => page.on('pageerror', error => errors.push(error.message)));
    await context.route('**/*', async route => {
      const request = route.request();
      const url = new URL(request.url());
      const key = `${request.method()} ${url.pathname}${url.search}`;
      const local = url.origin === baseURL;
      if (local && !/^\/(api|auth)(\/|$)/.test(url.pathname)) {
        await route.continue();
        return;
      }
      api.calls.push(key);
      const handler = local ? handlers.get(key) : undefined;
      if (!handler) {
        unexpected.push(`${key} (${url.origin})`);
        await route.abort('blockedbyclient');
        return;
      }
      await route.fulfill(await handler(request));
    });
    await context.routeWebSocket('**/*', socket => {
      const url = new URL(socket.url());
      if (url.origin === baseURL?.replace('http:', 'ws:') && url.pathname === '/') {
        socket.connectToServer(); // Local Vite HMR only.
      } else {
        unexpected.push(`WebSocket ${socket.url()}`);
        socket.close();
      }
    });
    await use(api);
    await context.unrouteAll({ behavior: 'wait' });
    expect(unexpected, 'Unmocked API or external requests').toEqual([]);
    expect(errors, 'Uncaught browser errors').toEqual([]);
  }, { auto: true }],
});

export { expect };
