import React, { useState, useEffect } from 'react';
import { useSearchParams } from 'react-router-dom';
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
  // createRole,  // Commented out - role management UI not yet implemented
  // deleteRole,
} from '../api/roles';
import { getCatalog, App } from '../api/apps';
import { getSettings, updateSetting, Setting } from '../api/settings';
import { Users, Link, UserPlus, Settings, Sliders, Lock, Menu, X, FileText, CheckCircle, XCircle, AlertTriangle, Trash2, LayoutDashboard, Activity, Shield, Clock, TrendingUp, Bell, Mail, Send, MessageSquare, Network, Cloud } from 'lucide-react';
import PermissionMatrix from '../components/permissions/PermissionMatrix';
import { auditApi, AuditLog, AuditStats, AuditLogQuery } from '../api/audit';
import { notificationsApi, NotificationChannel, NotificationEvent, NotificationLog } from '../api/notifications';
import { oauthApi, OAuthProvider } from '../api/oauth';
import { VpnTab } from '../components/vpn';
import { CloudflareTab } from '../components/cloudflare';

type ViewMode = 'list' | 'create' | 'edit';
type ApiErr = { response?: { data?: { detail?: string; error?: string } } };
type SettingsSection = 'dashboard' | 'general' | 'users' | 'pending' | 'invites' | 'permissions' | 'audit' | 'notifications' | 'vpn' | 'ddns' | 'letsencrypt' | 'cloudflare';

