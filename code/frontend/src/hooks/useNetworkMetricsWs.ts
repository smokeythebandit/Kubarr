import { useState, useEffect, useRef, useCallback } from 'react'
import { useQuery } from '@tanstack/react-query'
import { networkingApi, NetworkTopology, NetworkStats } from '../api/networking'

export type ConnectionMode = 'websocket' | 'polling' | 'disconnected'

interface NetworkMetricsMessage {
  type: 'network_metrics'
  timestamp: number
  topology: NetworkTopology
  stats: NetworkStats[]
}

interface UseNetworkMetricsWsResult {
  topology: NetworkTopology | null
  stats: NetworkStats[]
  isConnected: boolean
  connectionMode: ConnectionMode
  error: Error | null
}

const MAX_RECONNECT_DELAY = 30000 // 30 seconds
const INITIAL_RECONNECT_DELAY = 1000 // 1 second
const POLLING_INTERVAL = 1000 // 1 second

export function useNetworkMetricsWs(): UseNetworkMetricsWsResult {
  const [topology, setTopology] = useState<NetworkTopology | null>(null)
  const [stats, setStats] = useState<NetworkStats[]>([])
  const [connectionMode, setConnectionMode] = useState<ConnectionMode>('disconnected')
  const [error, setError] = useState<Error | null>(null)

  const wsRef = useRef<WebSocket | null>(null)
  const reconnectTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const reconnectDelayRef = useRef(INITIAL_RECONNECT_DELAY)
  const mountedRef = useRef(true)

  // Fallback to HTTP polling when WebSocket is not connected
  const shouldPoll = connectionMode !== 'websocket'

  const { data: polledTopology } = useQuery({
    queryKey: ['networking', 'topology', 'polling'],
    queryFn: networkingApi.getTopology,
    refetchInterval: shouldPoll ? POLLING_INTERVAL : false,
    enabled: shouldPoll,
  })

  const { data: polledStats } = useQuery({
    queryKey: ['networking', 'stats', 'polling'],
    queryFn: networkingApi.getStats,
    refetchInterval: shouldPoll ? POLLING_INTERVAL : false,
    enabled: shouldPoll,
  })

  // Use polled data as fallback when not using WebSocket
  useEffect(() => {
    if (shouldPoll) {
      if (polledTopology) {
        setTopology(polledTopology)
      }
      if (polledStats) {
        setStats(polledStats)
      }
      if (polledTopology || polledStats) {
        setConnectionMode('polling')
      }
    }
  }, [shouldPoll, polledTopology, polledStats])

  const connect = useCallback(() => {
    if (!mountedRef.current) return
    if (wsRef.current?.readyState === WebSocket.OPEN) return

    // Build WebSocket URL from current location
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    const wsUrl = `${protocol}//${window.location.host}/api/networking/ws`

    try {
      const ws = new WebSocket(wsUrl)
      wsRef.current = ws

      ws.onopen = () => {
        if (!mountedRef.current) return
        console.log('Network metrics WebSocket connected')
        setConnectionMode('websocket')
        setError(null)
        reconnectDelayRef.current = INITIAL_RECONNECT_DELAY // Reset delay on successful connection
      }

      ws.onmessage = (event) => {
        if (!mountedRef.current) return
        try {
          const message: NetworkMetricsMessage = JSON.parse(event.data)
          if (message.type === 'network_metrics') {
            setTopology(message.topology)
            setStats(message.stats)
          }
        } catch (e) {
          console.error('Failed to parse WebSocket message:', e)
        }
      }

      ws.onerror = (event) => {
        console.error('Network metrics WebSocket error:', event)
        setError(new Error('WebSocket connection error'))
      }

      ws.onclose = (event) => {
        if (!mountedRef.current) return
        console.log('Network metrics WebSocket closed:', event.code, event.reason)
        wsRef.current = null

        // Switch to polling immediately
        setConnectionMode('polling')

        // Schedule reconnection with exponential backoff
        if (mountedRef.current) {
          const delay = reconnectDelayRef.current
          console.log(`Scheduling WebSocket reconnection in ${delay}ms`)

          reconnectTimeoutRef.current = setTimeout(() => {
            if (mountedRef.current) {
              connect()
            }
          }, delay)

          // Exponential backoff (max 30 seconds)
          reconnectDelayRef.current = Math.min(delay * 2, MAX_RECONNECT_DELAY)
        }
      }
    } catch (e) {
      console.error('Failed to create WebSocket:', e)
      setError(e instanceof Error ? e : new Error('Failed to create WebSocket'))
      setConnectionMode('polling')

      // Schedule reconnection
      reconnectTimeoutRef.current = setTimeout(() => {
        if (mountedRef.current) {
          connect()
        }
      }, reconnectDelayRef.current)

      reconnectDelayRef.current = Math.min(reconnectDelayRef.current * 2, MAX_RECONNECT_DELAY)
    }
  }, [])

  useEffect(() => {
    mountedRef.current = true
    connect()

    return () => {
      mountedRef.current = false

      if (reconnectTimeoutRef.current) {
        clearTimeout(reconnectTimeoutRef.current)
        reconnectTimeoutRef.current = null
      }

      if (wsRef.current) {
        wsRef.current.close()
        wsRef.current = null
      }
    }
  }, [connect])

  return {
    topology,
    stats,
    isConnected: connectionMode !== 'disconnected',
    connectionMode,
    error,
  }
}
