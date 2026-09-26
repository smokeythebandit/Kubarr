import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { GpuInstallDialog, availableGpuResources, installRequest } from './GpuInstallDialog'

const getGpus = vi.fn()
vi.mock('../api/apps', () => ({ appsApi: { getGpus: (...args: unknown[]) => getGpus(...args) } }))

function renderDialog(updating = false, client = new QueryClient({ defaultOptions: { queries: { retry: false } } })) {
  const onInstall = vi.fn()
  const onUpdate = vi.fn()
  render(<QueryClientProvider client={client}>
    <GpuInstallDialog appName="plex" onClose={vi.fn()} {...(updating ? { onUpdate } : { onInstall })} />
  </QueryClientProvider>)
  return { onInstall, onUpdate }
}

const nodes = [
  { name: 'node-a', ready: true, schedulable: true, allocatable: { 'nvidia.com/gpu.shared': 2, 'amd.com/dri': 1, 'amd.com/gpu': 1, 'untrusted.com/gpu': 5 } },
  { name: 'node-b', ready: true, schedulable: true, allocatable: { 'nvidia.com/gpu.shared': 1, 'gpu.intel.com/xe': 1 } },
  { name: 'offline', ready: false, schedulable: true, allocatable: { 'gpu.intel.com/i915': 3 } },
  { name: 'cordoned', ready: true, schedulable: false, allocatable: { 'nvidia.com/gpu': 1 } },
]

describe('GPU selection', () => {
  beforeEach(() => { getGpus.mockReset(); getGpus.mockResolvedValue(nodes) })

  it('omits GPU for CPU installs', () => {
    const { onInstall } = renderDialog()
    fireEvent.click(screen.getByRole('button', { name: 'Install' }))
    expect(onInstall).toHaveBeenCalledWith({ app_name: 'plex', namespace: 'plex' })
    expect(installRequest('sonarr')).toEqual({ app_name: 'sonarr', namespace: 'sonarr' })
  })

  it('requests the exact selected node, vendor and resource, filtering unsupported nodes and resources', async () => {
    const { onInstall } = renderDialog()
    fireEvent.click(screen.getByRole('checkbox', { name: /Enable hardware acceleration/ }))
    const select = await screen.findByRole('combobox', { name: 'GPU node and resource' })
    await waitFor(() => expect(screen.getByRole('option', { name: 'node-b — nvidia.com/gpu.shared' })).toBeInTheDocument())
    expect(screen.queryByRole('option', { name: /untrusted/ })).not.toBeInTheDocument()
    expect(screen.queryByRole('option', { name: /offline/ })).not.toBeInTheDocument()
    expect(screen.getByRole('option', { name: 'node-a — amd.com/dri' })).toBeInTheDocument()
    expect(screen.queryByRole('option', { name: 'node-a — amd.com/gpu' })).not.toBeInTheDocument()
    expect(screen.getByRole('option', { name: 'node-b — gpu.intel.com/xe' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Install' })).toBeDisabled()
    fireEvent.change(select, { target: { value: JSON.stringify({ vendor: 'nvidia', node_name: 'node-b', resource_name: 'nvidia.com/gpu.shared' }) } })
    fireEvent.click(screen.getByRole('button', { name: 'Install' }))
    expect(onInstall).toHaveBeenCalledWith({ app_name: 'plex', namespace: 'plex', gpu: { vendor: 'nvidia', node_name: 'node-b', resource_name: 'nvidia.com/gpu.shared' } })
  })

  it('fails closed for unavailable discovery but allows CPU install', async () => {
    getGpus.mockRejectedValue(new Error('unavailable'))
    const { onInstall } = renderDialog()
    fireEvent.click(screen.getByRole('checkbox', { name: /Enable hardware acceleration/ }))
    await screen.findByText(/GPU discovery is unavailable/)
    expect(screen.getByRole('button', { name: 'Install' })).toBeDisabled()
    fireEvent.click(screen.getByRole('checkbox', { name: /Enable hardware acceleration/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Install' }))
    expect(onInstall).toHaveBeenCalledWith({ app_name: 'plex', namespace: 'plex' })
    expect(availableGpuResources({ nodes })).toEqual([])
    expect(availableGpuResources([{ name: 'bad', ready: true, schedulable: true, allocatable: { 'nvidia.com/gpu': '1' } }])).toEqual([])
  })

  it('updates only when opted in and explicitly disables via null', async () => {
    const { onUpdate } = renderDialog(true)
    expect(screen.getByRole('button', { name: 'Apply GPU settings' })).toBeDisabled()
    fireEvent.click(screen.getByRole('checkbox', { name: /Enable hardware acceleration/ }))
    await screen.findByRole('option', { name: 'node-a — amd.com/dri' })
    fireEvent.change(screen.getByRole('combobox'), { target: { value: JSON.stringify({ vendor: 'amd', node_name: 'node-a', resource_name: 'amd.com/dri' }) } })
    fireEvent.click(screen.getByRole('button', { name: 'Apply GPU settings' }))
    expect(onUpdate).toHaveBeenCalledWith({ vendor: 'amd', node_name: 'node-a', resource_name: 'amd.com/dri' })
    fireEvent.click(screen.getByRole('button', { name: 'Disable GPU' }))
    expect(onUpdate).toHaveBeenCalledWith(null)
  })

  it('does not submit a cached GPU choice before fresh discovery completes', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    client.setQueryData(['apps', 'gpu', 'nodes'], nodes)
    let resolve!: (value: typeof nodes) => void
    getGpus.mockReturnValue(new Promise<typeof nodes>(done => { resolve = done }))
    const { onInstall } = renderDialog(false, client)
    fireEvent.click(screen.getByRole('checkbox', { name: /Enable hardware acceleration/ }))
    const select = screen.getByRole('combobox', { name: 'GPU node and resource' })
    expect(select).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Install' })).toBeDisabled()
    resolve([])
    await waitFor(() => expect(select).toBeEnabled())
    expect(screen.queryByRole('option', { name: /node-a/ })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Install' })).toBeDisabled()
    expect(onInstall).not.toHaveBeenCalled()
  })

  it('allows retrying failed GPU discovery', async () => {
    getGpus.mockRejectedValueOnce(new Error('offline')).mockResolvedValueOnce(nodes)
    renderDialog(true)
    fireEvent.click(screen.getByRole('checkbox', { name: /Enable hardware acceleration/ }))
    await screen.findByText(/GPU discovery is unavailable/)
    fireEvent.click(screen.getByRole('button', { name: 'Refresh GPU nodes' }))
    expect(await screen.findByRole('option', { name: 'node-b — gpu.intel.com/xe' })).toBeInTheDocument()
  })
})
