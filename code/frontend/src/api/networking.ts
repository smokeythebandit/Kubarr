import apiClient from './client'

export interface NetworkNode {
  id: string
  name: string
  type: 'app' | 'external'
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

export const networkingApi = {
  getTopology: async (): Promise<NetworkTopology> => {
    const response = await apiClient.get<NetworkTopology>('/networking/topology')
    return response.data
  },

  getStats: async (): Promise<NetworkStats[]> => {
    const response = await apiClient.get<NetworkStats[]>('/networking/stats')
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
