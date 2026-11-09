import { describe, it, expect, vi, afterEach } from 'vitest';
import type { DomainConfig } from '../../api/domains';
import { buildInviteUrl, copyText, isPrivateHost, publicBaseUrl } from '../inviteLink';

const domain = (overrides: Partial<DomainConfig>): DomainConfig => ({
  id: 1, domain: 'example.com', kind: 'root', scope: 'public', primary: true, enabled: true,
  dns_mode: 'manual', ddns_profile_id: null, dns_status: 'ok', tls_mode: 'letsencrypt',
  letsencrypt_profile_id: null, tls_secret_name: null, certificate_status: 'ready',
  certificate_expires_at: null, created_at: '', updated_at: '', ...overrides,
});

describe('publicBaseUrl', () => {
  it('uses the enabled primary public domain over https', () => {
    expect(publicBaseUrl([domain({})])).toBe('https://example.com');
  });

  it('uses http when the domain has no TLS', () => {
    expect(publicBaseUrl([domain({ tls_mode: 'none' })])).toBe('http://example.com');
  });

  it('ignores disabled, private-only, wildcard and non-primary domains', () => {
    expect(publicBaseUrl([
      domain({ enabled: false }),
      domain({ scope: 'private' }),
      domain({ kind: 'wildcard', domain: '*.example.com' }),
      domain({ primary: false }),
    ])).toBeNull();
  });

  it('returns null when no domains are configured', () => {
    expect(publicBaseUrl([])).toBeNull();
    expect(publicBaseUrl(undefined)).toBeNull();
  });
});

describe('buildInviteUrl', () => {
  it('builds the registration link on the given base', () => {
    expect(buildInviteUrl('a b', 'https://example.com')).toBe('https://example.com/login?register=1&invite=a%20b');
  });

  it('falls back to the current origin', () => {
    expect(buildInviteUrl('abc')).toBe(`${window.location.origin}/login?register=1&invite=abc`);
  });
});

describe('isPrivateHost', () => {
  it.each([
    'http://192.168.1.240:8080/x', 'http://10.0.0.5/', 'http://172.16.3.1/', 'http://127.0.0.1/',
    'http://localhost:5173/', 'http://kubarr.local/', 'http://nas.lan/', 'http://kubarr/', 'http://[::1]/',
  ])('flags %s', url => expect(isPrivateHost(url)).toBe(true));

  it.each(['https://void-zero.com/login', 'http://172.32.0.1/', 'https://8.8.8.8/'])(
    'allows %s', url => expect(isPrivateHost(url)).toBe(false));
});

describe('copyText', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('uses the async clipboard when available', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText } });
    await expect(copyText('hello')).resolves.toBe(true);
    expect(writeText).toHaveBeenCalledWith('hello');
  });

  it('falls back to execCommand when the clipboard API is missing (plain HTTP)', async () => {
    vi.stubGlobal('navigator', { ...navigator, clipboard: undefined });
    const exec = vi.fn().mockReturnValue(true);
    document.execCommand = exec;
    await expect(copyText('hello')).resolves.toBe(true);
    expect(exec).toHaveBeenCalledWith('copy');
    expect(document.querySelector('textarea')).toBeNull();
  });

  it('reports failure instead of throwing', async () => {
    vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText: vi.fn().mockRejectedValue(new Error('denied')) } });
    document.execCommand = vi.fn().mockReturnValue(false);
    await expect(copyText('hello')).resolves.toBe(false);
  });
});
