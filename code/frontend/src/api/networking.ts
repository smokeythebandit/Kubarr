import apiClient from './client'

export interface NetworkNode {
  id: string
  name: string
  type: 'app' | 'external' | 'lan' | 'infra'
  rx_bytes_per_sec: number
  tx_bytes_per_sec: number
  total_traffic: number
  pod_count: number
  color: string
}

export interface NetworkEdge {
  source: string
  target: string
  type: string
  port?: number
  protocol?: string
  label: string
  /** Measured flow-event rate, present only when Hubble saw traffic here. */
  flows_per_sec?: number
  /** False for a Service-derived edge with no traffic behind it. */
  observed: boolean
}

export interface NetworkTopology {
  nodes: NetworkNode[]
  edges: NetworkEdge[]
}

export interface NetworkStats {
  namespace: string
  app_name: string
  rx_bytes_per_sec: number
  tx_bytes_per_sec: number
  rx_packets_per_sec: number
  tx_packets_per_sec: number
  rx_errors_per_sec: number
  tx_errors_per_sec: number
  rx_dropped_per_sec: number
  tx_dropped_per_sec: number
  pod_count: number
}

// ============================================================================
// Flow classification
//
// Hubble counts flow *events*, not bytes — its flow protocol carries no byte
// field. So everything below is in flows per second, and the byte figures above
// stay the only measure of volume. Mixing the two into one "traffic" number
// would read as measurement while being a guess.
// ============================================================================

/** Where traffic is headed. */
export type FlowClassName = 'internet' | 'lan' | 'cluster' | 'storage'

export const FLOW_CLASS_NAMES: FlowClassName[] = ['internet', 'lan', 'cluster', 'storage']

/**
 * Storage is measured, but not by the same thing as the rest.
 *
 * NFS is mounted by the node's own kernel rather than by the pods that use it,
 * so it never crosses a Cilium endpoint and neither Hubble nor the conntrack
 * map sees a byte of it. Its bytes come from the kernel's NFS client counters,
 * and it has no connection count at all.
 */
export const CLASSES_WITH_FLOW_COUNTS: FlowClassName[] = ['internet', 'lan', 'cluster']

export interface ClassRates {
  flows_per_sec: number
  drops_per_sec: number
  bytes_per_sec: number
}

export type ClassTotals = Record<FlowClassName, ClassRates>

export interface AppClassRates extends ClassTotals {
  app: string
}

export interface FlowSeriesPoint {
  t: number
  flows_per_sec: number
  bytes_per_sec: number
}

export type FlowSeries = Record<FlowClassName, FlowSeriesPoint[]>

export interface FlowClassResponse {
  observed_at: number
  /** True when Hubble reports nothing. Say so; never draw a convincing zero. */
  stale: boolean
  totals: ClassTotals
  by_app: AppClassRates[]
  series: FlowSeries
}

/** Windows the backend accepts, in the order they're offered. */
export const FLOW_WINDOWS = ['15m', '1h', '6h', '24h'] as const
export type FlowWindow = (typeof FLOW_WINDOWS)[number]

export const networkingApi = {
  getTopology: async (): Promise<NetworkTopology> => {
    const response = await apiClient.get<NetworkTopology>('/networking/topology')
    return response.data
  },

  getStats: async (): Promise<NetworkStats[]> => {
    const response = await apiClient.get<NetworkStats[]>('/networking/stats')
    return response.data
  },

  getFlowClasses: async (window: FlowWindow = '15m'): Promise<FlowClassResponse> => {
    const response = await apiClient.get<FlowClassResponse>('/networking/flow-classes', {
      params: { window },
    })
    return response.data
  },
}

// Helper function to format bytes
export function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B'
  const k = 1024
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.floor(Math.log(bytes) / Math.log(k))
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(2))} ${sizes[i]}`
}

// Helper function to format bandwidth
export function formatBandwidth(bytesPerSec: number): string {
  if (bytesPerSec === 0) return '0 B/s'
  const k = 1024
  const sizes = ['B/s', 'KB/s', 'MB/s', 'GB/s']
  const i = Math.floor(Math.log(bytesPerSec) / Math.log(k))
  return `${parseFloat((bytesPerSec / Math.pow(k, i)).toFixed(2))} ${sizes[i]}`
}

// Helper function to format packets
export function formatPackets(packetsPerSec: number): string {
  if (packetsPerSec === 0) return '0 pps'
  if (packetsPerSec < 1000) return `${packetsPerSec.toFixed(1)} pps`
  if (packetsPerSec < 1000000) return `${(packetsPerSec / 1000).toFixed(2)} Kpps`
  return `${(packetsPerSec / 1000000).toFixed(2)} Mpps`
}

/**
 * Flow rates are small numbers that matter at low magnitudes: 0.4/s of LAN
 * traffic is a real reading, so it must not round to "0".
 */
export function formatFlowRate(flowsPerSec: number): string {
  if (flowsPerSec === 0) return '0/s'
  if (flowsPerSec < 10) return `${flowsPerSec.toFixed(2)}/s`
  if (flowsPerSec < 1000) return `${flowsPerSec.toFixed(1)}/s`
  return `${(flowsPerSec / 1000).toFixed(2)}k/s`
}

export const EMPTY_CLASS_RATES: ClassRates = {
  flows_per_sec: 0,
  drops_per_sec: 0,
  bytes_per_sec: 0,
}

/** Zeroed totals, built from the class list so a new class is never missed. */
export function emptyClassTotals(): ClassTotals {
  return Object.fromEntries(
    FLOW_CLASS_NAMES.map(name => [name, { ...EMPTY_CLASS_RATES }])
  ) as ClassTotals
}

/** Sum of the classes that have a connection count. */
export function totalFlowRate(totals: ClassTotals): number {
  return CLASSES_WITH_FLOW_COUNTS.reduce(
    (sum, name) => sum + totals[name].flows_per_sec,
    0
  )
}

/** Sum of measured bytes across every class, storage included. */
export function totalByteRate(totals: ClassTotals): number {
  return FLOW_CLASS_NAMES.reduce((sum, name) => sum + totals[name].bytes_per_sec, 0)
}
