import { useQuery } from '@tanstack/react-query';
import { domainsApi, type DomainConfig } from '../api/domains';
import { useAuth } from '../contexts/AuthContext';

/**
 * Base URL of the enabled primary public domain, if one is configured.
 * Wildcard domains have no single host to link to, so they are skipped.
 */
export function publicBaseUrl(domains: DomainConfig[] | undefined): string | null {
  const primary = domains?.find(d =>
    d.enabled && d.primary && (d.scope === 'public' || d.scope === 'both') && d.kind !== 'wildcard');
  if (!primary) return null;
  return `${primary.tls_mode === 'none' ? 'http' : 'https'}://${primary.domain}`;
}

export function buildInviteUrl(code: string, baseUrl?: string | null): string {
  return `${baseUrl || window.location.origin}/login?register=1&invite=${encodeURIComponent(code)}`;
}

/** True when the link's host is only reachable from inside the local network. */
export function isPrivateHost(url: string): boolean {
  let host: string;
  try {
    host = new URL(url).hostname.toLowerCase();
  } catch {
    return false;
  }
  if (host === 'localhost' || host === '[::1]' || host.startsWith('[fc') || host.startsWith('[fd') || host.startsWith('[fe80')) {
    return true;
  }
  const ipv4 = host.match(/^(\d+)\.(\d+)\.\d+\.\d+$/);
  if (ipv4) {
    const [a, b] = [Number(ipv4[1]), Number(ipv4[2])];
    return a === 10 || a === 127 || (a === 172 && b >= 16 && b <= 31) || (a === 192 && b === 168) || (a === 169 && b === 254);
  }
  return !host.includes('.') || /\.(local|lan|home|internal|home\.arpa)$/.test(host);
}

/**
 * Copy text to the clipboard. The async Clipboard API only exists in secure
 * contexts, so plain-HTTP LAN access falls back to execCommand.
 */
export async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // fall through to the legacy path
  }
  const textarea = document.createElement('textarea');
  textarea.value = text;
  textarea.setAttribute('readonly', '');
  textarea.style.position = 'fixed';
  textarea.style.opacity = '0';
  document.body.appendChild(textarea);
  try {
    textarea.select();
    return document.execCommand('copy');
  } catch {
    return false;
  } finally {
    textarea.remove();
  }
}

/** Public base URL for invite links, or null to fall back to the current origin. */
export function useInviteBaseUrl(): string | null {
  const { hasPermission } = useAuth();
  const domains = useQuery({
    queryKey: ['domains'],
    queryFn: domainsApi.listDomains,
    enabled: hasPermission('settings.view'),
    retry: false,
  });
  return publicBaseUrl(domains.data);
}
