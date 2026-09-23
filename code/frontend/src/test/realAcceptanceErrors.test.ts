import { describe, expect, it } from 'vitest';
import {
  isOptionalCatalogIcon404,
  isOptionalCatalogIconConsoleError,
} from '../../tests/real/http-errors';

describe('real acceptance optional icon error classification', () => {
  const iconPath = '/api/apps/catalog/builtin-app/icon';
  const iconUrl = `http://127.0.0.1:43123${iconPath}`;

  it('allows only a GET 404 for a safe catalog icon path', () => {
    expect(isOptionalCatalogIcon404({ status: 404, method: 'GET', path: iconPath })).toBe(true);
    expect(isOptionalCatalogIcon404({ status: 500, method: 'GET', path: iconPath })).toBe(false);
    expect(isOptionalCatalogIcon404({ status: 404, method: 'DELETE', path: iconPath })).toBe(false);
  });

  it('does not classify auth or non-icon 404 responses as optional assets', () => {
    expect(isOptionalCatalogIcon404({ status: 404, method: 'GET', path: '/api/users/me' })).toBe(false);
    expect(isOptionalCatalogIcon404({ status: 404, method: 'GET', path: '/api/apps/catalog/builtin-app' })).toBe(false);
    expect(isOptionalCatalogIcon404({ status: 404, method: 'GET', path: '/api/apps/catalog/../secret/icon' })).toBe(false);
  });

  it('matches only the resource console error associated with an observed icon 404', () => {
    const observed = new Set([iconUrl]);
    expect(isOptionalCatalogIconConsoleError({
      text: 'Failed to load resource: the server responded with a status of 404 (Not Found)',
      locationUrl: iconUrl,
    }, 'http://127.0.0.1:43123', observed)).toBe(true);

    expect(isOptionalCatalogIconConsoleError({
      text: 'Uncaught TypeError: application crashed',
      locationUrl: iconUrl,
    }, 'http://127.0.0.1:43123', observed)).toBe(false);
    expect(isOptionalCatalogIconConsoleError({
      text: 'Failed to load resource: the server responded with a status of 404 (Not Found)',
      locationUrl: 'http://127.0.0.1:43123/api/users/me',
    }, 'http://127.0.0.1:43123', observed)).toBe(false);
  });
});
