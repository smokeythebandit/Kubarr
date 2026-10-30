import { test, expect } from './fixtures'
import type {
  FlowClassResponse,
  NetworkStats,
  NetworkTopology,
} from '../../src/api/networking'

/**
 * Seeded with the rates actually measured on the production cluster, so the
 * page is exercised against numbers it will really be handed: internet and LAN
 * within a factor of two of each other, in-cluster an order of magnitude above
 * both, and qBittorrent carrying nearly all of the internet traffic.
 */
const observedAt = 1793292168

function series(bytes: number, flows = 0) {
  return Array.from({ length: 16 }, (_, index) => {
    const scale = 0.8 + (index % 4) * 0.1
    return {
      t: observedAt - (15 - index) * 60,
      flows_per_sec: Number((flows * scale).toFixed(3)),
      bytes_per_sec: Number((bytes * scale).toFixed(0)),
    }
  })
}

/** Storage has bytes and no connection count; the others have both. */
function rates(bytes_per_sec: number, flows_per_sec = 0, drops_per_sec = 0) {
  return { bytes_per_sec, flows_per_sec, drops_per_sec }
}

const flowClasses: FlowClassResponse = {
  observed_at: observedAt,
  stale: false,
  totals: {
    internet: rates(2_949_120, 2.85),
    lan: rates(307_200, 3.43),
    cluster: rates(3_250_586, 61.42, 0.25),
    // Measured on the real cluster under load: 38.5 MB/s off the NFS share.
    storage: rates(40_395_593),
  },
  by_app: [
    {
      app: 'qbittorrent',
      internet: rates(0, 5.22),
      lan: rates(0),
      cluster: rates(0, 1.1),
      storage: rates(0),
    },
    {
      app: 'host',
      internet: rates(0),
      lan: rates(0, 3.43),
      cluster: rates(0),
      storage: rates(0),
    },
  ],
  series: {
    internet: series(2_949_120, 2.85),
    lan: series(307_200, 3.43),
    cluster: series(3_250_586, 61.42),
    storage: series(40_395_593),
  },
}

const stats: NetworkStats[] = [
  {
    namespace: 'qbittorrent',
    app_name: 'qBittorrent',
    rx_bytes_per_sec: 2_400_000,
    tx_bytes_per_sec: 850_000,
    rx_packets_per_sec: 1800,
    tx_packets_per_sec: 900,
    rx_errors_per_sec: 0,
    tx_errors_per_sec: 0,
    rx_dropped_per_sec: 0,
    tx_dropped_per_sec: 0,
    pod_count: 1,
  },
]

const topology: NetworkTopology = {
  nodes: [
    {
      id: 'internet',
      name: 'Internet',
      type: 'external',
      rx_bytes_per_sec: 2_400_000,
      tx_bytes_per_sec: 850_000,
      total_traffic: 3_250_000,
      pod_count: 0,
      color: '#3b82f6',
    },
    {
      id: 'qbittorrent',
      name: 'qBittorrent',
      type: 'app',
      rx_bytes_per_sec: 2_400_000,
      tx_bytes_per_sec: 850_000,
      total_traffic: 3_250_000,
      pod_count: 1,
      color: '#10b981',
    },
    {
      id: 'sonarr',
      name: 'Sonarr',
      type: 'app',
      rx_bytes_per_sec: 0,
      tx_bytes_per_sec: 0,
      total_traffic: 0,
      pod_count: 1,
      color: '#10b981',
    },
  ],
  edges: [
    {
      source: 'qbittorrent',
      target: 'internet',
      type: 'external',
      label: 'internet',
      flows_per_sec: 5.22,
      bytes_per_sec: 2_949_120,
      observed: true,
    },
    // Configured by a Service, never used. It must stay on the graph without
    // claiming traffic.
    {
      source: 'sonarr',
      target: 'qbittorrent',
      type: 'service',
      port: 8080,
      protocol: 'TCP',
      label: 'sonarr -> qbittorrent:8080',
      observed: false,
    },
  ],
}

test.beforeEach(async ({ api, context }) => {
  api.get('/api/networking/topology', topology)
  api.get('/api/networking/stats', stats)
  api.get('/api/vpn/apps', { configs: [] })
  api.get('/api/apps/catalog/icons', {})
  for (const app of ['qbittorrent', 'sonarr', 'host']) {
    api.on('GET', `/api/apps/catalog/${app}/icon`, () => ({
      status: 404,
      json: { detail: 'No fixture icon' },
    }))
  }
  for (const window of ['15m', '1h', '6h', '24h']) {
    api.on('GET', `/api/networking/flow-classes?window=${window}`, () => ({
      json: flowClasses,
    }))
  }

  // The page opens a metrics socket and falls back to HTTP polling when it
  // fails. Closing it here is what puts the page on the polling path, which is
  // the path these fixtures feed. Registered after the shared fixture's own
  // handler so this one matches first.
  await context.routeWebSocket('**/api/networking/ws', socket => socket.close())
})

