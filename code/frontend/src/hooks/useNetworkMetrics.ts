import { useQuery } from '@tanstack/react-query'
import { networkingApi, NetworkTopology, NetworkStats } from '../api/networking'

export type ConnectionMode = 'live' | 'disconnected'

interface UseNetworkMetricsResult {
  topology: NetworkTopology | null
  stats: NetworkStats[]
  connectionMode: ConnectionMode
  error: Error | null
}

// Both endpoints read cAdvisor counters from VictoriaMetrics, which scrapes
// them once a minute over a five-minute rate window. Polling faster than this
// only re-fetches the same numbers; Hubble edge rates move a little quicker,
// which is what keeps this below the scrape interval.
const POLLING_INTERVAL = 15000

/**
 * Topology and per-app stats for the networking page, polled over HTTP.
 *
 * This replaced a WebSocket fed by a per-process broadcaster. The socket
 * carried its own, older topology — Service edges only, no measured traffic —
 * so the graph drew every link as unobserved whenever the socket was up, and
 * only showed real traffic when it fell back to these same endpoints.
 */
export function useNetworkMetrics(): UseNetworkMetricsResult {
  const topologyQuery = useQuery({
    queryKey: ['networking', 'topology'],
    queryFn: networkingApi.getTopology,
    refetchInterval: POLLING_INTERVAL,
  })

  const statsQuery = useQuery({
    queryKey: ['networking', 'stats'],
    queryFn: networkingApi.getStats,
    refetchInterval: POLLING_INTERVAL,
  })

  const error = topologyQuery.error ?? statsQuery.error ?? null

  return {
    topology: topologyQuery.data ?? null,
    stats: statsQuery.data ?? [],
    // Still showing the last good data counts as live until a refetch fails.
    connectionMode: error ? 'disconnected' : 'live',
    error,
  }
}
