/**
 * Precache utility for preloading page data on hover
 * This allows pages to load instantly when navigated to
 */

import { getCurrentUser } from '../api/users'
import { appsApi } from '../api/apps'
import { monitoringApi } from '../api/monitoring'

// Simple in-memory cache with expiration
interface CacheEntry<T> {
  data: T
  timestamp: number
}

const cache = new Map<string, CacheEntry<unknown>>()
const CACHE_TTL = 30000 // 30 seconds

function getCached<T>(key: string): T | null {
  const entry = cache.get(key) as CacheEntry<T> | undefined
  if (!entry) return null
  if (Date.now() - entry.timestamp > CACHE_TTL) {
    cache.delete(key)
    return null
  }
  return entry.data
}

function setCache<T>(key: string, data: T): void {
  cache.set(key, { data, timestamp: Date.now() })
}

// Track in-flight requests to avoid duplicate fetches
const inFlight = new Set<string>()

async function fetchIfNeeded<T>(key: string, fetcher: () => Promise<T>): Promise<T | null> {
  // Return cached data if available
  const cached = getCached<T>(key)
  if (cached) return cached

  // Avoid duplicate in-flight requests
  if (inFlight.has(key)) return null

  try {
    inFlight.add(key)
    const data = await fetcher()
    setCache(key, data)
    return data
  } catch {
    // Silently fail - this is just a prefetch
    return null
  } finally {
    inFlight.delete(key)
  }
}

/**
 * Precache dashboard data - call this on hover before navigating to dashboard
 */
export async function precacheDashboard(): Promise<void> {
  // Fetch all in parallel
  await Promise.all([
    fetchIfNeeded('user', getCurrentUser),
    fetchIfNeeded('catalog', appsApi.getCatalog),
    fetchIfNeeded('installed', appsApi.getInstalled),
    fetchIfNeeded('metricsAvailable', monitoringApi.checkMetricsAvailable),
    fetchIfNeeded('clusterMetrics', monitoringApi.getClusterMetrics),
  ])

  // After we know installed apps, prefetch their statuses
  const installed = getCached<string[]>('installed')
  if (installed && installed.length > 0) {
    await Promise.all(
      installed.map(appName =>
        fetchIfNeeded(`podStatus:${appName}`, () => monitoringApi.getPodStatus(appName))
      )
    )
  }
}

/**
 * Precache user data - lighter weight precache for account switching
 */
export async function precacheUser(): Promise<void> {
  await fetchIfNeeded('user', getCurrentUser)
}

/**
 * Get precached data if available
 */
export function getPrecached<T>(key: string): T | null {
  return getCached<T>(key)
}

/**
 * Clear all precached data
 */
export function clearPrecache(): void {
  cache.clear()
}
