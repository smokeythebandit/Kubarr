import { auditApi, type AuditLog, type AuditStats } from './audit';

// Values are AuditAction's Display strings in the API, not Rust enum variant names.
export const SECURITY_ACTIONS = [
  'login', 'login_failed', 'logout', 'token_refresh', '2fa_enabled',
  '2fa_disabled', '2fa_verified', '2fa_failed', 'password_changed',
] as const;

export interface SecurityOverview {
  stats: AuditStats;
  recentLoginFailures: AuditLog[];
  recentSecurityEvents: AuditLog[];
}

export const securityApi = {
  // Fetch the latest 50 of EACH action so unrelated audit traffic cannot hide
  // security events. No unbounded scan or special server-side filter required.
  getRecentSecurityEvents: async (): Promise<AuditLog[]> => {
    const results = await Promise.all(SECURITY_ACTIONS.map(action =>
      auditApi.getLogs({ action, per_page: 50 })
    ));
    return results.flatMap(result => result.logs)
      .sort((a, b) => b.timestamp.localeCompare(a.timestamp) || b.id - a.id)
      .slice(0, 50);
  },

  getOverview: async (): Promise<SecurityOverview> => {
    const [stats, recentLoginFailures, recentSecurityEvents] = await Promise.all([
      auditApi.getStats(), securityApi.getLoginFailures(10), securityApi.getRecentSecurityEvents(),
    ]);
    return { stats, recentLoginFailures, recentSecurityEvents };
  },

  getLoginFailures: async (limit: number = 20): Promise<AuditLog[]> => {
    const response = await auditApi.getLogs({ action: 'login_failed', per_page: limit });
    return response.logs;
  },

  getSecurityEventsByAction: async (action: string, limit: number = 20): Promise<AuditLog[]> => {
    const response = await auditApi.getLogs({ action, per_page: limit });
    return response.logs;
  },
};
