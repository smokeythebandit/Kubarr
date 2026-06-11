import { AppIcon, useIconColors } from '../components/AppIcon'
import { useMonitoring } from '../contexts/MonitoringContext'
import { useQuery } from '@tanstack/react-query'
import { useMemo, useRef } from 'react'
import { monitoringApi } from '../api/monitoring'
import { MiniSparkline } from './MonitoringPage'
import { Cpu, MemoryStick, Server, Container, HardDrive, Activity, AlertCircle } from 'lucide-react'
import { Link } from 'react-router-dom'
import { appsApi } from '../api/apps'

function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B'
  const k = 1024
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.floor(Math.log(bytes) / Math.log(k))
  return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i]
}

function formatBytesPerSec(bytesPerSec: number): string {
  return formatBytes(bytesPerSec) + '/s'
}

function formatPercent(value: number): string {
  return `${value.toFixed(1)}%`
}

function formatCount(value: number): string {
  return String(Math.round(value))
}

interface AppCardProps {
  app: { name: string; display_name: string }
  isHealthy: boolean
  showLoading: boolean
  hasData: boolean
}

// Helper to convert rgb to rgba
function toRgba(rgb: string, alpha: number): string {
  return rgb.replace('rgb', 'rgba').replace(')', `, ${alpha})`)
}

function AppCard({ app, isHealthy, showLoading, hasData }: AppCardProps) {
  const colors = useIconColors(app.name)

  const handleAppClick = (e: React.MouseEvent) => {
    e.preventDefault()
    // Log access (fire and forget)
    appsApi.logAccess(app.name).catch(() => {})
    // Open the app
    window.open(`/${app.name}/`, '_blank', 'noopener,noreferrer')
  }

  // Create iOS-style glass effect with multiple color gradients
  const glassStyle: React.CSSProperties = {}

  if (colors.length >= 3) {
    // Three colors: top-left, top-right, bottom gradient
    glassStyle.background = `
      radial-gradient(ellipse at 0% 0%, ${toRgba(colors[0], 0.25)} 0%, transparent 50%),
      radial-gradient(ellipse at 100% 0%, ${toRgba(colors[1], 0.2)} 0%, transparent 50%),
      radial-gradient(ellipse at 50% 100%, ${toRgba(colors[2], 0.15)} 0%, transparent 60%)
    `
  } else if (colors.length === 2) {
    // Two colors: diagonal corners
    glassStyle.background = `
      radial-gradient(ellipse at 0% 0%, ${toRgba(colors[0], 0.25)} 0%, transparent 50%),
      radial-gradient(ellipse at 100% 100%, ${toRgba(colors[1], 0.2)} 0%, transparent 50%)
    `
  } else if (colors.length === 1) {
    // Single color: top-left gradient
    glassStyle.background = `
      radial-gradient(ellipse at 0% 0%, ${toRgba(colors[0], 0.2)} 0%, transparent 50%),
      radial-gradient(ellipse at 100% 100%, ${toRgba(colors[0], 0.1)} 0%, transparent 50%)
    `
  }

  const primaryColor = colors[0]
  const baseShadow = primaryColor
    ? `0 2px 8px ${toRgba(primaryColor, 0.15)}`
    : undefined
  const hoverShadow = primaryColor
    ? `0 12px 28px ${toRgba(primaryColor, 0.3)}, 0 0 0 1px ${toRgba(primaryColor, 0.2)}`
    : undefined

  return (
    <a
      href={`/${app.name}/`}
      onClick={handleAppClick}
      className="group flex flex-col items-center gap-2 p-4 h-[152px] cursor-pointer bg-white dark:bg-gray-800/80 rounded-xl border border-gray-200/50 dark:border-gray-700/50 backdrop-blur-sm hover:-translate-y-1 transition-all duration-200"
      style={{
        ...glassStyle,
        boxShadow: baseShadow,
      }}
      onMouseEnter={(e) => {
        if (hoverShadow) {
          e.currentTarget.style.boxShadow = hoverShadow
        }
      }}
      onMouseLeave={(e) => {
        e.currentTarget.style.boxShadow = baseShadow || ''
      }}
    >
      {/* Icon Container */}
      <div className="relative">
        <AppIcon
          appName={app.name}
          size={64}
          className="rounded-2xl shadow-md"
        />
      </div>

      {/* App Name */}
      <span className="text-sm font-medium text-gray-700 dark:text-gray-200 group-hover:text-gray-900 dark:group-hover:text-white transition-colors text-center line-clamp-2 leading-tight">
        {app.display_name}
      </span>

      {/* Status Label */}
      {hasData && !showLoading && (
        <span className={`text-xs font-medium ${
          isHealthy ? 'text-green-600 dark:text-green-400' : 'text-red-600 dark:text-red-400'
        }`}>
          {isHealthy ? 'Running' : 'Not Ready'}
        </span>
      )}
      {showLoading && (
        <span className="flex items-center gap-1.5 text-xs text-gray-500 dark:text-gray-400">
          <span className="w-1.5 h-1.5 rounded-full bg-gray-400 animate-pulse" />
          Loading
        </span>
      )}
    </a>
  )
}

