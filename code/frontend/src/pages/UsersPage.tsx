import React, { useState, useEffect } from 'react';
import { useAuth } from '../contexts/AuthContext';
import UserList from '../components/users/UserList';
import UserForm from '../components/users/UserForm';
import InviteLinkModal from '../components/users/InviteLinkModal';
import {
  User,
  Invite,
  getUsers,
  getPendingUsers,
  createUser,
  updateUser,
  deleteUser,
  approveUser,
  rejectUser,
  getInvites,
  createInvite,
  deleteInvite,
  CreateUserRequest,
  UpdateUserRequest,
} from '../api/users';
import {
  Role,
  getRoles,
  createRole,
  deleteRole,
  setRoleApps,
} from '../api/roles';
import { getCatalog, App } from '../api/apps';

type ViewMode = 'list' | 'create' | 'edit';
type ActiveTab = 'all' | 'pending' | 'invites' | 'roles';

// Inline component for editing role apps
interface RoleAppEditorProps {
  role: Role;
  apps: App[];
  onSave: (appNames: string[]) => void;
  onCancel: () => void;
}

const RoleAppEditor: React.FC<RoleAppEditorProps> = ({ role, apps, onSave, onCancel }) => {
  const [selectedApps, setSelectedApps] = useState<string[]>(role.app_names || []);

  const toggleApp = (appName: string) => {
    setSelectedApps(prev =>
      prev.includes(appName)
        ? prev.filter(a => a !== appName)
        : [...prev, appName]
    );
  };

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap gap-2">
        {apps.map((app) => (
          <button
            key={app.name}
            type="button"
            onClick={() => toggleApp(app.name)}
            className={`px-2 py-1 rounded text-xs transition-colors ${
              selectedApps.includes(app.name)
                ? 'bg-blue-600 text-white'
                : 'bg-gray-700 text-gray-300 hover:bg-gray-600'
            }`}
          >
            {app.name}
          </button>
        ))}
      </div>
      <div className="flex space-x-2">
        <button
          onClick={() => onSave(selectedApps)}
          className="px-2 py-1 bg-green-600 hover:bg-green-700 rounded text-xs text-white transition-colors"
        >
          Save
        </button>
        <button
          onClick={onCancel}
          className="px-2 py-1 bg-gray-600 hover:bg-gray-700 rounded text-xs text-white transition-colors"
        >
          Cancel
        </button>
      </div>
    </div>
  );
};

