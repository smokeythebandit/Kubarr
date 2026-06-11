import { useState, useMemo, useEffect, type JSX } from 'react'
import { useSearchParams } from 'react-router-dom'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { appsApi } from '../api/apps'
import { vpnApi, appVpnApi } from '../api/vpn'
import type { VpnProvider } from '../api/vpn'
import { VpnProviderForm } from '../components/vpn/VpnProviderForm'
import { AppIcon, useIconColors } from '../components/AppIcon'
import { useAuth } from '../contexts/AuthContext'
import { useMonitoring } from '../contexts/MonitoringContext'
import type { AppConfig } from '../types'

type FilterType = 'all' | 'installed' | 'healthy' | 'unhealthy' | 'available'

type OperationState = 'installing' | 'deleting' | 'error'

interface OperationStatus {
  state: OperationState
  message?: string
}

// Helper to convert rgb to rgba
function toRgba(rgb: string, alpha: number): string {
  return rgb.replace('rgb', 'rgba').replace(')', `, ${alpha})`)
}

// Category metadata for display
const categoryInfo: Record<string, { label: string; icon: JSX.Element; description: string }> = {
  'media-manager': {
    label: 'Media Managers',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M7 4v16M17 4v16M3 8h4m10 0h4M3 12h18M3 16h4m10 0h4M4 20h16a1 1 0 001-1V5a1 1 0 00-1-1H4a1 1 0 00-1 1v14a1 1 0 001 1z" />
      </svg>
    ),
    description: 'Organize and manage your movie and TV show collections'
  },
  'download-client': {
    label: 'Download Clients',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
      </svg>
    ),
    description: 'BitTorrent and Usenet clients for downloading content'
  },
  'media-server': {
    label: 'Media Servers',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2m-2-4h.01M17 16h.01" />
      </svg>
    ),
    description: 'Stream your media library to any device'
  },
  'request-manager': {
    label: 'Request Managers',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M8.228 9c.549-1.165 2.03-2 3.772-2 2.21 0 4 1.343 4 3 0 1.4-1.278 2.575-3.006 2.907-.542.104-.994.54-.994 1.093m0 3h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
      </svg>
    ),
    description: 'Allow users to request new content'
  },
  'indexer': {
    label: 'Indexers',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
      </svg>
    ),
    description: 'Search and index content from various sources'
  },
  'monitoring': {
    label: 'Monitoring',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z" />
      </svg>
    ),
    description: 'Metrics, logs, and dashboards'
  },
  'system': {
    label: 'System',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
      </svg>
    ),
    description: 'Core system services'
  }
}

// Default category info for unknown categories
const defaultCategoryInfo = {
  label: 'Other Apps',
  icon: (
    <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2H6a2 2 0 01-2-2V6zM14 6a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2h-2a2 2 0 01-2-2V6zM4 16a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2H6a2 2 0 01-2-2v-2zM14 16a2 2 0 012-2h2a2 2 0 012 2v2a2 2 0 01-2 2h-2a2 2 0 01-2-2v-2z" />
    </svg>
  ),
  description: 'Additional applications'
}

// Category display order
const categoryOrder = ['media-manager', 'download-client', 'media-server', 'request-manager', 'indexer', 'monitoring', 'system']

// App Card Component with glass effect
interface AppCardComponentProps {
  app: AppConfig
  isInstalled: boolean
  isHealthy: boolean
  effectiveState: string
  isSelected: boolean
  onInstall: () => void
  onDelete: () => void
  onOpen: () => void
  onClick: () => void
  isOperationPending: boolean
}

