import { createContext, useContext, useEffect, useState, useCallback, ReactNode } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { monitoringApi, ClusterMetrics } from '../api/monitoring'
import { appsApi } from '../api/apps'
import { preloadIcons } from '../components/AppIcon'
import type { AppConfig, PodStatus } from '../types'
import { getPrecached, clearPrecache } from '../utils/precache'

interface AppStatusInfo {
  healthy: boolean
  loading: boolean
  pods: PodStatus[]
}

interface MonitoringState {
  // Cluster metrics
  clusterMetrics: ClusterMetrics | null
  metricsLoading: boolean
  metricsAvailable: boolean | null

  // Apps data
  catalog: AppConfig[]
  catalogLoading: boolean
  installedApps: string[]
  appStatuses: Record<string, AppStatusInfo>

  // Actions
  refreshMetrics: () => Promise<void>
  refreshAppStatuses: () => Promise<void>
}

const MonitoringContext = createContext<MonitoringState | null>(null)

export function MonitoringProvider({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient()

  // Check for precached data on initial mount
  const precachedMetrics = getPrecached<ClusterMetrics>('clusterMetrics')
  const precachedCatalog = getPrecached<AppConfig[]>('catalog')
  const precachedInstalled = getPrecached<string[]>('installed')

  // Cluster metrics state - initialize from precache if available
  const [clusterMetrics, setClusterMetrics] = useState<ClusterMetrics | null>(precachedMetrics)
  const [metricsLoading, setMetricsLoading] = useState(!precachedMetrics)
  const [metricsAvailable, setMetricsAvailable] = useState<boolean | null>(precachedMetrics ? true : null)

  // Apps state - initialize from precache if available
  const [catalog, setCatalog] = useState<AppConfig[]>(precachedCatalog || [])
  const [catalogLoading, setCatalogLoading] = useState(!precachedCatalog)
  const [installedApps, setInstalledApps] = useState<string[]>(precachedInstalled || [])
  const [appStatuses, setAppStatuses] = useState<Record<string, AppStatusInfo>>({})

  // Fetch cluster metrics
  const refreshMetrics = useCallback(async () => {
    try {
      // Check VictoriaMetrics availability first
      const status = await monitoringApi.checkMetricsAvailable()
      setMetricsAvailable(status.available)

      if (status.available) {
        setMetricsLoading(true)
        const metrics = await monitoringApi.getClusterMetrics()
        setClusterMetrics(metrics)
        // Also update query cache for components using useQuery
        queryClient.setQueryData(['monitoring', 'cluster'], metrics)
      }
    } catch (error) {
      console.error('Failed to fetch cluster metrics:', error)
    } finally {
      setMetricsLoading(false)
    }
  }, [queryClient])

  // Fetch app statuses
  const refreshAppStatuses = useCallback(async () => {
    try {
      // Fetch catalog and installed apps
      const [catalogData, installedData] = await Promise.all([
        appsApi.getCatalog(),
        appsApi.getInstalled()
      ])

      setCatalog(catalogData)
      setInstalledApps(installedData)
      setCatalogLoading(false)

      // Preload all app icons in the background
      preloadIcons(catalogData.map(app => app.name))

      // Update query cache
      queryClient.setQueryData(['apps', 'catalog'], catalogData)
      queryClient.setQueryData(['apps', 'installed'], installedData)

      // Fetch pod status for each installed app
      const installedAppConfigs = catalogData.filter(app => installedData.includes(app.name))

      const statusPromises = installedAppConfigs.map(async (app) => {
        try {
          const pods = await monitoringApi.getPodStatus(app.name)
          const mainPods = pods.filter(p => p.app === app.name)
          const healthy = mainPods.length > 0 && mainPods.every(p => p.ready && p.status === 'Running')

          return {
            name: app.name,
            status: { healthy, loading: false, pods }
          }
        } catch {
          return {
            name: app.name,
            status: { healthy: false, loading: false, pods: [] }
          }
        }
      })

      const statuses = await Promise.all(statusPromises)
      const statusMap: Record<string, AppStatusInfo> = {}
      statuses.forEach(({ name, status }) => {
        statusMap[name] = status
        // Update query cache for individual pod queries
        queryClient.setQueryData(['monitoring', 'pods', name], status.pods)
      })

      setAppStatuses(statusMap)
    } catch (error) {
      console.error('Failed to fetch app statuses:', error)
    }
  }, [queryClient])

  // Initial fetch on mount
  useEffect(() => {
    // Clear precache after consuming it - data will now be managed by this provider
    clearPrecache()

    refreshMetrics()
    refreshAppStatuses()

    // Set up periodic refresh (continuous streaming)
    const metricsInterval = setInterval(refreshMetrics, 10000)
    const statusInterval = setInterval(refreshAppStatuses, 15000)

    return () => {
      clearInterval(metricsInterval)
      clearInterval(statusInterval)
    }
  }, [refreshMetrics, refreshAppStatuses])

  const value: MonitoringState = {
    clusterMetrics,
    metricsLoading,
    metricsAvailable,
    catalog,
    catalogLoading,
    installedApps,
    appStatuses,
    refreshMetrics,
    refreshAppStatuses,
  }

  return (
    <MonitoringContext.Provider value={value}>
      {children}
    </MonitoringContext.Provider>
  )
}

export function useMonitoring() {
  const context = useContext(MonitoringContext)
  if (!context) {
    throw new Error('useMonitoring must be used within a MonitoringProvider')
  }
  return context
}
