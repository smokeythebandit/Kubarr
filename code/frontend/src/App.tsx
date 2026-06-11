import { useState, useEffect, useRef, useCallback, useLayoutEffect } from 'react'
import { BrowserRouter, Routes, Route, Link, useLocation, Navigate } from 'react-router-dom'
import Dashboard from './pages/Dashboard'
import AppsPage from './pages/AppsPage'
import StoragePage from './pages/StoragePage'
import LogsPage from './pages/LogsPage'
import MonitoringPage from './pages/MonitoringPage'
import NetworkingPage from './pages/NetworkingPage'
import SecurityPage from './pages/SecurityPage'
import SettingsPage from './pages/SettingsPage'
import AccountPage from './pages/AccountPage'
import SetupPage from './pages/SetupPage'
import LoginPage from './pages/LoginPage'
import NotFoundPage from './pages/NotFoundPage'
import AppErrorPage from './pages/AppErrorPage'
import { AuthProvider, useAuth } from './contexts/AuthContext'
import { ThemeProvider, useTheme } from './contexts/ThemeContext'
import { MonitoringProvider, useMonitoring } from './contexts/MonitoringContext'
import { VersionFooter } from './components/VersionFooter'
import { PageTransition } from './components/PageTransition'
import { setupApi } from './api/setup'
import { sessionLogout } from './api/auth'
import { Grid3X3, HardDrive, FileText, Activity, Settings, User, LogOut, Ship, ChevronDown, Sun, Moon, Monitor, Network, Menu, X, Shield, Bell, Check, Trash2, AlertCircle, Info, AlertTriangle } from 'lucide-react'
import { notificationsApi, Notification } from './api/notifications'

function ThemeToggle() {
  const { theme, resolvedTheme, setTheme } = useTheme()
  const [dropdownOpen, setDropdownOpen] = useState(false)

  const themes = [
    { value: 'light' as const, icon: Sun, label: 'Light' },
    { value: 'dark' as const, icon: Moon, label: 'Dark' },
    { value: 'system' as const, icon: Monitor, label: 'System' },
  ]

  // Show Sun/Moon based on actual displayed theme, not the setting
  const ButtonIcon = resolvedTheme === 'dark' ? Moon : Sun
  const currentTheme = themes.find(t => t.value === theme) || themes[2]

  return (
    <div className="relative">
      <button
        onClick={() => setDropdownOpen(!dropdownOpen)}
        onBlur={() => setTimeout(() => setDropdownOpen(false), 150)}
        className="flex items-center justify-center w-9 h-9 rounded-md text-gray-600 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-700 transition-colors"
        title={`Theme: ${currentTheme.label}`}
      >
        <ButtonIcon size={18} strokeWidth={2} />
      </button>
      <div className={`absolute top-full right-0 mt-1 w-36 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-md shadow-lg z-50 transition-all duration-200 ease-out origin-top-right ${
        dropdownOpen
          ? 'opacity-100 scale-100 translate-y-0 pointer-events-auto'
          : 'opacity-0 scale-95 -translate-y-1 pointer-events-none'
      }`}>
          {themes.map(({ value, icon: Icon, label }) => (
            <button
              key={value}
              onClick={() => {
                setTheme(value)
                setDropdownOpen(false)
              }}
              className={`flex items-center gap-2 w-full px-3 py-2 text-sm transition-colors ${
                theme === value
                  ? 'bg-gray-100 dark:bg-gray-700 text-gray-900 dark:text-white'
                  : 'text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700'
              } first:rounded-t-md last:rounded-b-md`}
            >
              <Icon size={16} strokeWidth={2} />
              <span>{label}</span>
            </button>
          ))}
        </div>
    </div>
  )
}

