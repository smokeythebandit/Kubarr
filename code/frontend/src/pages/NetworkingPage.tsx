import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import {
  AlertCircle,
  Info,
  Maximize2,
  Minimize2,
  Network,
  RefreshCw,
} from 'lucide-react'
import { emptyClassTotals, FlowWindow } from '../api/networking'
import { appVpnApi } from '../api/vpn'
import { useNetworkMetricsWs } from '../hooks/useNetworkMetricsWs'
import { useFlowClasses } from '../hooks/useFlowClasses'
import { ConnectionStatus } from '../components/networking/ConnectionStatus'
import { FlowClassCards } from '../components/networking/FlowClassCards'
import { FlowClassChart } from '../components/networking/FlowClassChart'
import { FlowSplitBar } from '../components/networking/FlowSplitBar'
import { FlowWindowSelector } from '../components/networking/FlowWindowSelector'
import { NetworkFlowVisualization } from '../components/networking/NetworkFlowVisualization'
import { NetworkStatsTable } from '../components/networking/NetworkStatsTable'

// Infrastructure namespaces to hide by default
const INFRA_NAMESPACES = ['fluent-bit', 'victoriametrics', 'victorialogs']

const VPN_CONFIG_REFETCH_MS = 30000

export default function NetworkingPage() {
  const [isFullscreen, setIsFullscreen] = useState(false)
  const [showInfra, setShowInfra] = useState(false)
  const [flowWindow, setFlowWindow] = useState<FlowWindow>('15m')
  const networkFlowRef = useRef<HTMLDivElement>(null)
  const queryClient = useQueryClient()

  // Use WebSocket hook for real-time updates with HTTP polling fallback
  const {
    topology,
    stats,
    connectionMode,
    error: wsError,
  } = useNetworkMetricsWs()

  const {
    data: flowClasses,
    isLoading: flowClassesLoading,
    isError: flowClassesError,
  } = useFlowClasses(flowWindow)

  // VPN configs drive the shield badges on the graph. They live in the query
  // cache like everything else so a refresh invalidates them along with the
  // rest, instead of running its own timer.
  const { data: vpnConfigs = [] } = useQuery({
    queryKey: ['vpn', 'app-configs'],
    queryFn: appVpnApi.listConfigs,
    refetchInterval: VPN_CONFIG_REFETCH_MS,
  })

  const isLoading = !topology && !stats
  const hasError = wsError && connectionMode === 'disconnected'

  /**
   * Refresh used to reload the whole document, which threw away the router
   * state and every other cached panel to re-fetch two endpoints. Invalidating
   * the queries refetches exactly what this page shows.
   */
  const refresh = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: ['networking'] })
    void queryClient.invalidateQueries({ queryKey: ['vpn', 'app-configs'] })
  }, [queryClient])

  // Fullscreen toggle
  const toggleFullscreen = useCallback(() => {
    if (!networkFlowRef.current) return

    if (!document.fullscreenElement) {
      networkFlowRef.current.requestFullscreen().then(() => {
        setIsFullscreen(true)
      }).catch((err: unknown) => {
        console.error('Failed to enter fullscreen:', err)
      })
    } else {
      document.exitFullscreen().then(() => {
        setIsFullscreen(false)
      }).catch((err: unknown) => {
        console.error('Failed to exit fullscreen:', err)
      })
    }
  }, [])

  // Listen for fullscreen change events (e.g., Escape key)
  useEffect(() => {
    const handleFullscreenChange = () => {
      setIsFullscreen(!!document.fullscreenElement)
    }
    document.addEventListener('fullscreenchange', handleFullscreenChange)
    return () => document.removeEventListener('fullscreenchange', handleFullscreenChange)
  }, [])

  // Byte volume stays the measure of how much; it just no longer pretends to
  // say where the bytes went.
  const totalRx = stats.reduce((sum, s) => sum + s.rx_bytes_per_sec, 0)
  const totalTx = stats.reduce((sum, s) => sum + s.tx_bytes_per_sec, 0)

  // Hubble reporting nothing and the query failing look the same to a reader:
  // in both cases the honest answer is "no data", not a zero.
  const flowsUnavailable = flowClassesError || (flowClasses?.stale ?? false)

  const totals = flowClasses?.totals ?? emptyClassTotals()

  // Filter topology based on showInfra toggle
  const filteredTopology = useMemo(() => {
    if (!topology) return null
    if (showInfra) return topology

    const filteredNodes = topology.nodes.filter(
      node => !INFRA_NAMESPACES.includes(node.id)
    )
    const filteredNodeIds = new Set(filteredNodes.map(n => n.id))
    const filteredEdges = topology.edges.filter(
      edge => filteredNodeIds.has(edge.source) && filteredNodeIds.has(edge.target)
    )

    return { nodes: filteredNodes, edges: filteredEdges }
  }, [topology, showInfra])

  if (hasError) {
    return (
      <div className="flex flex-col items-center justify-center h-[60vh] text-center">
        <AlertCircle size={64} className="text-red-500 mb-4" />
        <h2 className="text-2xl font-bold mb-2 text-gray-900 dark:text-white">Error Loading Network Data</h2>
        <p className="text-gray-500 dark:text-gray-400 max-w-md">
          Could not connect to network metrics. Check your connection and try again.
        </p>
        <button
          onClick={refresh}
          className="mt-4 px-4 py-2 bg-blue-600 hover:bg-blue-700 rounded-lg text-white"
        >
          Retry
        </button>
      </div>
    )
  }

  return (
    <div className="space-y-8">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold flex items-center gap-2 text-gray-900 dark:text-white">
            <Network className="text-blue-500 dark:text-blue-400" />
            Networking
          </h1>
          <p className="text-gray-500 dark:text-gray-400 mt-1">
            Network topology and traffic flow
          </p>
        </div>
        <div className="flex items-center gap-3">
          <ConnectionStatus mode={connectionMode} />
          <button
            onClick={refresh}
            disabled={isLoading}
            className="flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-700 rounded-lg transition-colors disabled:opacity-50 text-white"
          >
            <RefreshCw size={18} className={isLoading ? 'animate-spin' : ''} />
            Refresh
          </button>
        </div>
      </div>

      {/* Summary Cards */}
      <FlowClassCards
        totals={totals}
        stale={flowsUnavailable}
        totalRx={totalRx}
        totalTx={totalTx}
      />

      {/* Traffic by destination */}
      <div>
        <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
          <div>
            <h2 className="flex items-center gap-2 text-xl font-semibold text-gray-900 dark:text-white">
              Where traffic goes
            </h2>
            <p className="mt-1 text-sm text-gray-500 dark:text-gray-400">
              Bytes per second, split by destination
            </p>
          </div>
          <FlowWindowSelector value={flowWindow} onChange={setFlowWindow} />
        </div>

        {flowsUnavailable ? (
          <div className="flex items-start gap-3 rounded-xl border border-gray-200 bg-white p-6 text-sm dark:border-gray-700 dark:bg-gray-800">
            <Info size={18} className="mt-0.5 shrink-0 text-blue-500" />
            <div>
              <p className="font-medium text-gray-900 dark:text-white">
                No flow data available
              </p>
              <p className="mt-1 text-gray-500 dark:text-gray-400">
                Traffic classification needs Hubble metrics enabled on Cilium and scraped by
                VictoriaMetrics. Until then the byte volumes above still work, and graph edges
                come from Kubernetes Services rather than measured traffic.
              </p>
            </div>
          </div>
        ) : (
          <div className="space-y-4">
            {/* Composition at a glance; the chart's own legend carries the
                numbers, so repeating them here would just be noise. */}
            <FlowSplitBar totals={totals} height={10} />
            {flowClasses && <FlowClassChart series={flowClasses.series} />}
            {flowClassesLoading && !flowClasses && (
              <div className="flex h-[220px] items-center justify-center rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
                <div className="h-8 w-8 animate-spin rounded-full border-b-2 border-blue-500" />
              </div>
            )}
          </div>
        )}
      </div>

      {/* Network Flow Visualization */}
      <div>
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-xl font-semibold flex items-center gap-2 text-gray-900 dark:text-white">
            <Network size={20} />
            Network Flow
            <span className="text-sm font-normal text-gray-500 dark:text-gray-400 ml-2">
              Click app to focus • Hover to highlight • Drag to pan
            </span>
          </h2>
          <div className="flex items-center gap-2">
            <button
              onClick={() => setShowInfra(!showInfra)}
              className={`flex items-center gap-2 px-3 py-1.5 rounded-lg transition-colors text-sm ${
                showInfra
                  ? 'bg-purple-100 dark:bg-purple-900/30 text-purple-700 dark:text-purple-300'
                  : 'bg-gray-100 dark:bg-gray-700 text-gray-500 dark:text-gray-400 hover:bg-gray-200 dark:hover:bg-gray-600'
              }`}
              title={showInfra ? 'Hide infrastructure' : 'Show infrastructure (fluent-bit, victorialogs, victoriametrics)'}
            >
              <span>{showInfra ? 'Infra: ON' : 'Infra: OFF'}</span>
            </button>
            <button
              onClick={toggleFullscreen}
              className="flex items-center gap-2 px-3 py-1.5 bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 rounded-lg transition-colors text-gray-700 dark:text-gray-300"
              title={isFullscreen ? 'Exit fullscreen' : 'Enter fullscreen'}
            >
              {isFullscreen ? <Minimize2 size={18} /> : <Maximize2 size={18} />}
              <span className="text-sm">{isFullscreen ? 'Exit' : 'Fullscreen'}</span>
            </button>
          </div>
        </div>
        <div
          ref={networkFlowRef}
          className={`bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 overflow-hidden ${
            isFullscreen ? 'fixed inset-0 z-50 rounded-none border-0' : ''
          }`}
          style={{ height: isFullscreen ? '100vh' : '665px' }}
        >
          {/* Fullscreen header */}
          {isFullscreen && (
            <div className="absolute top-4 right-4 z-20">
              <button
                onClick={toggleFullscreen}
                className="flex items-center gap-2 px-3 py-2 bg-gray-900/80 hover:bg-gray-900 text-white rounded-lg transition-colors shadow-lg"
              >
                <Minimize2 size={18} />
                <span className="text-sm">Exit Fullscreen</span>
              </button>
            </div>
          )}
          {isLoading ? (
            <div className="flex items-center justify-center h-full">
              <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-500"></div>
            </div>
          ) : filteredTopology ? (
            <NetworkFlowVisualization topology={filteredTopology} isFullscreen={isFullscreen} vpnConfigs={vpnConfigs} />
          ) : null}
        </div>
      </div>

      {/* Network Statistics Table */}
      <div>
        <h2 className="text-xl font-semibold mb-4 flex items-center gap-2 text-gray-900 dark:text-white">
          <Network size={20} />
          Network Statistics
        </h2>
        {isLoading ? (
          <div className="flex items-center justify-center py-12">
            <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500"></div>
          </div>
        ) : stats.length > 0 || (flowClasses?.by_app.length ?? 0) > 0 ? (
          <NetworkStatsTable
            stats={stats}
            byApp={flowClasses?.by_app ?? []}
            stale={flowsUnavailable}
          />
        ) : (
          <div className="bg-white dark:bg-gray-800 rounded-lg p-8 text-center text-gray-500 dark:text-gray-400 border border-gray-200 dark:border-gray-700">
            <Network size={48} className="mx-auto mb-4 opacity-50" />
            <p>No network statistics available</p>
            <p className="text-sm mt-1">Install some apps to see their network usage</p>
          </div>
        )}
      </div>

      {/* Connection mode indicator */}
      <div className="text-center text-sm text-gray-500 dark:text-gray-500">
        {connectionMode === 'websocket' && 'Real-time updates via WebSocket'}
        {connectionMode === 'polling' && 'Updating via HTTP polling (reconnecting...)'}
        {connectionMode === 'disconnected' && 'Disconnected'}
      </div>
    </div>
  )
}
