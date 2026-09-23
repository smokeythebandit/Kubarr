import { useQuery } from '@tanstack/react-query';
import { Link } from 'react-router-dom';
import { Shield, AlertTriangle, CheckCircle, XCircle, Key, Clock, User, Globe, RefreshCw } from 'lucide-react';
import { securityApi } from '../api/security';
import { auditApi, AuditLog } from '../api/audit';

function formatTimeAgo(dateString: string): string {
  const date = new Date(dateString);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffMins = Math.floor(diffMs / 60000);
  const diffHours = Math.floor(diffMs / 3600000);
  const diffDays = Math.floor(diffMs / 86400000);

  if (diffMins < 1) return 'Just now';
  if (diffMins < 60) return `${diffMins}m ago`;
  if (diffHours < 24) return `${diffHours}h ago`;
  if (diffDays < 7) return `${diffDays}d ago`;
  return date.toLocaleDateString();
}

function getActionIcon(action: string) {
  switch (action) {
    case 'login':
      return <CheckCircle size={16} className="text-green-500" />;
    case 'login_failed':
      return <XCircle size={16} className="text-red-500" />;
    case 'logout':
      return <User size={16} className="text-gray-500" />;
    case '2fa_enabled':
      return <Shield size={16} className="text-green-500" />;
    case '2fa_disabled':
      return <Shield size={16} className="text-yellow-500" />;
    case '2fa_verified':
      return <Key size={16} className="text-green-500" />;
    case '2fa_failed':
      return <Key size={16} className="text-red-500" />;
    case 'password_changed':
      return <Key size={16} className="text-blue-500" />;
    case 'token_refresh':
      return <RefreshCw size={16} className="text-gray-400" />;
    default:
      return <Clock size={16} className="text-gray-400" />;
  }
}

function getActionLabel(action: string): string {
  const labels: Record<string, string> = {
    login: 'Successful login',
    login_failed: 'Failed login attempt',
    logout: 'User logged out',
    token_refresh: 'Session refreshed',
    '2fa_enabled': '2FA enabled',
    '2fa_disabled': '2FA disabled',
    '2fa_verified': '2FA verified',
    '2fa_failed': '2FA verification failed',
    password_changed: 'Password changed',
  };
  return labels[action] || action;
}

function StatCard({ title, value, icon, color }: { title: string; value: string | number; icon: React.ReactNode; color: string }) {
  return (
    <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
      <div className="flex items-center justify-between">
        <div>
          <p className="text-sm text-gray-500 dark:text-gray-400">{title}</p>
          <p className={`text-2xl font-semibold mt-1 ${color}`}>{value}</p>
        </div>
        <div className={`p-3 rounded-full bg-gray-100 dark:bg-gray-700 ${color}`}>
          {icon}
        </div>
      </div>
    </div>
  );
}

function SecurityEventRow({ log }: { log: AuditLog }) {
  return (
    <tr className="hover:bg-gray-50 dark:hover:bg-gray-700/50">
      <td className="px-4 py-3 whitespace-nowrap">
        <div className="flex items-center gap-2">
          {getActionIcon(log.action)}
          <span className="text-sm font-medium text-gray-900 dark:text-white">
            {getActionLabel(log.action)}
          </span>
        </div>
      </td>
      <td className="px-4 py-3 whitespace-nowrap">
        <span className="text-sm text-gray-600 dark:text-gray-300">
          {log.username || 'Unknown'}
        </span>
      </td>
      <td className="px-4 py-3 whitespace-nowrap">
        <div className="flex items-center gap-1 text-sm text-gray-500 dark:text-gray-400">
          <Globe size={14} />
          <span>{log.ip_address || 'N/A'}</span>
        </div>
      </td>
      <td className="px-4 py-3 whitespace-nowrap text-sm text-gray-500 dark:text-gray-400">
        {formatTimeAgo(log.timestamp)}
      </td>
      <td className="px-4 py-3 whitespace-nowrap">
        {log.success ? (
          <span className="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-400">
            Success
          </span>
        ) : (
          <span className="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-medium bg-red-100 text-red-800 dark:bg-red-900/30 dark:text-red-400">
            Failed
          </span>
        )}
      </td>
    </tr>
  );
}