export default function Dashboard() {
  const {
    clusterMetrics,
    metricsAvailable,
    catalog,
    catalogLoading,
    installedApps: installedAppNames,
    appStatuses,
  } = useMonitoring()

  const installedApps = useMemo(() => catalog.filter((app) => installedAppNames.includes(app.name)), [catalog, installedAppNames])
  // Only show apps that can be opened (browseable)
  const openableApps = useMemo(() => installedApps.filter((app) => app.is_browseable), [installedApps])

  // Cache the last known app count in localStorage so loading skeletons match
  const skeletonAppCount = useRef(
    parseInt(localStorage.getItem('kubarr-app-count') || '4', 10)
  )
  if (openableApps.length > 0) {
    skeletonAppCount.current = openableApps.length
    localStorage.setItem('kubarr-app-count', String(openableApps.length))
  }

  const healthyApps = useMemo(() => installedApps.filter((app) => {
    const status = appStatuses[app.name]
    return status?.healthy ?? false
  }), [installedApps, appStatuses])

  // Fetch cluster network history for sparkline
  const { data: networkHistory } = useQuery({
    queryKey: ['monitoring', 'vm', 'cluster', 'network-history'],
    queryFn: () => monitoringApi.getClusterNetworkHistory('15m'),
    refetchInterval: 10000,
    enabled: metricsAvailable === true && !!clusterMetrics,
  })

  // Fetch cluster metrics history for sparklines on all KPI cards
  const { data: metricsHistory } = useQuery({
    queryKey: ['monitoring', 'vm', 'cluster', 'metrics-history'],
    queryFn: () => monitoringApi.getClusterMetricsHistory('15m'),
    refetchInterval: 10000,
    enabled: metricsAvailable === true && !!clusterMetrics,
  })

  return (
    <div className="space-y-8">
      {/* Metrics not available message */}
      {metricsAvailable === false && (
        <div className="relative bg-gradient-to-br from-white to-gray-50 dark:from-gray-800 dark:to-gray-900 rounded-xl p-6 flex items-center gap-4 border border-yellow-200/60 dark:border-yellow-700/40 shadow-[0_4px_12px_rgba(234,179,8,0.1),0_1px_3px_rgba(0,0,0,0.1)] dark:shadow-[0_4px_12px_rgba(0,0,0,0.3)]">
          <div className="p-3 bg-gradient-to-br from-yellow-500/20 to-yellow-600/10 rounded-xl shadow-inner">
            <AlertCircle className="w-6 h-6 text-yellow-500" />
          </div>
          <div>
            <p className="text-gray-700 dark:text-gray-300 font-medium">Metrics server is not available</p>
            <p className="text-sm text-gray-500 dark:text-gray-500">VictoriaMetrics may be starting up or experiencing issues</p>
          </div>
        </div>
      )}

      <h2 className="text-2xl font-bold">Dashboard</h2>
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-4">
        {/* App Overview Card */}
        <Link to="/apps" className="relative bg-gradient-to-br from-white to-gray-50 dark:from-gray-800 dark:to-gray-900 rounded-xl p-5 h-[125px] border border-gray-200/60 dark:border-gray-700/60 shadow-[0_4px_12px_rgba(0,0,0,0.05),0_1px_3px_rgba(0,0,0,0.1)] dark:shadow-[0_4px_12px_rgba(0,0,0,0.3),inset_0_1px_0_rgba(255,255,255,0.05)] hover:shadow-[0_8px_24px_rgba(0,0,0,0.1),0_2px_6px_rgba(0,0,0,0.08)] dark:hover:shadow-[0_8px_24px_rgba(0,0,0,0.4)] hover:-translate-y-0.5 transition-all duration-200 cursor-pointer group overflow-hidden flex flex-col justify-between">
          <div className="absolute inset-0 rounded-xl bg-gradient-to-br from-indigo-500/5 via-transparent to-purple-500/5 pointer-events-none" />
          <div className="absolute inset-0 bg-gradient-to-br from-indigo-500/10 via-transparent to-transparent opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none" />
          <div className="relative">
            {catalogLoading ? (
              <div className="animate-pulse">
                <div className="h-5 w-24 bg-gray-200 dark:bg-gray-700 rounded mb-2" />
                <div className="h-9 w-16 bg-gray-200 dark:bg-gray-700 rounded" />
              </div>
            ) : (
              <div className="flex items-center gap-3">
                <div className="flex-1 min-w-0">
                  <div className="text-sm font-medium text-gray-500 dark:text-gray-400">Apps</div>
                  <div className="flex items-baseline gap-1.5">
                    <span className="text-xl font-bold">{installedApps.length}</span>
                    <span className="text-xs text-gray-500 dark:text-gray-400">installed</span>
                  </div>
                </div>
                {/* Stacked app icons */}
                <div className="flex -space-x-1.5 flex-shrink-0">
                  {installedApps.slice(0, 5).map((app, i) => (
                    <div key={app.name} className="relative rounded-md overflow-hidden" style={{ zIndex: 5 - i }}>
                      <AppIcon appName={app.name} size={24} className="rounded-md" />
                    </div>
                  ))}
                  {installedApps.length > 5 && (
                    <div className="relative w-6 h-6 rounded-md bg-gray-100 dark:bg-gray-700 flex items-center justify-center text-[9px] font-bold text-gray-500 dark:text-gray-400" style={{ zIndex: 0 }}>
                      +{installedApps.length - 5}
                    </div>
                  )}
                </div>
              </div>
            )}
          </div>
          {/* Bottom stats row */}
          {!catalogLoading && (
            <div className="relative flex items-center gap-3 text-xs">
              <span className="flex items-center gap-1">
                <span className="w-1.5 h-1.5 rounded-full bg-green-500 shadow-[0_0_4px_rgba(34,197,94,0.5)]" />
                <span className="font-semibold text-green-600 dark:text-green-400">{healthyApps.length}</span>
                <span className="text-gray-400 dark:text-gray-500">healthy</span>
              </span>
              {installedApps.length - healthyApps.length > 0 && (
                <span className="flex items-center gap-1">
                  <span className="w-1.5 h-1.5 rounded-full bg-red-500 shadow-[0_0_4px_rgba(239,68,68,0.5)]" />
                  <span className="font-semibold text-red-600 dark:text-red-400">{installedApps.length - healthyApps.length}</span>
                  <span className="text-gray-400 dark:text-gray-500">unhealthy</span>
                </span>
              )}
              <span className="flex items-center gap-1 ml-auto">
                <span className="font-semibold text-indigo-600 dark:text-indigo-400">{(catalog?.length || 0) - installedApps.length}</span>
                <span className="text-gray-400 dark:text-gray-500">available</span>
              </span>
            </div>
          )}
        </Link>

        {/* System Resource Cards */}
        {(!clusterMetrics || !metricsHistory || !networkHistory) && metricsAvailable !== false && (
          <>
            {[...Array(7)].map((_, i) => (
              <div key={i} className="relative bg-gradient-to-br from-white to-gray-50 dark:from-gray-800 dark:to-gray-900 rounded-xl p-5 h-[125px] border border-gray-200/60 dark:border-gray-700/60 shadow-[0_4px_12px_rgba(0,0,0,0.05),0_1px_3px_rgba(0,0,0,0.1)] dark:shadow-[0_4px_12px_rgba(0,0,0,0.3),inset_0_1px_0_rgba(255,255,255,0.05)] animate-pulse overflow-hidden flex flex-col justify-between">
                <div className="flex items-center gap-3">
                  <div className="w-10 h-10 bg-gray-200 dark:bg-gray-700 rounded-xl" />
                  <div className="flex-1">
                    <div className="h-4 w-20 bg-gray-200 dark:bg-gray-700 rounded mb-2" />
                    <div className="h-6 w-14 bg-gray-200 dark:bg-gray-700 rounded" />
                  </div>
                </div>
                <div className="relative -mx-5 -mb-5 h-[45px] bg-gray-100 dark:bg-gray-800 rounded-b-xl" />
              </div>
            ))}
          </>
        )}
        {clusterMetrics && metricsHistory && networkHistory && (
          <>
            {/* CPU Usage */}
            <Link
              to="/resources"
              className="relative bg-gradient-to-br from-white to-gray-50 dark:from-gray-800 dark:to-gray-900 rounded-xl p-5 h-[125px] border border-gray-200/60 dark:border-gray-700/60 shadow-[0_4px_12px_rgba(0,0,0,0.05),0_1px_3px_rgba(0,0,0,0.1)] dark:shadow-[0_4px_12px_rgba(0,0,0,0.3),inset_0_1px_0_rgba(255,255,255,0.05)] hover:shadow-[0_12px_28px_rgba(59,130,246,0.2),0_4px_8px_rgba(0,0,0,0.1)] dark:hover:shadow-[0_12px_28px_rgba(59,130,246,0.25)] hover:-translate-y-1 transition-all duration-200 cursor-pointer group overflow-hidden flex flex-col justify-between"
            >
              <div className="absolute inset-0 bg-gradient-to-br from-blue-500/10 via-transparent to-transparent opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none" />
              <div className="relative flex items-center gap-3">
                <div className="p-2.5 bg-gradient-to-br from-blue-500/20 to-blue-600/10 rounded-xl shadow-inner group-hover:from-blue-500/30 group-hover:to-blue-600/20 transition-colors">
                  <Cpu className="w-5 h-5 text-blue-500 dark:text-blue-400" />
                </div>
                <div className="flex-1">
                  <div className="text-gray-500 dark:text-gray-400 text-sm font-medium">CPU Usage</div>
                  <div className="text-xl font-bold">
                    {clusterMetrics.cpu_usage_percent.toFixed(1)}%
                  </div>
                </div>
                <div className="text-right">
                  <div className="text-lg font-bold text-blue-500 dark:text-blue-400">
                    {clusterMetrics.used_cpu_cores.toFixed(2)}
                  </div>
                  <div className="text-xs text-gray-500 dark:text-gray-500">
                    / {clusterMetrics.total_cpu_cores.toFixed(0)} cores
                  </div>
                </div>
              </div>
              {metricsHistory?.cpu_series && metricsHistory.cpu_series.length >= 2 && (
                <div className="relative -mx-5 -mb-5">
                  <MiniSparkline data={metricsHistory.cpu_series} color="blue" height={45} interactive formatValue={formatPercent} />
                </div>
              )}
            </Link>

            {/* Memory Usage */}
            <Link
              to="/resources"
              className="relative bg-gradient-to-br from-white to-gray-50 dark:from-gray-800 dark:to-gray-900 rounded-xl p-5 h-[125px] border border-gray-200/60 dark:border-gray-700/60 shadow-[0_4px_12px_rgba(0,0,0,0.05),0_1px_3px_rgba(0,0,0,0.1)] dark:shadow-[0_4px_12px_rgba(0,0,0,0.3),inset_0_1px_0_rgba(255,255,255,0.05)] hover:shadow-[0_12px_28px_rgba(168,85,247,0.2),0_4px_8px_rgba(0,0,0,0.1)] dark:hover:shadow-[0_12px_28px_rgba(168,85,247,0.25)] hover:-translate-y-1 transition-all duration-200 cursor-pointer group overflow-hidden flex flex-col justify-between"
            >
              <div className="absolute inset-0 bg-gradient-to-br from-purple-500/10 via-transparent to-transparent opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none" />
              <div className="relative flex items-center gap-3">
                <div className="p-2.5 bg-gradient-to-br from-purple-500/20 to-purple-600/10 rounded-xl shadow-inner group-hover:from-purple-500/30 group-hover:to-purple-600/20 transition-colors">
                  <MemoryStick className="w-5 h-5 text-purple-500 dark:text-purple-400" />
                </div>
                <div className="flex-1">
                  <div className="text-gray-500 dark:text-gray-400 text-sm font-medium">Memory Usage</div>
                  <div className="text-xl font-bold">
                    {clusterMetrics.memory_usage_percent.toFixed(1)}%
                  </div>
                </div>
                <div className="text-right">
                  <div className="text-lg font-bold text-purple-500 dark:text-purple-400">
                    {formatBytes(clusterMetrics.used_memory_bytes)}
                  </div>
                  <div className="text-xs text-gray-500 dark:text-gray-500">
                    / {formatBytes(clusterMetrics.total_memory_bytes)}
                  </div>
                </div>
              </div>
              {metricsHistory?.memory_series && metricsHistory.memory_series.length >= 2 && (
                <div className="relative -mx-5 -mb-5">
                  <MiniSparkline data={metricsHistory.memory_series} color="purple" height={45} interactive formatValue={formatPercent} />
                </div>
              )}
            </Link>

            {/* Storage Usage */}
            <Link
              to="/storage"
              className="relative bg-gradient-to-br from-white to-gray-50 dark:from-gray-800 dark:to-gray-900 rounded-xl p-5 h-[125px] border border-gray-200/60 dark:border-gray-700/60 shadow-[0_4px_12px_rgba(0,0,0,0.05),0_1px_3px_rgba(0,0,0,0.1)] dark:shadow-[0_4px_12px_rgba(0,0,0,0.3),inset_0_1px_0_rgba(255,255,255,0.05)] hover:shadow-[0_12px_28px_rgba(244,63,94,0.2),0_4px_8px_rgba(0,0,0,0.1)] dark:hover:shadow-[0_12px_28px_rgba(244,63,94,0.25)] hover:-translate-y-1 transition-all duration-200 cursor-pointer group overflow-hidden flex flex-col justify-between"
            >
              <div className="absolute inset-0 bg-gradient-to-br from-rose-500/10 via-transparent to-transparent opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none" />
              <div className="relative flex items-center gap-3">
                <div className="p-2.5 bg-gradient-to-br from-rose-500/20 to-rose-600/10 rounded-xl shadow-inner group-hover:from-rose-500/30 group-hover:to-rose-600/20 transition-colors">
                  <HardDrive className="w-5 h-5 text-rose-500 dark:text-rose-400" />
                </div>
                <div className="flex-1">
                  <div className="text-gray-500 dark:text-gray-400 text-sm font-medium">Storage</div>
                  <div className="text-xl font-bold">
                    {clusterMetrics.total_storage_bytes > 0
                      ? `${clusterMetrics.storage_usage_percent.toFixed(1)}%`
                      : 'N/A'}
                  </div>
                </div>
                {clusterMetrics.total_storage_bytes > 0 && (
                  <div className="text-right">
                    <div className="text-lg font-bold text-rose-500 dark:text-rose-400">
                      {formatBytes(clusterMetrics.used_storage_bytes)}
                    </div>
                    <div className="text-xs text-gray-500 dark:text-gray-500">
                      / {formatBytes(clusterMetrics.total_storage_bytes)}
                    </div>
                  </div>
                )}
              </div>
              {metricsHistory?.storage_series && metricsHistory.storage_series.length >= 2 && (
                <div className="relative -mx-5 -mb-5">
                  <MiniSparkline data={metricsHistory.storage_series} color="red" height={45} interactive formatValue={formatPercent} />
                </div>
              )}
            </Link>

            {/* Network Traffic */}
            <Link
              to="/resources"
              className="relative bg-gradient-to-br from-white to-gray-50 dark:from-gray-800 dark:to-gray-900 rounded-xl p-5 h-[125px] border border-gray-200/60 dark:border-gray-700/60 shadow-[0_4px_12px_rgba(0,0,0,0.05),0_1px_3px_rgba(0,0,0,0.1)] dark:shadow-[0_4px_12px_rgba(0,0,0,0.3),inset_0_1px_0_rgba(255,255,255,0.05)] hover:shadow-[0_12px_28px_rgba(34,197,94,0.2),0_4px_8px_rgba(0,0,0,0.1)] dark:hover:shadow-[0_12px_28px_rgba(34,197,94,0.25)] hover:-translate-y-1 transition-all duration-200 cursor-pointer group overflow-hidden flex flex-col justify-between"
            >
              <div className="absolute inset-0 bg-gradient-to-br from-green-500/10 via-transparent to-transparent opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none" />
              <div className="relative flex items-center gap-3">
                <div className="p-2.5 bg-gradient-to-br from-green-500/20 to-green-600/10 rounded-xl shadow-inner group-hover:from-green-500/30 group-hover:to-green-600/20 transition-colors">
                  <Activity className="w-5 h-5 text-green-500 dark:text-green-400" />
                </div>
                <div className="flex-1">
                  <div className="text-gray-500 dark:text-gray-400 text-sm font-medium">Network I/O</div>
                </div>
                <div className="text-right">
                  <div className="text-sm font-bold text-green-500 dark:text-green-400">↓ {formatBytesPerSec(clusterMetrics.network_receive_bytes_per_sec)}</div>
                  <div className="text-sm font-bold text-red-500 dark:text-red-400">↑ {formatBytesPerSec(clusterMetrics.network_transmit_bytes_per_sec)}</div>
                </div>
              </div>
              {networkHistory?.rx_series && networkHistory.rx_series.length >= 2 && (
                <div className="relative -mx-5 -mb-5">
                  <MiniSparkline data={networkHistory.rx_series} color="green" secondaryData={networkHistory.tx_series} secondaryColor="red" height={45} interactive formatValue={formatBytesPerSec} />
                </div>
              )}
            </Link>

            {/* Running Pods */}
            <Link
              to="/resources"
              className="relative bg-gradient-to-br from-white to-gray-50 dark:from-gray-800 dark:to-gray-900 rounded-xl p-5 h-[125px] border border-gray-200/60 dark:border-gray-700/60 shadow-[0_4px_12px_rgba(0,0,0,0.05),0_1px_3px_rgba(0,0,0,0.1)] dark:shadow-[0_4px_12px_rgba(0,0,0,0.3),inset_0_1px_0_rgba(255,255,255,0.05)] hover:shadow-[0_12px_28px_rgba(6,182,212,0.2),0_4px_8px_rgba(0,0,0,0.1)] dark:hover:shadow-[0_12px_28px_rgba(6,182,212,0.25)] hover:-translate-y-1 transition-all duration-200 cursor-pointer group overflow-hidden flex flex-col justify-between"
            >
              <div className="absolute inset-0 bg-gradient-to-br from-cyan-500/10 via-transparent to-transparent opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none" />
              <div className="relative flex items-center gap-3">
                <div className="p-2.5 bg-gradient-to-br from-cyan-500/20 to-cyan-600/10 rounded-xl shadow-inner group-hover:from-cyan-500/30 group-hover:to-cyan-600/20 transition-colors">
                  <Server className="w-5 h-5 text-cyan-500 dark:text-cyan-400" />
                </div>
                <div className="flex-1">
                  <div className="text-gray-500 dark:text-gray-400 text-sm font-medium">Running Pods</div>
                  <div className="text-xl font-bold">{clusterMetrics.pod_count}</div>
                </div>
              </div>
              {metricsHistory?.pod_series && metricsHistory.pod_series.length >= 2 && (
                <div className="relative -mx-5 -mb-5">
                  <MiniSparkline data={metricsHistory.pod_series} color="cyan" height={45} interactive formatValue={formatCount} />
                </div>
              )}
            </Link>

            {/* Running Containers */}
            <Link
              to="/resources"
              className="relative bg-gradient-to-br from-white to-gray-50 dark:from-gray-800 dark:to-gray-900 rounded-xl p-5 h-[125px] border border-gray-200/60 dark:border-gray-700/60 shadow-[0_4px_12px_rgba(0,0,0,0.05),0_1px_3px_rgba(0,0,0,0.1)] dark:shadow-[0_4px_12px_rgba(0,0,0,0.3),inset_0_1px_0_rgba(255,255,255,0.05)] hover:shadow-[0_12px_28px_rgba(234,179,8,0.2),0_4px_8px_rgba(0,0,0,0.1)] dark:hover:shadow-[0_12px_28px_rgba(234,179,8,0.25)] hover:-translate-y-1 transition-all duration-200 cursor-pointer group overflow-hidden flex flex-col justify-between"
            >
              <div className="absolute inset-0 bg-gradient-to-br from-yellow-500/10 via-transparent to-transparent opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none" />
              <div className="relative flex items-center gap-3">
                <div className="p-2.5 bg-gradient-to-br from-yellow-500/20 to-yellow-600/10 rounded-xl shadow-inner group-hover:from-yellow-500/30 group-hover:to-yellow-600/20 transition-colors">
                  <Container className="w-5 h-5 text-yellow-500 dark:text-yellow-400" />
                </div>
                <div className="flex-1">
                  <div className="text-gray-500 dark:text-gray-400 text-sm font-medium">Running Containers</div>
                  <div className="text-xl font-bold">{clusterMetrics.container_count}</div>
                </div>
              </div>
              {metricsHistory?.container_series && metricsHistory.container_series.length >= 2 && (
                <div className="relative -mx-5 -mb-5">
                  <MiniSparkline data={metricsHistory.container_series} color="yellow" height={45} interactive formatValue={formatCount} />
                </div>
              )}
            </Link>

            {/* Cluster Health Summary */}
            <Link
              to="/resources"
              className="relative bg-gradient-to-br from-white to-gray-50 dark:from-gray-800 dark:to-gray-900 rounded-xl p-5 h-[125px] border border-gray-200/60 dark:border-gray-700/60 shadow-[0_4px_12px_rgba(0,0,0,0.05),0_1px_3px_rgba(0,0,0,0.1)] dark:shadow-[0_4px_12px_rgba(0,0,0,0.3),inset_0_1px_0_rgba(255,255,255,0.05)] hover:shadow-[0_12px_28px_rgba(16,185,129,0.2),0_4px_8px_rgba(0,0,0,0.1)] dark:hover:shadow-[0_12px_28px_rgba(16,185,129,0.25)] hover:-translate-y-1 transition-all duration-200 cursor-pointer group overflow-hidden"
            >
              <div className="absolute inset-0 bg-gradient-to-br from-emerald-500/10 via-transparent to-transparent opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none" />
              <div className="relative flex items-center gap-3">
                <div className="p-2.5 bg-gradient-to-br from-emerald-500/20 to-emerald-600/10 rounded-xl shadow-inner group-hover:from-emerald-500/30 group-hover:to-emerald-600/20 transition-colors">
                  <Activity className="w-5 h-5 text-emerald-500 dark:text-emerald-400" />
                </div>
                <div className="flex-1">
                  <div className="text-gray-500 dark:text-gray-400 text-sm font-medium">Cluster Status</div>
                  <div className={`text-xl font-bold ${
                    healthyApps.length === installedApps.length ? 'text-emerald-500 dark:text-emerald-400' : 'text-yellow-500 dark:text-yellow-400'
                  }`}>
                    {healthyApps.length === installedApps.length ? 'All Systems Operational' : 'Degraded'}
                  </div>
                </div>
                <div className="flex flex-col items-end text-sm">
                  <span className="text-emerald-500 dark:text-emerald-400 font-semibold">{healthyApps.length} healthy</span>
                  {installedApps.length - healthyApps.length > 0 && (
                    <span className="text-red-500 dark:text-red-400 font-semibold">{installedApps.length - healthyApps.length} unhealthy</span>
                  )}
                </div>
              </div>
            </Link>
          </>
        )}
      </div>

      {/* App Grid - Launchpad Style */}
      {catalogLoading ? (
        <div>
          <h2 className="text-2xl font-bold mb-4">Installed Apps</h2>
          <div className="grid grid-cols-2 xs:grid-cols-3 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 xl:grid-cols-8 gap-4">
            {[...Array(skeletonAppCount.current)].map((_, i) => (
              <div key={i} className="flex flex-col items-center gap-2 p-4 h-[152px] bg-white dark:bg-gray-800/80 rounded-xl border border-gray-200/50 dark:border-gray-700/50 animate-pulse">
                <div className="w-16 h-16 bg-gray-200 dark:bg-gray-700 rounded-2xl" />
                <div className="h-4 w-16 bg-gray-200 dark:bg-gray-700 rounded" />
                <div className="h-3 w-12 bg-gray-200 dark:bg-gray-700 rounded" />
              </div>
            ))}
          </div>
        </div>
      ) : openableApps.length > 0 ? (
        <div>
          <h2 className="text-2xl font-bold mb-4">Installed Apps</h2>
          <div className="grid grid-cols-2 xs:grid-cols-3 sm:grid-cols-4 md:grid-cols-5 lg:grid-cols-6 xl:grid-cols-8 gap-4">
            {openableApps.map((app) => {
              const status = appStatuses[app.name]
              const isHealthy = status?.healthy ?? false
              const showLoading = status?.loading ?? false
              const hasData = status !== undefined

              return (
                <AppCard
                  key={app.name}
                  app={app}
                  isHealthy={isHealthy}
                  showLoading={showLoading}
                  hasData={hasData}
                />
              )
            })}
          </div>
        </div>
      ) : (
        <div className="relative bg-gradient-to-br from-white to-gray-50 dark:from-gray-800 dark:to-gray-900 rounded-xl border border-gray-200/60 dark:border-gray-700/60 shadow-[0_4px_12px_rgba(0,0,0,0.05),0_1px_3px_rgba(0,0,0,0.1)] dark:shadow-[0_4px_12px_rgba(0,0,0,0.3)] text-center py-12 px-6">
          <p className="text-gray-500 dark:text-gray-400 mb-4">No apps installed yet.</p>
          <Link
            to="/apps"
            className="inline-block bg-gradient-to-r from-blue-600 to-blue-500 hover:from-blue-500 hover:to-blue-400 text-white font-medium py-2.5 px-6 rounded-xl shadow-[0_4px_12px_rgba(59,130,246,0.3)] hover:shadow-[0_6px_16px_rgba(59,130,246,0.4)] hover:-translate-y-0.5 transition-all duration-200"
          >
            Browse Apps
          </Link>
        </div>
      )}
    </div>
  )
}
