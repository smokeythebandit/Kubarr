import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { AppConfig, AppOperation, AppState } from '../types'
import AppsPage from './AppsPage'

const mocks = vi.hoisted(() => ({
  restart: vi.fn(),
  getOperations: vi.fn(),
  listProviders: vi.fn(),
  getVpnConfig: vi.fn(),
  getPublicIp: vi.fn(),
  getForwardedPort: vi.fn(),
  assignVpn: vi.fn(),
  removeVpn: vi.fn(),
  refreshAppStatuses: vi.fn(),
  canRestart: true,
  canViewVpn: false,
  canManageVpn: false,
}))

vi.mock('../api/apps', () => ({
  appsApi: {
    getSyncStatus: vi.fn().mockResolvedValue({ last_synced: null }),
    getOperations: (...args: unknown[]) => mocks.getOperations(...args),
    restart: (...args: unknown[]) => mocks.restart(...args),
    syncCatalog: vi.fn(),
    install: vi.fn(),
    update: vi.fn(),
    delete: vi.fn(),
    logAccess: vi.fn().mockResolvedValue(undefined),
  },
}))

vi.mock('../api/monitoring', () => ({
  monitoringApi: { getEndpoints: vi.fn().mockResolvedValue([]) },
}))

vi.mock('../api/vpn', () => ({
  vpnApi: { listProviders: (...args: unknown[]) => mocks.listProviders(...args) },
  appVpnApi: {
    getConfig: (...args: unknown[]) => mocks.getVpnConfig(...args),
    getPublicIp: (...args: unknown[]) => mocks.getPublicIp(...args),
    getForwardedPort: (...args: unknown[]) => mocks.getForwardedPort(...args),
    assignVpn: (...args: unknown[]) => mocks.assignVpn(...args),
    removeVpn: (...args: unknown[]) => mocks.removeVpn(...args),
  },
}))

vi.mock('../components/AppIcon', () => ({
  AppIcon: ({ appName }: { appName: string }) => <span>{appName} icon</span>,
  useIconColors: () => [],
}))

vi.mock('../components/vpn/VpnProviderForm', () => ({
  VpnProviderForm: () => null,
}))

vi.mock('../contexts/AuthContext', () => ({
  useAuth: () => ({
    hasPermission: (permission: string) => {
      if (permission === 'apps.restart') return mocks.canRestart
      if (permission === 'vpn.view') return mocks.canViewVpn
      if (permission === 'vpn.manage') return mocks.canManageVpn
      return false
    },
  }),
}))

const app: AppConfig = {
  name: 'sonarr',
  display_name: 'Sonarr',
  description: 'TV manager',
  icon: null,
  version: '1.0.0',
  container_image: 'sonarr:latest',
  default_port: 8989,
  resource_requirements: {
    cpu_request: '100m',
    cpu_limit: '1',
    memory_request: '128Mi',
    memory_limit: '512Mi',
  },
  environment_variables: {},
  volumes: [],
  category: 'media-manager',
  is_system: false,
  is_hidden: false,
  is_browseable: true,
}

const appState: AppState = {
  app_name: 'sonarr',
  namespace: 'sonarr',
  desired_state: 'installed',
  observed_state: 'installed',
  healthy: true,
  message: null,
  installed_chart_version: '1.0.0',
  available_chart_version: '1.0.0',
  update_available: false,
  last_operation_id: null,
  last_checked_at: null,
  updated_at: new Date().toISOString(),
}

const queuedRestart: AppOperation = {
  id: 'restart-1',
  app_name: 'sonarr',
  operation: 'restart',
  status: 'queued',
  message: null,
  error: null,
  attempts: 0,
  created_by: 1,
  created_at: new Date().toISOString(),
  started_at: null,
  finished_at: null,
  updated_at: new Date().toISOString(),
}

vi.mock('../contexts/MonitoringContext', () => ({
  useMonitoring: () => ({
    catalog: [app],
    installedApps: ['sonarr'],
    appStates: { sonarr: appState },
    appStatuses: { sonarr: { healthy: true, loading: false, pods: [] } },
    refreshAppStatuses: mocks.refreshAppStatuses,
  }),
}))

function renderPage() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  })

  const view = render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter>
        <AppsPage />
      </MemoryRouter>
    </QueryClientProvider>,
  )

  return { ...view, queryClient }
}

