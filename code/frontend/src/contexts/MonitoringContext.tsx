import { createContext, useContext, useEffect, useMemo, ReactNode } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { monitoringApi, ClusterMetrics } from '../api/monitoring'
import { appsApi } from '../api/apps'
import { preloadIcons } from '../components/AppIcon'
import type { AppConfig, AppState, PodStatus } from '../types'

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
  appStates: Record<string, AppState>
  appStatuses: Record<string, AppStatusInfo>

  // Actions
  refreshMetrics: () => Promise<void>
  refreshAppStatuses: () => Promise<void>
}

const MonitoringContext = createContext<MonitoringState | null>(null)

// Query keys are shared so that any component asking the same question joins
// the same request. Four independent fetch layers previously made one
// dashboard load fetch twenty-one URLs two to four times each.
export const monitoringKeys = {
  metricsAvailable: ['monitoring', 'available'] as const,
  clusterMetrics: ['monitoring', 'cluster'] as const,
  catalog: ['apps', 'catalog'] as const,
  installed: ['apps', 'installed'] as const,
  states: ['apps', 'states'] as const,
  pods: (namespaces: string[]) => ['monitoring', 'pods', namespaces.join(',')] as const,
}

export function MonitoringProvider({ children }: { children: ReactNode }) {
  const queryClient = useQueryClient()

  const availability = useQuery({
    queryKey: monitoringKeys.metricsAvailable,
    queryFn: () => monitoringApi.checkMetricsAvailable(),
  })
  const metricsAvailable = availability.data?.available ?? null

  const metrics = useQuery({
    queryKey: monitoringKeys.clusterMetrics,
    queryFn: () => monitoringApi.getClusterMetrics(),
    enabled: metricsAvailable === true,
    refetchInterval: 10_000,
  })

  const catalog = useQuery({
    queryKey: monitoringKeys.catalog,
    queryFn: () => appsApi.getCatalog(),
    // The catalog only changes when charts sync.
    staleTime: 60_000,
  })

  const installed = useQuery({
    queryKey: monitoringKeys.installed,
    queryFn: () => appsApi.getInstalled(),
    refetchInterval: 15_000,
  })

  const states = useQuery({
    queryKey: monitoringKeys.states,
    queryFn: () => appsApi.getStates().catch(() => []),
    refetchInterval: 15_000,
  })

  // Each installed app lives in a namespace of the same name, so the whole
  // dashboard needs one request rather than one per app.
  const namespaces = useMemo(
    () => (installed.data ?? []).slice().sort(),
    [installed.data],
  )

  const pods = useQuery({
    queryKey: monitoringKeys.pods(namespaces),
    queryFn: () => monitoringApi.getPodStatusFor(namespaces),
    enabled: namespaces.length > 0,
    refetchInterval: 15_000,
  })

  useEffect(() => {
    if (catalog.data) {
      preloadIcons(catalog.data.map((app) => app.name))
    }
  }, [catalog.data])

  const appStatuses = useMemo(() => {
    const byApp: Record<string, AppStatusInfo> = {}
    for (const name of namespaces) {
      const appPods = (pods.data ?? []).filter((pod) => pod.app === name)
      byApp[name] = {
        pods: appPods,
        loading: pods.isLoading,
        healthy:
          appPods.length > 0 &&
          appPods.every((pod) => pod.ready && pod.status === 'Running'),
      }
    }
    return byApp
  }, [namespaces, pods.data, pods.isLoading])

  const appStates = useMemo(
    () => Object.fromEntries((states.data ?? []).map((state) => [state.app_name, state])),
    [states.data],
  )

  const value: MonitoringState = {
    clusterMetrics: metrics.data ?? null,
    metricsLoading: metrics.isLoading,
    metricsAvailable,
    catalog: catalog.data ?? [],
    catalogLoading: catalog.isLoading,
    installedApps: installed.data ?? [],
    appStates,
    appStatuses,
    refreshMetrics: async () => {
      await queryClient.invalidateQueries({ queryKey: ['monitoring'] })
    },
    refreshAppStatuses: async () => {
      await queryClient.invalidateQueries({ queryKey: ['apps'] })
      await queryClient.invalidateQueries({ queryKey: ['monitoring', 'pods'] })
    },
  }

  return <MonitoringContext.Provider value={value}>{children}</MonitoringContext.Provider>
}

export function useMonitoring() {
  const context = useContext(MonitoringContext)
  if (!context) {
    throw new Error('useMonitoring must be used within a MonitoringProvider')
  }
  return context
}