export default function SecurityPage() {
  const { data: stats, isLoading: statsLoading, error: statsError, refetch: refetchStats } = useQuery({
    queryKey: ['audit-stats'],
    queryFn: () => auditApi.getStats(),
    refetchInterval: 30000,
  });

  const { data: securityLogs, isLoading: logsLoading, error: logsError, refetch: refetchLogs } = useQuery({
    queryKey: ['security-events'],
    queryFn: securityApi.getRecentSecurityEvents,
    refetchInterval: 30000,
  });

  const isLoading = statsLoading || logsLoading;

  if (statsError || logsError) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="text-center">
          <AlertTriangle size={48} className="mx-auto text-red-500 mb-4" />
            <h2 className="text-xl font-semibold text-gray-900 dark:text-white mb-2">
            Failed to load security data
          </h2>
          <p className="text-gray-500 dark:text-gray-400">
             Audit statistics or security events could not be loaded. Please try again.
           </p>
           <button onClick={() => { void refetchStats(); void refetchLogs(); }} className="mt-4 px-4 py-2 bg-blue-600 text-white rounded-md">Retry</button>
        </div>
      </div>
    );
  }

  const failedEvents = stats?.failed_events || 0;
  const totalEvents = stats?.total_events || 0;
  const eventsToday = stats?.events_today || 0;
  const recentFailures = stats?.recent_failures || [];

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <Shield size={28} className="text-blue-500" />
          <div>
            <h1 className="text-2xl font-bold text-gray-900 dark:text-white">Security</h1>
            <p className="text-sm text-gray-500 dark:text-gray-400">
              Monitor authentication events and security status
            </p>
          </div>
        </div>
        <div className="flex items-center gap-4">
          <Link to="/settings?section=audit" className="text-sm text-blue-600 dark:text-blue-400 hover:underline">View audit logs</Link>
          <button onClick={() => { void refetchStats(); void refetchLogs(); }} aria-label="Refresh security data" className="text-blue-600 dark:text-blue-400"><RefreshCw size={20} /></button>
        </div>
      </div>

      {/* Stats Grid */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
        <StatCard
          title="Events Today"
          value={isLoading ? '...' : eventsToday}
          icon={<Clock size={20} />}
          color="text-blue-600 dark:text-blue-400"
        />
        <StatCard
          title="Failed Events"
          value={isLoading ? '...' : failedEvents}
          icon={<AlertTriangle size={20} />}
          color={failedEvents > 0 ? 'text-red-600 dark:text-red-400' : 'text-gray-600 dark:text-gray-400'}
        />
        <StatCard
          title="Total Events"
          value={isLoading ? '...' : totalEvents}
          icon={<Shield size={20} />}
          color="text-gray-600 dark:text-gray-400"
        />
      </div>

      {/* Recent Failures Alert */}
      {recentFailures.length > 0 && (
        <div className="bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg p-4">
          <div className="flex items-start gap-3">
            <AlertTriangle size={20} className="text-red-600 dark:text-red-400 flex-shrink-0 mt-0.5" />
            <div>
               <h3 className="font-semibold text-red-800 dark:text-red-300">Recent Failed Events</h3>
              <p className="text-sm text-red-700 dark:text-red-400 mt-1">
                 {recentFailures.length} failed event{recentFailures.length !== 1 ? 's' : ''} detected recently.
              </p>
              <ul className="mt-2 space-y-1">
                {recentFailures.slice(0, 3).map((failure, idx) => (
                  <li key={idx} className="text-sm text-red-600 dark:text-red-400 flex items-center gap-2">
                    <XCircle size={14} />
                    <span>
                      {failure.username || 'Unknown user'} from {failure.ip_address || 'unknown IP'} - {formatTimeAgo(failure.timestamp)}
                    </span>
                  </li>
                ))}
              </ul>
            </div>
          </div>
        </div>
      )}

      {/* Security Events Table */}
      <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 overflow-hidden">
        <div className="px-4 py-3 border-b border-gray-200 dark:border-gray-700">
          <h2 className="text-lg font-semibold text-gray-900 dark:text-white">
            Security Events
          </h2>
          <p className="text-sm text-gray-500 dark:text-gray-400">
            Recent authentication and security-related activity
          </p>
        </div>

        {isLoading ? (
          <div className="flex items-center justify-center py-12">
            <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500"></div>
          </div>
        ) : securityLogs && securityLogs.length > 0 ? (
          <div className="overflow-x-auto">
            <table className="w-full">
              <thead className="bg-gray-50 dark:bg-gray-700/50">
                <tr>
                  <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                    Event
                  </th>
                  <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                    User
                  </th>
                  <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                    IP Address
                  </th>
                  <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                    Time
                  </th>
                  <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                    Status
                  </th>
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
                {securityLogs.map((log) => (
                  <SecurityEventRow key={log.id} log={log} />
                ))}
              </tbody>
            </table>
          </div>
        ) : (
          <div className="flex flex-col items-center justify-center py-12 text-gray-500 dark:text-gray-400">
            <Shield size={48} className="mb-4 opacity-50" />
            <p>No security events found</p>
          </div>
        )}
      </div>

      {/* Top Actions */}
      {stats?.top_actions && stats.top_actions.length > 0 && (
        <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
          <h2 className="text-lg font-semibold text-gray-900 dark:text-white mb-4">
            Event Distribution
          </h2>
          <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-5 gap-3">
            {stats.top_actions.map((action) => (
              <div
                key={action.action}
                className="bg-gray-50 dark:bg-gray-700/50 rounded-lg p-3 text-center"
              >
                <div className="flex justify-center mb-2">
                  {getActionIcon(action.action)}
                </div>
                <p className="text-xs text-gray-500 dark:text-gray-400 truncate">
                  {getActionLabel(action.action)}
                </p>
                <p className="text-lg font-semibold text-gray-900 dark:text-white mt-1">
                  {action.count}
                </p>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
