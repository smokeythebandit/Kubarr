import React from 'react';
import { User } from '../../api/users';

interface UserListProps {
  users: User[];
  onEdit?: (user: User) => void;
  onDelete?: (user: User) => void;
  onApprove?: (user: User) => void;
  onReject?: (user: User) => void;
  showActions?: boolean;
}

const UserList: React.FC<UserListProps> = ({
  users,
  onEdit,
  onDelete,
  onApprove,
  onReject,
  showActions = true,
}) => {
  const getStatusBadge = (user: User) => {
    if (!user.is_approved) {
      return <span className="px-2 py-1 text-xs font-semibold rounded-full bg-yellow-100 dark:bg-yellow-900 text-yellow-700 dark:text-yellow-300">Pending Approval</span>;
    }
    if (!user.is_active) {
      return <span className="px-2 py-1 text-xs font-semibold rounded-full bg-gray-200 dark:bg-gray-700 text-gray-600 dark:text-gray-300">Inactive</span>;
    }
    return <span className="px-2 py-1 text-xs font-semibold rounded-full bg-green-100 dark:bg-green-900 text-green-700 dark:text-green-300">Active</span>;
  };

  const getRoleBadges = (user: User) => {
    // Show roles if available
    if (user.roles && user.roles.length > 0) {
      return (
        <div className="flex flex-wrap gap-1">
          {user.roles.map((role) => (
            <span
              key={role.id}
              className={`px-2 py-1 text-xs font-semibold rounded-full ${
                role.name === 'admin'
                  ? 'bg-blue-100 dark:bg-blue-900 text-blue-700 dark:text-blue-300'
                  : role.name === 'viewer'
                  ? 'bg-purple-100 dark:bg-purple-900 text-purple-700 dark:text-purple-300'
                  : role.name === 'downloader'
                  ? 'bg-orange-100 dark:bg-orange-900 text-orange-700 dark:text-orange-300'
                  : 'bg-gray-200 dark:bg-gray-700 text-gray-600 dark:text-gray-300'
              }`}
            >
              {role.name}
            </span>
          ))}
        </div>
      );
    }
    // No roles assigned
    return (
      <span className="px-2 py-1 text-xs font-semibold rounded-full bg-gray-200 dark:bg-gray-700 text-gray-600 dark:text-gray-300">No roles</span>
    );
  };

  return (
    <div className="overflow-x-auto bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700">
      <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
        <thead className="bg-gray-50 dark:bg-gray-700">
          <tr>
            <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">ID</th>
            <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Username</th>
            <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Email</th>
            <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Roles</th>
            <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Status</th>
            <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Created</th>
            {showActions && <th className="px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Actions</th>}
          </tr>
        </thead>
        <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
          {users.length === 0 ? (
            <tr>
              <td colSpan={showActions ? 7 : 6} className="px-6 py-8 text-center text-gray-500 dark:text-gray-400">
                No users found
              </td>
            </tr>
          ) : (
            users.map((user) => (
              <tr key={user.id} className="hover:bg-gray-50 dark:hover:bg-gray-700/50 transition-colors">
                <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-600 dark:text-gray-300">{user.id}</td>
                <td className="px-6 py-4 whitespace-nowrap text-sm font-medium text-gray-900 dark:text-white">{user.username}</td>
                <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-600 dark:text-gray-300">{user.email}</td>
                <td className="px-6 py-4 whitespace-nowrap text-sm">{getRoleBadges(user)}</td>
                <td className="px-6 py-4 whitespace-nowrap text-sm">{getStatusBadge(user)}</td>
                <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-600 dark:text-gray-300">{new Date(user.created_at).toLocaleDateString()}</td>
                {showActions && (
                  <td className="px-6 py-4 whitespace-nowrap text-sm">
                    <div className="flex space-x-2">
                      {!user.is_approved && onApprove && (
                        <button
                          className="px-3 py-1 bg-green-600 hover:bg-green-700 text-white rounded text-xs font-medium transition-colors"
                          onClick={() => onApprove(user)}
                          title="Approve User"
                        >
                          Approve
                        </button>
                      )}
                      {!user.is_approved && onReject && (
                        <button
                          className="px-3 py-1 bg-red-600 hover:bg-red-700 text-white rounded text-xs font-medium transition-colors"
                          onClick={() => onReject(user)}
                          title="Reject User"
                        >
                          Reject
                        </button>
                      )}
                      {user.is_approved && onEdit && (
                        <button
                          className="px-3 py-1 bg-blue-600 hover:bg-blue-700 text-white rounded text-xs font-medium transition-colors"
                          onClick={() => onEdit(user)}
                          title="Edit User"
                        >
                          Edit
                        </button>
                      )}
                      {onDelete && (
                        <button
                          className="px-3 py-1 bg-red-600 hover:bg-red-700 text-white rounded text-xs font-medium transition-colors"
                          onClick={() => onDelete(user)}
                          title="Delete User"
                        >
                          Delete
                        </button>
                      )}
                    </div>
                  </td>
                )}
              </tr>
            ))
          )}
        </tbody>
      </table>
    </div>
  );
};

export default UserList;
