import { useState, useEffect } from 'react'
import { useAuth } from '../contexts/AuthContext'
import { useTheme } from '../contexts/ThemeContext'
import { User, Mail, Shield, Sun, Moon, Monitor, Check, Key, Smartphone, AlertTriangle, Eye, EyeOff, Loader2, Link2, Unlink, Palette, Info, Clock, Globe, Trash2, History, Pencil, X } from 'lucide-react'
import { QRCodeSVG } from 'qrcode.react'
import type { Theme, TwoFactorStatusResponse, TwoFactorSetupResponse } from '../api/users'
import { changeOwnPassword, get2FAStatus, getRecoveryCodeCount, setup2FA, enable2FA, disable2FA, updateOwnProfile, deleteOwnAccount } from '../api/users'
import { oauthApi, type LinkedAccount, type AvailableProvider } from '../api/oauth'
import { getSessions, revokeSession, type SessionInfo } from '../api/auth'
import { auditApi, type AuditLog } from '../api/audit'

export default function AccountPage() {
  const { user, isAdmin, checkAuth, logout } = useAuth()
  const { theme, setTheme } = useTheme()
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(false)

  // Profile edit state
  const [editingProfile, setEditingProfile] = useState(false)
  const [editUsername, setEditUsername] = useState('')
  const [editEmail, setEditEmail] = useState('')
  const [profileError, setProfileError] = useState('')
  const [profileSuccess, setProfileSuccess] = useState('')
  const [savingProfile, setSavingProfile] = useState(false)

  // Password change state
  const [currentPassword, setCurrentPassword] = useState('')
  const [newPassword, setNewPassword] = useState('')
  const [confirmPassword, setConfirmPassword] = useState('')
  const [showCurrentPassword, setShowCurrentPassword] = useState(false)
  const [showNewPassword, setShowNewPassword] = useState(false)
  const [passwordError, setPasswordError] = useState('')
  const [passwordSuccess, setPasswordSuccess] = useState('')
  const [changingPassword, setChangingPassword] = useState(false)

  // 2FA state
  const [twoFactorStatus, setTwoFactorStatus] = useState<TwoFactorStatusResponse | null>(null)
  const [loading2FA, setLoading2FA] = useState(true)
  const [setupData, setSetupData] = useState<TwoFactorSetupResponse | null>(null)
  const [verificationCode, setVerificationCode] = useState('')
  const [disablePassword, setDisablePassword] = useState('')
  const [twoFactorError, setTwoFactorError] = useState('')
  const [twoFactorSuccess, setTwoFactorSuccess] = useState('')
  const [processing2FA, setProcessing2FA] = useState(false)
  const [recoveryCodes, setRecoveryCodes] = useState<string[] | null>(null)
  const [recoveryCodeCount, setRecoveryCodeCount] = useState<number | null>(null)

  // Linked accounts state
  const [linkedAccounts, setLinkedAccounts] = useState<LinkedAccount[]>([])
  const [availableProviders, setAvailableProviders] = useState<AvailableProvider[]>([])
  const [loadingLinkedAccounts, setLoadingLinkedAccounts] = useState(true)
  const [unlinkingProvider, setUnlinkingProvider] = useState<string | null>(null)
  const [linkedAccountError, setLinkedAccountError] = useState('')
  const [linkedAccountSuccess, setLinkedAccountSuccess] = useState('')

  // Sessions state
  const [sessions, setSessions] = useState<SessionInfo[]>([])
  const [loadingSessions, setLoadingSessions] = useState(true)
  const [revokingSession, setRevokingSession] = useState<string | null>(null)

  // Audit log state
  const [auditLogs, setAuditLogs] = useState<AuditLog[]>([])
  const [loadingAudit, setLoadingAudit] = useState(true)

  // Delete account state
  const [showDeleteConfirm, setShowDeleteConfirm] = useState(false)
  const [deletePassword, setDeletePassword] = useState('')
  const [deleteError, setDeleteError] = useState('')
  const [deletingAccount, setDeletingAccount] = useState(false)

  // Load 2FA status on mount
  useEffect(() => {
    const loadStatus = async () => {
      try {
        const status = await get2FAStatus()
        setTwoFactorStatus(status)
        if (status.enabled) {
          try {
            const counts = await getRecoveryCodeCount()
            setRecoveryCodeCount(counts.remaining)
          } catch {
            // Non-critical, ignore
          }
        }
      } catch (err) {
        console.error('Failed to load 2FA status:', err)
      } finally {
        setLoading2FA(false)
      }
    }
    loadStatus()
  }, [])

  // Load linked accounts on mount
  useEffect(() => {
    const loadLinkedAccounts = async () => {
      try {
        const [accounts, providers] = await Promise.all([
          oauthApi.getLinkedAccounts(),
          oauthApi.getAvailableProviders()
        ])
        setLinkedAccounts(accounts)
        setAvailableProviders(providers)
      } catch (err) {
        console.error('Failed to load linked accounts:', err)
      } finally {
        setLoadingLinkedAccounts(false)
      }
    }
    loadLinkedAccounts()
  }, [])

  // Load sessions on mount
  useEffect(() => {
    const loadSessions = async () => {
      try {
        const data = await getSessions()
        setSessions(data)
      } catch (err) {
        console.error('Failed to load sessions:', err)
      } finally {
        setLoadingSessions(false)
      }
    }
    loadSessions()
  }, [])

  // Load audit logs for current user
  useEffect(() => {
    const loadAuditLogs = async () => {
      if (!user) return
      try {
        const data = await auditApi.getLogs({ user_id: user.id, per_page: 10 })
        setAuditLogs(data.logs)
      } catch (err) {
        console.error('Failed to load audit logs:', err)
      } finally {
        setLoadingAudit(false)
      }
    }
    loadAuditLogs()
  }, [user])

  // Password change handler
  const handlePasswordChange = async (e: React.FormEvent) => {
    e.preventDefault()
    setPasswordError('')
    setPasswordSuccess('')

    if (newPassword.length < 8) {
      setPasswordError('New password must be at least 8 characters')
      return
    }
    if (newPassword !== confirmPassword) {
      setPasswordError('Passwords do not match')
      return
    }

    setChangingPassword(true)
    try {
      await changeOwnPassword({ current_password: currentPassword, new_password: newPassword })
      setPasswordSuccess('Password changed successfully')
      setCurrentPassword('')
      setNewPassword('')
      setConfirmPassword('')
    } catch (err: unknown) {
      const error = err as { response?: { data?: { error?: string } } }
      setPasswordError(error.response?.data?.error || 'Failed to change password')
    } finally {
      setChangingPassword(false)
    }
  }

  // 2FA setup handler
  const handleSetup2FA = async () => {
    setTwoFactorError('')
    setProcessing2FA(true)
    try {
      const data = await setup2FA()
      setSetupData(data)
    } catch (err: unknown) {
      const error = err as { response?: { data?: { error?: string } } }
      setTwoFactorError(error.response?.data?.error || 'Failed to set up 2FA')
    } finally {
      setProcessing2FA(false)
    }
  }

  // 2FA enable handler
  const handleEnable2FA = async () => {
    setTwoFactorError('')
    if (!verificationCode || verificationCode.length !== 6) {
      setTwoFactorError('Please enter a 6-digit code')
      return
    }

    setProcessing2FA(true)
    try {
      const result = await enable2FA(verificationCode)
      setSetupData(null)
      setVerificationCode('')
      setRecoveryCodes(result.recovery_codes)
      setRecoveryCodeCount(result.recovery_codes.length)
      const status = await get2FAStatus()
      setTwoFactorStatus(status)
    } catch (err: unknown) {
      const error = err as { response?: { data?: { error?: string } } }
      setTwoFactorError(error.response?.data?.error || 'Invalid verification code')
    } finally {
      setProcessing2FA(false)
    }
  }

  // 2FA disable handler
  const handleDisable2FA = async () => {
    setTwoFactorError('')
    if (!disablePassword) {
      setTwoFactorError('Please enter your password')
      return
    }

    setProcessing2FA(true)
    try {
      await disable2FA(disablePassword)
      setTwoFactorSuccess('Two-factor authentication disabled')
      setDisablePassword('')
      setRecoveryCodeCount(null)
      const status = await get2FAStatus()
      setTwoFactorStatus(status)
    } catch (err: unknown) {
      const error = err as { response?: { data?: { error?: string } } }
      setTwoFactorError(error.response?.data?.error || 'Failed to disable 2FA')
    } finally {
      setProcessing2FA(false)
    }
  }

  // Cancel 2FA setup
  const cancelSetup = () => {
    setSetupData(null)
    setVerificationCode('')
    setTwoFactorError('')
  }

  // Unlink OAuth account
  const handleUnlinkAccount = async (provider: string) => {
    setLinkedAccountError('')
    setLinkedAccountSuccess('')
    setUnlinkingProvider(provider)
    try {
      await oauthApi.unlinkAccount(provider)
      setLinkedAccounts(prev => prev.filter(a => a.provider !== provider))
      setLinkedAccountSuccess(`${provider.charAt(0).toUpperCase() + provider.slice(1)} account unlinked`)
    } catch (err: unknown) {
      const error = err as { response?: { data?: { error?: string } } }
      setLinkedAccountError(error.response?.data?.error || `Failed to unlink ${provider} account`)
    } finally {
      setUnlinkingProvider(null)
    }
  }

  // Link OAuth account
  const handleLinkAccount = (provider: string) => {
    window.location.href = oauthApi.getLinkUrl(provider)
  }

  // Revoke session
  const handleRevokeSession = async (sessionId: string) => {
    setRevokingSession(sessionId)
    try {
      await revokeSession(sessionId)
      setSessions(prev => prev.filter(s => s.id !== sessionId))
    } catch (err) {
      console.error('Failed to revoke session:', err)
    } finally {
      setRevokingSession(null)
    }
  }

  // Format user agent for display
  const formatUserAgent = (ua: string | null): string => {
    if (!ua) return 'Unknown device'
    if (ua.includes('Firefox')) return 'Firefox'
    if (ua.includes('Chrome')) return 'Chrome'
    if (ua.includes('Safari')) return 'Safari'
    if (ua.includes('Edge')) return 'Edge'
    return ua.slice(0, 30) + '...'
  }

  // Format relative time
  const formatRelativeTime = (dateStr: string): string => {
    const date = new Date(dateStr)
    const now = new Date()
    const diff = now.getTime() - date.getTime()
    const minutes = Math.floor(diff / 60000)
    const hours = Math.floor(diff / 3600000)
    const days = Math.floor(diff / 86400000)
    if (minutes < 1) return 'Just now'
    if (minutes < 60) return `${minutes}m ago`
    if (hours < 24) return `${hours}h ago`
    return `${days}d ago`
  }

  // Delete account handler
  const handleDeleteAccount = async () => {
    setDeleteError('')
    if (!deletePassword) {
      setDeleteError('Please enter your password')
      return
    }

    setDeletingAccount(true)
    try {
      await deleteOwnAccount({ password: deletePassword })
      // Account deleted successfully - log out and redirect
      logout()
      window.location.href = '/login'
    } catch (err: unknown) {
      const error = err as { response?: { data?: { error?: string } } }
      setDeleteError(error.response?.data?.error || 'Failed to delete account')
    } finally {
      setDeletingAccount(false)
    }
  }

  // Cancel delete account
  const cancelDeleteAccount = () => {
    setShowDeleteConfirm(false)
    setDeletePassword('')
    setDeleteError('')
  }

  // Start editing profile
  const startEditProfile = () => {
    if (user) {
      setEditUsername(user.username)
      setEditEmail(user.email)
      setProfileError('')
      setProfileSuccess('')
      setEditingProfile(true)
    }
  }

  // Cancel editing profile
  const cancelEditProfile = () => {
    setEditingProfile(false)
    setProfileError('')
  }

  // Save profile changes
  const handleSaveProfile = async (e: React.FormEvent) => {
    e.preventDefault()
    setProfileError('')
    setProfileSuccess('')

    if (!editUsername.trim()) {
      setProfileError('Username cannot be empty')
      return
    }
    if (editUsername.trim().length < 3) {
      setProfileError('Username must be at least 3 characters')
      return
    }
    if (!editEmail.trim() || !editEmail.includes('@')) {
      setProfileError('Please enter a valid email address')
      return
    }

    setSavingProfile(true)
    try {
      await updateOwnProfile({
        username: editUsername.trim(),
        email: editEmail.trim(),
      })
      await checkAuth() // Refresh user data
      setProfileSuccess('Profile updated successfully')
      setEditingProfile(false)
      setTimeout(() => setProfileSuccess(''), 3000)
    } catch (err: unknown) {
      const error = err as { response?: { data?: { error?: string } } }
      setProfileError(error.response?.data?.error || 'Failed to update profile')
    } finally {
      setSavingProfile(false)
    }
  }

  // Get provider icon
  const getProviderIcon = (provider: string) => {
    if (provider === 'google') {
      return (
        <svg className="w-5 h-5" viewBox="0 0 24 24">
          <path fill="#4285F4" d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.09z"/>
          <path fill="#34A853" d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z"/>
          <path fill="#FBBC05" d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z"/>
          <path fill="#EA4335" d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z"/>
        </svg>
      )
    }
    if (provider === 'microsoft') {
      return (
        <svg className="w-5 h-5" viewBox="0 0 21 21">
          <rect x="1" y="1" width="9" height="9" fill="#f25022"/>
          <rect x="1" y="11" width="9" height="9" fill="#00a4ef"/>
          <rect x="11" y="1" width="9" height="9" fill="#7fba00"/>
          <rect x="11" y="11" width="9" height="9" fill="#ffb900"/>
        </svg>
      )
    }
    return <Link2 size={20} />
  }

  const themes: { value: Theme; icon: typeof Sun; label: string; description: string }[] = [
    { value: 'light', icon: Sun, label: 'Light', description: 'Always use light mode' },
    { value: 'dark', icon: Moon, label: 'Dark', description: 'Always use dark mode' },
    { value: 'system', icon: Monitor, label: 'System', description: 'Follow your system preference' },
  ]

  const handleThemeChange = async (newTheme: Theme) => {
    setSaving(true)
    setSaved(false)
    await setTheme(newTheme)
    setSaving(false)
    setSaved(true)
    setTimeout(() => setSaved(false), 2000)
  }

  if (!user) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-500"></div>
      </div>
    )
  }

  return (
    <div className="h-full w-full overflow-auto p-4 md:p-6">
      {/* Page Header */}
      <div className="mb-6">
        <h1 className="text-2xl font-bold text-gray-900 dark:text-white">Account Settings</h1>
        <p className="text-gray-500 dark:text-gray-400 text-sm mt-1">
          Manage your profile, appearance, security, and linked accounts
        </p>
      </div>

      {/* Mosaic Grid */}
      <div className="grid grid-cols-1 lg:grid-cols-2 2xl:grid-cols-3 gap-6">
        {/* Profile Panel */}
        <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
          <div className="flex items-center justify-between mb-4">
            <h3 className="text-lg font-semibold text-gray-900 dark:text-white flex items-center gap-2">
              <User size={20} className="text-blue-500" />
              Profile
            </h3>
            {!editingProfile && (
              <button
                onClick={startEditProfile}
                className="p-2 text-gray-500 hover:text-blue-600 hover:bg-blue-50 dark:hover:bg-blue-900/20 rounded-lg transition-colors"
                title="Edit profile"
              >
                <Pencil size={18} />
              </button>
            )}
          </div>

          {profileSuccess && (
            <div className="p-3 mb-4 bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800 rounded-lg text-green-600 dark:text-green-400 text-sm flex items-center gap-2">
              <Check size={16} />
              {profileSuccess}
            </div>
          )}

          {editingProfile ? (
            <form onSubmit={handleSaveProfile} className="space-y-4">
              {profileError && (
                <div className="p-3 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg text-red-600 dark:text-red-400 text-sm">
                  {profileError}
                </div>
              )}
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                  Username
                </label>
                <input
                  type="text"
                  value={editUsername}
                  onChange={(e) => setEditUsername(e.target.value)}
                  className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-gray-100 focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                  minLength={3}
                  required
                />
              </div>
              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                  Email
                </label>
                <input
                  type="email"
                  value={editEmail}
                  onChange={(e) => setEditEmail(e.target.value)}
                  className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-gray-100 focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                  required
                />
              </div>
              <div className="flex gap-2">
                <button
                  type="submit"
                  disabled={savingProfile}
                  className="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-blue-400 text-white rounded-lg font-medium flex items-center gap-2"
                >
                  {savingProfile && <Loader2 size={16} className="animate-spin" />}
                  Save
                </button>
                <button
                  type="button"
                  onClick={cancelEditProfile}
                  className="px-4 py-2 border border-gray-300 dark:border-gray-600 text-gray-700 dark:text-gray-300 hover:bg-gray-50 dark:hover:bg-gray-700 rounded-lg font-medium flex items-center gap-1"
                >
                  <X size={16} />
                  Cancel
                </button>
              </div>
            </form>
          ) : (
            <div className="space-y-4">
              <div className="flex items-center gap-4">
                <div className="w-10 h-10 rounded-full bg-blue-100 dark:bg-blue-900/30 flex items-center justify-center">
                  <User size={20} className="text-blue-600 dark:text-blue-400" />
                </div>
                <div>
                  <div className="text-sm text-gray-500 dark:text-gray-400">Username</div>
                  <div className="font-medium">{user.username}</div>
                </div>
              </div>
              <div className="flex items-center gap-4">
                <div className="w-10 h-10 rounded-full bg-green-100 dark:bg-green-900/30 flex items-center justify-center">
                  <Mail size={20} className="text-green-600 dark:text-green-400" />
                </div>
                <div>
                  <div className="text-sm text-gray-500 dark:text-gray-400">Email</div>
                  <div className="font-medium">{user.email}</div>
                </div>
              </div>
              <div className="flex items-center gap-4">
                <div className="w-10 h-10 rounded-full bg-purple-100 dark:bg-purple-900/30 flex items-center justify-center">
                  <Shield size={20} className="text-purple-600 dark:text-purple-400" />
                </div>
                <div>
                  <div className="text-sm text-gray-500 dark:text-gray-400">Role</div>
                  <div className="font-medium">
                    {isAdmin ? (
                      <span className="inline-flex items-center gap-1">
                        Administrator
                        <span className="px-1.5 py-0.5 text-xs bg-blue-600 text-white rounded font-medium">Admin</span>
                      </span>
                    ) : (
                      user.roles?.length ? user.roles.map(r => r.name).join(', ') : 'User'
                    )}
                  </div>
                </div>
              </div>
            </div>
          )}

          {/* Account Info */}
          <div className="mt-6 pt-4 border-t border-gray-200 dark:border-gray-700">
            <h4 className="text-sm font-medium text-gray-700 dark:text-gray-300 mb-3 flex items-center gap-2">
              <Info size={16} />
              Account Information
            </h4>
            <div className="space-y-2 text-sm">
              <div className="flex justify-between">
                <span className="text-gray-500 dark:text-gray-400">Account created</span>
                <span>{new Date(user.created_at).toLocaleDateString()}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-gray-500 dark:text-gray-400">Last updated</span>
                <span>{new Date(user.updated_at).toLocaleDateString()}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-gray-500 dark:text-gray-400">Status</span>
                <span className={user.is_active ? 'text-green-600 dark:text-green-400' : 'text-red-600 dark:text-red-400'}>
                  {user.is_active ? 'Active' : 'Inactive'}
                </span>
              </div>
            </div>
          </div>
        </div>

        {/* Appearance Panel */}
        <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
          <div className="flex items-center justify-between mb-4">
            <h3 className="text-lg font-semibold text-gray-900 dark:text-white flex items-center gap-2">
              <Palette size={20} className="text-purple-500" />
              Appearance
            </h3>
            {saving && (
              <span className="text-sm text-gray-500 dark:text-gray-400">Saving...</span>
            )}
            {saved && (
              <span className="text-sm text-green-600 dark:text-green-400 flex items-center gap-1">
                <Check size={16} />
                Saved
              </span>
            )}
          </div>
          <p className="text-sm text-gray-500 dark:text-gray-400 mb-4">
            Choose how Kubarr looks to you. Select a single theme, or sync with your system.
          </p>
          <div className="grid grid-cols-3 gap-3">
            {themes.map(({ value, icon: Icon, label, description }) => (
              <button
                key={value}
                onClick={() => handleThemeChange(value)}
                disabled={saving}
                className={`relative flex flex-col items-center gap-2 p-4 rounded-lg border-2 transition-all ${
                  theme === value
                    ? 'border-blue-500 bg-blue-50 dark:bg-blue-900/20'
                    : 'border-gray-200 dark:border-gray-700 hover:border-gray-300 dark:hover:border-gray-600'
                }`}
              >
                {theme === value && (
                  <div className="absolute top-2 right-2">
                    <Check size={16} className="text-blue-500" />
                  </div>
                )}
                <Icon size={24} className={theme === value ? 'text-blue-500' : 'text-gray-500 dark:text-gray-400'} />
                <div className="text-sm font-medium">{label}</div>
                <div className="text-xs text-gray-500 dark:text-gray-400 text-center">{description}</div>
              </button>
            ))}
          </div>
        </div>

        {/* Linked Accounts Panel */}
        <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
          <h3 className="text-lg font-semibold text-gray-900 dark:text-white mb-4 flex items-center gap-2">
            <Link2 size={20} className="text-cyan-500" />
            Linked Accounts
          </h3>

          {loadingLinkedAccounts ? (
            <div className="flex items-center gap-2 text-gray-500">
              <Loader2 size={16} className="animate-spin" />
              Loading...
            </div>
          ) : (
            <>
              {linkedAccountError && (
                <div className="p-3 mb-4 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg text-red-600 dark:text-red-400 text-sm">
                  {linkedAccountError}
                </div>
              )}
              {linkedAccountSuccess && (
                <div className="p-3 mb-4 bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800 rounded-lg text-green-600 dark:text-green-400 text-sm flex items-center gap-2">
                  <Check size={16} />
                  {linkedAccountSuccess}
                </div>
              )}

              <p className="text-sm text-gray-600 dark:text-gray-400 mb-4">
                Link your account to sign in with Google or Microsoft.
              </p>

              {/* Linked accounts list */}
              {linkedAccounts.length > 0 && (
                <div className="space-y-3 mb-4">
                  {linkedAccounts.map(account => (
                    <div
                      key={account.provider}
                      className="flex items-center justify-between p-3 bg-gray-50 dark:bg-gray-900 rounded-lg"
                    >
                      <div className="flex items-center gap-3">
                        {getProviderIcon(account.provider)}
                        <div>
                          <div className="font-medium capitalize">{account.provider}</div>
                          <div className="text-sm text-gray-500 dark:text-gray-400">
                            {account.email || account.display_name || 'Connected'}
                          </div>
                        </div>
                      </div>
                      <button
                        onClick={() => handleUnlinkAccount(account.provider)}
                        disabled={unlinkingProvider === account.provider}
                        className="px-3 py-1.5 text-sm border border-gray-300 dark:border-gray-600 text-gray-700 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg font-medium flex items-center gap-1.5"
                      >
                        {unlinkingProvider === account.provider ? (
                          <Loader2 size={14} className="animate-spin" />
                        ) : (
                          <Unlink size={14} />
                        )}
                        Unlink
                      </button>
                    </div>
                  ))}
                </div>
              )}

              {/* Link new provider buttons */}
              {availableProviders.length > 0 && (
                <div className="flex flex-wrap gap-2">
                  {availableProviders
                    .filter(p => !linkedAccounts.some(a => a.provider === p.id))
                    .map(provider => (
                      <button
                        key={provider.id}
                        onClick={() => handleLinkAccount(provider.id)}
                        className="px-4 py-2 border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700 text-gray-700 dark:text-gray-300 hover:bg-gray-50 dark:hover:bg-gray-600 rounded-lg font-medium flex items-center gap-2"
                      >
                        {getProviderIcon(provider.id)}
                        Link {provider.name}
                      </button>
                    ))}
                </div>
              )}

              {availableProviders.length === 0 && linkedAccounts.length === 0 && (
                <p className="text-sm text-gray-500 dark:text-gray-400 italic">
                  No OAuth providers are currently configured by the administrator.
                </p>
              )}
            </>
          )}
        </div>

        {/* Security Panel - Change Password */}
        <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
          <h3 className="text-lg font-semibold text-gray-900 dark:text-white mb-4 flex items-center gap-2">
            <Key size={20} className="text-amber-500" />
            Change Password
          </h3>
          <form onSubmit={handlePasswordChange} className="space-y-4">
            {passwordError && (
              <div className="p-3 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg text-red-600 dark:text-red-400 text-sm">
                {passwordError}
              </div>
            )}
            {passwordSuccess && (
              <div className="p-3 bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800 rounded-lg text-green-600 dark:text-green-400 text-sm flex items-center gap-2">
                <Check size={16} />
                {passwordSuccess}
              </div>
            )}
            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                Current Password
              </label>
              <div className="relative">
                <input
                  type={showCurrentPassword ? 'text' : 'password'}
                  value={currentPassword}
                  onChange={(e) => setCurrentPassword(e.target.value)}
                  className="w-full px-3 py-2 pr-10 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-gray-100 focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                  required
                />
                <button
                  type="button"
                  onClick={() => setShowCurrentPassword(!showCurrentPassword)}
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-gray-400 hover:text-gray-600 dark:hover:text-gray-300"
                >
                  {showCurrentPassword ? <EyeOff size={18} /> : <Eye size={18} />}
                </button>
              </div>
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                New Password
              </label>
              <div className="relative">
                <input
                  type={showNewPassword ? 'text' : 'password'}
                  value={newPassword}
                  onChange={(e) => setNewPassword(e.target.value)}
                  className="w-full px-3 py-2 pr-10 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-gray-100 focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                  minLength={8}
                  required
                />
                <button
                  type="button"
                  onClick={() => setShowNewPassword(!showNewPassword)}
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-gray-400 hover:text-gray-600 dark:hover:text-gray-300"
                >
                  {showNewPassword ? <EyeOff size={18} /> : <Eye size={18} />}
                </button>
              </div>
              <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">Minimum 8 characters</p>
            </div>
            <div>
              <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                Confirm New Password
              </label>
              <input
                type="password"
                value={confirmPassword}
                onChange={(e) => setConfirmPassword(e.target.value)}
                className="w-full px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-gray-100 focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                required
              />
            </div>
            <button
              type="submit"
              disabled={changingPassword}
              className="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-blue-400 text-white rounded-lg font-medium flex items-center gap-2"
            >
              {changingPassword && <Loader2 size={16} className="animate-spin" />}
              Change Password
            </button>
          </form>
        </div>

        {/* Active Sessions Panel */}
        <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
          <h3 className="text-lg font-semibold text-gray-900 dark:text-white mb-4 flex items-center gap-2">
            <Globe size={20} className="text-blue-500" />
            Active Sessions
          </h3>

          {loadingSessions ? (
            <div className="flex items-center gap-2 text-gray-500">
              <Loader2 size={16} className="animate-spin" />
              Loading...
            </div>
          ) : sessions.length === 0 ? (
            <p className="text-sm text-gray-500 dark:text-gray-400">No active sessions</p>
          ) : (
            <div className="space-y-3">
              {sessions.map(session => (
                <div
                  key={session.id}
                  className={`p-3 rounded-lg border ${
                    session.is_current
                      ? 'border-blue-200 dark:border-blue-800 bg-blue-50 dark:bg-blue-900/20'
                      : 'border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-900'
                  }`}
                >
                  <div className="flex items-start justify-between gap-2">
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="font-medium text-sm truncate">
                          {formatUserAgent(session.user_agent)}
                        </span>
                        {session.is_current && (
                          <span className="px-1.5 py-0.5 text-xs bg-blue-600 text-white rounded font-medium">
                            Current
                          </span>
                        )}
                      </div>
                      <div className="text-xs text-gray-500 dark:text-gray-400 mt-1 flex items-center gap-3">
                        {session.ip_address && (
                          <span className="flex items-center gap-1">
                            <Globe size={12} />
                            {session.ip_address}
                          </span>
                        )}
                        <span className="flex items-center gap-1">
                          <Clock size={12} />
                          {formatRelativeTime(session.last_accessed_at)}
                        </span>
                      </div>
                    </div>
                    {!session.is_current && (
                      <button
                        onClick={() => handleRevokeSession(session.id)}
                        disabled={revokingSession === session.id}
                        className="p-1.5 text-red-600 hover:bg-red-100 dark:hover:bg-red-900/30 rounded transition-colors"
                        title="Revoke session"
                      >
                        {revokingSession === session.id ? (
                          <Loader2 size={16} className="animate-spin" />
                        ) : (
                          <Trash2 size={16} />
                        )}
                      </button>
                    )}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Recent Activity Panel */}
        <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6">
          <h3 className="text-lg font-semibold text-gray-900 dark:text-white mb-4 flex items-center gap-2">
            <History size={20} className="text-indigo-500" />
            Recent Activity
          </h3>

          {loadingAudit ? (
            <div className="flex items-center gap-2 text-gray-500">
              <Loader2 size={16} className="animate-spin" />
              Loading...
            </div>
          ) : auditLogs.length === 0 ? (
            <p className="text-sm text-gray-500 dark:text-gray-400">No recent activity</p>
          ) : (
            <div className="space-y-2">
              {auditLogs.map(log => (
                <div
                  key={log.id}
                  className={`p-2 rounded text-sm border-l-2 ${
                    log.success
                      ? 'border-l-green-500 bg-gray-50 dark:bg-gray-900'
                      : 'border-l-red-500 bg-red-50 dark:bg-red-900/20'
                  }`}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="font-medium truncate">{log.action.replace(/_/g, ' ')}</span>
                    <span className="text-xs text-gray-500 dark:text-gray-400 whitespace-nowrap">
                      {formatRelativeTime(log.timestamp)}
                    </span>
                  </div>
                  {log.resource_type && (
                    <div className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">
                      {log.resource_type}
                      {log.resource_id && `: ${log.resource_id}`}
                    </div>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Security Panel - Two-Factor Authentication */}
        <div className="bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 p-6 lg:col-span-2 2xl:col-span-1">
          <h3 className="text-lg font-semibold text-gray-900 dark:text-white mb-4 flex items-center gap-2">
            <Smartphone size={20} className="text-green-500" />
            Two-Factor Authentication
          </h3>

          {loading2FA ? (
            <div className="flex items-center gap-2 text-gray-500">
              <Loader2 size={16} className="animate-spin" />
              Loading...
            </div>
          ) : (
            <>
              {twoFactorError && (
                <div className="p-3 mb-4 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg text-red-600 dark:text-red-400 text-sm">
                  {twoFactorError}
                </div>
              )}
              {twoFactorSuccess && (
                <div className="p-3 mb-4 bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800 rounded-lg text-green-600 dark:text-green-400 text-sm flex items-center gap-2">
                  <Check size={16} />
                  {twoFactorSuccess}
                </div>
              )}

              {/* Status Display */}
              <div className="flex items-center gap-3 mb-4 flex-wrap">
                <span className="text-sm text-gray-600 dark:text-gray-400">Status:</span>
                {twoFactorStatus?.enabled ? (
                  <span className="inline-flex items-center gap-1 px-2 py-1 bg-green-100 dark:bg-green-900/30 text-green-700 dark:text-green-400 rounded text-sm font-medium">
                    <Check size={14} />
                    Enabled
                  </span>
                ) : (
                  <span className="inline-flex items-center gap-1 px-2 py-1 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 rounded text-sm font-medium">
                    Disabled
                  </span>
                )}
                {twoFactorStatus?.required_by_role && !twoFactorStatus?.enabled && (
                  <span className="inline-flex items-center gap-1 px-2 py-1 bg-amber-100 dark:bg-amber-900/30 text-amber-700 dark:text-amber-400 rounded text-sm font-medium">
                    <AlertTriangle size={14} />
                    Required by your role
                  </span>
                )}
                {twoFactorStatus?.enabled && recoveryCodeCount !== null && (
                  <span className="inline-flex items-center gap-1 px-2 py-1 bg-blue-100 dark:bg-blue-900/30 text-blue-700 dark:text-blue-400 rounded text-sm font-medium">
                    <Key size={14} />
                    {recoveryCodeCount} recovery {recoveryCodeCount === 1 ? 'code' : 'codes'} remaining
                  </span>
                )}
              </div>

              {/* One-time recovery codes display after enabling */}
              {recoveryCodes && recoveryCodes.length > 0 && (
                <div className="mb-4 p-4 bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-lg">
                  <div className="flex items-start gap-2 mb-3">
                    <AlertTriangle size={18} className="text-amber-600 dark:text-amber-400 flex-shrink-0 mt-0.5" />
                    <div>
                      <p className="text-sm font-semibold text-amber-800 dark:text-amber-300">
                        Save your recovery codes now
                      </p>
                      <p className="text-xs text-amber-700 dark:text-amber-400 mt-0.5">
                        These codes will only be shown once. Store them somewhere safe — you can use them to access your account if you lose your authenticator.
                      </p>
                    </div>
                  </div>
                  <div className="grid grid-cols-2 gap-2 mb-3">
                    {recoveryCodes.map((code, i) => (
                      <code
                        key={i}
                        className="px-3 py-1.5 bg-white dark:bg-gray-800 border border-amber-200 dark:border-amber-700 rounded text-sm font-mono text-center tracking-wider"
                      >
                        {code}
                      </code>
                    ))}
                  </div>
                  <div className="flex gap-2">
                    <button
                      onClick={() => {
                        navigator.clipboard.writeText(recoveryCodes.join('\n'))
                      }}
                      className="px-3 py-1.5 text-sm bg-amber-100 dark:bg-amber-800 hover:bg-amber-200 dark:hover:bg-amber-700 text-amber-800 dark:text-amber-200 rounded-lg font-medium flex items-center gap-1.5"
                    >
                      <Key size={14} />
                      Copy all
                    </button>
                    <button
                      onClick={() => setRecoveryCodes(null)}
                      className="px-3 py-1.5 text-sm border border-amber-300 dark:border-amber-700 text-amber-700 dark:text-amber-400 hover:bg-amber-50 dark:hover:bg-amber-900/30 rounded-lg font-medium"
                    >
                      I've saved my codes
                    </button>
                  </div>
                </div>
              )}

              {/* Setup Flow */}
              {!twoFactorStatus?.enabled && !setupData && (
                <div>
                  <p className="text-sm text-gray-600 dark:text-gray-400 mb-4">
                    Add an extra layer of security to your account by enabling two-factor authentication.
                    You'll need an authenticator app like Google Authenticator, Authy, or 1Password.
                  </p>
                  <button
                    onClick={handleSetup2FA}
                    disabled={processing2FA}
                    className="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:bg-blue-400 text-white rounded-lg font-medium flex items-center gap-2"
                  >
                    {processing2FA && <Loader2 size={16} className="animate-spin" />}
                    Set Up Two-Factor Authentication
                  </button>
                </div>
              )}

              {/* QR Code and Verification */}
              {setupData && (
                <div className="space-y-4">
                  <div className="p-4 bg-gray-50 dark:bg-gray-900 rounded-lg">
                    <p className="text-sm text-gray-600 dark:text-gray-400 mb-4">
                      Scan this QR code with your authenticator app, then enter the 6-digit code below to verify.
                    </p>
                    <div className="flex flex-col sm:flex-row gap-6 items-start">
                      <div className="bg-white p-3 rounded-lg">
                        <QRCodeSVG
                          value={setupData.provisioning_uri}
                          size={180}
                          level="M"
                        />
                      </div>
                      <div className="flex-1 space-y-3">
                        <div>
                          <label className="block text-xs font-medium text-gray-500 dark:text-gray-400 mb-1">
                            Manual Entry Key
                          </label>
                          <code className="block px-3 py-2 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded text-sm font-mono break-all">
                            {setupData.secret}
                          </code>
                        </div>
                        <div>
                          <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                            Verification Code
                          </label>
                          <input
                            type="text"
                            value={verificationCode}
                            onChange={(e) => setVerificationCode(e.target.value.replace(/\D/g, '').slice(0, 6))}
                            placeholder="000000"
                            maxLength={6}
                            className="w-32 px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-gray-100 focus:ring-2 focus:ring-blue-500 focus:border-transparent text-center font-mono text-lg tracking-wider"
                          />
                        </div>
                      </div>
                    </div>
                  </div>
                  <div className="flex gap-3">
                    <button
                      onClick={handleEnable2FA}
                      disabled={processing2FA || verificationCode.length !== 6}
                      className="px-4 py-2 bg-green-600 hover:bg-green-700 disabled:bg-green-400 text-white rounded-lg font-medium flex items-center gap-2"
                    >
                      {processing2FA && <Loader2 size={16} className="animate-spin" />}
                      Verify & Enable
                    </button>
                    <button
                      onClick={cancelSetup}
                      className="px-4 py-2 border border-gray-300 dark:border-gray-600 text-gray-700 dark:text-gray-300 hover:bg-gray-50 dark:hover:bg-gray-700 rounded-lg font-medium"
                    >
                      Cancel
                    </button>
                  </div>
                </div>
              )}

              {/* Disable 2FA */}
              {twoFactorStatus?.enabled && (
                <div className="space-y-4">
                  <p className="text-sm text-gray-600 dark:text-gray-400">
                    To disable two-factor authentication, enter your password below.
                  </p>
                  {twoFactorStatus.required_by_role && (
                    <div className="p-3 bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-lg text-amber-700 dark:text-amber-400 text-sm flex items-center gap-2">
                      <AlertTriangle size={16} />
                      Your role requires 2FA. You cannot disable it while assigned to this role.
                    </div>
                  )}
                  <div className="flex gap-3 items-end">
                    <div>
                      <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                        Password
                      </label>
                      <input
                        type="password"
                        value={disablePassword}
                        onChange={(e) => setDisablePassword(e.target.value)}
                        className="w-48 px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-gray-100 focus:ring-2 focus:ring-blue-500 focus:border-transparent"
                        disabled={twoFactorStatus.required_by_role}
                      />
                    </div>
                    <button
                      onClick={handleDisable2FA}
                      disabled={processing2FA || twoFactorStatus.required_by_role || !disablePassword}
                      className="px-4 py-2 bg-red-600 hover:bg-red-700 disabled:bg-red-400 disabled:cursor-not-allowed text-white rounded-lg font-medium flex items-center gap-2"
                    >
                      {processing2FA && <Loader2 size={16} className="animate-spin" />}
                      Disable 2FA
                    </button>
                  </div>
                </div>
              )}
            </>
          )}
        </div>

        {/* Danger Zone - Delete Account */}
        <div className="bg-white dark:bg-gray-800 rounded-lg border border-red-200 dark:border-red-800 p-6 lg:col-span-2 2xl:col-span-3">
          <h3 className="text-lg font-semibold text-red-600 dark:text-red-400 mb-4 flex items-center gap-2">
            <AlertTriangle size={20} />
            Danger Zone
          </h3>

          {!showDeleteConfirm ? (
            <div className="flex items-center justify-between">
              <div>
                <p className="font-medium text-gray-900 dark:text-white">Delete Account</p>
                <p className="text-sm text-gray-500 dark:text-gray-400">
                  Permanently delete your account and all associated data. This action cannot be undone.
                </p>
              </div>
              <button
                onClick={() => setShowDeleteConfirm(true)}
                className="px-4 py-2 bg-red-600 hover:bg-red-700 text-white rounded-lg font-medium flex items-center gap-2"
              >
                <Trash2 size={16} />
                Delete Account
              </button>
            </div>
          ) : (
            <div className="space-y-4">
              <div className="p-4 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg">
                <p className="text-red-700 dark:text-red-400 font-medium mb-2">
                  Are you sure you want to delete your account?
                </p>
                <p className="text-sm text-red-600 dark:text-red-400">
                  This will permanently delete your account, preferences, sessions, and all associated data.
                  This action cannot be undone.
                </p>
              </div>

              {deleteError && (
                <div className="p-3 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg text-red-600 dark:text-red-400 text-sm">
                  {deleteError}
                </div>
              )}

              <div>
                <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
                  Enter your password to confirm
                </label>
                <input
                  type="password"
                  value={deletePassword}
                  onChange={(e) => setDeletePassword(e.target.value)}
                  className="w-64 px-3 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-gray-100 focus:ring-2 focus:ring-red-500 focus:border-transparent"
                  placeholder="Your password"
                />
              </div>

              <div className="flex gap-3">
                <button
                  onClick={handleDeleteAccount}
                  disabled={deletingAccount || !deletePassword}
                  className="px-4 py-2 bg-red-600 hover:bg-red-700 disabled:bg-red-400 text-white rounded-lg font-medium flex items-center gap-2"
                >
                  {deletingAccount && <Loader2 size={16} className="animate-spin" />}
                  <Trash2 size={16} />
                  Yes, Delete My Account
                </button>
                <button
                  onClick={cancelDeleteAccount}
                  className="px-4 py-2 border border-gray-300 dark:border-gray-600 text-gray-700 dark:text-gray-300 hover:bg-gray-50 dark:hover:bg-gray-700 rounded-lg font-medium"
                >
                  Cancel
                </button>
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