describe('AppsPage restart action', () => {
  beforeEach(() => {
    mocks.canRestart = true
    mocks.canViewVpn = false
    mocks.canManageVpn = false
    mocks.getOperations.mockResolvedValue([])
    mocks.restart.mockReset()
    mocks.listProviders.mockReset()
    mocks.getVpnConfig.mockReset()
    mocks.getPublicIp.mockReset()
    mocks.getForwardedPort.mockReset()
    mocks.assignVpn.mockReset()
    mocks.removeVpn.mockReset()
    mocks.refreshAppStatuses.mockReset()
  })

  it('restarts the selected installed app and disables the action while the request is pending', async () => {
    mocks.restart.mockReturnValue(new Promise<AppOperation>(() => {}))
    renderPage()

    const restart = await screen.findByRole('button', { name: 'Restart', exact: true })
    await waitFor(() => expect(restart).toBeEnabled())

    fireEvent.click(restart)

    await waitFor(() => expect(mocks.restart).toHaveBeenCalledWith('sonarr'))
    await waitFor(() => expect(restart).toBeDisabled())
  })

  it('does not expose restart without the apps.restart permission', async () => {
    mocks.canRestart = false
    renderPage()

    await screen.findByRole('heading', { name: 'Sonarr', level: 2 })
    expect(screen.queryByRole('button', { name: 'Restart', exact: true })).not.toBeInTheDocument()
  })

  it('disables restart while the app already has a queued operation', async () => {
    mocks.getOperations.mockResolvedValue([queuedRestart])
    renderPage()

    const restart = await screen.findByRole('button', { name: 'Restart', exact: true })
    await waitFor(() => expect(restart).toBeDisabled())

    fireEvent.click(restart)
    expect(mocks.restart).not.toHaveBeenCalled()
  })
})

