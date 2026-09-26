import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { appsApi } from '../api/apps'
import type { DeploymentRequest, GpuSelection } from '../types'

const supportedResources: Record<string, GpuSelection['vendor']> = {
  'gpu.intel.com/i915': 'intel',
  'gpu.intel.com/xe': 'intel',
  'nvidia.com/gpu': 'nvidia',
  'nvidia.com/gpu.shared': 'nvidia',
  'amd.com/dri': 'amd',
}

// A selection is tied to a schedulable node, never just an extended resource name.
export function availableGpuResources(data: unknown): GpuSelection[] {
  if (!Array.isArray(data)) return []
  const choices: GpuSelection[] = []
  for (const node of data) {
    if (!node || typeof node.name !== 'string' || !node.name || node.ready !== true || node.schedulable !== true ||
        !node.allocatable || typeof node.allocatable !== 'object' || Array.isArray(node.allocatable)) continue
    for (const [resource_name, count] of Object.entries(node.allocatable)) {
      const vendor = Object.prototype.hasOwnProperty.call(supportedResources, resource_name) ? supportedResources[resource_name] : undefined
      if (vendor && typeof count === 'number' && Number.isFinite(count) && count >= 1) {
        choices.push({ vendor, node_name: node.name, resource_name })
      }
    }
  }
  return choices.sort((a, b) => a.node_name.localeCompare(b.node_name) || a.resource_name.localeCompare(b.resource_name))
}

export function installRequest(appName: string, gpu?: GpuSelection): DeploymentRequest {
  return { app_name: appName, namespace: appName, ...(gpu ? { gpu } : {}) }
}

interface Props {
  appName: string
  onClose: () => void
  onInstall?: (request: DeploymentRequest) => void
  onUpdate?: (gpu: GpuSelection | null) => void
}

export function GpuInstallDialog({ appName, onClose, onInstall, onUpdate }: Props) {
  const updating = !!onUpdate
  const [enabled, setEnabled] = useState(false)
  const [selection, setSelection] = useState('')
  const { data, isFetching, isError, refetch } = useQuery({
    queryKey: ['apps', 'gpu', 'nodes'],
    queryFn: appsApi.getGpus,
    retry: false,
  })
  const resources = availableGpuResources(data)
  const chosen = resources.find(item => JSON.stringify(item) === selection)

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4" role="presentation">
      <div role="dialog" aria-modal="true" aria-label={`${updating ? 'Change' : 'Install'} ${appName} GPU settings`} className="w-full max-w-lg rounded-xl bg-white p-6 shadow-xl dark:bg-gray-900">
        <h2 className="text-xl font-semibold">{updating ? `Change ${appName} GPU settings` : `Install ${appName}`}</h2>
        <label className="mt-5 flex items-center gap-2 text-sm">
          <input type="checkbox" checked={enabled} onChange={event => setEnabled(event.target.checked)} />
          Enable hardware acceleration (optional)
        </label>
        {enabled && (
          <div className="mt-4 space-y-2 text-sm">
            <label htmlFor="gpu-resource" className="block font-medium">GPU node and resource</label>
            <select id="gpu-resource" value={chosen ? selection : ''} disabled={isFetching || isError} onChange={event => setSelection(event.target.value)}
              className="w-full rounded-lg border border-gray-300 bg-white p-2 dark:border-gray-700 dark:bg-gray-800">
              <option value="">Select a node and resource</option>
              {resources.map(item => <option key={`${item.node_name}:${item.resource_name}`} value={JSON.stringify(item)}>{item.node_name} — {item.resource_name}</option>)}
            </select>
            {isFetching && <p>Checking cluster GPU resources…</p>}
            {isError && <p>GPU discovery is unavailable. {updating ? 'Try again later.' : 'Install without acceleration or try again later.'}</p>}
            {!isFetching && !isError && resources.length === 0 && <p>No supported GPU slots reported on ready, schedulable nodes. Configure a device plugin first.</p>}
            <button type="button" disabled={isFetching} onClick={() => { setSelection(''); void refetch() }} className="text-blue-600 disabled:opacity-50">Refresh GPU nodes</button>
            <p>One advertised GPU slot is requested on the selected node. Sharing is not guaranteed, even for a shared resource name; verify device plugin behavior and transcoding support yourself.</p>
            <p>Install vendor drivers and a compatible Kubernetes device plugin first. Intel, NVIDIA and AMD sharing behavior depends on the plugin; AMD sharing has not been verified.</p>
            <p>{appName === 'plex' ? 'Plex hardware transcoding requires Plex Pass. Enable hardware acceleration in Plex Settings → Transcoder.' : 'Enable hardware acceleration and select the matching device/API in Jellyfin Dashboard → Playback → Transcoding after installation.'}</p>
          </div>
        )}
        <div className="mt-6 flex justify-end gap-3">
          <button type="button" onClick={onClose} className="rounded-lg px-4 py-2 text-sm">Cancel</button>
          {updating && <button type="button" onClick={() => onUpdate?.(null)} className="rounded-lg border border-red-500 px-4 py-2 text-sm text-red-600">Disable GPU</button>}
          <button type="button" disabled={updating ? (!enabled || !chosen || isFetching || isError) : (enabled && (!chosen || isFetching || isError))}
            onClick={() => updating ? chosen && onUpdate?.(chosen) : onInstall?.(installRequest(appName, enabled ? chosen : undefined))}
            className="rounded-lg bg-blue-600 px-4 py-2 text-sm text-white disabled:cursor-not-allowed disabled:opacity-50">{updating ? 'Apply GPU settings' : 'Install'}</button>
        </div>
      </div>
    </div>
  )
}
