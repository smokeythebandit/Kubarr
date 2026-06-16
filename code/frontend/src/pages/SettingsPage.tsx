import React, { useState, useEffect } from 'react';
import { useSearchParams } from 'react-router-dom';
import { useAuth } from '../contexts/AuthContext';
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
import { Users, Link, UserPlus, Settings, Sliders, Lock, Menu, X, FileText, CheckCircle, XCircle, LayoutDashboard, Shield, Bell, Mail, Send, MessageSquare, Globe, Network } from 'lucide-react';
import { auditApi, AuditLog, AuditStats, AuditLogQuery } from '../api/audit';
import { notificationsApi, NotificationChannel, NotificationEvent, NotificationLog } from '../api/notifications';
import { oauthApi, OAuthProvider } from '../api/oauth';
import { VpnTab } from '../components/vpn';
import { AuditTab } from '../components/settings/tabs/AuditTab';
import { DashboardTab } from '../components/settings/tabs/DashboardTab';
import { DdnsTab } from '../components/settings/tabs/DdnsTab';
import { DomainsTab } from '../components/settings/tabs/DomainsTab';
import { GeneralTab } from '../components/settings/tabs/GeneralTab';
import { InvitesTab } from '../components/settings/tabs/InvitesTab';
import { LetsEncryptTab } from '../components/settings/tabs/LetsEncryptTab';
import { NotificationsTab } from '../components/settings/tabs/NotificationsTab';
import { PendingUsersTab } from '../components/settings/tabs/PendingUsersTab';
import { PermissionsTab } from '../components/settings/tabs/PermissionsTab';
import { UsersTab } from '../components/settings/tabs/UsersTab';

type ViewMode = 'list' | 'create' | 'edit';
type ApiErr = { response?: { data?: { detail?: string; error?: string } } };
type SettingsSection = 'dashboard' | 'general' | 'users' | 'pending' | 'invites' | 'permissions' | 'audit' | 'notifications' | 'vpn' | 'domains' | 'ddns' | 'letsencrypt';

const SettingsPage: React.FC = () => {
  const { isAdmin, hasPermission } = useAuth();
  const [searchParams, setSearchParams] = useSearchParams();
  const [users, setUsers] = useState<User[]>([]);
  const [pendingUsers, setPendingUsers] = useState<User[]>([]);
  const [invites, setInvites] = useState<Invite[]>([]);
  const [roles, setRoles] = useState<Role[]>([]);
  const [apps, setApps] = useState<App[]>([]);
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
    { id: 'domains' as SettingsSection, label: 'Domains', icon: Globe },
    { id: 'ddns' as SettingsSection, label: 'Dynamic DNS', icon: Network },
    { id: 'letsencrypt' as SettingsSection, label: "Let's Encrypt", icon: Lock },
    { id: 'vpn' as SettingsSection, label: 'VPN', icon: Shield },
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
            {activeSection === 'dashboard' && (
              <DashboardTab
                usersCount={users.length}
                pendingUsersCount={pendingUsers.length}
                invites={invites}
                roles={roles}
                auditStats={auditStats}
                isExpired={isExpired}
                formatAuditAction={formatAuditAction}
                onSelectSection={setActiveSection}
              />
            )}

            {activeSection === 'general' && (
              <GeneralTab
                systemSettings={systemSettings}
                savingSettings={savingSettings}
                oauthProviders={oauthProviders}
                editingOAuthProvider={editingOAuthProvider}
                oauthProviderConfig={oauthProviderConfig}
                savingOAuthProvider={savingOAuthProvider}
                onToggleSetting={handleToggleSetting}
                onToggleOAuthProvider={handleToggleOAuthProvider}
                onSaveOAuthProvider={handleSaveOAuthProvider}
                setEditingOAuthProvider={setEditingOAuthProvider}
                setOauthProviderConfig={setOauthProviderConfig}
                getOAuthProviderIcon={getOAuthProviderIcon}
              />
            )}

            {activeSection === 'users' && viewMode === 'list' && (
              <UsersTab
                users={users}
                onCreateUser={() => setViewMode('create')}
                onEditUser={handleEditUser}
                onDeleteUser={handleDeleteUser}
                onApproveUser={handleApproveUser}
              />
            )}

            {activeSection === 'pending' && viewMode === 'list' && (
              <PendingUsersTab
                pendingUsers={pendingUsers}
                onApproveUser={handleApproveUser}
                onRejectUser={handleRejectUser}
                onDeleteUser={handleDeleteUser}
              />
            )}

            {activeSection === 'invites' && viewMode === 'list' && (
              <InvitesTab
                invites={invites}
                creatingInvite={creatingInvite}
                copiedInviteId={copiedInviteId}
                onCreateInvite={handleCreateInvite}
                onDeleteInvite={handleDeleteInvite}
                onCopyInviteLink={copyInviteLink}
                formatDate={formatDate}
                isExpired={isExpired}
              />
            )}

            {activeSection === 'permissions' && <PermissionsTab />}

            {activeSection === 'audit' && (
              <AuditTab
                auditLogs={auditLogs}
                auditStats={auditStats}
                auditLoading={auditLoading}
                auditPage={auditPage}
                auditTotalPages={auditTotalPages}
                auditTotal={auditTotal}
                auditFilter={auditFilter}
                clearingLogs={clearingLogs}
                onClearOldLogs={handleClearOldLogs}
                onAuditFilterChange={handleAuditFilterChange}
                setAuditPage={setAuditPage}
                formatAuditAction={formatAuditAction}
                getActionIcon={getActionIcon}
              />
            )}

            {activeSection === 'notifications' && (
              <NotificationsTab
                notificationChannels={notificationChannels}
                notificationEvents={notificationEvents}
                notificationLogs={notificationLogs}
                notificationLogsTotal={notificationLogsTotal}
                notificationLoading={notificationLoading}
                testingChannel={testingChannel}
                testDestination={testDestination}
                editingChannel={editingChannel}
                channelConfig={channelConfig}
                setTestDestination={setTestDestination}
                setEditingChannel={setEditingChannel}
                setChannelConfig={setChannelConfig}
                onToggleChannel={handleToggleChannel}
                onToggleEvent={handleToggleEvent}
                onUpdateEventSeverity={handleUpdateEventSeverity}
                onTestChannel={handleTestChannel}
                onSaveChannelConfig={handleSaveChannelConfig}
                getChannelIcon={getChannelIcon}
                getChannelConfigFields={getChannelConfigFields}
                formatAuditAction={formatAuditAction}
              />
            )}

            {/* VPN Section */}
            {activeSection === 'vpn' && <VpnTab />}

            {activeSection === 'domains' && (
              <DomainsTab
                apps={apps}
              />
            )}

            {activeSection === 'ddns' && <DdnsTab />}

            {activeSection === 'letsencrypt' && <LetsEncryptTab />}

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
