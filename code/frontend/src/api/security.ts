import apiClient from './client';
import { auditApi, AuditLog, AuditStats } from './audit';

// Security-specific action types
export const SECURITY_ACTIONS = [
  'Login',
  'LoginFailed',
  'Logout',
  'TokenRefresh',
  'TwoFactorEnabled',
  'TwoFactorDisabled',
  'TwoFactorVerified',
  'TwoFactorFailed',
  'PasswordChanged',
] as const;

export interface SecurityOverview {
  stats: AuditStats;
  recentLoginFailures: AuditLog[];
  recentSecurityEvents: AuditLog[];
  twoFactorStats: TwoFactorStats;
}

export interface TwoFactorStats {
  total_users: number;
  enabled_count: number;
  disabled_count: number;
}

export interface ActiveSession {
  user_id: number;
  username: string;
  ip_address: string;
  user_agent: string;
  last_activity: string;
}

export const securityApi = {
  /**
   * Get security overview with stats and recent events
   */
  getOverview: async (): Promise<SecurityOverview> => {
    // Fetch audit stats and recent security-related logs in parallel
    const [stats, loginFailures, securityEvents, twoFactorStats] = await Promise.all([
      auditApi.getStats(),
      auditApi.getLogs({
        action: 'LoginFailed',
        per_page: 10,
      }),
      auditApi.getLogs({
        per_page: 20,
      }),
      securityApi.getTwoFactorStats(),
    ]);

    // Filter to only security-relevant events
    const securityOnlyEvents = securityEvents.logs.filter(log =>
      SECURITY_ACTIONS.includes(log.action as typeof SECURITY_ACTIONS[number])
    );

    return {
      stats,
      recentLoginFailures: loginFailures.logs,
      recentSecurityEvents: securityOnlyEvents,
      twoFactorStats,
    };
  },

  /**
   * Get 2FA statistics across all users (admin only)
   */
  getTwoFactorStats: async (): Promise<TwoFactorStats> => {
    try {
      const response = await apiClient.get<TwoFactorStats>('/security/2fa/stats');
      return response.data;
    } catch {
      // If endpoint doesn't exist, return empty stats
      return {
        total_users: 0,
        enabled_count: 0,
        disabled_count: 0,
      };
    }
  },

  /**
   * Get recent login failures
   */
  getLoginFailures: async (limit: number = 20): Promise<AuditLog[]> => {
    const response = await auditApi.getLogs({
      action: 'LoginFailed',
      per_page: limit,
    });
    return response.logs;
  },

  /**
   * Get security events by type
   */
  getSecurityEventsByAction: async (action: string, limit: number = 20): Promise<AuditLog[]> => {
    const response = await auditApi.getLogs({
      action,
      per_page: limit,
    });
    return response.logs;
  },
};