const SettingsPage: React.FC = () => {
  const { isAdmin, hasPermission } = useAuth();
  const [searchParams, setSearchParams] = useSearchParams();
  const [users, setUsers] = useState<User[]>([]);
  const [pendingUsers, setPendingUsers] = useState<User[]>([]);
  const [invites, setInvites] = useState<Invite[]>([]);
  const [roles, setRoles] = useState<Role[]>([]);
  const [_apps, setApps] = useState<App[]>([]);
  const [systemSettings, setSystemSettings] = useState<Record<string, Setting>>({});
  const [loading, setLoading] = useState(true);
  const [savingSettings, setSavingSettings] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Get state from URL params
  const activeSection = (searchParams.get('section') as SettingsSection) || 'dashboard';
  const viewMode = (searchParams.get('view') as ViewMode) || 'list';
  const selectedUserId = searchParams.get('user');

  // Find selected user from users list
  const selectedUser = selectedUserId ? users.find(u => u.id === parseInt(selectedUserId)) || null : null;

  // URL state setters
  const setActiveSection = (section: SettingsSection) => {
    // Don't include view when it's 'list' (default)
    setSearchParams({ section });
    setMobileSidebarOpen(false); // Close mobile sidebar when selecting
  };

  const setViewMode = (mode: ViewMode) => {
    if (mode === 'list') {
      // Remove view and user params when going back to list
      setSearchParams({ section: activeSection });
    } else {
      // Set view mode (create or edit)
      setSearchParams({ section: activeSection, view: mode });
    }
  };

  const setSelectedUser = (user: User | null) => {
    if (user) {
      setSearchParams({ section: activeSection, view: 'edit', user: user.id.toString() });
    } else {
      setSearchParams({ section: activeSection, view: 'list' });
    }
  };
  const [creatingInvite, setCreatingInvite] = useState(false);
  const [copiedInviteId, setCopiedInviteId] = useState<number | null>(null);
  const [newInvite, setNewInvite] = useState<Invite | null>(null);
  // Role editing state - commented out until UI is implemented
  // const [newRoleName, setNewRoleName] = useState('');
  // const [newRoleDescription, setNewRoleDescription] = useState('');
  // const [creatingRole, setCreatingRole] = useState(false);
  const [mobileSidebarOpen, setMobileSidebarOpen] = useState(false);

  // Audit log state
  const [auditLogs, setAuditLogs] = useState<AuditLog[]>([]);
  const [auditStats, setAuditStats] = useState<AuditStats | null>(null);
  const [auditLoading, setAuditLoading] = useState(false);
  const [auditPage, setAuditPage] = useState(1);
  const [auditTotalPages, setAuditTotalPages] = useState(1);
  const [auditTotal, setAuditTotal] = useState(0);
  const [auditFilter, setAuditFilter] = useState<AuditLogQuery>({ per_page: 20 });
  const [clearingLogs, setClearingLogs] = useState(false);

  // Notification state
  const [notificationChannels, setNotificationChannels] = useState<NotificationChannel[]>([]);
  const [notificationEvents, setNotificationEvents] = useState<NotificationEvent[]>([]);
  const [notificationLogs, setNotificationLogs] = useState<NotificationLog[]>([]);
  const [notificationLogsTotal, setNotificationLogsTotal] = useState(0);
  const [notificationLoading, setNotificationLoading] = useState(false);
  const [testingChannel, setTestingChannel] = useState<string | null>(null);
  const [testDestination, setTestDestination] = useState<Record<string, string>>({});
  const [editingChannel, setEditingChannel] = useState<string | null>(null);
  const [channelConfig, setChannelConfig] = useState<Record<string, Record<string, string>>>({});

  // OAuth providers state
  const [oauthProviders, setOauthProviders] = useState<OAuthProvider[]>([]);
  const [editingOAuthProvider, setEditingOAuthProvider] = useState<string | null>(null);
  const [oauthProviderConfig, setOauthProviderConfig] = useState<{ client_id: string; client_secret: string }>({ client_id: '', client_secret: '' });
  const [savingOAuthProvider, setSavingOAuthProvider] = useState(false);

  useEffect(() => {
    if (isAdmin) {
      loadUsers();
      loadPendingUsers();
      loadInvites();
      loadRoles();
      loadApps();
      loadSettings();
    }
  }, [isAdmin]);

  // Load audit data when section is active
  useEffect(() => {
    if (isAdmin && activeSection === 'audit') {
      loadAuditLogs();
      loadAuditStats();
    }
    if (isAdmin && activeSection === 'dashboard') {
      loadAuditStats();
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isAdmin, activeSection, auditPage, auditFilter]);

  // Load notification data when section is active
  useEffect(() => {
    if (isAdmin && activeSection === 'notifications') {
      loadNotificationChannels();
      loadNotificationEvents();
      loadNotificationLogs();
    }
  }, [isAdmin, activeSection]);

  // Load OAuth providers when general section is active
  useEffect(() => {
    if (isAdmin && activeSection === 'general') {
      loadOAuthProviders();
    }
  }, [isAdmin, activeSection]);

  const loadUsers = async () => {
    try {
      setLoading(true);
      const data = await getUsers();
      setUsers(data);
      setError(null);
    } catch (err: unknown) {
      setError((err as ApiErr).response?.data?.detail || 'Failed to load users');
    } finally {
      setLoading(false);
    }
  };

  const loadPendingUsers = async () => {
    try {
      const data = await getPendingUsers();
      setPendingUsers(data);
    } catch (err: unknown) {
      console.error('Failed to load pending users:', err);
    }
  };

  const loadInvites = async () => {
    try {
      const data = await getInvites();
      setInvites(data);
    } catch (err: unknown) {
      console.error('Failed to load invites:', err);
    }
  };

  const loadRoles = async () => {
    try {
      const data = await getRoles();
      setRoles(data);
    } catch (err: unknown) {
      console.error('Failed to load roles:', err);
    }
  };

  const loadApps = async () => {
    try {
      const data = await getCatalog();
      setApps(data);
    } catch (err: unknown) {
      console.error('Failed to load apps:', err);
    }
  };

  const loadSettings = async () => {
    try {
      const data = await getSettings();
      setSystemSettings(data);
    } catch (err: unknown) {
      console.error('Failed to load settings:', err);
    }
  };

  const loadOAuthProviders = async () => {
    try {
      const providers = await oauthApi.getProviders();
      setOauthProviders(providers);
    } catch (err: unknown) {
      console.error('Failed to load OAuth providers:', err);
    }
  };

  const handleToggleOAuthProvider = async (providerId: string, enabled: boolean) => {
    try {
      await oauthApi.updateProvider(providerId, { enabled });
      await loadOAuthProviders();
    } catch (err: unknown) {
      setError((err as ApiErr).response?.data?.error || 'Failed to update OAuth provider');
    }
  };

  const handleSaveOAuthProvider = async (providerId: string) => {
    try {
      setSavingOAuthProvider(true);
      await oauthApi.updateProvider(providerId, {
        client_id: oauthProviderConfig.client_id || undefined,
        client_secret: oauthProviderConfig.client_secret || undefined,
      });
      setEditingOAuthProvider(null);
      setOauthProviderConfig({ client_id: '', client_secret: '' });
      await loadOAuthProviders();
    } catch (err: unknown) {
      setError((err as ApiErr).response?.data?.error || 'Failed to save OAuth provider');
    } finally {
      setSavingOAuthProvider(false);
    }
  };

  const getOAuthProviderIcon = (providerId: string) => {
    if (providerId === 'google') {
      return (
        <svg className="w-5 h-5" viewBox="0 0 24 24">
          <path fill="#4285F4" d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.09z"/>
          <path fill="#34A853" d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z"/>
          <path fill="#FBBC05" d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z"/>
          <path fill="#EA4335" d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z"/>
        </svg>
      );
    }
    if (providerId === 'microsoft') {
      return (
        <svg className="w-5 h-5" viewBox="0 0 21 21">
          <rect x="1" y="1" width="9" height="9" fill="#f25022"/>
          <rect x="1" y="11" width="9" height="9" fill="#00a4ef"/>
          <rect x="11" y="1" width="9" height="9" fill="#7fba00"/>
          <rect x="11" y="11" width="9" height="9" fill="#ffb900"/>
        </svg>
      );
    }
    return <Shield className="w-5 h-5" />;
  };

  const loadAuditLogs = async () => {
    try {
      setAuditLoading(true);
      const response = await auditApi.getLogs({ ...auditFilter, page: auditPage });
      setAuditLogs(response.logs);
      setAuditTotalPages(response.total_pages);
      setAuditTotal(response.total);
    } catch (err: unknown) {
      console.error('Failed to load audit logs:', err);
    } finally {
      setAuditLoading(false);
    }
  };

  const loadAuditStats = async () => {
    try {
      const stats = await auditApi.getStats();
      setAuditStats(stats);
    } catch (err: unknown) {
      console.error('Failed to load audit stats:', err);
    }
  };

  const handleClearOldLogs = async () => {
    if (!window.confirm('Are you sure you want to delete audit logs older than 90 days? This action cannot be undone.')) {
      return;
    }
    try {
      setClearingLogs(true);
      const result = await auditApi.clearOldLogs(90);
      alert(result.message);
      loadAuditLogs();
      loadAuditStats();
    } catch (err: unknown) {
      setError((err as ApiErr).response?.data?.detail || 'Failed to clear audit logs');
    } finally {
      setClearingLogs(false);
    }
  };

  const handleAuditFilterChange = (key: keyof AuditLogQuery, value: string | boolean | undefined) => {
    setAuditPage(1);
    setAuditFilter(prev => ({ ...prev, [key]: value === '' ? undefined : value }));
  };

  // Notification functions
  const loadNotificationChannels = async () => {
    try {
      setNotificationLoading(true);
      const channels = await notificationsApi.getChannels();
      setNotificationChannels(channels);
    } catch (err: unknown) {
      console.error('Failed to load notification channels:', err);
    } finally {
      setNotificationLoading(false);
    }
  };

  const loadNotificationEvents = async () => {
    try {
      const events = await notificationsApi.getEvents();
      setNotificationEvents(events);
    } catch (err: unknown) {
      console.error('Failed to load notification events:', err);
    }
  };

  const loadNotificationLogs = async () => {
    try {
      const response = await notificationsApi.getLogs({ limit: 50 });
      setNotificationLogs(response.logs);
      setNotificationLogsTotal(response.total);
    } catch (err: unknown) {
      console.error('Failed to load notification logs:', err);
    }
  };

  const handleToggleChannel = async (channelType: string, enabled: boolean) => {
    try {
      await notificationsApi.updateChannel(channelType, { enabled });
      await loadNotificationChannels();
    } catch (err: unknown) {
      const error = err as { response?: { data?: { detail?: string } } };
      setError(error.response?.data?.detail || 'Failed to update channel');
    }
  };

  const handleToggleEvent = async (eventType: string, enabled: boolean) => {
    try {
      await notificationsApi.updateEvent(eventType, { enabled });
      await loadNotificationEvents();
    } catch (err: unknown) {
      const error = err as { response?: { data?: { detail?: string } } };
      setError(error.response?.data?.detail || 'Failed to update event');
    }
  };

  const handleUpdateEventSeverity = async (eventType: string, severity: string) => {
    try {
      await notificationsApi.updateEvent(eventType, { severity });
      await loadNotificationEvents();
    } catch (err: unknown) {
      const error = err as { response?: { data?: { detail?: string } } };
      setError(error.response?.data?.detail || 'Failed to update event severity');
    }
  };

  const handleTestChannel = async (channelType: string) => {
    const destination = testDestination[channelType];
    if (!destination) {
      setError('Please enter a test destination');
      return;
    }
    try {
      setTestingChannel(channelType);
      const result = await notificationsApi.testChannel(channelType, destination);
      if (result.success) {
        alert('Test notification sent successfully!');
      } else {
        setError(result.error || 'Test failed');
      }
    } catch (err: unknown) {
      const error = err as { response?: { data?: { detail?: string } } };
      setError(error.response?.data?.detail || 'Failed to test channel');
    } finally {
      setTestingChannel(null);
    }
  };

  const handleSaveChannelConfig = async (channelType: string) => {
    try {
      const config = channelConfig[channelType] || {};
      await notificationsApi.updateChannel(channelType, { config });
      setEditingChannel(null);
      await loadNotificationChannels();
    } catch (err: unknown) {
      const error = err as { response?: { data?: { detail?: string } } };
      setError(error.response?.data?.detail || 'Failed to save channel config');
    }
  };

  const getChannelIcon = (channelType: string) => {
    switch (channelType) {
      case 'email':
        return <Mail className="w-5 h-5" />;
      case 'telegram':
        return <Send className="w-5 h-5" />;
      case 'messagebird':
        return <MessageSquare className="w-5 h-5" />;
      default:
        return <Bell className="w-5 h-5" />;
    }
  };

  const getChannelConfigFields = (channelType: string): { key: string; label: string; type: string; placeholder: string }[] => {
    switch (channelType) {
      case 'email':
        return [
          { key: 'smtp_host', label: 'SMTP Host', type: 'text', placeholder: 'smtp.example.com' },
          { key: 'smtp_port', label: 'SMTP Port', type: 'number', placeholder: '587' },
          { key: 'username', label: 'Username', type: 'text', placeholder: 'user@example.com' },
          { key: 'password', label: 'Password', type: 'password', placeholder: '********' },
          { key: 'from_address', label: 'From Address', type: 'email', placeholder: 'noreply@example.com' },
          { key: 'from_name', label: 'From Name', type: 'text', placeholder: 'Kubarr' },
        ];
      case 'telegram':
        return [
          { key: 'bot_token', label: 'Bot Token', type: 'password', placeholder: '123456:ABC-DEF...' },
        ];
      case 'messagebird':
        return [
          { key: 'api_key', label: 'API Key', type: 'password', placeholder: 'live_...' },
          { key: 'originator', label: 'Originator', type: 'text', placeholder: 'Kubarr' },
        ];
      default:
        return [];
    }
  };

  const formatAuditAction = (action: string): string => {
    return action.replace(/_/g, ' ').replace(/\b\w/g, c => c.toUpperCase());
  };

  const getActionIcon = (action: string, success: boolean) => {
    if (!success) return <XCircle className="w-4 h-4 text-red-500" />;
    if (action.includes('login') || action.includes('2fa')) return <CheckCircle className="w-4 h-4 text-green-500" />;
    if (action.includes('failed')) return <XCircle className="w-4 h-4 text-red-500" />;
    return <CheckCircle className="w-4 h-4 text-blue-500" />;
  };

  const handleToggleSetting = async (key: string) => {
    try {
      setSavingSettings(true);
      const currentValue = systemSettings[key]?.value === 'true';
      const newValue = (!currentValue).toString();
      await updateSetting(key, newValue);
      await loadSettings();
    } catch (err: unknown) {
      setError((err as ApiErr).response?.data?.detail || `Failed to update setting ${key}`);
    } finally {
      setSavingSettings(false);
    }
  };

  const handleCreateUser = async (data: CreateUserRequest | UpdateUserRequest) => {
    await createUser(data as CreateUserRequest);
    await loadUsers();
    setSearchParams({ section: activeSection });
  };

  const handleUpdateUser = async (data: CreateUserRequest | UpdateUserRequest) => {
    if (selectedUser) {
      await updateUser(selectedUser.id, data as UpdateUserRequest);
      await loadUsers();
      setSearchParams({ section: activeSection });
    }
  };

  const handleDeleteUser = async (user: User) => {
    if (window.confirm(`Are you sure you want to delete user "${user.username}"?`)) {
      try {
        await deleteUser(user.id);
        await loadUsers();
        await loadPendingUsers();
      } catch (err: unknown) {
        setError((err as ApiErr).response?.data?.detail || 'Failed to delete user');
      }
    }
  };

  const handleApproveUser = async (user: User) => {
    try {
      await approveUser(user.id);
      await loadUsers();
      await loadPendingUsers();
    } catch (err: unknown) {
      setError((err as ApiErr).response?.data?.detail || 'Failed to approve user');
    }
  };

  const handleRejectUser = async (user: User) => {
    if (window.confirm(`Are you sure you want to reject user "${user.username}"?`)) {
      try {
        await rejectUser(user.id);
        await loadPendingUsers();
      } catch (err: unknown) {
        setError((err as ApiErr).response?.data?.detail || 'Failed to reject user');
      }
    }
  };

  const handleEditUser = (user: User) => {
    // setSelectedUser already sets view to 'edit'
    setSelectedUser(user);
  };

  const handleCancel = () => {
    // Go back to list view, clearing user selection
    setSearchParams({ section: activeSection });
  };

  const handleCreateInvite = async () => {
    try {
      setCreatingInvite(true);
      const invite = await createInvite({ expires_in_days: 7 });
      setNewInvite(invite);
      await loadInvites();
    } catch (err: unknown) {
      setError((err as ApiErr).response?.data?.detail || 'Failed to create invite');
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
      } catch (err: unknown) {
        setError((err as ApiErr).response?.data?.detail || 'Failed to delete invite');
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

  // Role management functions - commented out until UI is added
  // const handleCreateRole = async () => {
  //   if (!newRoleName.trim()) {
  //     setError('Role name is required');
  //     return;
  //   }
  //   try {
  //     setCreatingRole(true);
  //     await createRole({
  //       name: newRoleName,
  //       description: newRoleDescription || undefined,
  //     });
  //     setNewRoleName('');
  //     setNewRoleDescription('');
  //     await loadRoles();
  //   } catch (err: unknown) {
  //     setError((err as ApiErr).response?.data?.detail || 'Failed to create role');
  //   } finally {
  //     setCreatingRole(false);
  //   }
  // };

  // const handleDeleteRole = async (role: Role) => {
  //   if (role.is_system) {
  //     setError('Cannot delete system roles');
  //     return;
  //   }
  //   if (window.confirm(`Are you sure you want to delete role "${role.name}"?`)) {
  //     try {
  //       await deleteRole(role.id);
  //       await loadRoles();
  //     } catch (err: unknown) {
  //       setError((err as ApiErr).response?.data?.detail || 'Failed to delete role');
  //     }
  //   }
  // };

  if (!isAdmin) {
    return (
      <div className="max-w-7xl mx-auto px-4 py-8">
        <div className="bg-red-100 dark:bg-red-900 border border-red-300 dark:border-red-700 text-red-800 dark:text-white px-4 py-3 rounded">
          You do not have permission to access this page. Admin privileges required.
        </div>
      </div>
    );
  }

  const systemItems = [
    { id: 'dashboard' as SettingsSection, label: 'Dashboard', icon: LayoutDashboard },
    { id: 'notifications' as SettingsSection, label: 'Notifications', icon: Bell },
    { id: 'audit' as SettingsSection, label: 'Audit Logs', icon: FileText },
  ];

  const networkingItems = [
    { id: 'vpn' as SettingsSection, label: 'VPN', icon: Shield },
    { id: 'ddns' as SettingsSection, label: 'Dynamic DNS', icon: Network },
    { id: 'letsencrypt' as SettingsSection, label: "Let's Encrypt", icon: Lock },
    { id: 'cloudflare' as SettingsSection, label: 'Cloudflare Tunnel', icon: Cloud },
  ];

  const accessManagementItems = [
    { id: 'general' as SettingsSection, label: 'General', icon: Sliders },
    { id: 'users' as SettingsSection, label: 'All Users', icon: Users, count: users.length },
    { id: 'pending' as SettingsSection, label: 'Pending Approval', icon: UserPlus, count: pendingUsers.length },
    { id: 'invites' as SettingsSection, label: 'Invite Links', icon: Link, count: invites.filter(i => !i.is_used && !isExpired(i.expires_at)).length },
    { id: 'permissions' as SettingsSection, label: 'Permissions', icon: Lock },
  ];

  return (
    <div className="flex h-full w-full relative">
      {/* Mobile Sidebar Toggle Button */}
      <button
        onClick={() => setMobileSidebarOpen(!mobileSidebarOpen)}
        className="md:hidden fixed bottom-4 right-4 z-50 flex items-center justify-center w-14 h-14 bg-blue-600 hover:bg-blue-700 text-white rounded-full shadow-lg transition-colors"
        aria-label="Toggle settings menu"
      >
        {mobileSidebarOpen ? <X size={24} /> : <Menu size={24} />}
      </button>

      {/* Mobile Sidebar Overlay */}
      {mobileSidebarOpen && (
        <div
          className="md:hidden fixed inset-0 bg-black/50 z-40"
          onClick={() => setMobileSidebarOpen(false)}
        />
      )}

      {/* Left Sidebar */}
      <div className={`
        ${mobileSidebarOpen ? 'translate-x-0' : '-translate-x-full'}
        md:translate-x-0
        fixed md:relative
        inset-y-0 left-0
        z-40 md:z-auto
        w-64 bg-white dark:bg-gray-800 border-r border-gray-200 dark:border-gray-700 flex-shrink-0 flex flex-col
        transition-transform duration-200 ease-in-out
      `}>
        <div className="p-4 border-b border-gray-200 dark:border-gray-700 flex-shrink-0">
          <div className="flex items-center justify-between">
            <div className="flex items-center space-x-2">
              <Settings size={20} className="text-gray-500 dark:text-gray-400" />
              <h2 className="text-lg font-semibold text-gray-900 dark:text-white">Settings</h2>
            </div>
            <button
              onClick={() => setMobileSidebarOpen(false)}
              className="md:hidden p-1 rounded-md text-gray-500 hover:bg-gray-100 dark:hover:bg-gray-700"
            >
              <X size={20} />
            </button>
          </div>
        </div>
        <nav className="p-2 flex-1 overflow-auto">
          {/* System Section */}
          <div className="mb-4">
            <div className="px-3 py-2 text-xs font-semibold text-gray-400 dark:text-gray-500 uppercase tracking-wider">
              System
            </div>
            {systemItems.map((item) => {
              const Icon = item.icon;
              return (
                <button
                  key={item.id}
                  onClick={() => setActiveSection(item.id)}
                  className={`w-full flex items-center justify-between px-3 py-3 md:py-2 rounded-md mb-1 transition-colors ${
                    activeSection === item.id
                      ? 'bg-blue-600 text-white'
                      : 'text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 hover:text-gray-900 dark:hover:text-white'
                  }`}
                >
                  <div className="flex items-center space-x-3">
                    <Icon size={18} />
                    <span>{item.label}</span>
                  </div>
                </button>
              );
            })}
          </div>

          {/* Networking Section */}
          <div className="mb-4">
            <div className="px-3 py-2 text-xs font-semibold text-gray-400 dark:text-gray-500 uppercase tracking-wider">
              Networking
            </div>
            {networkingItems.map((item) => {
              const Icon = item.icon;
              return (
                <button
                  key={item.id}
                  onClick={() => setActiveSection(item.id)}
                  className={`w-full flex items-center justify-between px-3 py-3 md:py-2 rounded-md mb-1 transition-colors ${
                    activeSection === item.id
                      ? 'bg-blue-600 text-white'
                      : 'text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 hover:text-gray-900 dark:hover:text-white'
                  }`}
                >
                  <div className="flex items-center space-x-3">
                    <Icon size={18} />
                    <span>{item.label}</span>
                  </div>
                </button>
              );
            })}
          </div>

          {/* Access Management Section */}
          <div className="mb-2">
            <div className="px-3 py-2 text-xs font-semibold text-gray-400 dark:text-gray-500 uppercase tracking-wider">
              Access Management
            </div>
            {accessManagementItems.map((item) => {
              const Icon = item.icon;
              return (
                <button
                  key={item.id}
                  onClick={() => setActiveSection(item.id)}
                  className={`w-full flex items-center justify-between px-3 py-3 md:py-2 rounded-md mb-1 transition-colors ${
                    activeSection === item.id
                      ? 'bg-blue-600 text-white'
                      : 'text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 hover:text-gray-900 dark:hover:text-white'
                  }`}
                >
                  <div className="flex items-center space-x-3">
                    <Icon size={18} />
                    <span>{item.label}</span>
                  </div>
                  {item.count !== undefined && item.count > 0 && (
                    <span className={`px-2 py-0.5 text-xs rounded-full ${
                      activeSection === item.id
                        ? 'bg-blue-500 text-white'
                        : 'bg-gray-200 dark:bg-gray-700 text-gray-600 dark:text-gray-300'
                    }`}>
                      {item.count}
                    </span>
                  )}
                </button>
              );
            })}
          </div>
        </nav>
      </div>

      {/* Main Content */}
      <div className="flex-1 overflow-auto p-4 md:p-6 min-w-0">
        {/* Mobile Section Header */}
        <div className="md:hidden mb-4 pb-4 border-b border-gray-200 dark:border-gray-700">
          <button
            onClick={() => setMobileSidebarOpen(true)}
            className="flex items-center gap-2 text-gray-600 dark:text-gray-300 hover:text-gray-900 dark:hover:text-white"
          >
            <Menu size={20} />
            <span className="text-sm">Settings Menu</span>
          </button>
        </div>

        {error && (
          <div className="bg-red-100 dark:bg-red-900 border border-red-300 dark:border-red-700 text-red-800 dark:text-white px-4 py-3 rounded mb-4 flex justify-between items-center">
            <span>{error}</span>
            <button
              onClick={() => setError(null)}
              className="text-red-600 dark:text-white hover:text-red-800 dark:hover:text-gray-300"
            >
              <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 20 20">
                <path fillRule="evenodd" d="M4.293 4.293a1 1 0 011.414 0L10 8.586l4.293-4.293a1 1 0 111.414 1.414L11.414 10l4.293 4.293a1 1 0 01-1.414 1.414L10 11.414l-4.293 4.293a1 1 0 01-1.414-1.414L8.586 10 4.293 5.707a1 1 0 010-1.414z" clipRule="evenodd" />
              </svg>
            </button>
          </div>
        )}

        {loading ? (
          <div className="flex justify-center items-center py-12">
            <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-500"></div>
          </div>
        ) : (
          <>
            {/* Dashboard Section */}
            {activeSection === 'dashboard' && (
              <div className="space-y-6">
                <div>
                  <h3 className="text-2xl font-bold text-gray-900 dark:text-white">System Dashboard</h3>
                  <p className="text-gray-500 dark:text-gray-400 text-sm mt-1">
                    Overview of system activity and health.
                  </p>
                </div>

                {/* Quick Stats */}
                <div className="grid grid-cols-2 lg:grid-cols-4 gap-4">
                  <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
                    <div className="flex items-center gap-3">
                      <div className="p-2 bg-blue-100 dark:bg-blue-900/30 rounded-lg">
                        <Users className="w-5 h-5 text-blue-600 dark:text-blue-400" />
                      </div>
                      <div>
                        <div className="text-2xl font-bold text-gray-900 dark:text-white">{users.length}</div>
                        <div className="text-sm text-gray-500 dark:text-gray-400">Total Users</div>
                      </div>
                    </div>
                  </div>
                  <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
                    <div className="flex items-center gap-3">
                      <div className="p-2 bg-yellow-100 dark:bg-yellow-900/30 rounded-lg">
                        <Clock className="w-5 h-5 text-yellow-600 dark:text-yellow-400" />
                      </div>
                      <div>
                        <div className="text-2xl font-bold text-gray-900 dark:text-white">{pendingUsers.length}</div>
                        <div className="text-sm text-gray-500 dark:text-gray-400">Pending Approval</div>
                      </div>
                    </div>
                  </div>
                  <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
                    <div className="flex items-center gap-3">
                      <div className="p-2 bg-green-100 dark:bg-green-900/30 rounded-lg">
                        <Link className="w-5 h-5 text-green-600 dark:text-green-400" />
                      </div>
                      <div>
                        <div className="text-2xl font-bold text-gray-900 dark:text-white">
                          {invites.filter(i => !i.is_used && !isExpired(i.expires_at)).length}
                        </div>
                        <div className="text-sm text-gray-500 dark:text-gray-400">Active Invites</div>
                      </div>
                    </div>
                  </div>
                  <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
                    <div className="flex items-center gap-3">
                      <div className="p-2 bg-purple-100 dark:bg-purple-900/30 rounded-lg">
                        <Shield className="w-5 h-5 text-purple-600 dark:text-purple-400" />
                      </div>
                      <div>
                        <div className="text-2xl font-bold text-gray-900 dark:text-white">{roles.length}</div>
                        <div className="text-sm text-gray-500 dark:text-gray-400">Roles</div>
                      </div>
                    </div>
                  </div>
                </div>

                {/* Activity Overview */}
                {auditStats && (
                  <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
                    {/* Activity Stats */}
                    <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
                      <div className="flex items-center gap-2 mb-4">
                        <Activity className="w-5 h-5 text-gray-500" />
                        <h4 className="text-lg font-medium text-gray-900 dark:text-white">Activity Overview</h4>
                      </div>
                      <div className="space-y-4">
                        <div className="flex justify-between items-center">
                          <span className="text-gray-600 dark:text-gray-400">Events Today</span>
                          <span className="text-xl font-semibold text-blue-600 dark:text-blue-400">{auditStats.events_today.toLocaleString()}</span>
                        </div>
                        <div className="flex justify-between items-center">
                          <span className="text-gray-600 dark:text-gray-400">Events This Week</span>
                          <span className="text-xl font-semibold text-gray-900 dark:text-white">{auditStats.events_this_week.toLocaleString()}</span>
                        </div>
                        <div className="flex justify-between items-center">
                          <span className="text-gray-600 dark:text-gray-400">Total Events</span>
                          <span className="text-xl font-semibold text-gray-900 dark:text-white">{auditStats.total_events.toLocaleString()}</span>
                        </div>
                        <div className="pt-4 border-t border-gray-200 dark:border-gray-700">
                          <div className="flex justify-between items-center mb-2">
                            <span className="text-sm text-gray-500 dark:text-gray-400">Success Rate</span>
                            <span className="text-sm font-medium text-green-600 dark:text-green-400">
                              {auditStats.total_events > 0
                                ? ((auditStats.successful_events / auditStats.total_events) * 100).toFixed(1)
                                : 0}%
                            </span>
                          </div>
                          <div className="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-2">
                            <div
                              className="bg-green-600 h-2 rounded-full transition-all duration-300"
                              style={{
                                width: `${auditStats.total_events > 0
                                  ? (auditStats.successful_events / auditStats.total_events) * 100
                                  : 0}%`
                              }}
                            />
                          </div>
                        </div>
                      </div>
                    </div>

                    {/* Top Actions */}
                    <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
                      <div className="flex items-center gap-2 mb-4">
                        <TrendingUp className="w-5 h-5 text-gray-500" />
                        <h4 className="text-lg font-medium text-gray-900 dark:text-white">Top Actions</h4>
                      </div>
                      {auditStats.top_actions.length === 0 ? (
                        <p className="text-gray-500 dark:text-gray-400 text-sm">No activity recorded yet.</p>
                      ) : (
                        <div className="space-y-3">
                          {auditStats.top_actions.slice(0, 5).map((action, index) => (
                            <div key={action.action} className="flex items-center gap-3">
                              <span className="text-sm font-medium text-gray-400 w-4">{index + 1}</span>
                              <div className="flex-1">
                                <div className="flex justify-between items-center mb-1">
                                  <span className="text-sm text-gray-900 dark:text-white">{formatAuditAction(action.action)}</span>
                                  <span className="text-sm text-gray-500">{action.count.toLocaleString()}</span>
                                </div>
                                <div className="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-1.5">
                                  <div
                                    className="bg-blue-600 h-1.5 rounded-full transition-all duration-300"
                                    style={{
                                      width: `${(action.count / auditStats.top_actions[0].count) * 100}%`
                                    }}
                                  />
                                </div>
                              </div>
                            </div>
                          ))}
                        </div>
                      )}
                    </div>
                  </div>
                )}

                {/* Recent Failures */}
                {auditStats && auditStats.recent_failures.length > 0 && (
                  <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
                    <div className="flex items-center gap-2 mb-4">
                      <AlertTriangle className="w-5 h-5 text-red-500" />
                      <h4 className="text-lg font-medium text-gray-900 dark:text-white">Recent Failed Events</h4>
                    </div>
                    <div className="overflow-x-auto">
                      <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
                        <thead>
                          <tr>
                            <th className="px-4 py-2 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">Time</th>
                            <th className="px-4 py-2 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">User</th>
                            <th className="px-4 py-2 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase">Action</th>
                            <th className="px-4 py-2 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase hidden md:table-cell">Error</th>
                          </tr>
                        </thead>
                        <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
                          {auditStats.recent_failures.slice(0, 5).map((log) => (
                            <tr key={log.id}>
                              <td className="px-4 py-2 whitespace-nowrap text-sm text-gray-600 dark:text-gray-300">
                                {new Date(log.timestamp).toLocaleString()}
                              </td>
                              <td className="px-4 py-2 whitespace-nowrap text-sm text-gray-900 dark:text-white">
                                {log.username || 'Unknown'}
                              </td>
                              <td className="px-4 py-2 whitespace-nowrap">
                                <span className="inline-flex items-center gap-1 text-sm text-red-600 dark:text-red-400">
                                  <XCircle className="w-4 h-4" />
                                  {formatAuditAction(log.action)}
                                </span>
                              </td>
                              <td className="px-4 py-2 text-sm text-gray-500 dark:text-gray-400 hidden md:table-cell max-w-xs truncate" title={log.error_message || ''}>
                                {log.error_message || '-'}
                              </td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                    <div className="mt-4 pt-4 border-t border-gray-200 dark:border-gray-700">
                      <button
                        onClick={() => setActiveSection('audit')}
                        className="text-sm text-blue-600 dark:text-blue-400 hover:underline"
                      >
                        View all audit logs →
                      </button>
                    </div>
                  </div>
                )}

                {/* Quick Actions */}
                <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
                  <h4 className="text-lg font-medium text-gray-900 dark:text-white mb-4">Quick Actions</h4>
                  <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
                    <button
                      onClick={() => setActiveSection('users')}
                      className="flex flex-col items-center gap-2 p-4 rounded-lg border border-gray-200 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-700 transition-colors"
                    >
                      <Users className="w-6 h-6 text-blue-600 dark:text-blue-400" />
                      <span className="text-sm text-gray-700 dark:text-gray-300">Manage Users</span>
                    </button>
                    <button
                      onClick={() => setActiveSection('invites')}
                      className="flex flex-col items-center gap-2 p-4 rounded-lg border border-gray-200 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-700 transition-colors"
                    >
                      <Link className="w-6 h-6 text-green-600 dark:text-green-400" />
                      <span className="text-sm text-gray-700 dark:text-gray-300">Create Invite</span>
                    </button>
                    <button
                      onClick={() => setActiveSection('permissions')}
                      className="flex flex-col items-center gap-2 p-4 rounded-lg border border-gray-200 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-700 transition-colors"
                    >
                      <Lock className="w-6 h-6 text-purple-600 dark:text-purple-400" />
                      <span className="text-sm text-gray-700 dark:text-gray-300">Permissions</span>
                    </button>
                    <button
                      onClick={() => setActiveSection('audit')}
                      className="flex flex-col items-center gap-2 p-4 rounded-lg border border-gray-200 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-700 transition-colors"
                    >
                      <FileText className="w-6 h-6 text-orange-600 dark:text-orange-400" />
                      <span className="text-sm text-gray-700 dark:text-gray-300">Audit Logs</span>
                    </button>
                  </div>
                </div>
              </div>
            )}

            {/* General Section */}
            {activeSection === 'general' && (
              <div className="space-y-6">
                <div>
                  <h3 className="text-2xl font-bold text-gray-900 dark:text-white">General Settings</h3>
                  <p className="text-gray-500 dark:text-gray-400 text-sm mt-1">
                    Configure general access management settings.
                  </p>
                </div>

                <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6 space-y-6">
                  {/* Registration Settings */}
                  <div>
                    <h4 className="text-lg font-medium text-gray-900 dark:text-white mb-4">User Registration</h4>

                    {/* Registration Enabled Toggle */}
                    <div className="flex items-center justify-between py-4 border-b border-gray-200 dark:border-gray-700">
                      <div className="flex-1">
                        <div className="font-medium text-gray-900 dark:text-white">Allow Open Registration</div>
                        <div className="text-sm text-gray-500 dark:text-gray-400 mt-1">
                          When disabled, users can only register using invite links.
                        </div>
                      </div>
                      <button
                        onClick={() => handleToggleSetting('registration_enabled')}
                        disabled={savingSettings}
                        className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 focus:ring-offset-white dark:focus:ring-offset-gray-800 ${
                          systemSettings.registration_enabled?.value === 'true'
                            ? 'bg-blue-600'
                            : 'bg-gray-300 dark:bg-gray-600'
                        } ${savingSettings ? 'opacity-50 cursor-not-allowed' : ''}`}
                      >
                        <span
                          className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                            systemSettings.registration_enabled?.value === 'true'
                              ? 'translate-x-6'
                              : 'translate-x-1'
                          }`}
                        />
                      </button>
                    </div>

                    {/* Require Approval Toggle */}
                    <div className="flex items-center justify-between py-4">
                      <div className="flex-1">
                        <div className="font-medium text-gray-900 dark:text-white">Require Admin Approval</div>
                        <div className="text-sm text-gray-500 dark:text-gray-400 mt-1">
                          New registrations require admin approval before users can log in. Users with invite links are auto-approved.
                        </div>
                      </div>
                      <button
                        onClick={() => handleToggleSetting('registration_require_approval')}
                        disabled={savingSettings}
                        className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2 focus:ring-offset-white dark:focus:ring-offset-gray-800 ${
                          systemSettings.registration_require_approval?.value === 'true'
                            ? 'bg-blue-600'
                            : 'bg-gray-300 dark:bg-gray-600'
                        } ${savingSettings ? 'opacity-50 cursor-not-allowed' : ''}`}
                      >
                        <span
                          className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                            systemSettings.registration_require_approval?.value === 'true'
                              ? 'translate-x-6'
                              : 'translate-x-1'
                          }`}
                        />
                      </button>
                    </div>
                  </div>
                </div>

                {/* OAuth Providers */}
                <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6 space-y-6">
                  <div>
                    <h4 className="text-lg font-medium text-gray-900 dark:text-white mb-2">OAuth Providers</h4>
                    <p className="text-sm text-gray-500 dark:text-gray-400">
                      Allow users to sign in with their Google or Microsoft accounts. Configure your OAuth app credentials below.
                    </p>
                  </div>

                  <div className="space-y-4">
                    {oauthProviders.map((provider) => (
                      <div key={provider.id} className="border border-gray-200 dark:border-gray-700 rounded-lg p-4">
                        <div className="flex items-center justify-between mb-4">
                          <div className="flex items-center gap-3">
                            <div className={`p-2 rounded-lg ${provider.enabled ? 'bg-blue-100 dark:bg-blue-900/30' : 'bg-gray-100 dark:bg-gray-700'}`}>
                              {getOAuthProviderIcon(provider.id)}
                            </div>
                            <div>
                              <div className="font-medium text-gray-900 dark:text-white">{provider.name}</div>
                              <div className="text-sm text-gray-500 dark:text-gray-400">
                                {provider.enabled ? (
                                  provider.client_id ? 'Configured' : 'Enabled but not configured'
                                ) : 'Disabled'}
                              </div>
                            </div>
                          </div>
                          <button
                            onClick={() => handleToggleOAuthProvider(provider.id, !provider.enabled)}
                            disabled={!provider.client_id && !provider.enabled}
                            title={!provider.client_id && !provider.enabled ? 'Configure credentials first' : ''}
                            className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 ${
                              provider.enabled ? 'bg-blue-600' : 'bg-gray-300 dark:bg-gray-600'
                            } ${!provider.client_id && !provider.enabled ? 'opacity-50 cursor-not-allowed' : ''}`}
                          >
                            <span
                              className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                                provider.enabled ? 'translate-x-6' : 'translate-x-1'
                              }`}
                            />
                          </button>
                        </div>

                        {editingOAuthProvider === provider.id ? (
                          <div className="mt-4 space-y-3 border-t border-gray-200 dark:border-gray-700 pt-4">
                            <div>
                              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                                Client ID
                              </label>
                              <input
                                type="text"
                                value={oauthProviderConfig.client_id}
                                onChange={(e) => setOauthProviderConfig(prev => ({ ...prev, client_id: e.target.value }))}
                                placeholder={provider.client_id || 'Enter client ID'}
                                className="w-full px-3 py-2 bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md text-gray-900 dark:text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500"
                              />
                            </div>
                            <div>
                              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                                Client Secret
                              </label>
                              <input
                                type="password"
                                value={oauthProviderConfig.client_secret}
                                onChange={(e) => setOauthProviderConfig(prev => ({ ...prev, client_secret: e.target.value }))}
                                placeholder={provider.has_secret ? '••••••••' : 'Enter client secret'}
                                className="w-full px-3 py-2 bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md text-gray-900 dark:text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500"
                              />
                              <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
                                Leave blank to keep existing secret
                              </p>
                            </div>
                            <div className="flex gap-2 mt-4">
                              <button
                                onClick={() => handleSaveOAuthProvider(provider.id)}
                                disabled={savingOAuthProvider}
                                className="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-blue-400 text-white rounded-md font-medium transition-colors"
                              >
                                {savingOAuthProvider ? 'Saving...' : 'Save'}
                              </button>
                              <button
                                onClick={() => {
                                  setEditingOAuthProvider(null);
                                  setOauthProviderConfig({ client_id: '', client_secret: '' });
                                }}
                                className="px-4 py-2 bg-gray-200 dark:bg-gray-700 hover:bg-gray-300 dark:hover:bg-gray-600 text-gray-700 dark:text-gray-300 rounded-md font-medium transition-colors"
                              >
                                Cancel
                              </button>
                            </div>
                          </div>
                        ) : (
                          <div className="mt-4 border-t border-gray-200 dark:border-gray-700 pt-4">
                            <button
                              onClick={() => {
                                setEditingOAuthProvider(provider.id);
                                setOauthProviderConfig({
                                  client_id: provider.client_id || '',
                                  client_secret: '',
                                });
                              }}
                              className="px-3 py-1.5 text-sm bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 text-gray-700 dark:text-gray-300 rounded-md transition-colors"
                            >
                              Configure
                            </button>
                          </div>
                        )}
                      </div>
                    ))}

                    {oauthProviders.length === 0 && (
                      <p className="text-sm text-gray-500 dark:text-gray-400 italic">
                        Loading OAuth providers...
                      </p>
                    )}
                  </div>
                </div>
              </div>
            )}

            {/* Users Section */}
            {activeSection === 'users' && viewMode === 'list' && (
              <div className="space-y-4">
                <div className="flex justify-between items-center">
                  <h3 className="text-2xl font-bold text-gray-900 dark:text-white">All Users</h3>
                  <button
                    className="px-4 py-2 bg-blue-600 hover:bg-blue-700 rounded-md font-medium transition-colors"
                    onClick={() => setViewMode('create')}
                  >
                    Create New User
                  </button>
                </div>
                <UserList
                  users={users}
                  onEdit={handleEditUser}
                  onDelete={handleDeleteUser}
                  onApprove={handleApproveUser}
                  showActions={true}
                />
              </div>
            )}

            {/* Pending Users Section */}
            {activeSection === 'pending' && viewMode === 'list' && (
              <div className="space-y-4">
                <h3 className="text-2xl font-bold text-gray-900 dark:text-white">Pending Approval</h3>
                <p className="text-gray-500 dark:text-gray-400">Users waiting for approval to access the system.</p>
                <UserList
                  users={pendingUsers}
                  onApprove={handleApproveUser}
                  onReject={handleRejectUser}
                  onDelete={handleDeleteUser}
                  showActions={true}
                />
              </div>
            )}

            {/* Invites Section */}
            {activeSection === 'invites' && viewMode === 'list' && (
              <div className="space-y-4">
                <div className="flex justify-between items-center">
                  <div>
                    <h3 className="text-2xl font-bold text-gray-900 dark:text-white">Invite Links</h3>
                    <p className="text-gray-500 dark:text-gray-400 text-sm mt-1">
                      Create invite links to share with users. Each link can only be used once.
                    </p>
                  </div>
                  <button
                    onClick={handleCreateInvite}
                    disabled={creatingInvite}
                    className="px-4 py-2 bg-green-600 hover:bg-green-700 disabled:bg-gray-600 disabled:cursor-not-allowed rounded-md font-medium transition-colors"
                  >
                    {creatingInvite ? 'Creating...' : 'Create Invite'}
                  </button>
                </div>

                {invites.length === 0 ? (
                  <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-8 text-center">
                    <p className="text-gray-500 dark:text-gray-400">No invites created yet. Create one to get started.</p>
                  </div>
                ) : (
                  <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 overflow-x-auto">
                    <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
                      <thead className="bg-gray-50 dark:bg-gray-700">
                        <tr>
                          <th className="px-4 md:px-6 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Status</th>
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
                            <td className="px-4 md:px-6 py-4 whitespace-nowrap text-sm text-gray-600 dark:text-gray-300">{invite.created_by_username}</td>
                            <td className="px-4 md:px-6 py-4 whitespace-nowrap text-sm text-gray-500 dark:text-gray-400 hidden sm:table-cell">{formatDate(invite.created_at)}</td>
                            <td className="px-4 md:px-6 py-4 whitespace-nowrap text-sm text-gray-500 dark:text-gray-400 hidden sm:table-cell">{invite.expires_at ? formatDate(invite.expires_at) : 'Never'}</td>
                            <td className="px-4 md:px-6 py-4 whitespace-nowrap text-sm text-gray-600 dark:text-gray-300 hidden md:table-cell">{invite.used_by_username || '-'}</td>
                            <td className="px-4 md:px-6 py-4 whitespace-nowrap text-right text-sm space-x-2">
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

            {/* Permissions Section */}
            {activeSection === 'permissions' && (
              <div className="space-y-6">
                <div>
                  <h3 className="text-2xl font-bold text-gray-900 dark:text-white">Permissions</h3>
                  <p className="text-gray-500 dark:text-gray-400 text-sm mt-1">
                    Configure fine-grained access control for each role, including which applications users can access.
                  </p>
                </div>
                <PermissionMatrix />
              </div>
            )}

            {/* Audit Logs Section */}
            {activeSection === 'audit' && (
              <div className="space-y-6">
                <div className="flex justify-between items-start">
                  <div>
                    <h3 className="text-2xl font-bold text-gray-900 dark:text-white">Audit Logs</h3>
                    <p className="text-gray-500 dark:text-gray-400 text-sm mt-1">
                      Monitor system activity and security events.
                    </p>
                  </div>
                  <button
                    onClick={handleClearOldLogs}
                    disabled={clearingLogs}
                    className="flex items-center gap-2 px-4 py-2 bg-red-600 hover:bg-red-700 disabled:bg-gray-600 disabled:cursor-not-allowed rounded-md font-medium text-white transition-colors"
                  >
                    <Trash2 size={16} />
                    {clearingLogs ? 'Clearing...' : 'Clear Old Logs'}
                  </button>
                </div>

                {/* Stats Cards */}
                {auditStats && (
                  <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
                    <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
                      <div className="text-sm text-gray-500 dark:text-gray-400">Total Events</div>
                      <div className="text-2xl font-bold text-gray-900 dark:text-white">{auditStats.total_events.toLocaleString()}</div>
                    </div>
                    <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
                      <div className="text-sm text-gray-500 dark:text-gray-400">Today</div>
                      <div className="text-2xl font-bold text-blue-600 dark:text-blue-400">{auditStats.events_today.toLocaleString()}</div>
                    </div>
                    <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
                      <div className="text-sm text-gray-500 dark:text-gray-400">Successful</div>
                      <div className="text-2xl font-bold text-green-600 dark:text-green-400">{auditStats.successful_events.toLocaleString()}</div>
                    </div>
                    <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
                      <div className="text-sm text-gray-500 dark:text-gray-400">Failed</div>
                      <div className="text-2xl font-bold text-red-600 dark:text-red-400">{auditStats.failed_events.toLocaleString()}</div>
                    </div>
                  </div>
                )}

                {/* Recent Failures Alert */}
                {auditStats && auditStats.recent_failures.length > 0 && (
                  <div className="bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg p-4">
                    <div className="flex items-center gap-2 text-red-800 dark:text-red-200 font-medium mb-2">
                      <AlertTriangle size={16} />
                      Recent Failed Events
                    </div>
                    <div className="space-y-1 text-sm text-red-700 dark:text-red-300">
                      {auditStats.recent_failures.slice(0, 3).map((log) => (
                        <div key={log.id} className="flex items-center gap-2">
                          <span className="font-mono text-xs">{new Date(log.timestamp).toLocaleString()}</span>
                          <span>{formatAuditAction(log.action)}</span>
                          {log.username && <span className="text-red-600 dark:text-red-400">by {log.username}</span>}
                        </div>
                      ))}
                    </div>
                  </div>
                )}

                {/* Filters */}
                <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-4">
                  <div className="grid grid-cols-1 md:grid-cols-4 gap-4">
                    <div>
                      <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Search</label>
                      <input
                        type="text"
                        value={auditFilter.search || ''}
                        onChange={(e) => handleAuditFilterChange('search', e.target.value)}
                        placeholder="Search logs..."
                        className="w-full px-3 py-2 bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md text-gray-900 dark:text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500"
                      />
                    </div>
                    <div>
                      <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Action</label>
                      <select
                        value={auditFilter.action || ''}
                        onChange={(e) => handleAuditFilterChange('action', e.target.value)}
                        className="w-full px-3 py-2 bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-blue-500"
                      >
                        <option value="">All Actions</option>
                        <option value="login">Login</option>
                        <option value="login_failed">Login Failed</option>
                        <option value="logout">Logout</option>
                        <option value="2fa_verified">2FA Verified</option>
                        <option value="2fa_failed">2FA Failed</option>
                        <option value="user_created">User Created</option>
                        <option value="user_updated">User Updated</option>
                        <option value="user_deleted">User Deleted</option>
                        <option value="role_assigned">Role Assigned</option>
                        <option value="app_installed">App Installed</option>
                        <option value="app_started">App Started</option>
                        <option value="app_stopped">App Stopped</option>
                      </select>
                    </div>
                    <div>
                      <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Resource Type</label>
                      <select
                        value={auditFilter.resource_type || ''}
                        onChange={(e) => handleAuditFilterChange('resource_type', e.target.value)}
                        className="w-full px-3 py-2 bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-blue-500"
                      >
                        <option value="">All Types</option>
                        <option value="user">User</option>
                        <option value="role">Role</option>
                        <option value="app">App</option>
                        <option value="session">Session</option>
                        <option value="system">System</option>
                        <option value="invite">Invite</option>
                      </select>
                    </div>
                    <div>
                      <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">Status</label>
                      <select
                        value={auditFilter.success === undefined ? '' : auditFilter.success.toString()}
                        onChange={(e) => handleAuditFilterChange('success', e.target.value === '' ? undefined : e.target.value === 'true')}
                        className="w-full px-3 py-2 bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-blue-500"
                      >
                        <option value="">All</option>
                        <option value="true">Successful</option>
                        <option value="false">Failed</option>
                      </select>
                    </div>
                  </div>
                </div>

                {/* Logs Table */}
                <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 overflow-hidden">
                  {auditLoading ? (
                    <div className="flex justify-center items-center py-12">
                      <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500"></div>
                    </div>
                  ) : auditLogs.length === 0 ? (
                    <div className="text-center py-12 text-gray-500 dark:text-gray-400">
                      No audit logs found.
                    </div>
                  ) : (
                    <>
                      <div className="overflow-x-auto">
                        <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
                          <thead className="bg-gray-50 dark:bg-gray-700">
                            <tr>
                              <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Time</th>
                              <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">User</th>
                              <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Action</th>
                              <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider hidden md:table-cell">Resource</th>
                              <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Status</th>
                            </tr>
                          </thead>
                          <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
                            {auditLogs.map((log) => (
                              <tr key={log.id} className={!log.success ? 'bg-red-50 dark:bg-red-900/10' : ''}>
                                <td className="px-4 py-3 whitespace-nowrap text-sm text-gray-600 dark:text-gray-300">
                                  <div>{new Date(log.timestamp).toLocaleDateString()}</div>
                                  <div className="text-xs text-gray-500">{new Date(log.timestamp).toLocaleTimeString()}</div>
                                </td>
                                <td className="px-4 py-3 whitespace-nowrap text-sm text-gray-900 dark:text-white">
                                  {log.username || <span className="text-gray-500">System</span>}
                                </td>
                                <td className="px-4 py-3 whitespace-nowrap">
                                  <div className="flex items-center gap-2">
                                    {getActionIcon(log.action, log.success)}
                                    <span className="text-sm text-gray-900 dark:text-white">{formatAuditAction(log.action)}</span>
                                  </div>
                                </td>
                                <td className="px-4 py-3 whitespace-nowrap text-sm text-gray-600 dark:text-gray-300 hidden md:table-cell">
                                  <span className="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-gray-100 dark:bg-gray-700 text-gray-800 dark:text-gray-200">
                                    {log.resource_type}
                                  </span>
                                  {log.resource_id && <span className="ml-2 text-gray-500">#{log.resource_id}</span>}
                                </td>
                                <td className="px-4 py-3 whitespace-nowrap">
                                  {log.success ? (
                                    <span className="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-green-100 dark:bg-green-900 text-green-800 dark:text-green-200">
                                      Success
                                    </span>
                                  ) : (
                                    <span className="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-red-100 dark:bg-red-900 text-red-800 dark:text-red-200" title={log.error_message || ''}>
                                      Failed
                                    </span>
                                  )}
                                </td>
                              </tr>
                            ))}
                          </tbody>
                        </table>
                      </div>

                      {/* Pagination */}
                      <div className="flex items-center justify-between px-4 py-3 border-t border-gray-200 dark:border-gray-700">
                        <div className="text-sm text-gray-500 dark:text-gray-400">
                          Showing {((auditPage - 1) * 20) + 1} to {Math.min(auditPage * 20, auditTotal)} of {auditTotal.toLocaleString()} entries
                        </div>
                        <div className="flex items-center gap-2">
                          <button
                            onClick={() => setAuditPage(p => Math.max(1, p - 1))}
                            disabled={auditPage === 1}
                            className="px-3 py-1 bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 disabled:opacity-50 disabled:cursor-not-allowed rounded text-sm text-gray-700 dark:text-gray-300"
                          >
                            Previous
                          </button>
                          <span className="text-sm text-gray-700 dark:text-gray-300">
                            Page {auditPage} of {auditTotalPages}
                          </span>
                          <button
                            onClick={() => setAuditPage(p => Math.min(auditTotalPages, p + 1))}
                            disabled={auditPage === auditTotalPages}
                            className="px-3 py-1 bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 disabled:opacity-50 disabled:cursor-not-allowed rounded text-sm text-gray-700 dark:text-gray-300"
                          >
                            Next
                          </button>
                        </div>
                      </div>
                    </>
                  )}
                </div>
              </div>
            )}

            {/* Notifications Section */}
            {activeSection === 'notifications' && (
              <div className="space-y-6">
                <div>
                  <h3 className="text-2xl font-bold text-gray-900 dark:text-white">Notifications</h3>
                  <p className="text-gray-500 dark:text-gray-400 text-sm mt-1">
                    Configure notification channels and event triggers.
                  </p>
                </div>

                {notificationLoading ? (
                  <div className="flex justify-center items-center py-12">
                    <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500"></div>
                  </div>
                ) : (
                  <>
                    {/* Channels */}
                    <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
                      <h4 className="text-lg font-medium text-gray-900 dark:text-white mb-4">Notification Channels</h4>
                      <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
                        Configure and enable notification channels. Users can set their own preferences for each enabled channel.
                      </p>
                      <div className="space-y-4">
                        {notificationChannels.map((channel) => (
                          <div key={channel.channel_type} className="border border-gray-200 dark:border-gray-700 rounded-lg p-4">
                            <div className="flex items-center justify-between mb-4">
                              <div className="flex items-center gap-3">
                                <div className={`p-2 rounded-lg ${channel.enabled ? 'bg-blue-100 dark:bg-blue-900/30 text-blue-600 dark:text-blue-400' : 'bg-gray-100 dark:bg-gray-700 text-gray-500'}`}>
                                  {getChannelIcon(channel.channel_type)}
                                </div>
                                <div>
                                  <div className="font-medium text-gray-900 dark:text-white capitalize">{channel.channel_type}</div>
                                  <div className="text-sm text-gray-500 dark:text-gray-400">
                                    {channel.enabled ? 'Enabled' : 'Disabled'}
                                  </div>
                                </div>
                              </div>
                              <button
                                onClick={() => handleToggleChannel(channel.channel_type, !channel.enabled)}
                                className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 ${
                                  channel.enabled ? 'bg-blue-600' : 'bg-gray-300 dark:bg-gray-600'
                                }`}
                              >
                                <span
                                  className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                                    channel.enabled ? 'translate-x-6' : 'translate-x-1'
                                  }`}
                                />
                              </button>
                            </div>

                            {/* Configuration */}
                            {editingChannel === channel.channel_type ? (
                              <div className="mt-4 space-y-3 border-t border-gray-200 dark:border-gray-700 pt-4">
                                {getChannelConfigFields(channel.channel_type).map((field) => (
                                  <div key={field.key}>
                                    <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                                      {field.label}
                                    </label>
                                    <input
                                      type={field.type}
                                      placeholder={field.placeholder}
                                      value={channelConfig[channel.channel_type]?.[field.key] || ''}
                                      onChange={(e) =>
                                        setChannelConfig((prev) => ({
                                          ...prev,
                                          [channel.channel_type]: {
                                            ...prev[channel.channel_type],
                                            [field.key]: e.target.value,
                                          },
                                        }))
                                      }
                                      className="w-full px-3 py-2 bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md text-gray-900 dark:text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500"
                                    />
                                  </div>
                                ))}
                                <div className="flex gap-2 mt-4">
                                  <button
                                    onClick={() => handleSaveChannelConfig(channel.channel_type)}
                                    className="px-4 py-2 bg-blue-600 hover:bg-blue-700 text-white rounded-md font-medium transition-colors"
                                  >
                                    Save
                                  </button>
                                  <button
                                    onClick={() => setEditingChannel(null)}
                                    className="px-4 py-2 bg-gray-200 dark:bg-gray-700 hover:bg-gray-300 dark:hover:bg-gray-600 text-gray-700 dark:text-gray-300 rounded-md font-medium transition-colors"
                                  >
                                    Cancel
                                  </button>
                                </div>
                              </div>
                            ) : (
                              <div className="mt-4 flex items-center gap-2 border-t border-gray-200 dark:border-gray-700 pt-4">
                                <button
                                  onClick={() => {
                                    setEditingChannel(channel.channel_type);
                                    setChannelConfig((prev) => ({
                                      ...prev,
                                      [channel.channel_type]: channel.config as Record<string, string> || {},
                                    }));
                                  }}
                                  className="px-3 py-1.5 text-sm bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 text-gray-700 dark:text-gray-300 rounded-md transition-colors"
                                >
                                  Configure
                                </button>
                                {channel.enabled && (
                                  <div className="flex items-center gap-2">
                                    <input
                                      type="text"
                                      placeholder={channel.channel_type === 'email' ? 'test@example.com' : channel.channel_type === 'telegram' ? 'Chat ID' : 'Phone number'}
                                      value={testDestination[channel.channel_type] || ''}
                                      onChange={(e) =>
                                        setTestDestination((prev) => ({
                                          ...prev,
                                          [channel.channel_type]: e.target.value,
                                        }))
                                      }
                                      className="px-3 py-1.5 text-sm bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md text-gray-900 dark:text-white placeholder-gray-500 focus:outline-none focus:ring-2 focus:ring-blue-500"
                                    />
                                    <button
                                      onClick={() => handleTestChannel(channel.channel_type)}
                                      disabled={testingChannel === channel.channel_type}
                                      className="px-3 py-1.5 text-sm bg-green-600 hover:bg-green-700 disabled:bg-gray-600 text-white rounded-md transition-colors"
                                    >
                                      {testingChannel === channel.channel_type ? 'Testing...' : 'Test'}
                                    </button>
                                  </div>
                                )}
                              </div>
                            )}
                          </div>
                        ))}
                      </div>
                    </div>

                    {/* Event Settings */}
                    <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
                      <h4 className="text-lg font-medium text-gray-900 dark:text-white mb-4">Event Triggers</h4>
                      <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
                        Choose which events trigger notifications.
                      </p>
                      <div className="overflow-x-auto">
                        <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
                          <thead className="bg-gray-50 dark:bg-gray-700">
                            <tr>
                              <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Event</th>
                              <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Severity</th>
                              <th className="px-4 py-3 text-right text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Enabled</th>
                            </tr>
                          </thead>
                          <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
                            {notificationEvents.map((event) => (
                              <tr key={event.event_type}>
                                <td className="px-4 py-3 whitespace-nowrap text-sm text-gray-900 dark:text-white">
                                  {formatAuditAction(event.event_type)}
                                </td>
                                <td className="px-4 py-3 whitespace-nowrap">
                                  <select
                                    value={event.severity}
                                    onChange={(e) => handleUpdateEventSeverity(event.event_type, e.target.value)}
                                    className="px-2 py-1 text-sm bg-gray-50 dark:bg-gray-900 border border-gray-200 dark:border-gray-700 rounded-md text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-blue-500"
                                  >
                                    <option value="info">Info</option>
                                    <option value="warning">Warning</option>
                                    <option value="critical">Critical</option>
                                  </select>
                                </td>
                                <td className="px-4 py-3 whitespace-nowrap text-right">
                                  <button
                                    onClick={() => handleToggleEvent(event.event_type, !event.enabled)}
                                    className={`relative inline-flex h-5 w-9 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-blue-500 ${
                                      event.enabled ? 'bg-blue-600' : 'bg-gray-300 dark:bg-gray-600'
                                    }`}
                                  >
                                    <span
                                      className={`inline-block h-3 w-3 transform rounded-full bg-white transition-transform ${
                                        event.enabled ? 'translate-x-5' : 'translate-x-1'
                                      }`}
                                    />
                                  </button>
                                </td>
                              </tr>
                            ))}
                          </tbody>
                        </table>
                      </div>
                    </div>

                    {/* Delivery Logs */}
                    <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
                      <div className="flex items-center justify-between mb-4">
                        <h4 className="text-lg font-medium text-gray-900 dark:text-white">Delivery Logs</h4>
                        <span className="text-sm text-gray-500 dark:text-gray-400">{notificationLogsTotal} total</span>
                      </div>
                      {notificationLogs.length === 0 ? (
                        <div className="text-center py-8 text-gray-500 dark:text-gray-400">
                          No notification logs yet.
                        </div>
                      ) : (
                        <div className="overflow-x-auto">
                          <table className="min-w-full divide-y divide-gray-200 dark:divide-gray-700">
                            <thead className="bg-gray-50 dark:bg-gray-700">
                              <tr>
                                <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Time</th>
                                <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Channel</th>
                                <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Event</th>
                                <th className="px-4 py-3 text-left text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">Status</th>
                              </tr>
                            </thead>
                            <tbody className="divide-y divide-gray-200 dark:divide-gray-700">
                              {notificationLogs.slice(0, 20).map((log) => (
                                <tr key={log.id} className={log.status === 'failed' ? 'bg-red-50 dark:bg-red-900/10' : ''}>
                                  <td className="px-4 py-3 whitespace-nowrap text-sm text-gray-600 dark:text-gray-300">
                                    {new Date(log.created_at).toLocaleString()}
                                  </td>
                                  <td className="px-4 py-3 whitespace-nowrap">
                                    <span className="inline-flex items-center gap-1.5 text-sm text-gray-900 dark:text-white capitalize">
                                      {getChannelIcon(log.channel_type)}
                                      {log.channel_type}
                                    </span>
                                  </td>
                                  <td className="px-4 py-3 whitespace-nowrap text-sm text-gray-600 dark:text-gray-300">
                                    {formatAuditAction(log.event_type)}
                                  </td>
                                  <td className="px-4 py-3 whitespace-nowrap">
                                    {log.status === 'sent' ? (
                                      <span className="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-green-100 dark:bg-green-900 text-green-800 dark:text-green-200">
                                        Sent
                                      </span>
                                    ) : (
                                      <span className="inline-flex items-center px-2 py-0.5 rounded text-xs font-medium bg-red-100 dark:bg-red-900 text-red-800 dark:text-red-200" title={log.error_message || ''}>
                                        Failed
                                      </span>
                                    )}
                                  </td>
                                </tr>
                              ))}
                            </tbody>
                          </table>
                        </div>
                      )}
                    </div>
                  </>
                )}
              </div>
            )}

            {/* VPN Section */}
            {activeSection === 'vpn' && <VpnTab />}

            {/* Dynamic DNS Section */}
            {activeSection === 'ddns' && (
              <div className="space-y-6">
                <div>
                  <h3 className="text-2xl font-bold text-gray-900 dark:text-white">Dynamic DNS</h3>
                  <p className="text-gray-500 dark:text-gray-400 text-sm mt-1">
                    Keep your domain pointed to your dynamic IP address.
                  </p>
                </div>

                <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
                  <div className="text-gray-500 dark:text-gray-400 text-sm py-8 text-center">
                    DDNS configuration coming soon.
                  </div>
                </div>
              </div>
            )}

            {/* Cloudflare Tunnel Section */}
            {activeSection === 'cloudflare' && <CloudflareTab />}

            {/* Let's Encrypt Section */}
            {activeSection === 'letsencrypt' && (
              <div className="space-y-6">
                <div>
                  <h3 className="text-2xl font-bold text-gray-900 dark:text-white">Let's Encrypt</h3>
                  <p className="text-gray-500 dark:text-gray-400 text-sm mt-1">
                    Automatically provision and renew SSL certificates for your domains.
                  </p>
                </div>

                <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
                  <div className="text-gray-500 dark:text-gray-400 text-sm py-8 text-center">
                    Let's Encrypt configuration coming soon.
                  </div>
                </div>
              </div>
            )}

            {/* Create/Edit User Forms */}
            {viewMode === 'create' && (
              <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
                <UserForm
                  roles={roles}
                  onSubmit={handleCreateUser}
                  onCancel={handleCancel}
                  isEdit={false}
                />
              </div>
            )}

            {viewMode === 'edit' && selectedUser && (
              <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
                <UserForm
                  user={selectedUser}
                  roles={roles}
                  onSubmit={handleUpdateUser}
                  onCancel={handleCancel}
                  isEdit={true}
                  canResetPassword={hasPermission('users.reset_password')}
                />
              </div>
            )}
          </>
        )}

        {newInvite && (
          <InviteLinkModal invite={newInvite} onClose={handleCloseInviteModal} />
        )}
      </div>
    </div>
  );
};

export default SettingsPage;
