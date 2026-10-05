import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, within } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import MonitoringPage from './MonitoringPage'

const mocks = vi.hoisted(() => ({ getAppDetailMetrics: vi.fn(), getGpuMetrics: vi.fn() }))

vi.mock('../api/monitoring', () => ({
  monitoringApi: {
    checkMetricsAvailable: vi.fn().mockResolvedValue({ available: true }),
    getClusterMetrics: vi.fn().mockResolvedValue({}),
    getAppMetrics: vi.fn().mockResolvedValue([]),
    getClusterNetworkHistory: vi.fn().mockResolvedValue({}),
    getClusterMetricsHistory: vi.fn().mockResolvedValue({}),
    getAppDetailMetrics: (...args: unknown[]) => mocks.getAppDetailMetrics(...args),
    getGpuMetrics: () => mocks.getGpuMetrics(),
  },
}))

vi.mock('../components/AppIcon', () => ({
  AppIcon: () => <span>App icon</span>,
}))

function renderPage() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={['/?app=qbittorrent']}>
        <MonitoringPage />
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

const pod = {
  name: 'qbittorrent-abc',
  namespace: 'qbittorrent',
  status: 'Running',
  ready: true,
  restarts: 3,
  age: '2h',
  node: 'node-1',
  ip: '10.0.0.1',
}

describe('MonitoringPage app detail pods', () => {
  beforeEach(() => {
    mocks.getAppDetailMetrics.mockReset()
    mocks.getGpuMetrics.mockReset().mockResolvedValue({ available: true, devices: [] })
  })

  it('nests the main container and sidecars within one pod without inflating the pod count', async () => {
    mocks.getAppDetailMetrics.mockResolvedValue({
      pods: [{
        ...pod,
        containers: [
          { name: 'qbittorrent', ready: true, restart_count: 0, state: 'running' },
          { name: 'gluetun', ready: false, restart_count: 2, state: 'waiting' },
          { name: 'exporter', ready: true, restart_count: 1, state: 'running' },
        ],
      }],
    })

    renderPage()
    fireEvent.click(await screen.findByRole('button', { name: 'Pods (1)' }))

    const table = screen.getByRole('table', { name: 'Pods' })
    expect(within(table).getAllByRole('row')).toHaveLength(2) // header + one pod
    const row = within(table).getAllByRole('row')[1]
    expect(within(row).getByText('qbittorrent-abc')).toBeInTheDocument()
    expect(within(row).getByLabelText('Pod ready')).toBeInTheDocument()

    const containers = within(row).getByRole('list', { name: 'Containers in pod qbittorrent-abc' })
    const entries = within(containers).getAllByRole('listitem')
    expect(entries).toHaveLength(3)
    const expected = [
      ['qbittorrent', 'Ready', 0],
      ['gluetun', 'Not ready', 2],
      ['exporter', 'Ready', 1],
    ] as const
    entries.forEach((entry, i) => {
      const [name, readiness, restarts] = expected[i]
      expect(within(entry).getByText(name)).toBeInTheDocument()
      expect(within(entry).getByText(readiness)).toBeInTheDocument()
      expect(within(entry).getByText(`Restarts: ${restarts}`)).toBeInTheDocument()
    })
    expect(within(row).getAllByRole('cell')[5]).toHaveTextContent('3') // pod total remains independent
  })

  it('renders pods from an older backend without a containers field', async () => {
    mocks.getAppDetailMetrics.mockResolvedValue({ pods: [pod] })
    renderPage()
    fireEvent.click(await screen.findByRole('button', { name: 'Pods (1)' }))

    const row = screen.getByRole('row', { name: /qbittorrent-abc/ })
    expect(within(row).getByText('qbittorrent-abc')).toBeInTheDocument()
    expect(within(row).queryByRole('list')).not.toBeInTheDocument()
  })
})

describe('MonitoringPage GPU telemetry', () => {
  beforeEach(() => {
    mocks.getGpuMetrics.mockReset()
    mocks.getAppDetailMetrics.mockResolvedValue({ pods: [] })
  })

  it('shows per-device telemetry with real zero utilization and memory', async () => {
    mocks.getGpuMetrics.mockResolvedValue({ available: true, devices: [{
      node: 'worker-a', vendor: 'NVIDIA', device: 'GPU-123',
      utilization_percent: 0, memory_used_bytes: 0, memory_total_bytes: 1073741824,
    }] })
    renderPage()
    const table = await screen.findByRole('table', { name: 'GPU devices' })
    expect(within(table).getByText('worker-a')).toBeInTheDocument()
    expect(within(table).getByText('NVIDIA')).toBeInTheDocument()
    expect(within(table).getByText('0.0%')).toBeInTheDocument()
    expect(within(table).getByText('0 B / 1 GB')).toBeInTheDocument()
  })

  it('shows missing fields and no fresh devices as unavailable', async () => {
    mocks.getGpuMetrics.mockResolvedValueOnce({ available: true, devices: [{
      node: 'worker-b', vendor: 'NVIDIA', device: 'GPU-456',
      utilization_percent: null, memory_used_bytes: null, memory_total_bytes: null,
    }] })
    renderPage()
    const row = await screen.findByRole('row', { name: /worker-b/ })
    expect(within(row).getAllByText('Unavailable')).toHaveLength(2)
  })

  it('keeps Resources usable on a GPU query outage', async () => {
    mocks.getGpuMetrics.mockResolvedValue({ available: false, devices: [] })
    renderPage()
    expect(await screen.findByText(/GPU telemetry unavailable \(metrics query failed\)/)).toBeInTheDocument()
    expect(screen.getByText('Cluster Overview')).toBeInTheDocument()
  })
})