function AppCardComponent({
  app,
  isInstalled,
  isHealthy,
  effectiveState,
  isSelected,
  onInstall,
  onDelete,
  onOpen,
  onClick,
  isOperationPending
}: AppCardComponentProps) {
  const colors = useIconColors(app.name)

  // Create iOS-style glass effect with multiple color gradients
  const glassStyle: React.CSSProperties = {}

  if (colors.length >= 3) {
    glassStyle.background = `
      radial-gradient(ellipse at 0% 0%, ${toRgba(colors[0], 0.15)} 0%, transparent 50%),
      radial-gradient(ellipse at 100% 0%, ${toRgba(colors[1], 0.12)} 0%, transparent 50%),
      radial-gradient(ellipse at 50% 100%, ${toRgba(colors[2], 0.1)} 0%, transparent 60%)
    `
  } else if (colors.length === 2) {
    glassStyle.background = `
      radial-gradient(ellipse at 0% 0%, ${toRgba(colors[0], 0.15)} 0%, transparent 50%),
      radial-gradient(ellipse at 100% 100%, ${toRgba(colors[1], 0.12)} 0%, transparent 50%)
    `
  } else if (colors.length === 1) {
    glassStyle.background = `
      radial-gradient(ellipse at 0% 0%, ${toRgba(colors[0], 0.12)} 0%, transparent 50%),
      radial-gradient(ellipse at 100% 100%, ${toRgba(colors[0], 0.08)} 0%, transparent 50%)
    `
  }

  const primaryColor = colors[0]
  const baseShadow = primaryColor
    ? `0 2px 8px ${toRgba(primaryColor, 0.1)}`
    : undefined
  const hoverShadow = primaryColor
    ? `0 8px 24px ${toRgba(primaryColor, 0.2)}, 0 0 0 1px ${toRgba(primaryColor, 0.15)}`
    : undefined

  const categoryLabel = categoryInfo[app.category || 'other']?.label || app.category?.replace(/-/g, ' ').replace(/\b\w/g, l => l.toUpperCase())

  const selectedShadow = primaryColor
    ? `0 8px 24px ${toRgba(primaryColor, 0.25)}, 0 0 0 2px ${toRgba(primaryColor, 0.3)}`
    : '0 8px 24px rgba(59,130,246,0.15), 0 0 0 2px rgba(59,130,246,0.2)'

  return (
    <div
      className={`group relative bg-white dark:bg-gray-800/90 rounded-xl border backdrop-blur-sm hover:-translate-y-1 transition-all duration-200 overflow-hidden cursor-pointer ${
        isSelected
          ? 'border-blue-400/60 dark:border-blue-500/40 -translate-y-1'
          : 'border-gray-200/60 dark:border-gray-700/60'
      }`}
      style={{
        ...glassStyle,
        boxShadow: isSelected ? selectedShadow : baseShadow,
      }}
      onMouseEnter={(e) => {
        if (!isSelected && hoverShadow) {
          e.currentTarget.style.boxShadow = hoverShadow
        }
      }}
      onMouseLeave={(e) => {
        if (!isSelected) {
          e.currentTarget.style.boxShadow = baseShadow || ''
        }
      }}
      onClick={onClick}
    >
      {/* Status indicator bar at top */}
      {(isInstalled || app.is_system) && (
        <div
          className={`h-1 w-full ${
            effectiveState === 'installing' || effectiveState === 'deleting'
              ? 'bg-gradient-to-r from-blue-400 via-blue-500 to-blue-400 animate-pulse'
              : effectiveState === 'error'
              ? 'bg-red-500'
              : isHealthy
              ? 'bg-green-500'
              : 'bg-yellow-500'
          }`}
        />
      )}

      <div className="p-5">
        <div className="flex items-start gap-4">
          {/* Icon with glow effect */}
          <div className="relative flex-shrink-0">
            <AppIcon appName={app.name} size={56} className="rounded-xl shadow-lg" />
            {primaryColor && (
              <div
                className="absolute inset-0 rounded-xl opacity-0 group-hover:opacity-100 transition-opacity duration-300 -z-10 blur-xl"
                style={{ background: toRgba(primaryColor, 0.4) }}
              />
            )}
          </div>

          <div className="flex-1 min-w-0">
            <div className="flex items-start justify-between gap-2">
              <div>
                <h3 className="text-lg font-semibold text-gray-900 dark:text-white truncate group-hover:text-gray-700 dark:group-hover:text-gray-100 transition-colors">
                  {app.display_name}
                </h3>
                <span className="text-xs text-gray-500 dark:text-gray-400">{categoryLabel}</span>
              </div>

              {/* Status badges */}
              <div className="flex items-center gap-1 flex-shrink-0">
                {app.is_system && (
                  <span className="inline-flex items-center gap-1 bg-purple-500/20 text-purple-500 dark:text-purple-400 text-xs px-2 py-0.5 rounded-full">
                    <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
                    </svg>
                    System
                  </span>
                )}
                {!app.is_system && effectiveState === 'installed' && (
                  <span className={`inline-flex items-center gap-1 text-xs px-2 py-0.5 rounded-full ${
                    isHealthy
                      ? 'bg-green-500/20 text-green-600 dark:text-green-400'
                      : 'bg-yellow-500/20 text-yellow-600 dark:text-yellow-400'
                  }`}>
                    <span className={`w-1.5 h-1.5 rounded-full ${isHealthy ? 'bg-green-500' : 'bg-yellow-500'}`}></span>
                    {isHealthy ? 'Healthy' : 'Unhealthy'}
                  </span>
                )}
                {effectiveState === 'loading' && (
                  <span className="inline-flex items-center gap-1 bg-gray-500/20 text-gray-500 dark:text-gray-400 text-xs px-2 py-0.5 rounded-full animate-pulse">
                    <span className="w-1.5 h-1.5 bg-gray-400 rounded-full"></span>
                    Loading
                  </span>
                )}
                {effectiveState === 'installing' && (
                  <span className="inline-flex items-center gap-1 bg-blue-500/20 text-blue-500 dark:text-blue-400 text-xs px-2 py-0.5 rounded-full animate-pulse">
                    <span className="w-1.5 h-1.5 bg-blue-400 rounded-full"></span>
                    Installing
                  </span>
                )}
                {effectiveState === 'deleting' && (
                  <span className="inline-flex items-center gap-1 bg-red-500/20 text-red-500 dark:text-red-400 text-xs px-2 py-0.5 rounded-full animate-pulse">
                    <span className="w-1.5 h-1.5 bg-red-400 rounded-full"></span>
                    Removing
                  </span>
                )}
                {effectiveState === 'error' && (
                  <span className="inline-flex items-center gap-1 bg-red-500/20 text-red-500 dark:text-red-400 text-xs px-2 py-0.5 rounded-full">
                    <span className="w-1.5 h-1.5 bg-red-400 rounded-full"></span>
                    Error
                  </span>
                )}
              </div>
            </div>

            <p className="text-sm text-gray-600 dark:text-gray-400 mt-2 line-clamp-2">{app.description}</p>
          </div>
        </div>

        {/* Action buttons */}
        <div className="flex gap-2 mt-4" onClick={(e) => e.stopPropagation()}>
          {app.is_system && app.is_hidden ? (
            <div className="w-full bg-gray-100 dark:bg-gray-700/50 text-gray-500 dark:text-gray-400 text-sm font-medium py-2.5 px-4 rounded-lg text-center">
              Background Service
            </div>
          ) : app.is_system ? (
            <button
              onClick={onOpen}
              className="w-full bg-blue-600 hover:bg-blue-500 text-white text-sm font-medium py-2.5 px-4 rounded-lg transition-colors"
            >
              Open
            </button>
          ) : effectiveState === 'loading' ? (
            <button
              disabled
              className="w-full bg-gray-100 dark:bg-gray-700/50 cursor-not-allowed text-gray-500 dark:text-gray-400 text-sm font-medium py-2.5 px-4 rounded-lg"
            >
              Loading...
            </button>
          ) : effectiveState === 'installed' ? (
            <>
              {app.is_browseable && (
                <button
                  onClick={onOpen}
                  className="flex-1 bg-blue-600 hover:bg-blue-500 text-white text-sm font-medium py-2.5 px-4 rounded-lg transition-colors flex items-center justify-center gap-2"
                >
                  <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
                  </svg>
                  Open
                </button>
              )}
              <button
                onClick={onDelete}
                disabled={isOperationPending}
                className={`${!app.is_browseable ? 'w-full' : ''} bg-gray-100 dark:bg-gray-700/50 hover:bg-red-600 disabled:bg-gray-100 dark:disabled:bg-gray-800 disabled:cursor-not-allowed text-gray-600 dark:text-gray-300 hover:text-white text-sm font-medium py-2.5 px-4 rounded-lg transition-colors flex items-center justify-center gap-2`}
                title="Uninstall"
              >
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                </svg>
                {!app.is_browseable && 'Uninstall'}
              </button>
            </>
          ) : effectiveState === 'idle' || effectiveState === 'error' ? (
            <button
              onClick={onInstall}
              className="w-full bg-blue-600 hover:bg-blue-500 text-white text-sm font-medium py-2.5 px-4 rounded-lg transition-colors flex items-center justify-center gap-2"
            >
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
              </svg>
              {effectiveState === 'error' ? 'Retry Install' : 'Install'}
            </button>
          ) : (
            <button
              disabled
              className="w-full bg-gray-100 dark:bg-gray-700/50 cursor-not-allowed text-gray-500 dark:text-gray-400 text-sm font-medium py-2.5 px-4 rounded-lg"
            >
              {effectiveState === 'installing' ? 'Installing...' : 'Removing...'}
            </button>
          )}
        </div>
      </div>
    </div>
  )
}