test('the page separates internet, LAN, in-cluster and storage traffic', async ({ page }) => {
  await page.goto('/networking')
  await expect(page.getByRole('heading', { name: 'Networking' })).toBeVisible()

  // Four classes, each with its own measured byte rate. Each appears twice —
  // once on its card, once in the split bar's legend — so take the first.
  await expect(page.getByText('2.81 MB/s').first()).toBeVisible()
  await expect(page.getByText('300 KB/s').first()).toBeVisible()
  await expect(page.getByText('3.1 MB/s').first()).toBeVisible()
  // Storage is the one Cilium cannot see at all; it comes from the kernel.
  await expect(page.getByText('38.52 MB/s').first()).toBeVisible()

  // Shares are of the three destinations. Storage is the same bytes written
  // down after arriving, so it is not in the denominator and claims no share.
  await expect(page.getByText('50% of traffic')).toBeVisible()
  await expect(page.getByText('read + written')).toBeVisible()

  // Connection counts survive where one exists, and storage shows none.
  await expect(page.getByText('61.4/s conn')).toBeVisible()

  // A denied connection is surfaced where the denial happened.
  await expect(page.getByText('0.25/s denied')).toBeVisible()

  await expect(page.getByRole('heading', { name: 'Where traffic goes' })).toBeVisible()
})

test('switching the window refetches that window', async ({ page, api }) => {
  await page.goto('/networking')
  await expect(page.getByRole('heading', { name: 'Networking' })).toBeVisible()

  const windows = page.getByRole('group', { name: 'Time window' })
  await windows.getByRole('button', { name: '6h' }).click()

  await expect
    .poll(() => api.calls.some(call => call.includes('flow-classes?window=6h')))
    .toBe(true)
  await expect(windows.getByRole('button', { name: '6h' })).toHaveAttribute(
    'aria-pressed',
    'true'
  )
})

test('the per-app table keeps a flow source that has no pod', async ({ page }) => {
  await page.goto('/networking')

  const table = page.getByRole('table')
  await expect(table).toBeVisible()

  // qBittorrent has both measures.
  const qbit = table.getByRole('row', { name: /qBittorrent/ })
  await expect(qbit).toContainText('5.22/s')

  // The node's own LAN traffic has no pod behind it, so its byte columns are
  // blank — but dropping the row would hide where LAN traffic comes from.
  const host = table.getByRole('row', { name: /host/ })
  await expect(host).toContainText('3.43/s')
  await expect(host).toContainText('—')
})

test('an unmeasured edge is drawn but not animated', async ({ page }) => {
  await page.goto('/networking')

  await expect(page.getByText('Measured traffic — thicker moves more data')).toBeVisible()
  await expect(page.getByText('Can talk, nothing seen')).toBeVisible()

  // The measured edge carries particles; the Service-only edge is dashed and
  // carries none. React renders the dash as a CSS style rather than an SVG
  // attribute, so read it back from the computed style.
  const edgePaths = page.locator('g.react-flow__edge path.react-flow__edge-path')
  await expect(edgePaths).toHaveCount(2)

  const dashes = await edgePaths.evaluateAll(paths =>
    paths.map(path => getComputedStyle(path).strokeDasharray)
  )
  const dashed = dashes.filter(dash => dash && dash !== 'none')
  expect(dashed).toHaveLength(1)

  // Exactly one edge animates, and it is the measured one: six particles, all
  // on the same path.
  const motions = page.locator('g.react-flow__edge animateMotion')
  await expect(motions).toHaveCount(6)
})

test('with no Hubble data the page says so instead of showing zeros', async ({ page, api }) => {
  for (const window of ['15m', '1h', '6h', '24h']) {
    api.on('GET', `/api/networking/flow-classes?window=${window}`, () => ({
      json: {
        observed_at: observedAt,
        stale: true,
        totals: {
          internet: rates(0),
          lan: rates(0),
          cluster: rates(0),
          storage: rates(0),
        },
        by_app: [],
        series: { internet: [], lan: [], cluster: [], storage: [] },
      } satisfies FlowClassResponse,
    }))
  }

  await page.goto('/networking')
  await expect(page.getByRole('heading', { name: 'Networking' })).toBeVisible()

  await expect(page.getByText('No data')).toHaveCount(4)
  await expect(page.getByText('No flow data available')).toBeVisible()
  // The byte volumes are a different source and keep working.
  await expect(page.getByText('Total Receive')).toBeVisible()
})