describe('AppsPage VPN assignment', () => {
  const provider = {
    id: 41,
    name: 'Fixture VPN',
    vpn_type: 'wireguard' as const,
    service_provider: null,
    enabled: true,
    kill_switch: true,
    firewall_outbound_subnets: '',
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    app_count: 0,
  }
  const config = {
    app_name: 'sonarr',
    vpn_provider_id: 41,
    vpn_provider_name: 'Fixture VPN',
    kill_switch_override: null,
    effective_kill_switch: true,
    port_forwarding: false,
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
  }

  beforeEach(() => {
    mocks.canRestart = true
    mocks.canViewVpn = true
    mocks.canManageVpn = true
    mocks.getOperations.mockReset()
    mocks.getOperations.mockResolvedValue([])
    mocks.listProviders.mockReset()
    mocks.listProviders.mockResolvedValue([provider])
    mocks.getVpnConfig.mockReset()
    mocks.getVpnConfig.mockResolvedValue(null)
    mocks.getPublicIp.mockReset()
    mocks.getForwardedPort.mockReset()
    mocks.assignVpn.mockReset()
    mocks.removeVpn.mockReset()
    mocks.refreshAppStatuses.mockReset()
  })

  it('invalidates deployment state after a VPN assignment is queued', async () => {
    mocks.assignVpn.mockResolvedValue({ ...config, operation_id: 'vpn-operation-1' })
    const { queryClient } = renderPage()
    const invalidateQueries = vi.spyOn(queryClient, 'invalidateQueries')

    const providerSelect = await screen.findByDisplayValue('No VPN')
    fireEvent.change(providerSelect, { target: { value: '41' } })
    fireEvent.click(await screen.findByRole('button', { name: 'Enable VPN' }))

    await waitFor(() => expect(mocks.assignVpn).toHaveBeenCalledWith('sonarr', {
      vpn_provider_id: 41,
      kill_switch_override: undefined,
      port_forwarding: false,
    }))
    await screen.findByText('VPN change queued')
    expect(invalidateQueries).toHaveBeenCalledWith({ queryKey: ['app-operations'] })
    expect(invalidateQueries).toHaveBeenCalledWith({ queryKey: ['apps', 'states'] })
    expect(invalidateQueries).toHaveBeenCalledWith({ queryKey: ['apps', 'installed'] })
    expect(invalidateQueries).toHaveBeenCalledWith({ queryKey: ['monitoring', 'pods', 'sonarr'] })
    expect(invalidateQueries).toHaveBeenCalledWith({ queryKey: ['app-vpn-config', 'sonarr'] })
  })

  it('labels the database assignment as configured rather than active', async () => {
    mocks.getVpnConfig.mockResolvedValue(config)
    renderPage()

    expect(await screen.findByText('VPN configured via Fixture VPN (kill switch on)')).toBeVisible()
    expect(screen.queryByText(/VPN active via/)).not.toBeInTheDocument()
  })

  it('shows the IP reported by the VPN endpoint only for an assigned app', async () => {
    mocks.getVpnConfig.mockResolvedValue(config)
    mocks.getPublicIp.mockResolvedValue({ public_ip: '203.0.113.42' })
    renderPage()

    expect(await screen.findByText('VPN public IP: 203.0.113.42')).toBeVisible()
    expect(mocks.getPublicIp).toHaveBeenCalledWith('sonarr')
  })

  it('shows loading then unavailable when the VPN endpoint has no IP yet', async () => {
    mocks.getVpnConfig.mockResolvedValue(config)
    let resolveIp!: (value: { public_ip: string | null }) => void
    mocks.getPublicIp.mockReturnValue(new Promise<{ public_ip: string | null }>(resolve => { resolveIp = resolve }))
    renderPage()

    expect(await screen.findByText('VPN public IP: loading...')).toBeVisible()
    resolveIp({ public_ip: null })
    expect(await screen.findByText('VPN public IP: unavailable (VPN may still be connecting)')).toBeVisible()
  })

  it('shows an error instead of a previously cached IP when refreshing fails', async () => {
    mocks.getVpnConfig.mockResolvedValue(config)
    mocks.getPublicIp.mockResolvedValueOnce({ public_ip: '203.0.113.42' })
    const { queryClient } = renderPage()

    expect(await screen.findByText('VPN public IP: 203.0.113.42')).toBeVisible()
    mocks.getPublicIp.mockRejectedValueOnce(new Error('unavailable'))
    await queryClient.invalidateQueries({ queryKey: ['vpn-public-ip', 'sonarr'] })
    expect(await screen.findByText('VPN public IP: unable to retrieve (retrying...)')).toBeVisible()
    expect(screen.queryByText('VPN public IP: 203.0.113.42')).not.toBeInTheDocument()
  })

  it('does not request or display a VPN IP without an assignment or permission', async () => {
    const { unmount } = renderPage()
    await screen.findByDisplayValue('No VPN')
    expect(mocks.getPublicIp).not.toHaveBeenCalled()
    expect(screen.queryByText(/VPN public IP:/)).not.toBeInTheDocument()

    unmount()
    mocks.canViewVpn = false
    mocks.getVpnConfig.mockResolvedValue(config)
    renderPage()
    await screen.findByRole('heading', { name: 'Sonarr', level: 2 })
    expect(mocks.getPublicIp).not.toHaveBeenCalled()
    expect(screen.queryByText(/VPN public IP:/)).not.toBeInTheDocument()
  })

  it('shows pending status and disables VPN controls while an app operation is queued', async () => {
    mocks.getOperations.mockResolvedValue([queuedRestart])
    mocks.getVpnConfig.mockResolvedValue(config)
    renderPage()

    expect(await screen.findByRole('status')).toHaveTextContent('VPN change pending')
    expect(screen.getByRole('button', { name: 'Update VPN' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Remove VPN' })).toBeDisabled()
  })

  it('distinguishes loading, pending port zero, and an assigned forwarded port', async () => {
    mocks.getVpnConfig.mockResolvedValue({ ...config, port_forwarding: true })
    let resolvePort!: (value: { port: number }) => void
    mocks.getForwardedPort.mockReturnValueOnce(new Promise<{ port: number }>(resolve => { resolvePort = resolve }))
    const { queryClient } = renderPage()

    expect(await screen.findByText('Port forwarding: loading...')).toBeVisible()
    expect(mocks.getForwardedPort).toHaveBeenCalledWith('sonarr')

    resolvePort({ port: 0 })
    expect(await screen.findByText('Port forwarding: negotiating...')).toBeVisible()
    expect(screen.queryByText('Port forwarding: loading...')).not.toBeInTheDocument()

    mocks.getForwardedPort.mockResolvedValue({ port: 54321 })
    await queryClient.invalidateQueries({ queryKey: ['vpn-forwarded-port', 'sonarr'] })
    expect(await screen.findByText('Forwarded port: 54321')).toBeVisible()
  })

  it('shows a retrying error rather than negotiating after the forwarded-port request fails', async () => {
    mocks.getVpnConfig.mockResolvedValue({ ...config, port_forwarding: true })
    mocks.getForwardedPort.mockRejectedValueOnce(new Error('unavailable'))
    const { queryClient } = renderPage()

    expect(await screen.findByText('Port forwarding: unable to retrieve port (retrying...)')).toBeVisible()
    expect(screen.queryByText('Port forwarding: negotiating...')).not.toBeInTheDocument()

    mocks.getForwardedPort.mockResolvedValue({ port: 0 })
    await queryClient.invalidateQueries({ queryKey: ['vpn-forwarded-port', 'sonarr'] })
    expect(await screen.findByText('Port forwarding: negotiating...')).toBeVisible()
  })
})