// App Detail Panel (right sidebar)
interface AppDetailPanelProps {
  app: AppConfig | null
  isInstalled: boolean
  isHealthy: boolean
  effectiveState: string
  onInstall: () => void
  onDelete: () => void
  onOpen: () => void
  isOperationPending: boolean
}

function AppDetailPanel({
  app,
  isInstalled,
  isHealthy,
  effectiveState,
  onInstall,
  onDelete,
  onOpen,
  isOperationPending
}: AppDetailPanelProps) {
  const colors = useIconColors(app?.name || '')
  const { hasPermission } = useAuth()
  const canViewVpn = hasPermission('vpn.view')
  const canManageVpn = hasPermission('vpn.manage')
  const queryClient = useQueryClient()
  const [selectedProviderId, setSelectedProviderId] = useState<number | null>(null)
  const [killSwitchOverride, setKillSwitchOverride] = useState<boolean | null>(null)
  const [portForwarding, setPortForwarding] = useState(false)
  const [showVpnForm, setShowVpnForm] = useState(false)

  // VPN queries (only fetch if user has vpn.view permission)
  const { data: vpnProviders } = useQuery({
    queryKey: ['vpn-providers'],
    queryFn: vpnApi.listProviders,
    staleTime: 30000,
    enabled: canViewVpn,
  })

  const { data: appVpnConfig, isLoading: vpnConfigLoading } = useQuery({
    queryKey: ['app-vpn-config', app?.name],
    queryFn: () => appVpnApi.getConfig(app!.name),
    enabled: canViewVpn && !!app && isInstalled && !app.is_system,
    staleTime: 10000,
  })

  // Query forwarded port when VPN is active with port forwarding enabled
  const { data: forwardedPortData } = useQuery({
    queryKey: ['vpn-forwarded-port', app?.name],
    queryFn: () => appVpnApi.getForwardedPort(app!.name),
    enabled: canViewVpn && !!app && !!appVpnConfig?.port_forwarding,
    refetchInterval: 10000,
    staleTime: 5000,
  })

  // Sync local state when config changes
  useEffect(() => {
    if (appVpnConfig) {
      setSelectedProviderId(appVpnConfig.vpn_provider_id)
      setKillSwitchOverride(appVpnConfig.kill_switch_override)
      setPortForwarding(appVpnConfig.port_forwarding)
    } else {
      setSelectedProviderId(null)
      setKillSwitchOverride(null)
      setPortForwarding(false)
    }
  }, [appVpnConfig])

  const assignVpnMutation = useMutation({
    mutationFn: ({ appName, providerId, killSwitch, portFwd }: { appName: string; providerId: number; killSwitch?: boolean; portFwd?: boolean }) =>
      appVpnApi.assignVpn(appName, { vpn_provider_id: providerId, kill_switch_override: killSwitch, port_forwarding: portFwd }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['app-vpn-config', app?.name] })
    },
  })

  const removeVpnMutation = useMutation({
    mutationFn: (appName: string) => appVpnApi.removeVpn(appName),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['app-vpn-config', app?.name] })
      setSelectedProviderId(null)
      setKillSwitchOverride(null)
      setPortForwarding(false)
    },
  })

  const enabledProviders = useMemo(() => vpnProviders?.filter((p: VpnProvider) => p.enabled) || [], [vpnProviders])

  if (!app) return null

  const primaryColor = colors[0]
  const categoryLabel = categoryInfo[app.category || 'other']?.label || app.category?.replace(/-/g, ' ').replace(/\b\w/g, l => l.toUpperCase())
  const categoryIcon = categoryInfo[app.category || 'other']?.icon || defaultCategoryInfo.icon

  // Create background gradient
  const bgGradient = primaryColor
    ? `radial-gradient(ellipse at 0% 0%, ${toRgba(primaryColor, 0.15)} 0%, transparent 40%),
       radial-gradient(ellipse at 100% 100%, ${toRgba(primaryColor, 0.1)} 0%, transparent 40%)`
    : undefined

  return (
    <div
      className="w-[480px] flex-shrink-0 overflow-y-auto bg-white dark:bg-gray-900 border-l border-gray-200 dark:border-gray-700 shadow-xl"
      style={{ background: bgGradient }}
    >
        {/* Status bar */}
        {(isInstalled || app.is_system) && (
          <div
            className={`h-1.5 w-full ${
              effectiveState === 'installing' || effectiveState === 'deleting'
                ? 'bg-gradient-to-r from-blue-400 via-blue-500 to-blue-400 animate-pulse'
                : effectiveState === 'error'
                ? 'bg-red-500'
                : isHealthy
                ? 'bg-green-500'
                : 'bg-yellow-500'
            }`}
          />
        )}

        {/* Content */}
        <div className="p-6">
          {/* Header */}
          <div className="flex items-start gap-5">
            <div className="relative">
              <AppIcon appName={app.name} size={80} className="rounded-2xl shadow-xl" />
              {primaryColor && (
                <div
                  className="absolute inset-0 rounded-2xl -z-10 blur-2xl opacity-50"
                  style={{ background: primaryColor }}
                />
              )}
            </div>

            <div className="flex-1 min-w-0 pt-1">
              <div className="flex items-start justify-between gap-3">
                <div>
                  <h2 className="text-2xl font-bold text-gray-900 dark:text-white">{app.display_name}</h2>
                  <div className="flex items-center gap-2 mt-1">
                    <span className="inline-flex items-center gap-1.5 text-sm text-gray-500 dark:text-gray-400">
                      {categoryIcon}
                      {categoryLabel}
                    </span>
                  </div>
                </div>

                {/* Action buttons */}
                <div className="flex flex-col gap-2 flex-shrink-0">
                  {app.is_browseable && (isInstalled || app.is_system) && !app.is_hidden && (
                    <button
                      onClick={onOpen}
                      className="bg-blue-600 hover:bg-blue-500 text-white text-sm font-semibold py-2 px-4 rounded-xl transition-colors flex items-center gap-1.5"
                    >
                      <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14" />
                      </svg>
                      Open
                    </button>
                  )}
                  {!app.is_system && isInstalled && effectiveState === 'installed' && (
                    <button
                      onClick={onDelete}
                      disabled={isOperationPending}
                      className="bg-gray-100 dark:bg-gray-800 hover:bg-red-600 disabled:cursor-not-allowed text-gray-600 dark:text-gray-300 hover:text-white text-sm font-semibold py-2 px-4 rounded-xl transition-colors flex items-center gap-1.5"
                    >
                      <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                      </svg>
                      Uninstall
                    </button>
                  )}
                  {!app.is_system && !isInstalled && (effectiveState === 'idle' || effectiveState === 'error') && (
                    <button
                      onClick={onInstall}
                      className="bg-blue-600 hover:bg-blue-500 text-white text-sm font-semibold py-2 px-4 rounded-xl transition-colors flex items-center gap-1.5"
                    >
                      <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
                      </svg>
                      {effectiveState === 'error' ? 'Retry' : 'Install'}
                    </button>
                  )}
                  {(effectiveState === 'installing' || effectiveState === 'deleting') && (
                    <button
                      disabled
                      className="bg-gray-100 dark:bg-gray-800 cursor-not-allowed text-gray-500 dark:text-gray-400 text-sm font-semibold py-2 px-4 rounded-xl flex items-center gap-1.5"
                    >
                      <svg className="w-4 h-4 animate-spin" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                      </svg>
                      {effectiveState === 'installing' ? 'Installing...' : 'Removing...'}
                    </button>
                  )}
                </div>
              </div>

              {/* Status */}
              <div className="flex items-center gap-2 mt-2">
                {app.is_system && (
                  <span className="inline-flex items-center gap-1.5 bg-purple-500/20 text-purple-600 dark:text-purple-400 text-sm px-3 py-1 rounded-full">
                    <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
                    </svg>
                    System App
                  </span>
                )}
                {!app.is_system && isInstalled && (
                  <span className={`inline-flex items-center gap-1.5 text-sm px-3 py-1 rounded-full ${
                    isHealthy
                      ? 'bg-green-500/20 text-green-600 dark:text-green-400'
                      : 'bg-yellow-500/20 text-yellow-600 dark:text-yellow-400'
                  }`}>
                    <span className={`w-2 h-2 rounded-full ${isHealthy ? 'bg-green-500' : 'bg-yellow-500'}`}></span>
                    {isHealthy ? 'Running' : 'Not Ready'}
                  </span>
                )}
                {!app.is_system && !isInstalled && effectiveState === 'idle' && (
                  <span className="inline-flex items-center gap-1.5 bg-gray-500/20 text-gray-600 dark:text-gray-400 text-sm px-3 py-1 rounded-full">
                    Not Installed
                  </span>
                )}
              </div>
            </div>
          </div>

          {/* Description */}
          <div className="mt-6">
            <h3 className="text-sm font-semibold text-gray-900 dark:text-white mb-2">About</h3>
            <p className="text-gray-600 dark:text-gray-400 leading-relaxed">{app.description}</p>
          </div>

          {/* Details */}
          <div className="mt-6 grid grid-cols-2 gap-4">
            <div className="bg-gray-50 dark:bg-gray-800/50 rounded-lg p-4">
              <span className="text-xs text-gray-500 dark:text-gray-400 uppercase tracking-wide">Category</span>
              <p className="text-sm font-medium text-gray-900 dark:text-white mt-1">{categoryLabel}</p>
            </div>
            <div className="bg-gray-50 dark:bg-gray-800/50 rounded-lg p-4">
              <span className="text-xs text-gray-500 dark:text-gray-400 uppercase tracking-wide">Type</span>
              <p className="text-sm font-medium text-gray-900 dark:text-white mt-1">
                {app.is_system ? 'System' : app.is_browseable ? 'Web App' : 'Background'}
              </p>
            </div>
          </div>

          {/* VPN Configuration */}
          {canViewVpn && !app.is_system && isInstalled && (
            <div className="mt-6">
              <h3 className="text-sm font-semibold text-gray-900 dark:text-white mb-3 flex items-center gap-2">
                <svg className="w-4 h-4 text-gray-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 15v2m-6 4h12a2 2 0 002-2v-6a2 2 0 00-2-2H6a2 2 0 00-2 2v6a2 2 0 002 2zm10-10V7a4 4 0 00-8 0v4h8z" />
                </svg>
                VPN
              </h3>

              {vpnConfigLoading ? (
                <div className="bg-gray-50 dark:bg-gray-800/50 rounded-lg p-4 animate-pulse">
                  <div className="h-4 w-24 bg-gray-200 dark:bg-gray-700 rounded" />
                </div>
              ) : enabledProviders.length === 0 ? (
                <div className="bg-gray-50 dark:bg-gray-800/50 rounded-lg p-4 space-y-3">
                  <p className="text-sm text-gray-500 dark:text-gray-400">
                    No VPN providers configured.
                  </p>
                  {canManageVpn && (
                    <button
                      onClick={() => setShowVpnForm(true)}
                      className="w-full bg-blue-600 hover:bg-blue-500 text-white text-xs font-semibold py-2 px-3 rounded-lg transition-colors flex items-center justify-center gap-1.5"
                    >
                      <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
                      </svg>
                      Add VPN Provider
                    </button>
                  )}
                </div>
              ) : (
                <div className="bg-gray-50 dark:bg-gray-800/50 rounded-lg p-4 space-y-3">
                  {/* Provider selector */}
                  <div>
                    <label className="text-xs text-gray-500 dark:text-gray-400 uppercase tracking-wide">Provider</label>
                    <div className="flex gap-2 mt-1">
                      <select
                        value={selectedProviderId ?? ''}
                        onChange={(e) => {
                          const val = e.target.value
                          setSelectedProviderId(val ? Number(val) : null)
                        }}
                        disabled={!canManageVpn}
                        className="flex-1 min-w-0 px-3 py-2 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg text-sm text-gray-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-blue-500 disabled:opacity-60 disabled:cursor-not-allowed"
                      >
                        <option value="">No VPN</option>
                        {enabledProviders.map((p: VpnProvider) => (
                          <option key={p.id} value={p.id}>{p.name} ({p.vpn_type === 'wireguard' ? 'WireGuard' : 'OpenVPN'})</option>
                        ))}
                      </select>
                      {canManageVpn && (
                        <button
                          onClick={() => setShowVpnForm(true)}
                          className="flex-shrink-0 p-2 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg text-gray-500 hover:text-blue-600 hover:border-blue-300 dark:hover:border-blue-600 transition-colors"
                          title="Add VPN provider"
                        >
                          <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 4v16m8-8H4" />
                          </svg>
                        </button>
                      )}
                    </div>
                  </div>

                  {/* Kill switch override */}
                  {canManageVpn && selectedProviderId && (
                    <div className="flex items-center justify-between">
                      <div>
                        <label className="text-xs text-gray-500 dark:text-gray-400 uppercase tracking-wide">Kill Switch</label>
                        <p className="text-xs text-gray-400 dark:text-gray-500 mt-0.5">
                          {killSwitchOverride === null ? 'Using provider default' : killSwitchOverride ? 'Forced on' : 'Forced off'}
                        </p>
                      </div>
                      <div className="flex items-center gap-1.5">
                        {[
                          { label: 'Default', value: null },
                          { label: 'On', value: true },
                          { label: 'Off', value: false },
                        ].map((opt) => (
                          <button
                            key={String(opt.value)}
                            onClick={() => setKillSwitchOverride(opt.value as boolean | null)}
                            className={`px-2.5 py-1 text-xs font-medium rounded-md transition-colors ${
                              killSwitchOverride === opt.value
                                ? 'bg-blue-600 text-white'
                                : 'bg-white dark:bg-gray-700 text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-600'
                            }`}
                          >
                            {opt.label}
                          </button>
                        ))}
                      </div>
                    </div>
                  )}

                  {/* Port forwarding toggle */}
                  {canManageVpn && selectedProviderId && (
                    <div className="flex items-center justify-between">
                      <div>
                        <label className="text-xs text-gray-500 dark:text-gray-400 uppercase tracking-wide">Port Forwarding</label>
                        <p className="text-xs text-gray-400 dark:text-gray-500 mt-0.5">
                          {portForwarding ? 'NAT-PMP enabled' : 'Disabled'}
                        </p>
                      </div>
                      <div className="flex items-center gap-1.5">
                        {[
                          { label: 'Off', value: false },
                          { label: 'On', value: true },
                        ].map((opt) => (
                          <button
                            key={String(opt.value)}
                            onClick={() => setPortForwarding(opt.value)}
                            className={`px-2.5 py-1 text-xs font-medium rounded-md transition-colors ${
                              portForwarding === opt.value
                                ? 'bg-blue-600 text-white'
                                : 'bg-white dark:bg-gray-700 text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-600'
                            }`}
                          >
                            {opt.label}
                          </button>
                        ))}
                      </div>
                    </div>
                  )}

                  {/* Action buttons */}
                  {canManageVpn && (
                    <div className="flex gap-2 pt-1">
                      {selectedProviderId && (
                        <button
                          onClick={() => assignVpnMutation.mutate({
                            appName: app.name,
                            providerId: selectedProviderId,
                            killSwitch: killSwitchOverride ?? undefined,
                            portFwd: portForwarding,
                          })}
                          disabled={assignVpnMutation.isPending || (appVpnConfig?.vpn_provider_id === selectedProviderId && appVpnConfig?.kill_switch_override === killSwitchOverride && appVpnConfig?.port_forwarding === portForwarding)}
                          className="flex-1 bg-blue-600 hover:bg-blue-500 disabled:bg-blue-600/50 disabled:cursor-not-allowed text-white text-xs font-semibold py-2 px-3 rounded-lg transition-colors"
                        >
                          {assignVpnMutation.isPending ? 'Saving...' : appVpnConfig ? 'Update VPN' : 'Enable VPN'}
                        </button>
                      )}
                      {appVpnConfig && (
                        <button
                          onClick={() => removeVpnMutation.mutate(app.name)}
                          disabled={removeVpnMutation.isPending}
                          className="bg-gray-200 dark:bg-gray-700 hover:bg-red-600 disabled:cursor-not-allowed text-gray-600 dark:text-gray-300 hover:text-white text-xs font-semibold py-2 px-3 rounded-lg transition-colors"
                        >
                          {removeVpnMutation.isPending ? 'Removing...' : 'Remove VPN'}
                        </button>
                      )}
                    </div>
                  )}

                  {/* Current status */}
                  {appVpnConfig && (
                    <div className="space-y-1 pt-1">
                      <div className="flex items-center gap-2 text-xs text-green-600 dark:text-green-400">
                        <span className="w-1.5 h-1.5 bg-green-500 rounded-full"></span>
                        VPN active via {appVpnConfig.vpn_provider_name}
                        {appVpnConfig.effective_kill_switch && ' (kill switch on)'}
                      </div>
                      {appVpnConfig.port_forwarding && (
                        <div className="flex items-center gap-2 text-xs text-blue-600 dark:text-blue-400">
                          <span className="w-1.5 h-1.5 bg-blue-500 rounded-full"></span>
                          {forwardedPortData?.port
                            ? `Forwarded port: ${forwardedPortData.port}`
                            : 'Port forwarding: negotiating...'}
                        </div>
                      )}
                    </div>
                  )}
                </div>
              )}
            </div>
          )}

        </div>

      {/* VPN Provider Form Modal */}
      {showVpnForm && (
        <VpnProviderForm
          onClose={() => setShowVpnForm(false)}
          onSave={() => {
            setShowVpnForm(false)
            queryClient.invalidateQueries({ queryKey: ['vpn-providers'] })
          }}
        />
      )}
    </div>
  )
}

