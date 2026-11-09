import { useState } from 'react';
import type { Invite } from '../../../api/users';
import type { Role } from '../../../api/roles';
import { SettingsEmptyState, SettingsTabLayout } from './SettingsTabLayout';

type InvitesTabProps = {
  invites: Invite[];
  /** Roles an invite can grant. Empty when the viewer cannot list roles. */
  roles: Role[];
  creatingInvite: boolean;
  copiedInviteId: number | null;
  onCreateInvite: (roleId: number | null) => void;
  onDeleteInvite: (invite: Invite) => void;
  onCopyInviteLink: (invite: Invite) => void;
  formatDate: (dateString: string) => string;
  isExpired: (expiresAt: string | null) => boolean;
};

export function InvitesTab({
  invites,
  roles,
  creatingInvite,
  copiedInviteId,
  onCreateInvite,
  onDeleteInvite,
  onCopyInviteLink,
  formatDate,
  isExpired,
}: InvitesTabProps) {
  const [roleId, setRoleId] = useState<number | null>(null);
  return (
    <SettingsTabLayout
      title="Invite Links"
      description="Create invite links to share with users. Each link can only be used once."
      actions={(
        <div className="flex flex-wrap items-center gap-2">
          {roles.length > 0 && (
            <label className="flex items-center gap-2 text-sm text-gray-600 dark:text-gray-300">
              <span>Role</span>
              <select
                aria-label="Role for invited user"
                value={roleId ?? ''}
                onChange={(e) => setRoleId(e.target.value ? Number(e.target.value) : null)}
                className="px-3 py-2 bg-white dark:bg-gray-800 border border-gray-300 dark:border-gray-600 rounded-md text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-blue-500"
              >
                <option value="">No role</option>
                {roles.map(role => <option key={role.id} value={role.id}>{role.name}</option>)}
              </select>
            </label>
          )}
          <button
            onClick={() => onCreateInvite(roleId)}
            disabled={creatingInvite}
            className="px-4 py-2 bg-green-600 hover:bg-green-700 disabled:bg-gray-600 disabled:cursor-not-allowed rounded-md font-medium text-white transition-colors"
          >
            {creatingInvite ? 'Creating...' : 'Create Invite'}
          </button>
        </div>
      )}
    >

      {invites.length === 0 ? (
        <SettingsEmptyState>No invites created yet. Create one to get started.</SettingsEmptyState>
      ) : (
        <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 overflow-x-auto">
          <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
            <thead className="bg-gray-50 dark:bg-gray-700">
              <tr>
                <th className="px-4 md:px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Status</th>
                <th className="px-4 md:px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Role</th>
                <th className="px-4 md:px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Created By</th>
                <th className="px-4 md:px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider hidden sm:table-cell">Created At</th>
                <th className="px-4 md:px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider hidden sm:table-cell">Expires At</th>
                <th className="px-4 md:px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider hidden md:table-cell">Used By</th>
                <th className="px-4 md:px-6 py-3 text-right text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Actions</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
              {invites.map((invite) => (
                <tr key={invite.id} className={invite.is_used || isExpired(invite.expires_at) ? 'opacity-50' : ''}>
                  <td className="px-4 md:px-6 py-4 whitespace-nowrap">
                    {invite.is_used ? (
                      <span className="px-2 py-1 text-xs bg-gray-200 dark:bg-gray-600 text-gray-600 dark:text-gray-300 rounded">Used</span>
                    ) : isExpired(invite.expires_at) ? (
                      <span className="px-2 py-1 text-xs bg-red-600 text-white rounded">Expired</span>
                    ) : (
                      <span className="px-2 py-1 text-xs bg-green-600 text-white rounded">Active</span>
                    )}
                  </td>
                  <td className="px-4 md:px-6 py-4 whitespace-nowrap text-sm text-gray-600 dark:text-gray-300">{invite.role_name ?? <span className="text-gray-400 dark:text-gray-500">None</span>}</td>
                  <td className="px-4 md:px-6 py-4 whitespace-nowrap text-sm text-gray-600 dark:text-gray-300">{invite.created_by_username}</td>
                  <td className="px-4 md:px-6 py-4 whitespace-nowrap text-sm text-gray-500 dark:text-gray-400 hidden sm:table-cell">{formatDate(invite.created_at)}</td>
                  <td className="px-4 md:px-6 py-4 whitespace-nowrap text-sm text-gray-500 dark:text-gray-400 hidden sm:table-cell">{invite.expires_at ? formatDate(invite.expires_at) : 'Never'}</td>
                  <td className="px-4 md:px-6 py-4 whitespace-nowrap text-sm text-gray-600 dark:text-gray-300 hidden md:table-cell">{invite.used_by_username || '-'}</td>
                  <td className="px-4 md:px-6 py-4 whitespace-nowrap text-right text-sm space-x-2">
                    {!invite.is_used && !isExpired(invite.expires_at) && (
                      <button
                        onClick={() => onCopyInviteLink(invite)}
                        className="px-3 py-1 bg-blue-600 hover:bg-blue-700 rounded text-white transition-colors"
                      >
                        {copiedInviteId === invite.id ? 'Copied!' : 'Copy Link'}
                      </button>
                    )}
                    <button
                      onClick={() => onDeleteInvite(invite)}
                      className="px-3 py-1 bg-red-600 hover:bg-red-700 rounded text-white transition-colors"
                    >
                      Delete
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </SettingsTabLayout>
  );
}