const UsersPage: React.FC = () => {
  const { isAdmin } = useAuth();
  const [users, setUsers] = useState<User[]>([]);
  const [pendingUsers, setPendingUsers] = useState<User[]>([]);
  const [invites, setInvites] = useState<Invite[]>([]);
  const [roles, setRoles] = useState<Role[]>([]);
  const [apps, setApps] = useState<App[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<ViewMode>('list');
  const [selectedUser, setSelectedUser] = useState<User | null>(null);
  const [activeTab, setActiveTab] = useState<ActiveTab>('all');
  const [creatingInvite, setCreatingInvite] = useState(false);
  const [copiedInviteId, setCopiedInviteId] = useState<number | null>(null);
  const [newInvite, setNewInvite] = useState<Invite | null>(null);
  // Role editing state
  const [editingRole, setEditingRole] = useState<Role | null>(null);
  const [newRoleName, setNewRoleName] = useState('');
  const [newRoleDescription, setNewRoleDescription] = useState('');
  const [newRoleApps, setNewRoleApps] = useState<string[]>([]);
  const [creatingRole, setCreatingRole] = useState(false);

  useEffect(() => {
    if (isAdmin) {
      loadUsers();
      loadPendingUsers();
      loadInvites();
      loadRoles();
      loadApps();
    }
  }, [isAdmin]);

  const loadUsers = async () => {
    try {
      setLoading(true);
      const data = await getUsers();
      setUsers(data);
      setError(null);
    } catch (err: any) {
      setError(err.response?.data?.detail || 'Failed to load users');
    } finally {
      setLoading(false);
    }
  };

  const loadPendingUsers = async () => {
    try {
      const data = await getPendingUsers();
      setPendingUsers(data);
    } catch (err: any) {
      console.error('Failed to load pending users:', err);
    }
  };

  const loadInvites = async () => {
    try {
      const data = await getInvites();
      setInvites(data);
    } catch (err: any) {
      console.error('Failed to load invites:', err);
    }
  };

  const loadRoles = async () => {
    try {
      const data = await getRoles();
      setRoles(data);
    } catch (err: any) {
      console.error('Failed to load roles:', err);
    }
  };

  const loadApps = async () => {
    try {
      const data = await getCatalog();
      setApps(data);
    } catch (err: any) {
      console.error('Failed to load apps:', err);
    }
  };

  const handleCreateUser = async (data: CreateUserRequest | UpdateUserRequest) => {
    await createUser(data as CreateUserRequest);
    await loadUsers();
    setViewMode('list');
  };

  const handleUpdateUser = async (data: CreateUserRequest | UpdateUserRequest) => {
    if (selectedUser) {
      await updateUser(selectedUser.id, data as UpdateUserRequest);
      await loadUsers();
      setViewMode('list');
      setSelectedUser(null);
    }
  };

  const handleDeleteUser = async (user: User) => {
    if (window.confirm(`Are you sure you want to delete user "${user.username}"?`)) {
      try {
        await deleteUser(user.id);
        await loadUsers();
        await loadPendingUsers();
      } catch (err: any) {
        setError(err.response?.data?.detail || 'Failed to delete user');
      }
    }
  };

  const handleApproveUser = async (user: User) => {
    try {
      await approveUser(user.id);
      await loadUsers();
      await loadPendingUsers();
    } catch (err: any) {
      setError(err.response?.data?.detail || 'Failed to approve user');
    }
  };

  const handleRejectUser = async (user: User) => {
    if (window.confirm(`Are you sure you want to reject user "${user.username}"?`)) {
      try {
        await rejectUser(user.id);
        await loadPendingUsers();
      } catch (err: any) {
        setError(err.response?.data?.detail || 'Failed to reject user');
      }
    }
  };

  const handleEditUser = (user: User) => {
    setSelectedUser(user);
    setViewMode('edit');
  };

  const handleCancel = () => {
    setViewMode('list');
    setSelectedUser(null);
  };

  const handleCreateInvite = async () => {
    try {
      setCreatingInvite(true);
      console.log('Creating invite...');
      const invite = await createInvite({ expires_in_days: 7 });
      console.log('Invite created:', invite);
      setNewInvite(invite);
      console.log('newInvite state set');
      await loadInvites();
    } catch (err: any) {
      console.error('Failed to create invite:', err);
      setError(err.response?.data?.detail || 'Failed to create invite');
    } finally {
      setCreatingInvite(false);
    }
  };

  const handleCloseInviteModal = () => {
    setNewInvite(null);
  };

  const handleDeleteInvite = async (invite: Invite) => {
    if (window.confirm('Are you sure you want to delete this invite?')) {
      try {
        await deleteInvite(invite.id);
        await loadInvites();
      } catch (err: any) {
        setError(err.response?.data?.detail || 'Failed to delete invite');
      }
    }
  };

  const copyInviteLink = (invite: Invite) => {
    const baseUrl = window.location.origin;
    const inviteUrl = `${baseUrl}/auth/register?invite=${invite.code}`;
    navigator.clipboard.writeText(inviteUrl);
    setCopiedInviteId(invite.id);
    setTimeout(() => setCopiedInviteId(null), 2000);
  };

  const formatDate = (dateString: string) => {
    return new Date(dateString).toLocaleDateString('en-US', {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    });
  };

  const isExpired = (expiresAt: string | null) => {
    if (!expiresAt) return false;
    return new Date(expiresAt) < new Date();
  };

  // Role management functions
  const handleCreateRole = async () => {
    if (!newRoleName.trim()) {
      setError('Role name is required');
      return;
    }
    try {
      setCreatingRole(true);
      await createRole({
        name: newRoleName,
        description: newRoleDescription || undefined,
        app_names: newRoleApps,
      });
      setNewRoleName('');
      setNewRoleDescription('');
      setNewRoleApps([]);
      await loadRoles();
    } catch (err: any) {
      setError(err.response?.data?.detail || 'Failed to create role');
    } finally {
      setCreatingRole(false);
    }
  };

  const handleDeleteRole = async (role: Role) => {
    if (role.is_system) {
      setError('Cannot delete system roles');
      return;
    }
    if (window.confirm(`Are you sure you want to delete role "${role.name}"?`)) {
      try {
        await deleteRole(role.id);
        await loadRoles();
      } catch (err: any) {
        setError(err.response?.data?.detail || 'Failed to delete role');
      }
    }
  };

  const handleEditRoleApps = (role: Role) => {
    setEditingRole(role);
  };

  const handleSaveRoleApps = async (roleId: number, appNames: string[]) => {
    try {
      await setRoleApps(roleId, { app_names: appNames });
      setEditingRole(null);
      await loadRoles();
    } catch (err: any) {
      setError(err.response?.data?.detail || 'Failed to update role apps');
    }
  };

  const toggleAppForNewRole = (appName: string) => {
    setNewRoleApps(prev =>
      prev.includes(appName)
        ? prev.filter(a => a !== appName)
        : [...prev, appName]
    );
  };

  if (!isAdmin) {
    return (
      <div className="max-w-7xl mx-auto px-4 py-8">
        <div className="bg-red-900 border border-red-700 text-white px-4 py-3 rounded">
          You do not have permission to access this page. Admin privileges required.
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex justify-between items-center">
        <h1 className="text-3xl font-bold">User Management</h1>
        {viewMode === 'list' && (
          <button
            className="px-4 py-2 bg-blue-600 hover:bg-blue-700 rounded-md font-medium transition-colors"
            onClick={() => setViewMode('create')}
          >
            Create New User
          </button>
        )}
      </div>

      {error && (
        <div className="bg-red-900 border border-red-700 text-white px-4 py-3 rounded flex justify-between items-center">
          <span>{error}</span>
          <button
            onClick={() => setError(null)}
            className="text-white hover:text-gray-300"
            aria-label="Close"
          >
            <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 20 20">
              <path fillRule="evenodd" d="M4.293 4.293a1 1 0 011.414 0L10 8.586l4.293-4.293a1 1 0 111.414 1.414L11.414 10l4.293 4.293a1 1 0 01-1.414 1.414L10 11.414l-4.293 4.293a1 1 0 01-1.414-1.414L8.586 10 4.293 5.707a1 1 0 010-1.414z" clipRule="evenodd" />
            </svg>
          </button>
        </div>
      )}

      {viewMode === 'list' && (
        <>
          <div className="border-b border-gray-700">
            <nav className="flex space-x-8">
              <button
                className={`py-4 px-1 border-b-2 font-medium text-sm transition-colors ${
                  activeTab === 'all'
                    ? 'border-blue-500 text-blue-500'
                    : 'border-transparent text-gray-400 hover:text-gray-300 hover:border-gray-300'
                }`}
                onClick={() => setActiveTab('all')}
              >
                All Users ({users.length})
              </button>
              <button
                className={`py-4 px-1 border-b-2 font-medium text-sm transition-colors ${
                  activeTab === 'pending'
                    ? 'border-blue-500 text-blue-500'
                    : 'border-transparent text-gray-400 hover:text-gray-300 hover:border-gray-300'
                }`}
                onClick={() => setActiveTab('pending')}
              >
                Pending Approval ({pendingUsers.length})
              </button>
              <button
                className={`py-4 px-1 border-b-2 font-medium text-sm transition-colors ${
                  activeTab === 'invites'
                    ? 'border-blue-500 text-blue-500'
                    : 'border-transparent text-gray-400 hover:text-gray-300 hover:border-gray-300'
                }`}
                onClick={() => setActiveTab('invites')}
              >
                Invite Links ({invites.filter(i => !i.is_used && !isExpired(i.expires_at)).length})
              </button>
              <button
                className={`py-4 px-1 border-b-2 font-medium text-sm transition-colors ${
                  activeTab === 'roles'
                    ? 'border-blue-500 text-blue-500'
                    : 'border-transparent text-gray-400 hover:text-gray-300 hover:border-gray-300'
                }`}
                onClick={() => setActiveTab('roles')}
              >
                Roles ({roles.length})
              </button>
            </nav>
          </div>

          {loading ? (
            <div className="flex justify-center items-center py-12">
              <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-500"></div>
            </div>
          ) : (
            <>
              {activeTab === 'all' && (
                <UserList
                  users={users}
                  onEdit={handleEditUser}
                  onDelete={handleDeleteUser}
                  onApprove={handleApproveUser}
                  showActions={true}
                />
              )}
              {activeTab === 'pending' && (
                <UserList
                  users={pendingUsers}
                  onApprove={handleApproveUser}
                  onReject={handleRejectUser}
                  onDelete={handleDeleteUser}
                  showActions={true}
                />
              )}
              {activeTab === 'invites' && (
                <div className="space-y-4">
                  <div className="flex justify-between items-center">
                    <p className="text-gray-400 text-sm">
                      Create invite links to share with users. Each link can only be used once.
                    </p>
                    <button
                      onClick={handleCreateInvite}
                      disabled={creatingInvite}
                      className="px-4 py-2 bg-green-600 hover:bg-green-700 disabled:bg-gray-600 disabled:cursor-not-allowed rounded-md font-medium transition-colors"
                    >
                      {creatingInvite ? 'Creating...' : 'Create Invite'}
                    </button>
                  </div>

                  {invites.length === 0 ? (
                    <div className="bg-gray-800 rounded-lg border border-gray-700 p-8 text-center">
                      <p className="text-gray-400">No invites created yet. Create one to get started.</p>
                    </div>
                  ) : (
                    <div className="bg-gray-800 rounded-lg border border-gray-700 overflow-hidden">
                      <table className="min-w-full divide-y divide-gray-700">
                        <thead className="bg-gray-750">
                          <tr>
                            <th className="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">
                              Status
                            </th>
                            <th className="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">
                              Created By
                            </th>
                            <th className="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">
                              Created At
                            </th>
                            <th className="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">
                              Expires At
                            </th>
                            <th className="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">
                              Used By
                            </th>
                            <th className="px-6 py-3 text-right text-xs font-medium text-gray-400 uppercase tracking-wider">
                              Actions
                            </th>
                          </tr>
                        </thead>
                        <tbody className="divide-y divide-gray-700">
                          {invites.map((invite) => (
                            <tr key={invite.id} className={invite.is_used || isExpired(invite.expires_at) ? 'opacity-50' : ''}>
                              <td className="px-6 py-4 whitespace-nowrap">
                                {invite.is_used ? (
                                  <span className="px-2 py-1 text-xs bg-gray-600 text-gray-300 rounded">Used</span>
                                ) : isExpired(invite.expires_at) ? (
                                  <span className="px-2 py-1 text-xs bg-red-600 text-white rounded">Expired</span>
                                ) : (
                                  <span className="px-2 py-1 text-xs bg-green-600 text-white rounded">Active</span>
                                )}
                              </td>
                              <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-300">
                                {invite.created_by_username}
                              </td>
                              <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-400">
                                {formatDate(invite.created_at)}
                              </td>
                              <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-400">
                                {invite.expires_at ? formatDate(invite.expires_at) : 'Never'}
                              </td>
                              <td className="px-6 py-4 whitespace-nowrap text-sm text-gray-300">
                                {invite.used_by_username || '-'}
                              </td>
                              <td className="px-6 py-4 whitespace-nowrap text-right text-sm space-x-2">
                                {!invite.is_used && !isExpired(invite.expires_at) && (
                                  <button
                                    onClick={() => copyInviteLink(invite)}
                                    className="px-3 py-1 bg-blue-600 hover:bg-blue-700 rounded text-white transition-colors"
                                  >
                                    {copiedInviteId === invite.id ? 'Copied!' : 'Copy Link'}
                                  </button>
                                )}
                                <button
                                  onClick={() => handleDeleteInvite(invite)}
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
                </div>
              )}
              {activeTab === 'roles' && (
                <div className="space-y-6">
                  {/* Create new role form */}
                  <div className="bg-gray-800 rounded-lg border border-gray-700 p-6">
                    <h3 className="text-lg font-medium mb-4">Create New Role</h3>
                    <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                      <div>
                        <label className="block text-sm font-medium text-gray-300 mb-2">
                          Role Name
                        </label>
                        <input
                          type="text"
                          value={newRoleName}
                          onChange={(e) => setNewRoleName(e.target.value)}
                          className="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-blue-500"
                          placeholder="e.g., media-viewer"
                        />
                      </div>
                      <div>
                        <label className="block text-sm font-medium text-gray-300 mb-2">
                          Description
                        </label>
                        <input
                          type="text"
                          value={newRoleDescription}
                          onChange={(e) => setNewRoleDescription(e.target.value)}
                          className="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded-md text-white placeholder-gray-400 focus:outline-none focus:ring-2 focus:ring-blue-500"
                          placeholder="Description of this role"
                        />
                      </div>
                    </div>
                    <div className="mt-4">
                      <label className="block text-sm font-medium text-gray-300 mb-2">
                        App Permissions
                      </label>
                      <div className="flex flex-wrap gap-2">
                        {apps.map((app) => (
                          <button
                            key={app.name}
                            type="button"
                            onClick={() => toggleAppForNewRole(app.name)}
                            className={`px-3 py-1 rounded-full text-sm transition-colors ${
                              newRoleApps.includes(app.name)
                                ? 'bg-blue-600 text-white'
                                : 'bg-gray-700 text-gray-300 hover:bg-gray-600'
                            }`}
                          >
                            {app.name}
                          </button>
                        ))}
                      </div>
                    </div>
                    <div className="mt-4">
                      <button
                        onClick={handleCreateRole}
                        disabled={creatingRole || !newRoleName.trim()}
                        className="px-4 py-2 bg-green-600 hover:bg-green-700 disabled:bg-gray-600 disabled:cursor-not-allowed rounded-md font-medium transition-colors"
                      >
                        {creatingRole ? 'Creating...' : 'Create Role'}
                      </button>
                    </div>
                  </div>

                  {/* Existing roles list */}
                  <div className="bg-gray-800 rounded-lg border border-gray-700 overflow-hidden">
                    <table className="min-w-full divide-y divide-gray-700">
                      <thead className="bg-gray-750">
                        <tr>
                          <th className="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">
                            Role
                          </th>
                          <th className="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">
                            Description
                          </th>
                          <th className="px-6 py-3 text-left text-xs font-medium text-gray-400 uppercase tracking-wider">
                            Apps
                          </th>
                          <th className="px-6 py-3 text-right text-xs font-medium text-gray-400 uppercase tracking-wider">
                            Actions
                          </th>
                        </tr>
                      </thead>
                      <tbody className="divide-y divide-gray-700">
                        {roles.map((role) => (
                          <tr key={role.id}>
                            <td className="px-6 py-4 whitespace-nowrap">
                              <div className="flex items-center">
                                <span className="font-medium text-white">{role.name}</span>
                                {role.is_system && (
                                  <span className="ml-2 px-2 py-1 text-xs bg-gray-600 text-gray-300 rounded">
                                    System
                                  </span>
                                )}
                              </div>
                            </td>
                            <td className="px-6 py-4 text-sm text-gray-400">
                              {role.description || '-'}
                            </td>
                            <td className="px-6 py-4">
                              {editingRole?.id === role.id ? (
                                <RoleAppEditor
                                  role={role}
                                  apps={apps}
                                  onSave={(appNames) => handleSaveRoleApps(role.id, appNames)}
                                  onCancel={() => setEditingRole(null)}
                                />
                              ) : (
                                <div className="flex flex-wrap gap-1">
                                  {role.app_names.length === 0 ? (
                                    <span className="text-gray-500 text-sm">
                                      {role.name === 'admin' ? 'All apps' : 'No apps'}
                                    </span>
                                  ) : (
                                    role.app_names.map((appName) => (
                                      <span
                                        key={appName}
                                        className="px-2 py-1 text-xs bg-gray-700 text-gray-300 rounded"
                                      >
                                        {appName}
                                      </span>
                                    ))
                                  )}
                                </div>
                              )}
                            </td>
                            <td className="px-6 py-4 whitespace-nowrap text-right text-sm space-x-2">
                              {editingRole?.id !== role.id && (
                                <button
                                  onClick={() => handleEditRoleApps(role)}
                                  className="px-3 py-1 bg-blue-600 hover:bg-blue-700 rounded text-white transition-colors"
                                >
                                  Edit Apps
                                </button>
                              )}
                              {!role.is_system && (
                                <button
                                  onClick={() => handleDeleteRole(role)}
                                  className="px-3 py-1 bg-red-600 hover:bg-red-700 rounded text-white transition-colors"
                                >
                                  Delete
                                </button>
                              )}
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                </div>
              )}
            </>
          )}
        </>
      )}

      {viewMode === 'create' && (
        <div className="bg-gray-800 rounded-lg border border-gray-700 p-6">
          <UserForm
            roles={roles}
            onSubmit={handleCreateUser}
            onCancel={handleCancel}
            isEdit={false}
          />
        </div>
      )}

      {viewMode === 'edit' && selectedUser && (
        <div className="bg-gray-800 rounded-lg border border-gray-700 p-6">
          <UserForm
            user={selectedUser}
            roles={roles}
            onSubmit={handleUpdateUser}
            onCancel={handleCancel}
            isEdit={true}
          />
        </div>
      )}

      {newInvite && (
        <InviteLinkModal invite={newInvite} onClose={handleCloseInviteModal} />
      )}
    </div>
  );
};

export default UsersPage;