export default function AppsPage() {
  const { catalog, installedApps: installed, appStatuses: globalAppStatuses, refreshAppStatuses } = useMonitoring()
  const [searchParams, setSearchParams] = useSearchParams()
  const [toast, setToast] = useState<{ message: string; type: 'success' | 'error' } | null>(null)
  const [operationStatuses, setOperationStatuses] = useState<Record<string, OperationStatus>>({})
  const [selectedApp, setSelectedApp] = useState<AppConfig | null>(null)

  const filter = (searchParams.get('filter') as FilterType) || 'all'

  const clearFilter = () => {
    setSearchParams({})
  }

  const isLoading = catalog.length === 0

  // Filter apps based on filter type
  const filteredCatalog = useMemo(() => {
    if (!catalog) return []
    if (filter === 'all') return catalog

    return catalog.filter(app => {
      const isInstalled = installed?.includes(app.name) || app.is_system
      const appStatus = globalAppStatuses[app.name]
      const isHealthy = appStatus?.healthy === true

      switch (filter) {
        case 'installed':
          return isInstalled
        case 'healthy':
          return isInstalled && isHealthy
        case 'unhealthy':
          return isInstalled && !isHealthy
        case 'available':
          return !isInstalled && !app.is_system
        default:
          return true
      }
    })
  }, [catalog, installed, globalAppStatuses, filter])

  // Group apps by category
  const appsByCategory = useMemo(() => {
    if (!filteredCatalog) return {}

    const grouped: Record<string, AppConfig[]> = {}

    filteredCatalog.forEach(app => {
      const category = app.category || 'other'
      if (!grouped[category]) {
        grouped[category] = []
      }
      grouped[category].push(app)
    })

    return grouped
  }, [filteredCatalog])

  // Get sorted categories
  const sortedCategories = useMemo(() => {
    const categories = Object.keys(appsByCategory)
    return categories.sort((a, b) => {
      const aIndex = categoryOrder.indexOf(a)
      const bIndex = categoryOrder.indexOf(b)
      if (aIndex === -1 && bIndex === -1) return a.localeCompare(b)
      if (aIndex === -1) return 1
      if (bIndex === -1) return -1
      return aIndex - bIndex
    })
  }, [appsByCategory])

  // Always have an app selected — pick first from top-left category
  useEffect(() => {
    if (!selectedApp && sortedCategories.length > 0) {
      const firstCategory = sortedCategories[0]
      const firstApp = appsByCategory[firstCategory]?.[0]
      if (firstApp) setSelectedApp(firstApp)
    }
  }, [sortedCategories, appsByCategory, selectedApp])

  const showToast = (message: string, type: 'success' | 'error') => {
    setToast({ message, type })
    setTimeout(() => setToast(null), 5000)
  }

  const setOperationState = (appName: string, state: OperationState | null, message?: string) => {
    if (state === null) {
      setOperationStatuses(prev => {
        const { [appName]: _, ...rest } = prev
        return rest
      })
    } else {
      setOperationStatuses(prev => ({
        ...prev,
        [appName]: { state, message }
      }))
    }
  }

  const pollHealth = async (appName: string) => {
    const maxAttempts = 60
    let attempts = 0

    const checkHealth = async (): Promise<boolean> => {
      try {
        const health = await appsApi.checkHealth(appName)

        if (health.healthy && health.status === 'healthy') {
          setOperationState(appName, null)
          refreshAppStatuses()
          showToast(`${appName} installed successfully`, 'success')
          return true
        }

        attempts++
        if (attempts >= maxAttempts) {
          setOperationState(appName, 'error', 'Installation timeout - deployments not healthy')
          showToast(`${appName} installation timed out`, 'error')
          return true
        }

        setTimeout(() => checkHealth(), 2000)
        return false
      } catch {
        attempts++
        if (attempts >= maxAttempts) {
          setOperationState(appName, 'error', 'Health check failed')
          showToast(`${appName} health check failed`, 'error')
          return true
        }
        setTimeout(() => checkHealth(), 2000)
        return false
      }
    }

    checkHealth()
  }

  const pollDeletion = async (appName: string) => {
    const maxAttempts = 60
    let attempts = 0

    const checkDeletion = async (): Promise<boolean> => {
      try {
        const { exists } = await appsApi.checkExists(appName)

        if (!exists) {
          setOperationState(appName, null)
          refreshAppStatuses()
          showToast(`${appName} uninstalled successfully`, 'success')
          return true
        }

        attempts++
        if (attempts >= maxAttempts) {
          setOperationState(appName, 'error', 'Deletion timeout')
          showToast(`${appName} deletion timed out`, 'error')
          return true
        }

        setTimeout(() => checkDeletion(), 2000)
        return false
      } catch {
        attempts++
        if (attempts >= maxAttempts) {
          setOperationState(appName, 'error', 'Deletion check failed')
          showToast(`${appName} deletion check failed`, 'error')
          return true
        }
        setTimeout(() => checkDeletion(), 2000)
        return false
      }
    }

    checkDeletion()
  }

  const installMutation = useMutation({
    mutationFn: (appName: string) => {
      setOperationState(appName, 'installing')
      return appsApi.install({ app_name: appName, namespace: appName })
    },
    onSuccess: (_data, appName) => {
      pollHealth(appName)
    },
    onError: (error: any, appName) => {
      setOperationState(appName, 'error', error.response?.data?.detail || error.message)
      showToast(`Failed to install ${appName}: ${error.response?.data?.detail || error.message}`, 'error')
    },
  })

  const deleteMutation = useMutation({
    mutationFn: (appName: string) => {
      setOperationState(appName, 'deleting')
      return appsApi.delete(appName)
    },
    onSuccess: (_data, appName) => {
      pollDeletion(appName)
    },
    onError: (error: any, appName) => {
      setOperationState(appName, 'error', error.response?.data?.detail || error.message)
      showToast(`Failed to uninstall ${appName}: ${error.response?.data?.detail || error.message}`, 'error')
    },
  })

  const getAppState = (app: AppConfig) => {
    const isInstalled = installed?.includes(app.name)
    const operationStatus = operationStatuses[app.name]
    const globalStatus = globalAppStatuses[app.name]
    const isHealthy = globalStatus?.healthy === true

    let effectiveState: string
    if (app.is_system) {
      effectiveState = 'installed'
    } else if (operationStatus) {
      effectiveState = operationStatus.state
    } else if (isInstalled) {
      effectiveState = globalStatus?.loading ? 'loading' : 'installed'
    } else {
      effectiveState = 'idle'
    }

    return { isInstalled: isInstalled || app.is_system, isHealthy, effectiveState }
  }

  const handleOpen = (app: AppConfig) => {
    appsApi.logAccess(app.name).catch(() => {})
    window.open(`/${app.name}/`, '_blank', 'noopener,noreferrer')
  }

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-64">
        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-500"></div>
      </div>
    )
  }

  return (
    <div className="flex h-[calc(100vh-4rem-2.5rem)] -mt-8 -mx-4 sm:-mx-6 lg:-mx-8 xl:-mx-12 2xl:-mx-16">
      {/* Toast Notification */}
      {toast && (
        <div className={`fixed top-4 right-4 z-50 px-6 py-4 rounded-xl shadow-lg border backdrop-blur-sm ${
          toast.type === 'success'
            ? 'bg-green-100/90 dark:bg-green-900/90 border-green-300 dark:border-green-700 text-green-800 dark:text-green-100'
            : 'bg-red-100/90 dark:bg-red-900/90 border-red-300 dark:border-red-700 text-red-800 dark:text-red-100'
        }`}>
          <div className="flex items-center gap-3">
            <div className="flex-shrink-0">
              {toast.type === 'success' ? (
                <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 20 20">
                  <path fillRule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zm3.707-9.293a1 1 0 00-1.414-1.414L9 10.586 7.707 9.293a1 1 0 00-1.414 1.414l2 2a1 1 0 001.414 0l4-4z" clipRule="evenodd" />
                </svg>
              ) : (
                <svg className="w-5 h-5" fill="currentColor" viewBox="0 0 20 20">
                  <path fillRule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zM8.707 7.293a1 1 0 00-1.414 1.414L8.586 10l-1.293 1.293a1 1 0 101.414 1.414L10 11.414l1.293 1.293a1 1 0 001.414-1.414L11.414 10l1.293-1.293a1 1 0 00-1.414-1.414L10 8.586 8.707 7.293z" clipRule="evenodd" />
                </svg>
              )}
            </div>
            <div className="flex-1">{toast.message}</div>
            <button
              onClick={() => setToast(null)}
              className="flex-shrink-0 hover:opacity-75"
            >
              <svg className="w-4 h-4" fill="currentColor" viewBox="0 0 20 20">
                <path fillRule="evenodd" d="M4.293 4.293a1 1 0 011.414 0L10 8.586l4.293-4.293a1 1 0 111.414 1.414L11.414 10l4.293 4.293a1 1 0 01-1.414 1.414L10 11.414l-4.293 4.293a1 1 0 01-1.414-1.414L8.586 10 4.293 5.707a1 1 0 010-1.414z" clipRule="evenodd" />
              </svg>
            </button>
          </div>
        </div>
      )}

      {/* Left: Header + Content */}
      <div className="flex-1 min-w-0 flex flex-col">
        {/* Header */}
        <div className="flex-shrink-0 flex items-center justify-between border-b border-gray-200 dark:border-gray-800 px-4 sm:px-6 lg:px-8 xl:px-12 2xl:px-16 py-4">
          <div>
            <h1 className="text-3xl font-bold">Apps</h1>
            <p className="text-gray-500 dark:text-gray-400 mt-2">Browse and install applications for your media server</p>
          </div>
          <div className="flex items-center gap-3">
            <select
              value={filter}
              onChange={(e) => {
                const newFilter = e.target.value as FilterType
                if (newFilter === 'all') {
                  setSearchParams({})
                } else {
                  setSearchParams({ filter: newFilter })
                }
              }}
              className="px-4 py-2.5 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-xl text-sm font-medium text-gray-700 dark:text-gray-200 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:border-blue-500 cursor-pointer shadow-sm"
            >
              <option value="all">All Apps</option>
              <option value="installed">Installed</option>
              <option value="healthy">Healthy</option>
              <option value="unhealthy">Unhealthy</option>
              <option value="available">Available</option>
            </select>
          </div>
        </div>

        {/* Main content */}
        <div className="flex-1 min-h-0 overflow-y-auto px-4 sm:px-6 lg:px-8 xl:px-12 2xl:px-16 py-8 space-y-8">
          {/* Empty State */}
          {sortedCategories.length === 0 && filter !== 'all' && (
            <div className="flex flex-col items-center justify-center py-16 text-center">
              <div className="p-4 bg-gray-100 dark:bg-gray-800 rounded-full mb-4">
                <svg className="w-12 h-12 text-gray-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M9.172 16.172a4 4 0 015.656 0M9 10h.01M15 10h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z" />
                </svg>
              </div>
              <h3 className="text-lg font-semibold text-gray-900 dark:text-white mb-2">
                No {filter} apps found
              </h3>
              <p className="text-gray-500 dark:text-gray-400 mb-4">
                {filter === 'installed' && "You haven't installed any apps yet."}
                {filter === 'healthy' && "No apps are currently healthy."}
                {filter === 'unhealthy' && "All your installed apps are healthy!"}
                {filter === 'available' && "You've installed all available apps!"}
              </p>
              <button
                onClick={clearFilter}
                className="px-4 py-2 text-sm font-medium text-blue-600 dark:text-blue-400 hover:text-blue-800 dark:hover:text-blue-200 transition-colors"
              >
                View all apps
              </button>
            </div>
          )}

          {/* Category Sections */}
          {sortedCategories.map(category => {
            const info = categoryInfo[category] || { ...defaultCategoryInfo, label: category.replace(/-/g, ' ').replace(/\b\w/g, l => l.toUpperCase()) }
            const apps = appsByCategory[category]

            return (
              <section key={category} className="space-y-4">
                {/* Category Header */}
                <div className="flex items-center gap-3">
                  <div className="p-2.5 bg-gradient-to-br from-gray-100 to-gray-50 dark:from-gray-800 dark:to-gray-800/50 rounded-xl text-blue-600 dark:text-blue-400 shadow-sm">
                    {info.icon}
                  </div>
                  <div>
                    <h2 className="text-xl font-semibold">{info.label}</h2>
                    <p className="text-sm text-gray-500">{info.description}</p>
                  </div>
                  <div className="ml-auto">
                    <span className="text-sm text-gray-500 bg-gray-100 dark:bg-gray-800 px-3 py-1 rounded-full">
                      {apps.length} app{apps.length !== 1 ? 's' : ''}
                    </span>
                  </div>
                </div>

                {/* Apps Grid */}
                <div className="grid gap-4 grid-cols-1 sm:grid-cols-2 lg:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">
                  {apps.map(app => {
                    const { isInstalled, isHealthy, effectiveState } = getAppState(app)
                    return (
                      <AppCardComponent
                        key={app.name}
                        app={app}
                        isInstalled={isInstalled}
                        isHealthy={isHealthy}
                        effectiveState={effectiveState}
                        isSelected={selectedApp?.name === app.name}
                        onInstall={() => installMutation.mutate(app.name)}
                        onDelete={() => deleteMutation.mutate(app.name)}
                        onOpen={() => handleOpen(app)}
                        onClick={() => setSelectedApp(app)}
                        isOperationPending={installMutation.isPending || deleteMutation.isPending}
                      />
                    )
                  })}
                </div>
              </section>
            )
          })}
        </div>
      </div>

      {/* App Detail Panel (right sidebar) */}
      {selectedApp && (() => {
        const { isInstalled, isHealthy, effectiveState } = getAppState(selectedApp)
        return (
          <AppDetailPanel
            app={selectedApp}
            isInstalled={isInstalled}
            isHealthy={isHealthy}
            effectiveState={effectiveState}
            onInstall={() => installMutation.mutate(selectedApp.name)}
            onDelete={() => deleteMutation.mutate(selectedApp.name)}
            onOpen={() => handleOpen(selectedApp)}
            isOperationPending={installMutation.isPending || deleteMutation.isPending}
          />
        )
      })()}
    </div>
  )
}
