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

function series(base: number) {
  return Array.from({ length: 16 }, (_, index) => ({
    t: observedAt - (15 - index) * 60,
    flows_per_sec: Number((base * (0.8 + (index % 4) * 0.1)).toFixed(3)),
  }))
}

const flowClasses: FlowClassResponse = {
  observed_at: observedAt,
  stale: false,
  totals: {
    internet: { flows_per_sec: 2.85, drops_per_sec: 0 },
    lan: { flows_per_sec: 3.43, drops_per_sec: 0 },
    cluster: { flows_per_sec: 61.42, drops_per_sec: 0.25 },
  },
  by_app: [
    {
      app: 'qbittorrent',
      internet: { flows_per_sec: 5.22, drops_per_sec: 0 },
      lan: { flows_per_sec: 0, drops_per_sec: 0 },
      cluster: { flows_per_sec: 1.1, drops_per_sec: 0 },
    },
    {
      app: 'host',
      internet: { flows_per_sec: 0, drops_per_sec: 0 },
      lan: { flows_per_sec: 3.43, drops_per_sec: 0 },
      cluster: { flows_per_sec: 0, drops_per_sec: 0 },
    },
  ],
  series: {
    internet: series(2.85),
    lan: series(3.43),
    cluster: series(61.42),
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

test('the page separates internet, LAN and in-cluster traffic', async ({ page }) => {
  await page.goto('/networking')
  await expect(page.getByRole('heading', { name: 'Networking' })).toBeVisible()

  // The three classes, each with its own measured rate. Each appears twice —
  // once on its card, once in the split bar's legend — so take the first.
  await expect(page.getByText('2.85/s').first()).toBeVisible()
  await expect(page.getByText('3.43/s').first()).toBeVisible()
  await expect(page.getByText('61.4/s').first()).toBeVisible()

  // In-cluster dominates, and the card says so as a share rather than leaving
  // the reader to divide three numbers.
  await expect(page.getByText('91% of flows')).toBeVisible()

  // A denied flow is surfaced where the denial happened.
  await expect(page.getByText('0.25/s dropped')).toBeVisible()

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

  await expect(page.getByText('Measured traffic — thicker is busier')).toBeVisible()
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
          internet: { flows_per_sec: 0, drops_per_sec: 0 },
          lan: { flows_per_sec: 0, drops_per_sec: 0 },
          cluster: { flows_per_sec: 0, drops_per_sec: 0 },
        },
        by_app: [],
        series: { internet: [], lan: [], cluster: [] },
      } satisfies FlowClassResponse,
    }))
  }

  await page.goto('/networking')
  await expect(page.getByRole('heading', { name: 'Networking' })).toBeVisible()

  await expect(page.getByText('No data')).toHaveCount(3)
  await expect(page.getByText('No flow data available')).toBeVisible()
  // The byte volumes are a different source and keep working.
  await expect(page.getByText('Total Receive')).toBeVisible()
})
