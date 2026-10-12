import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { MemoryRouter } from 'react-router-dom'
import { describe, expect, it, vi } from 'vitest'
import { InstallWizard } from './InstallWizard'

const mocks = vi.hoisted(() => ({
  options: vi.fn(), providers: vi.fn(), config: vi.fn(), gpus: vi.fn(),
  canInstall: true, canViewVpn: true, canManageVpn: true,
}))
vi.mock('../api/apps', () => ({ appsApi: { getInstallOptions: (...args: unknown[]) => mocks.options(...args), getGpus: (...args: unknown[]) => mocks.gpus(...args) } }))
vi.mock('../api/vpn', () => ({ vpnApi: { listProviders: (...args: unknown[]) => mocks.providers(...args) }, appVpnApi: { getConfig: (...args: unknown[]) => mocks.config(...args) } }))
vi.mock('../contexts/AuthContext', () => ({ useAuth: () => ({ hasPermission: (key: string) => key === 'apps.install' ? mocks.canInstall : key === 'vpn.view' ? mocks.canViewVpn : key === 'vpn.manage' ? mocks.canManageVpn : false }) }))

const defaults = {
  app_name: 'redmine', chart_version: '1.2.3', vpn_supported: true, gpu_supported: false,
  mounts: [{ name: 'attachments', source: 'development/redmine/files', target: '/usr/src/redmine/files', mode: 'read-write' }],
  workloads: { postgres: [{ name: 'database', source: 'development/redmine/db', target: '/var/lib/postgresql/data', mode: 'read-write' }] },
  config: null,
  required: { main: ['/usr/src/redmine/files'], postgres: ['/var/lib/postgresql/data'] },
  reserved: { main: ['/config'], postgres: ['/dev/shm'] },
}
function setup(options = defaults, error: string | null = null) {
  mocks.options.mockResolvedValue(options)
  mocks.providers.mockResolvedValue([{ id: 5, name: 'Safe VPN', vpn_type: 'wireguard', enabled: true, kill_switch: true }, { id: 6, name: 'Disabled', enabled: false }])
  mocks.config.mockResolvedValue(null)
  mocks.gpus.mockResolvedValue([])
  const onSubmit = vi.fn()
  const onClose = vi.fn()
  render(<QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}><MemoryRouter><InstallWizard appName="redmine" onClose={onClose} onSubmit={onSubmit} pending={false} error={error} /></MemoryRouter></QueryClientProvider>)
  return { onSubmit, onClose }
}
describe('install wizard', () => {
  it('requires review, preserves component mounts and submits a typed provider selection', async () => {
    const { onSubmit } = setup()
    await screen.findByRole('dialog', { name: 'Install redmine' })
    fireEvent.change(screen.getByRole('textbox', { name: 'Main mount 1 source' }), { target: { value: 'new/files' } })
    expect(onSubmit).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Next' }))
    await screen.findByRole('combobox', { name: 'VPN provider' })
    await waitFor(() => expect(screen.getByRole('button', { name: 'Next' })).toBeEnabled())
    fireEvent.change(screen.getByRole('combobox', { name: 'VPN provider' }), { target: { value: '5' } })
    fireEvent.change(screen.getByRole('combobox', { name: 'Kill switch' }), { target: { value: 'off' } })
    fireEvent.click(screen.getByRole('button', { name: 'Next' }))
    expect(screen.getByText(/new\/files/)).toBeInTheDocument()
    expect(screen.getByText(/development\/redmine\/db/)).toBeInTheDocument()
    expect(onSubmit).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Confirm install' }))
    expect(onSubmit).toHaveBeenCalledWith({ app_name: 'redmine', chart_version: '1.2.3', vpn: { vpn_provider_id: 5, kill_switch_override: false, port_forwarding: false }, storage: {
      mounts: [{ name: 'attachments', source: 'new/files', target: '/usr/src/redmine/files', mode: 'read-write' }],
      workloads: { postgres: { mounts: defaults.workloads.postgres } },
    } })
  })

  it('blocks removing required targets, invalid paths and source overlaps, then allows explicit No VPN', async () => {
    const { onSubmit } = setup()
    await screen.findByRole('dialog')
    expect(screen.getByRole('button', { name: 'Remove attachments' })).toBeDisabled()
    fireEvent.change(screen.getByRole('textbox', { name: 'Main mount 1 source' }), { target: { value: '../private' } })
    expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled()
    fireEvent.change(screen.getByRole('textbox', { name: 'Main mount 1 source' }), { target: { value: 'safe/files' } })
    fireEvent.click(screen.getByRole('button', { name: 'Next' }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Next' })).toBeEnabled())
    fireEvent.click(screen.getByRole('button', { name: 'Next' }))
    fireEvent.click(screen.getByRole('button', { name: 'Confirm install' }))
    expect(onSubmit.mock.calls[0][0].vpn).toBeNull()
  })

  it('retains entered settings on server error; cancel has no mutation', async () => {
    const { onSubmit, onClose } = setup(defaults, 'Chart changed; review again')
    await screen.findByRole('dialog')
    fireEvent.change(screen.getByRole('textbox', { name: 'Main mount 1 source' }), { target: { value: 'edited' } })
    expect(screen.getByRole('alert')).toHaveTextContent('Chart changed')
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    expect(onClose).toHaveBeenCalledTimes(1)
    expect(onSubmit).not.toHaveBeenCalled()
  })

  it('does not downgrade to No VPN if provider listing fails', async () => {
    const { onSubmit } = setup()
    mocks.providers.mockRejectedValue(new Error('failed'))
    await screen.findByRole('dialog')
    fireEvent.click(screen.getByRole('button', { name: 'Next' }))
    await screen.findByText(/VPN settings could not be loaded/)
    expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled()
    expect(onSubmit).not.toHaveBeenCalled()
  })

  it('validates a private claim selection and skips VPN provider discovery for unsupported charts', async () => {
    const { onSubmit } = setup({ ...defaults, vpn_supported: false, config: { create: true, existing_claim: '', storage_class_name: '', size: '5Gi' } })
    await screen.findByRole('dialog')
    fireEvent.change(screen.getByRole('combobox', { name: 'Private config claim source' }), { target: { value: 'existing' } })
    expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled()
    fireEvent.change(screen.getByRole('textbox', { name: 'Existing private config claim' }), { target: { value: 'private-config' } })
    fireEvent.click(screen.getByRole('button', { name: 'Next' }))
    expect(screen.getByText('This chart does not support app VPN routing.')).toBeInTheDocument()
    expect(mocks.providers).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Next' }))
    expect(screen.getByText(/existing claim private-config/)).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Confirm install' }))
    expect(onSubmit.mock.calls[0][0].storage.config).toEqual({ create: false, existing_claim: 'private-config', storage_class_name: '', size: '5Gi' })
    expect(onSubmit.mock.calls[0][0].vpn).toBeNull()
  })

  it('hydrates persisted desired mounts, component mounts and private claim instead of new chart defaults', async () => {
    const savedConfig = { create: false, existing_claim: 'redmine-private', storage_class_name: '', size: '5Gi' }
    const { onSubmit } = setup({ ...defaults,
      vpn_supported: false,
      mounts: [{ ...defaults.mounts[0], source: 'chart/new-default' }],
      config: { create: true, existing_claim: '', storage_class_name: '', size: '5Gi' },
      saved_storage: { mounts: [{ ...defaults.mounts[0], source: 'saved/attachments' }],
        workloads: { postgres: { mounts: [{ ...defaults.workloads.postgres[0], source: 'saved/database' }] } }, config: savedConfig },
      saved_chart_version: '0.9.0',
    })
    await screen.findByRole('dialog')
    expect(screen.getByText(/Loaded saved desired storage from chart 0.9.0/)).toBeInTheDocument()
    expect(screen.getByRole('textbox', { name: 'Main mount 1 source' })).toHaveValue('saved/attachments')
    expect(screen.getByRole('textbox', { name: 'postgres mount 1 source' })).toHaveValue('saved/database')
    expect(screen.getByRole('textbox', { name: 'Existing private config claim' })).toHaveValue('redmine-private')
    fireEvent.click(screen.getByRole('button', { name: 'Next' }))
    fireEvent.click(screen.getByRole('button', { name: 'Next' }))
    fireEvent.click(screen.getByRole('button', { name: 'Confirm install' }))
    const request = onSubmit.mock.calls[0][0]
    expect(request.storage.mounts[0].source).toBe('saved/attachments')
    expect(request.storage.workloads.postgres.mounts[0].source).toBe('saved/database')
    expect(request.storage.config).toEqual(savedConfig)
    expect(request.chart_version).toBe('1.2.3')
  })
})