function NotificationInbox() {
  const { isAuthenticated } = useAuth()
  const [dropdownOpen, setDropdownOpen] = useState(false)
  const [notifications, setNotifications] = useState<Notification[]>([])
  const [unreadCount, setUnreadCount] = useState(0)
  const [loading, setLoading] = useState(false)
  const notifRef = useRef<HTMLDivElement>(null)

  // Close on click outside
  useEffect(() => {
    if (!dropdownOpen) return
    const handler = (e: MouseEvent) => {
      if (notifRef.current && !notifRef.current.contains(e.target as Node)) {
        setDropdownOpen(false)
      }
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [dropdownOpen])

  // Fetch unread count on mount and periodically
  useEffect(() => {
    if (!isAuthenticated) return

    const fetchUnreadCount = async () => {
      try {
        const count = await notificationsApi.getUnreadCount()
        setUnreadCount(count)
      } catch {
        // Silently fail - not critical
      }
    }

    fetchUnreadCount()
    const interval = setInterval(fetchUnreadCount, 30000) // Every 30 seconds
    return () => clearInterval(interval)
  }, [isAuthenticated])

  // Fetch notifications when dropdown opens
  const handleOpen = async () => {
    if (!dropdownOpen) {
      setLoading(true)
      try {
        const response = await notificationsApi.getInbox(10, 0)
        setNotifications(response.notifications)
        setUnreadCount(response.unread)
      } catch {
        // Silently fail
      } finally {
        setLoading(false)
      }
    }
    setDropdownOpen(!dropdownOpen)
  }

  const handleMarkAsRead = async (id: number, e: React.MouseEvent) => {
    e.stopPropagation()
    try {
      await notificationsApi.markAsRead(id)
      setNotifications(prev => prev.map(n => n.id === id ? { ...n, read: true } : n))
      setUnreadCount(prev => Math.max(0, prev - 1))
    } catch {
      // Silently fail
    }
  }

  const handleMarkAllAsRead = async () => {
    try {
      await notificationsApi.markAllAsRead()
      setNotifications(prev => prev.map(n => ({ ...n, read: true })))
      setUnreadCount(0)
    } catch {
      // Silently fail
    }
  }

  const handleDelete = async (id: number, e: React.MouseEvent) => {
    e.stopPropagation()
    try {
      await notificationsApi.deleteNotification(id)
      const wasUnread = notifications.find(n => n.id === id && !n.read)
      setNotifications(prev => prev.filter(n => n.id !== id))
      if (wasUnread) {
        setUnreadCount(prev => Math.max(0, prev - 1))
      }
    } catch {
      // Silently fail
    }
  }

  const getSeverityIcon = (severity: string) => {
    switch (severity) {
      case 'critical':
        return <AlertCircle size={16} className="text-red-500" />
      case 'warning':
        return <AlertTriangle size={16} className="text-yellow-500" />
      default:
        return <Info size={16} className="text-blue-500" />
    }
  }

  const formatTime = (dateStr: string) => {
    const date = new Date(dateStr)
    const now = new Date()
    const diff = now.getTime() - date.getTime()
    const minutes = Math.floor(diff / 60000)
    const hours = Math.floor(diff / 3600000)
    const days = Math.floor(diff / 86400000)

    if (minutes < 1) return 'Just now'
    if (minutes < 60) return `${minutes}m ago`
    if (hours < 24) return `${hours}h ago`
    if (days < 7) return `${days}d ago`
    return date.toLocaleDateString()
  }

  if (!isAuthenticated) return null

  return (
    <div ref={notifRef} className="relative">
      <button
        onClick={handleOpen}
        className="relative flex items-center justify-center w-9 h-9 rounded-md text-gray-600 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-700 transition-colors"
        title="Notifications"
      >
        <Bell size={18} strokeWidth={2} />
        {unreadCount > 0 && (
          <span className="absolute -top-1 -right-1 flex items-center justify-center min-w-[18px] h-[18px] px-1 text-xs font-bold text-white bg-red-500 rounded-full">
            {unreadCount > 99 ? '99+' : unreadCount}
          </span>
        )}
      </button>
      <div className={`absolute top-full right-0 mt-1 w-80 bg-white dark:bg-gray-800 border border-gray-200/60 dark:border-gray-700/60 rounded-lg shadow-[0_8px_24px_rgba(0,0,0,0.1)] dark:shadow-[0_8px_24px_rgba(0,0,0,0.4)] z-50 overflow-hidden transition-all duration-200 ease-out origin-top-right ${
        dropdownOpen
          ? 'opacity-100 scale-100 translate-y-0 pointer-events-auto'
          : 'opacity-0 scale-95 -translate-y-1 pointer-events-none'
      }`}>
          <div className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-gray-700">
            <h3 className="font-semibold text-gray-900 dark:text-white">Notifications</h3>
            {unreadCount > 0 && (
              <button
                onClick={handleMarkAllAsRead}
                className="text-xs text-blue-600 dark:text-blue-400 hover:underline"
              >
                Mark all as read
              </button>
            )}
          </div>
          <div className="max-h-80 overflow-y-auto">
            {loading ? (
              <div className="flex items-center justify-center py-8">
                <div className="text-gray-500 dark:text-gray-400">Loading...</div>
              </div>
            ) : notifications.length === 0 ? (
              <div className="flex flex-col items-center justify-center py-8 text-gray-500 dark:text-gray-400">
                <Bell size={24} className="mb-2 opacity-50" />
                <span>No notifications</span>
              </div>
            ) : (
              notifications.map(notification => (
                <div
                  key={notification.id}
                  className={`px-4 py-3 border-b border-gray-100 dark:border-gray-700 last:border-b-0 hover:bg-gray-50 dark:hover:bg-gray-700/50 transition-colors ${
                    !notification.read ? 'bg-blue-50 dark:bg-blue-900/20' : ''
                  }`}
                >
                  <div className="flex items-start gap-3">
                    <div className="mt-0.5">
                      {getSeverityIcon(notification.severity)}
                    </div>
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center justify-between gap-2">
                        <h4 className={`text-sm font-medium truncate ${
                          !notification.read
                            ? 'text-gray-900 dark:text-white'
                            : 'text-gray-700 dark:text-gray-300'
                        }`}>
                          {notification.title}
                        </h4>
                        <div className="flex items-center gap-1 flex-shrink-0">
                          {!notification.read && (
                            <button
                              onClick={(e) => handleMarkAsRead(notification.id, e)}
                              className="p-1 text-gray-400 hover:text-green-500 transition-colors"
                              title="Mark as read"
                            >
                              <Check size={14} />
                            </button>
                          )}
                          <button
                            onClick={(e) => handleDelete(notification.id, e)}
                            className="p-1 text-gray-400 hover:text-red-500 transition-colors"
                            title="Delete"
                          >
                            <Trash2 size={14} />
                          </button>
                        </div>
                      </div>
                      <p className="text-xs text-gray-500 dark:text-gray-400 mt-1 line-clamp-2">
                        {notification.message}
                      </p>
                      <span className="text-xs text-gray-400 dark:text-gray-500 mt-1 block">
                        {formatTime(notification.created_at)}
                      </span>
                    </div>
                  </div>
                </div>
              ))
            )}
          </div>
          <div className="px-4 py-2 border-t border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-800/50">
            <Link
              to="/account"
              className="text-xs text-blue-600 dark:text-blue-400 hover:underline"
              onClick={() => setDropdownOpen(false)}
            >
              Notification preferences
            </Link>
          </div>
        </div>
    </div>
  )
}

function UserMenu() {
  const { user, loading, logout, otherAccounts, switchAccount } = useAuth()
  const [dropdownOpen, setDropdownOpen] = useState(false)
  const location = useLocation()
  const userMenuRef = useRef<HTMLDivElement>(null)

  // Close on click outside
  useEffect(() => {
    if (!dropdownOpen) return
    const handler = (e: MouseEvent) => {
      if (userMenuRef.current && !userMenuRef.current.contains(e.target as Node)) {
        setDropdownOpen(false)
      }
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [dropdownOpen])

  const handleLogout = async () => {
    try {
      await sessionLogout()
    } catch (e) {
      // Ignore errors - cookie will be cleared anyway
    }
    logout()
    window.location.href = '/login'
  }

  const handleSwitchAccount = async (slot: number) => {
    try {
      await switchAccount(slot)
    } catch (e) {
      console.error('Failed to switch account:', e)
    }
  }

  const handleAddAccount = () => {
    window.location.href = '/login?add_account=true'
  }

  if (loading || !user) return null

  const isAccountActive = location.pathname === '/account'

  return (
    <div
      ref={userMenuRef}
      className="relative"
    >
      <button
        onClick={() => setDropdownOpen(!dropdownOpen)}
        className={`flex items-center gap-2 px-3 py-2 rounded-md text-sm font-medium transition-colors ${
          isAccountActive
            ? 'bg-gray-200 dark:bg-gray-700 text-gray-900 dark:text-white'
            : 'text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700'
        }`}
      >
        <User size={18} strokeWidth={2} />
        <span>{user.username}</span>
        <ChevronDown size={16} strokeWidth={2} className={`transition-transform ${dropdownOpen ? 'rotate-180' : ''}`} />
      </button>
      <div className={`absolute top-full right-0 pt-1 w-56 z-50 transition-all duration-200 ease-out origin-top-right ${
        dropdownOpen
          ? 'opacity-100 scale-100 translate-y-0 pointer-events-auto'
          : 'opacity-0 scale-95 -translate-y-1 pointer-events-none'
      }`}>
          <div className="bg-white dark:bg-gray-800 border border-gray-200/60 dark:border-gray-700/60 rounded-lg shadow-[0_8px_24px_rgba(0,0,0,0.1)] dark:shadow-[0_8px_24px_rgba(0,0,0,0.4)]">
            {/* Current account indicator */}
            <div className="px-4 py-2 border-b border-gray-200 dark:border-gray-700">
              <div className="text-xs text-gray-500 dark:text-gray-400">Signed in as</div>
              <div className="font-medium text-gray-900 dark:text-white truncate">{user.username}</div>
            </div>

            {/* Other accounts */}
            {otherAccounts.length > 0 && (
              <div className="border-b border-gray-200 dark:border-gray-700">
                <div className="px-4 py-1.5 text-xs text-gray-500 dark:text-gray-400">Switch account</div>
                {otherAccounts.map((account) => (
                  <button
                    key={account.slot}
                    onClick={() => handleSwitchAccount(account.slot)}
                    className="flex items-center gap-2 w-full px-4 py-2 text-sm text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700"
                  >
                    <User size={14} strokeWidth={2} />
                    <span className="truncate">{account.username}</span>
                  </button>
                ))}
              </div>
            )}

            {/* Add account */}
            <button
              onClick={handleAddAccount}
              className="flex items-center gap-2 w-full px-4 py-2 text-sm text-blue-600 dark:text-blue-400 hover:bg-gray-100 dark:hover:bg-gray-700"
            >
              <User size={16} strokeWidth={2} />
              <span>Add another account</span>
            </button>

            <Link
              to="/account"
              className="flex items-center gap-2 px-4 py-2 text-sm text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700"
            >
              <Settings size={16} strokeWidth={2} />
              <span>Account Settings</span>
            </Link>
            <button
              onClick={handleLogout}
              className="flex items-center gap-2 w-full px-4 py-2 text-sm text-red-600 dark:text-red-400 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-b-md"
            >
              <LogOut size={16} strokeWidth={2} />
              <span>Logout</span>
            </button>
          </div>
        </div>
    </div>
  )
}

function formatNavBandwidth(bytesPerSec: number): string {
  if (bytesPerSec === 0) return '0 B/s'
  const k = 1024
  const sizes = ['B/s', 'KB/s', 'MB/s', 'GB/s']
  const i = Math.floor(Math.log(bytesPerSec) / Math.log(k))
  return parseFloat((bytesPerSec / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i]
}

function Navigation() {
  const { hasPermission, logout } = useAuth()
  const location = useLocation()
  const [systemDropdownOpen, setSystemDropdownOpen] = useState(false)
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false)
  const [mobileSystemOpen, setMobileSystemOpen] = useState(false)
  const { clusterMetrics } = useMonitoring()

  const isActive = (path: string) => location.pathname === path

  // Click outside for system dropdown
  const systemDropdownRef = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    if (!systemDropdownOpen) return
    const handler = (e: MouseEvent) => {
      if (systemDropdownRef.current && !systemDropdownRef.current.contains(e.target as Node)) {
        setSystemDropdownOpen(false)
      }
    }
    document.addEventListener('mousedown', handler)
    return () => document.removeEventListener('mousedown', handler)
  }, [systemDropdownOpen])

  // Sliding indicator
  const navContainerRef = useRef<HTMLDivElement>(null)
  const navItemRefs = useRef<Record<string, HTMLElement | null>>({})
  const [indicator, setIndicator] = useState<{ left: number; width: number } | null>(null)

  const setNavRef = useCallback((path: string) => (el: HTMLElement | null) => {
    navItemRefs.current[path] = el
  }, [])

  // Determine which path is active for the indicator
  const activePath = ['/', '/apps', '/settings'].find(p => isActive(p))
    || (['/storage', '/logs', '/resources', '/networking', '/security'].includes(location.pathname) ? '__system' : null)

  useLayoutEffect(() => {
    if (!activePath || !navContainerRef.current) {
      setIndicator(null)
      return
    }
    const el = navItemRefs.current[activePath]
    if (!el) { setIndicator(null); return }
    const containerRect = navContainerRef.current.getBoundingClientRect()
    const elRect = el.getBoundingClientRect()
    setIndicator({
      left: elRect.left - containerRect.left,
      width: elRect.width,
    })
  }, [activePath, location.pathname])

  const handleLogout = async () => {
    setMobileMenuOpen(false)
    try {
      await sessionLogout()
    } catch (e) {
      // Ignore errors
    }
    logout()
    window.location.href = '/login'
  }

  // Check which system menu items are visible
  const canViewResources = hasPermission('monitoring.view')
  const canViewStorage = hasPermission('storage.view')
  const canViewLogs = hasPermission('logs.view')
  const canViewApps = hasPermission('apps.view')
  const canViewSecurity = hasPermission('audit.view')

  // Show System dropdown if user has any system permission
  const hasAnySystemPermission = canViewResources || canViewStorage || canViewLogs || canViewSecurity
  const isSystemActive = ['/storage', '/logs', '/resources', '/networking', '/security'].includes(location.pathname)

  // Show Settings if user has any settings/users/roles permission
  const canViewSettings = hasPermission('settings.view') || hasPermission('users.view') || hasPermission('roles.view')

  // Close mobile menu on navigation
  useEffect(() => {
    setMobileMenuOpen(false)
    setMobileSystemOpen(false)
  }, [location.pathname])

  return (
    <nav className="sticky top-0 z-40 bg-white/60 dark:bg-gray-900/60 backdrop-blur-xl border-b border-gray-200/60 dark:border-gray-700/60 shadow-[0_1px_3px_rgba(0,0,0,0.05)] dark:shadow-[0_1px_3px_rgba(0,0,0,0.3),inset_0_1px_0_rgba(255,255,255,0.05)]">
      <div className="w-full px-4 sm:px-6 lg:px-8 xl:px-12 2xl:px-16">
        <div className="flex items-center justify-between h-16">
          {/* Logo + Metrics */}
          <div className="flex items-center gap-5">
            <Link to="/" className="flex items-center space-x-2 text-xl font-bold text-gray-900 dark:text-white hover:text-gray-600 dark:hover:text-gray-300 transition-colors">
              <Ship size={28} className="text-blue-500 dark:text-blue-400" strokeWidth={2} />
              <span>Kubarr</span>
            </Link>
            {clusterMetrics ? (
              <Link to="/resources" className="hidden lg:grid grid-cols-[auto_auto] gap-x-2 gap-y-0 text-[10px] leading-[14px] pl-5 border-l border-gray-200/60 dark:border-gray-700/60 w-fit" title="View resources">
                <span className="text-blue-500 dark:text-blue-400 font-medium tabular-nums">CPU {clusterMetrics.cpu_usage_percent.toFixed(1)}%</span>
                <span className="text-green-500 dark:text-green-400 font-medium tabular-nums">NET ↓ {formatNavBandwidth(clusterMetrics.network_receive_bytes_per_sec)}</span>
                <span className="text-purple-500 dark:text-purple-400 font-medium tabular-nums">RAM {clusterMetrics.memory_usage_percent.toFixed(1)}%</span>
                <span className="text-orange-500 dark:text-orange-400 font-medium tabular-nums">NET ↑ {formatNavBandwidth(clusterMetrics.network_transmit_bytes_per_sec)}</span>
              </Link>
            ) : (
              <div className="hidden lg:grid grid-cols-[auto_auto] gap-x-2 gap-y-0.5 text-[10px] leading-[14px] pl-5 border-l border-gray-200/60 dark:border-gray-700/60 w-fit animate-pulse">
                <div className="h-3 w-14 bg-gray-200 dark:bg-gray-700 rounded" />
                <div className="h-3 w-16 bg-gray-200 dark:bg-gray-700 rounded" />
                <div className="h-3 w-14 bg-gray-200 dark:bg-gray-700 rounded" />
                <div className="h-3 w-16 bg-gray-200 dark:bg-gray-700 rounded" />
              </div>
            )}
          </div>

          {/* Desktop Navigation */}
          <div ref={navContainerRef} className="hidden md:flex items-center space-x-1 relative">
            {/* Sliding indicator */}
            {indicator && (
              <div
                className="absolute top-1/2 -translate-y-1/2 h-9 rounded-lg bg-blue-500/10 dark:bg-blue-500/15 transition-all duration-150 ease-out pointer-events-none"
                style={{ left: indicator.left, width: indicator.width }}
              />
            )}
            <Link
              ref={setNavRef('/')}
              to="/"
              className={`relative flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium transition-colors duration-200 ${
                isActive('/')
                  ? 'text-blue-700 dark:text-blue-300'
                  : 'text-gray-600 dark:text-gray-300 hover:bg-gray-100/80 dark:hover:bg-gray-700/60 hover:text-gray-900 dark:hover:text-white'
              }`}
            >
              <Ship size={18} strokeWidth={2} />
              <span>Dashboard</span>
            </Link>
            {canViewApps && (
              <Link
                ref={setNavRef('/apps')}
                to="/apps"
                className={`relative flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium transition-colors duration-200 ${
                  isActive('/apps')
                    ? 'text-blue-700 dark:text-blue-300'
                    : 'text-gray-600 dark:text-gray-300 hover:bg-gray-100/80 dark:hover:bg-gray-700/60 hover:text-gray-900 dark:hover:text-white'
                }`}
              >
                <Grid3X3 size={18} strokeWidth={2} />
                <span>Apps</span>
              </Link>
            )}
            {hasAnySystemPermission && (
              <div
                ref={(el) => { systemDropdownRef.current = el; setNavRef('__system')(el) }}
                className="relative"
              >
                <button
                  onClick={() => setSystemDropdownOpen(!systemDropdownOpen)}
                  className={`flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium transition-colors duration-200 ${
                    isSystemActive
                      ? 'text-blue-700 dark:text-blue-300'
                      : 'text-gray-600 dark:text-gray-300 hover:bg-gray-100/80 dark:hover:bg-gray-700/60 hover:text-gray-900 dark:hover:text-white'
                  }`}
                >
                  <Activity size={18} strokeWidth={2} />
                  <span>Status</span>
                  <ChevronDown size={16} strokeWidth={2} className={`transition-transform ${systemDropdownOpen ? 'rotate-180' : ''}`} />
                </button>
                <div className={`absolute top-full left-0 pt-1 w-48 z-50 transition-all duration-200 ease-out origin-top ${
                  systemDropdownOpen
                    ? 'opacity-100 scale-100 translate-y-0 pointer-events-auto'
                    : 'opacity-0 scale-95 -translate-y-1 pointer-events-none'
                }`}>
                    <div className="bg-white dark:bg-gray-800 border border-gray-200/60 dark:border-gray-700/60 rounded-lg shadow-[0_8px_24px_rgba(0,0,0,0.1)] dark:shadow-[0_8px_24px_rgba(0,0,0,0.4)]">
                    {canViewResources && (
                      <Link
                        to="/resources"
                        className={`flex items-center gap-2 px-4 py-2 text-sm transition-colors ${
                          isActive('/resources')
                            ? 'bg-gray-100 dark:bg-gray-700 text-gray-900 dark:text-white'
                            : 'text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 hover:text-gray-900 dark:hover:text-white'
                        } first:rounded-t-md last:rounded-b-md`}
                      >
                        <Activity size={16} strokeWidth={2} />
                        <span>Resources</span>
                      </Link>
                    )}
                    {canViewResources && (
                      <Link
                        to="/networking"
                        className={`flex items-center gap-2 px-4 py-2 text-sm transition-colors ${
                          isActive('/networking')
                            ? 'bg-gray-100 dark:bg-gray-700 text-gray-900 dark:text-white'
                            : 'text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 hover:text-gray-900 dark:hover:text-white'
                        } first:rounded-t-md last:rounded-b-md`}
                      >
                        <Network size={16} strokeWidth={2} />
                        <span>Networking</span>
                      </Link>
                    )}
                    {canViewStorage && (
                      <Link
                        to="/storage"
                        className={`flex items-center gap-2 px-4 py-2 text-sm transition-colors ${
                          isActive('/storage')
                            ? 'bg-gray-100 dark:bg-gray-700 text-gray-900 dark:text-white'
                            : 'text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 hover:text-gray-900 dark:hover:text-white'
                        } first:rounded-t-md last:rounded-b-md`}
                      >
                        <HardDrive size={16} strokeWidth={2} />
                        <span>Storage</span>
                      </Link>
                    )}
                    {canViewLogs && (
                      <Link
                        to="/logs"
                        className={`flex items-center gap-2 px-4 py-2 text-sm transition-colors ${
                          isActive('/logs')
                            ? 'bg-gray-100 dark:bg-gray-700 text-gray-900 dark:text-white'
                            : 'text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 hover:text-gray-900 dark:hover:text-white'
                        } first:rounded-t-md last:rounded-b-md`}
                      >
                        <FileText size={16} strokeWidth={2} />
                        <span>Logs</span>
                      </Link>
                    )}
                    {canViewSecurity && (
                      <Link
                        to="/security"
                        className={`flex items-center gap-2 px-4 py-2 text-sm transition-colors ${
                          isActive('/security')
                            ? 'bg-gray-100 dark:bg-gray-700 text-gray-900 dark:text-white'
                            : 'text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 hover:text-gray-900 dark:hover:text-white'
                        } first:rounded-t-md last:rounded-b-md`}
                      >
                        <Shield size={16} strokeWidth={2} />
                        <span>Security</span>
                      </Link>
                    )}
                    </div>
                  </div>
              </div>
            )}
            {canViewSettings && (
              <Link
                ref={setNavRef('/settings')}
                to="/settings"
                className={`relative flex items-center gap-2 px-4 py-2 rounded-lg text-sm font-medium transition-colors duration-200 ${
                  isActive('/settings')
                    ? 'text-blue-700 dark:text-blue-300'
                    : 'text-gray-600 dark:text-gray-300 hover:bg-gray-100/80 dark:hover:bg-gray-700/60 hover:text-gray-900 dark:hover:text-white'
                }`}
              >
                <Settings size={18} strokeWidth={2} />
                <span>Settings</span>
              </Link>
            )}
            <div className="flex items-center gap-2 ml-4 border-l border-gray-200 dark:border-gray-700 pl-4">
              <NotificationInbox />
              <UserMenu />
              <ThemeToggle />
            </div>
          </div>

          {/* Mobile: Theme toggle and hamburger menu */}
          <div className="flex md:hidden items-center gap-2">
            <ThemeToggle />
            <button
              onClick={() => setMobileMenuOpen(!mobileMenuOpen)}
              className="flex items-center justify-center w-10 h-10 rounded-md text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors"
              aria-label="Toggle menu"
            >
              {mobileMenuOpen ? <X size={24} strokeWidth={2} /> : <Menu size={24} strokeWidth={2} />}
            </button>
          </div>
        </div>
      </div>

      {/* Mobile Menu Overlay */}
      {mobileMenuOpen && (
        <div className="md:hidden border-t border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800">
          <div className="px-4 py-3 space-y-1">
            {/* Dashboard link */}
            <Link
              to="/"
              className={`flex items-center gap-3 px-4 py-3 rounded-md text-base font-medium transition-colors ${
                isActive('/')
                  ? 'bg-gray-200 dark:bg-gray-700 text-gray-900 dark:text-white'
                  : 'text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700'
              }`}
            >
              <Ship size={20} strokeWidth={2} />
              <span>Dashboard</span>
            </Link>

            {canViewApps && (
              <Link
                to="/apps"
                className={`flex items-center gap-3 px-4 py-3 rounded-md text-base font-medium transition-colors ${
                  isActive('/apps')
                    ? 'bg-gray-200 dark:bg-gray-700 text-gray-900 dark:text-white'
                    : 'text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700'
                }`}
              >
                <Grid3X3 size={20} strokeWidth={2} />
                <span>Apps</span>
              </Link>
            )}

            {/* Status submenu */}
            {hasAnySystemPermission && (
              <div>
                <button
                  onClick={() => setMobileSystemOpen(!mobileSystemOpen)}
                  className={`flex items-center justify-between w-full px-4 py-3 rounded-md text-base font-medium transition-colors ${
                    isSystemActive
                      ? 'bg-gray-200 dark:bg-gray-700 text-gray-900 dark:text-white'
                      : 'text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700'
                  }`}
                >
                  <div className="flex items-center gap-3">
                    <Activity size={20} strokeWidth={2} />
                    <span>Status</span>
                  </div>
                  <ChevronDown size={20} strokeWidth={2} className={`transition-transform ${mobileSystemOpen ? 'rotate-180' : ''}`} />
                </button>
                {mobileSystemOpen && (
                  <div className="ml-6 mt-1 space-y-1">
                    {canViewResources && (
                      <Link
                        to="/resources"
                        className={`flex items-center gap-3 px-4 py-3 rounded-md text-base transition-colors ${
                          isActive('/resources')
                            ? 'bg-gray-100 dark:bg-gray-700 text-gray-900 dark:text-white'
                            : 'text-gray-500 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700'
                        }`}
                      >
                        <Activity size={18} strokeWidth={2} />
                        <span>Resources</span>
                      </Link>
                    )}
                    {canViewResources && (
                      <Link
                        to="/networking"
                        className={`flex items-center gap-3 px-4 py-3 rounded-md text-base transition-colors ${
                          isActive('/networking')
                            ? 'bg-gray-100 dark:bg-gray-700 text-gray-900 dark:text-white'
                            : 'text-gray-500 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700'
                        }`}
                      >
                        <Network size={18} strokeWidth={2} />
                        <span>Networking</span>
                      </Link>
                    )}
                    {canViewStorage && (
                      <Link
                        to="/storage"
                        className={`flex items-center gap-3 px-4 py-3 rounded-md text-base transition-colors ${
                          isActive('/storage')
                            ? 'bg-gray-100 dark:bg-gray-700 text-gray-900 dark:text-white'
                            : 'text-gray-500 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700'
                        }`}
                      >
                        <HardDrive size={18} strokeWidth={2} />
                        <span>Storage</span>
                      </Link>
                    )}
                    {canViewLogs && (
                      <Link
                        to="/logs"
                        className={`flex items-center gap-3 px-4 py-3 rounded-md text-base transition-colors ${
                          isActive('/logs')
                            ? 'bg-gray-100 dark:bg-gray-700 text-gray-900 dark:text-white'
                            : 'text-gray-500 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700'
                        }`}
                      >
                        <FileText size={18} strokeWidth={2} />
                        <span>Logs</span>
                      </Link>
                    )}
                    {canViewSecurity && (
                      <Link
                        to="/security"
                        className={`flex items-center gap-3 px-4 py-3 rounded-md text-base transition-colors ${
                          isActive('/security')
                            ? 'bg-gray-100 dark:bg-gray-700 text-gray-900 dark:text-white'
                            : 'text-gray-500 dark:text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700'
                        }`}
                      >
                        <Shield size={18} strokeWidth={2} />
                        <span>Security</span>
                      </Link>
                    )}
                  </div>
                )}
              </div>
            )}

            {canViewSettings && (
              <Link
                to="/settings"
                className={`flex items-center gap-3 px-4 py-3 rounded-md text-base font-medium transition-colors ${
                  isActive('/settings')
                    ? 'bg-gray-200 dark:bg-gray-700 text-gray-900 dark:text-white'
                    : 'text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700'
                }`}
              >
                <Settings size={20} strokeWidth={2} />
                <span>Settings</span>
              </Link>
            )}

            {/* Divider */}
            <div className="border-t border-gray-200 dark:border-gray-700 my-2" />

            {/* Account link */}
            <Link
              to="/account"
              className={`flex items-center gap-3 px-4 py-3 rounded-md text-base font-medium transition-colors ${
                isActive('/account')
                  ? 'bg-gray-200 dark:bg-gray-700 text-gray-900 dark:text-white'
                  : 'text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-700'
              }`}
            >
              <User size={20} strokeWidth={2} />
              <span>Account</span>
            </Link>

            {/* Logout button */}
            <button
              onClick={handleLogout}
              className="flex items-center gap-3 w-full px-4 py-3 rounded-md text-base font-medium text-red-600 dark:text-red-400 hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors"
            >
              <LogOut size={20} strokeWidth={2} />
              <span>Logout</span>
            </button>
          </div>
        </div>
      )}
    </nav>
  )
}

function ProtectedRoute({ children }: { children: React.ReactNode }) {
  const { isAuthenticated, loading } = useAuth()

  if (loading) {
    return (
      <div className="h-screen bg-gray-50 dark:bg-gray-900 text-gray-900 dark:text-white flex items-center justify-center">
        <div className="text-xl">Loading...</div>
      </div>
    )
  }

  // If not authenticated, redirect to login page
  if (!isAuthenticated) {
    window.location.href = '/login'
    return null
  }

  return <>{children}</>
}

function AppContent() {
  const location = useLocation()
  const [setupRequired, setSetupRequired] = useState<boolean | null>(null)
  const [databasePending, setDatabasePending] = useState(false)
  const [setupCheckLoading, setSetupCheckLoading] = useState(true)

  const isSettingsPage = location.pathname === '/settings'
  const isAccountPage = location.pathname === '/account'
  const isLogsPage = location.pathname === '/logs'
  const isSetupPage = location.pathname === '/setup'
  const isLoginPage = location.pathname === '/login'

  // Check if setup is required on mount, with polling when database is pending
  useEffect(() => {
    let cancelled = false
    let timer: ReturnType<typeof setTimeout> | null = null

    const checkSetup = async () => {
      try {
        const { setup_required, database_pending } = await setupApi.checkRequired()
        if (cancelled) return

        if (database_pending) {
          setDatabasePending(true)
          setSetupRequired(false)
          setSetupCheckLoading(false)
          // Retry in 3 seconds
          timer = setTimeout(checkSetup, 3000)
        } else {
          setDatabasePending(false)
          setSetupRequired(setup_required)
          setSetupCheckLoading(false)
        }
      } catch (err) {
        if (cancelled) return
        // If we can't check, assume setup is not required
        setSetupRequired(false)
        setSetupCheckLoading(false)
      }
    }
    checkSetup()

    return () => {
      cancelled = true
      if (timer) clearTimeout(timer)
    }
  }, [])

  // Show loading while checking setup
  if (setupCheckLoading) {
    return (
      <div className="h-screen bg-gray-50 dark:bg-gray-900 text-gray-900 dark:text-white flex items-center justify-center">
        <div className="text-xl">Loading...</div>
      </div>
    )
  }

  // Show waiting screen while database is coming up
  if (databasePending) {
    return (
      <div className="h-screen bg-gray-50 dark:bg-gray-900 text-gray-900 dark:text-white flex items-center justify-center">
        <div className="text-center">
          <div className="text-xl font-semibold mb-2">Waiting for database...</div>
          <div className="text-sm text-gray-500 dark:text-gray-400">PostgreSQL is starting up. This page will refresh automatically.</div>
        </div>
      </div>
    )
  }

  // Render login page without navigation and without protection
  if (isLoginPage) {
    return (
      <Routes>
        <Route path="/login" element={<LoginPage />} />
      </Routes>
    )
  }

  // Redirect to setup if required and not already on setup page
  if (setupRequired && !isSetupPage) {
    return <Navigate to="/setup" replace />
  }

  // Render setup page without navigation
  if (isSetupPage) {
    return (
      <Routes>
        <Route path="/setup" element={<SetupPage />} />
      </Routes>
    )
  }

  return (
    <ProtectedRoute>
      <MonitoringProvider>
      <div className="h-screen bg-gray-50 dark:bg-gray-900 text-gray-900 dark:text-white overflow-auto">
        <Navigation />
        {isSettingsPage || isAccountPage || isLogsPage ? (
          <main className="p-6" style={{ minHeight: 'calc(100vh - 4rem)' }}>
            <PageTransition className="h-full">
              <Routes>
                <Route path="/settings" element={<SettingsPage />} />
                <Route path="/account" element={<AccountPage />} />
                <Route path="/logs" element={<LogsPage />} />
                <Route path="*" element={<NotFoundPage />} />
              </Routes>
            </PageTransition>
          </main>
        ) : (
          <main className="pb-10">
            <PageTransition>
              <div className="w-full px-4 sm:px-6 lg:px-8 xl:px-12 2xl:px-16 py-8">
                <Routes>
                  <Route path="/" element={<Dashboard />} />
                  <Route path="/apps" element={<AppsPage />} />
                  <Route path="/storage" element={<StoragePage />} />
                  <Route path="/resources" element={<MonitoringPage />} />
                  <Route path="/networking" element={<NetworkingPage />} />
                  <Route path="/security" element={<SecurityPage />} />
                  <Route path="/app-error" element={<AppErrorPage />} />
                  <Route path="*" element={<NotFoundPage />} />
                </Routes>
              </div>
            </PageTransition>
            <VersionFooter />
          </main>
        )}
      </div>
      </MonitoringProvider>
    </ProtectedRoute>
  )
}

function App() {
  return (
    <BrowserRouter>
      <AuthProvider>
        <ThemeProvider>
          <AppContent />
        </ThemeProvider>
      </AuthProvider>
    </BrowserRouter>
  )
}

export default App
