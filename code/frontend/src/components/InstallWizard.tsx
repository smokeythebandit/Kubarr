import { useEffect, useMemo, useRef, useState } from 'react'
import { Link } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { appsApi } from '../api/apps'
import { appVpnApi, vpnApi } from '../api/vpn'
import { useAuth } from '../contexts/AuthContext'
import { availableGpuResources } from './GpuInstallDialog'
import type { DeploymentRequest, InstallOptions, InstallStorage, PrivateConfig, StorageMount } from '../types'

function overlaps(a: string, b: string) { return a === b || a.startsWith(`${b}/`) || b.startsWith(`${a}/`) }
function errorForMounts(mounts: StorageMount[], required: string[], reserved: string[]): string | null {
  if (mounts.length > 64) return 'At most 64 mounts are allowed'
  const names = new Set<string>()
  for (const [i, mount] of mounts.entries()) {
    if (!/^[a-z0-9]([-a-z0-9]*[a-z0-9])?$/.test(mount.name) || mount.name.length > 63 || names.has(mount.name)) return `Mount ${i + 1}: name must be a unique DNS label`
    names.add(mount.name)
    if (mount.source.length > 512 || !/^[A-Za-z0-9_-]+(\/[A-Za-z0-9_-]+)*$/.test(mount.source) || mount.source === 'config' || mount.source.startsWith('config/')) return `Mount ${i + 1}: source must be a safe relative directory outside config`
    if (mount.target.length > 512 || !/^\/(?:[A-Za-z0-9_-]+\/)*[A-Za-z0-9_-]+$/.test(mount.target)) return `Mount ${i + 1}: target must be an absolute container path`
    if (reserved.some(path => overlaps(mount.target, path)) || mounts.slice(0, i).some(other => overlaps(mount.target, other.target))) return `Mount ${i + 1}: target overlaps another or a reserved path`
    if (mounts.slice(0, i).some(other => other.mode !== mount.mode && overlaps(other.source, mount.source))) return `Mount ${i + 1}: overlapping sources cannot have different modes`
  }
  for (const target of required) if (!mounts.some(mount => mount.target === target && mount.mode === 'read-write')) return `A read-write mount at ${target} is required`
  return null
}

function configError(config: PrivateConfig | null): string | null {
  if (!config) return null
  if (config.create) {
    if (config.existing_claim || !/^[1-9][0-9]*(Ki|Mi|Gi|Ti|Pi|Ei)$/.test(config.size)) return 'Select a valid private config size without an existing claim'
    if (config.storage_class_name && (config.storage_class_name.length > 253 || !config.storage_class_name.split('.').every(label => label.length <= 63 && /^[a-z0-9]([-a-z0-9]*[a-z0-9])?$/.test(label)))) return 'Invalid StorageClass name'
  } else if (config.existing_claim.length > 63 || !/^[a-z0-9]([-a-z0-9]*[a-z0-9])?$/.test(config.existing_claim) || config.existing_claim === 'media-data' || config.storage_class_name) return 'Select a distinct existing private config claim'
  return null
}

function MountEditor({ label, mounts, onChange, required, reserved }: { label: string; mounts: StorageMount[]; onChange: (mounts: StorageMount[]) => void; required: string[]; reserved: string[] }) {
  const change = (index: number, patch: Partial<StorageMount>) => onChange(mounts.map((mount, i) => i === index ? { ...mount, ...patch } : mount))
  return <fieldset className="space-y-3 rounded-lg border border-gray-200 p-3 dark:border-gray-700">
    <legend className="font-semibold">{label} mounts</legend>
    {required.length > 0 && <p className="text-sm text-amber-700 dark:text-amber-400">Required writable targets: {required.join(', ')}</p>}
    {mounts.map((mount, index) => <div key={index} className="grid gap-2 rounded-md border border-gray-200 p-2 dark:border-gray-700 sm:grid-cols-2">
      {(['name', 'source', 'target'] as const).map(field => <label key={field} className="text-sm capitalize">{field}
        <input aria-label={`${label} mount ${index + 1} ${field}`} value={mount[field]} onChange={event => change(index, { [field]: event.target.value })} className="mt-1 w-full rounded border bg-white p-2 dark:bg-gray-800" />
      </label>)}
      <label className="text-sm">Mode<select aria-label={`${label} mount ${index + 1} mode`} value={mount.mode} onChange={event => change(index, { mode: event.target.value as StorageMount['mode'] })} className="mt-1 w-full rounded border bg-white p-2 dark:bg-gray-800"><option value="read">Read</option><option value="read-write">Read/write</option></select></label>
      <button type="button" className="text-left text-sm text-red-600" onClick={() => onChange(mounts.filter((_, i) => i !== index))} disabled={required.includes(mount.target)}>Remove {mount.name || `mount ${index + 1}`}</button>
    </div>)}
    <button type="button" className="rounded border px-3 py-2 text-sm" onClick={() => onChange([...mounts, { name: '', source: '', target: '', mode: 'read' }])}>Add mount</button>
    {errorForMounts(mounts, required, reserved) && <p role="alert" className="text-sm text-red-600">{errorForMounts(mounts, required, reserved)}</p>}
  </fieldset>
}

function WizardBody({ options, onClose, onSubmit, pending, submitError }: { options: InstallOptions; onClose: () => void; onSubmit: (request: DeploymentRequest) => void; pending: boolean; submitError: string | null }) {
  const dialogRef = useRef<HTMLDivElement>(null)
  useEffect(() => { dialogRef.current?.focus() }, [])
  const trapFocus = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== 'Tab') return
    const controls = Array.from(dialogRef.current?.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled), a[href]') || [])
    if (!controls.length) return
    const first = controls[0]
    const last = controls[controls.length - 1]
    if (event.shiftKey && (document.activeElement === first || document.activeElement === dialogRef.current)) { event.preventDefault(); last.focus() }
    else if (!event.shiftKey && (document.activeElement === last || document.activeElement === dialogRef.current)) { event.preventDefault(); first.focus() }
  }
  const { hasPermission } = useAuth()
  const canViewVpn = hasPermission('vpn.view')
  const canManageVpn = hasPermission('vpn.manage')
  const [step, setStep] = useState(0)
  const [mounts, setMounts] = useState<StorageMount[] | null>(() => {
    const selected = options.saved_storage?.mounts ?? options.mounts
    return selected ? structuredClone(selected) : null
  })
  const [workloads, setWorkloads] = useState<Record<string, StorageMount[]>>(() =>
    Object.fromEntries(Object.entries(options.workloads).map(([name, defaults]) => [name, structuredClone(options.saved_storage?.workloads?.[name]?.mounts ?? defaults)])))
  const [config, setConfig] = useState<PrivateConfig | null>(() => {
    const selected = options.saved_storage?.config ?? options.config
    return selected ? { ...selected } : null
  })
  const [providerId, setProviderId] = useState<number | null>(null)
  const [override, setOverride] = useState<'default' | 'on' | 'off'>('default')
  const [portForwarding, setPortForwarding] = useState(false)
  const [gpuEnabled, setGpuEnabled] = useState(false)
  const [gpuValue, setGpuValue] = useState('')
  const steps = options.gpu_supported ? ['Storage', 'VPN', 'GPU', 'Review'] : ['Storage', 'VPN', 'Review']
  const providers = useQuery({ queryKey: ['vpn-providers'], queryFn: vpnApi.listProviders, enabled: options.vpn_supported && canViewVpn, retry: false })
  const currentVpn = useQuery({ queryKey: ['app-vpn-config', options.app_name], queryFn: () => appVpnApi.getConfig(options.app_name), enabled: options.vpn_supported && canViewVpn, retry: false })
  const gpu = useQuery({ queryKey: ['apps', 'gpu', 'nodes'], queryFn: appsApi.getGpus, enabled: options.gpu_supported && step === 2, retry: false })
  const gpuChoices = availableGpuResources(gpu.data)
  const chosenGpu = gpuChoices.find(choice => JSON.stringify(choice) === gpuValue)
  const enabledProviders = providers.data?.filter(provider => provider.enabled) || []
  const selectedProvider = enabledProviders.find(provider => provider.id === providerId)
  const validation = useMemo(() => {
    if (mounts) {
      const error = errorForMounts(mounts, options.required.main || [], options.reserved.main || [])
      if (error) return error
    }
    for (const [name, entries] of Object.entries(workloads)) {
      const error = errorForMounts(entries, options.required[name] || [], options.reserved[name] || [])
      if (error) return `${name}: ${error}`
    }
    return configError(config)
  }, [mounts, workloads, config, options])
  const vpnError = options.vpn_supported && ((!canViewVpn && canManageVpn) || (canViewVpn && (providers.isPending || currentVpn.isPending || providers.isError || currentVpn.isError)))
  const vpnBlocked = !!providerId && (!canManageVpn || !selectedProvider)
  const removeBlocked = !!currentVpn.data && !providerId && !canManageVpn
  const gpuBlocked = gpuEnabled && (!chosenGpu || gpu.isFetching || gpu.isError)
  const nextDisabled = pending || (step === 0 && !!validation) || (step === 1 && (vpnError || vpnBlocked || removeBlocked)) || (steps[step] === 'GPU' && gpuBlocked)
  const storage: InstallStorage = {
    ...(mounts === null ? {} : { mounts }),
    ...(Object.keys(workloads).length ? { workloads: Object.fromEntries(Object.entries(workloads).map(([key, value]) => [key, { mounts: value }])) } : {}),
    ...(config ? { config } : {}),
  }
  const submit = () => {
    if (nextDisabled || validation || gpuBlocked) return
    onSubmit({ app_name: options.app_name, chart_version: options.chart_version, storage,
      vpn: providerId ? { vpn_provider_id: providerId, ...(override !== 'default' ? { kill_switch_override: override === 'on' } : {}), port_forwarding: portForwarding } : null,
      ...(gpuEnabled && chosenGpu ? { gpu: chosenGpu } : {}) })
  }

  return <div ref={dialogRef} tabIndex={-1} onKeyDown={trapFocus} role="dialog" aria-modal="true" aria-label={`Install ${options.app_name}`} className="relative max-h-[min(90dvh,900px)] w-full max-w-3xl overflow-y-auto rounded-xl bg-white p-4 shadow-2xl dark:bg-gray-900 sm:p-6">
    <div className="flex items-start justify-between gap-4"><div><h2 className="text-xl font-bold">Install {options.app_name}</h2><p className="text-sm text-gray-500">Chart {options.chart_version}</p>{options.saved_storage && <p className="text-xs text-blue-600">Loaded saved desired storage from chart {options.saved_chart_version}. Review before reinstalling.</p>}</div><button type="button" onClick={onClose} disabled={pending} aria-label="Close install wizard" className="rounded border px-3 py-1">Close</button></div>
    <nav aria-label="Install steps" className="my-5 flex flex-wrap gap-2">{steps.map((name, i) => <span key={name} aria-current={i === step ? 'step' : undefined} className={`rounded-full px-3 py-1 text-sm ${i === step ? 'bg-blue-600 text-white' : 'bg-gray-100 dark:bg-gray-800'}`}>{i + 1}. {name}</span>)}</nav>
    {step === 0 && <div className="space-y-5">
      {mounts === null ? <p>This chart has no shared NFS mount list.</p> : <MountEditor label="Main" mounts={mounts} onChange={setMounts} required={options.required.main || []} reserved={options.reserved.main || []} />}
      {Object.entries(workloads).map(([name, entries]) => <MountEditor key={name} label={name} mounts={entries} onChange={value => setWorkloads(previous => ({ ...previous, [name]: value }))} required={options.required[name] || []} reserved={options.reserved[name] || []} />)}
      {config && <fieldset className="space-y-3 rounded-lg border p-3"><legend className="font-semibold">Private config PVC</legend>
        <label className="block text-sm">Claim source<select aria-label="Private config claim source" value={config.create ? 'create' : 'existing'} onChange={event => setConfig(previous => previous && { ...previous, create: event.target.value === 'create', existing_claim: '', storage_class_name: '' })} className="ml-2 rounded border bg-white p-2 dark:bg-gray-800"><option value="create">Create private claim</option><option value="existing">Use existing private claim</option></select></label>
        {config.create ? <div className="grid gap-3 sm:grid-cols-2"><label className="text-sm">StorageClass (blank = cluster default)<input aria-label="Private config StorageClass" value={config.storage_class_name} onChange={event => setConfig({ ...config, storage_class_name: event.target.value })} className="mt-1 w-full rounded border bg-white p-2 dark:bg-gray-800" /></label><label className="text-sm">Size<input aria-label="Private config size" value={config.size} onChange={event => setConfig({ ...config, size: event.target.value })} className="mt-1 w-full rounded border bg-white p-2 dark:bg-gray-800" /></label></div> : <label className="block text-sm">Existing claim name<input aria-label="Existing private config claim" value={config.existing_claim} onChange={event => setConfig({ ...config, existing_claim: event.target.value })} className="mt-1 w-full rounded border bg-white p-2 dark:bg-gray-800" /></label>}
        <p className="text-xs text-gray-500">Config stays on a private claim, separate from shared NFS. Confirm the chosen StorageClass or pre-provisioned claim before installing.</p>
      </fieldset>}
      {validation && <p role="alert" className="text-red-600">{validation}</p>}
    </div>}
    {steps[step] === 'VPN' && <div className="space-y-4">
      {!options.vpn_supported ? <p>This chart does not support app VPN routing.</p> : <>
        {!canViewVpn && <p>{canManageVpn ? 'vpn.view is required to review an existing assignment before changing it.' : 'VPN provider listing requires vpn.view. This install will request No VPN; an existing assignment cannot be removed without vpn.manage.'}</p>}
        {canViewVpn && <><label className="block text-sm">VPN provider<select aria-label="VPN provider" value={providerId ?? ''} onChange={event => setProviderId(event.target.value ? Number(event.target.value) : null)} disabled={providers.isPending || providers.isError || currentVpn.isPending || currentVpn.isError} className="mt-1 w-full rounded border bg-white p-2 dark:bg-gray-800"><option value="">No VPN</option>{canManageVpn && enabledProviders.map(provider => <option key={provider.id} value={provider.id}>{provider.name} ({provider.vpn_type})</option>)}</select></label>
          {providers.isPending || currentVpn.isPending ? <p role="status">Loading VPN settings…</p> : null}
          {(providers.isError || currentVpn.isError) && <p role="alert">VPN settings could not be loaded. Retry before installing; no VPN choice will be assumed.</p>}
          {currentVpn.data && <p className="text-sm text-amber-700">Existing assignment: {currentVpn.data.vpn_provider_name}. Selecting No VPN will remove it at install submission.</p>}
          <button type="button" className="rounded border px-3 py-1 text-sm" onClick={() => { void providers.refetch(); void currentVpn.refetch() }}>Refresh VPN providers</button>
          {canManageVpn && <Link to="/settings?section=vpn" target="_blank" rel="noopener noreferrer" className="text-sm text-blue-600">Manage VPN providers in Settings</Link>}
        </>}
        {providerId && selectedProvider && <div className="space-y-3"><label className="block text-sm">Kill switch<select aria-label="Kill switch" value={override} onChange={event => setOverride(event.target.value as typeof override)} className="ml-2 rounded border bg-white p-2 dark:bg-gray-800"><option value="default">Provider default ({selectedProvider.kill_switch ? 'on' : 'off'})</option><option value="on">Force on</option><option value="off">Force off</option></select></label><label className="flex gap-2 text-sm"><input type="checkbox" checked={portForwarding} onChange={event => setPortForwarding(event.target.checked)} />Port forwarding</label></div>}
        {removeBlocked && <p role="alert">vpn.manage is required to remove the existing VPN assignment.</p>}
      </>}
    </div>}
    {steps[step] === 'GPU' && <div className="space-y-3"><label className="flex gap-2"><input type="checkbox" checked={gpuEnabled} onChange={event => setGpuEnabled(event.target.checked)} />Enable hardware acceleration (optional)</label>
      {gpuEnabled && <><label className="block text-sm">GPU node and resource<select aria-label="GPU node and resource" value={gpuValue} onChange={event => setGpuValue(event.target.value)} className="mt-1 w-full rounded border bg-white p-2 dark:bg-gray-800"><option value="">Select a GPU</option>{gpuChoices.map(choice => <option key={`${choice.node_name}-${choice.resource_name}`} value={JSON.stringify(choice)}>{choice.node_name} — {choice.resource_name}</option>)}</select></label>{gpu.isFetching && <p>Checking GPU nodes…</p>}{gpu.isError && <p role="alert">GPU discovery failed. Install without GPU or retry.</p>}</>}
    </div>}
    {steps[step] === 'Review' && <div className="space-y-4 text-sm"><h3 className="font-semibold">Review before queueing</h3>
      <p>Shared claim: <code>media-data</code>. Chart: <code>{options.chart_version}</code>.</p>
      {mounts === null && <p>No configurable main NFS mounts.</p>}
      {mounts && <div><strong>Main mounts ({mounts.length})</strong>{mounts.map((mount, i) => <p key={i}><code>{mount.source}</code> → <code>{mount.target}</code> ({mount.mode}, {mount.name})</p>)}</div>}
      {Object.entries(workloads).map(([name, entries]) => <div key={name}><strong>{name} mounts ({entries.length})</strong>{entries.map((mount, i) => <p key={i}><code>{mount.source}</code> → <code>{mount.target}</code> ({mount.mode}, {mount.name})</p>)}</div>)}
      {config && <p>Private config: {config.create ? `new ${config.size} claim ${options.private_claim || '(Helm-generated name)'} (${config.storage_class_name || 'cluster default StorageClass'})` : `existing claim ${config.existing_claim}`}</p>}
      <p>VPN: {selectedProvider ? `${selectedProvider.name}, kill switch ${override === 'default' ? `provider default (${selectedProvider.kill_switch ? 'on' : 'off'})` : override}, port forwarding ${portForwarding ? 'on' : 'off'}` : 'No VPN'}</p>
      {options.gpu_supported && <p>GPU: {gpuEnabled && chosenGpu ? `${chosenGpu.node_name} — ${chosenGpu.resource_name}` : 'None'}</p>}
    </div>}
    {submitError && <p role="alert" className="mt-4 rounded bg-red-50 p-3 text-sm text-red-700">{submitError}</p>}
    <div className="mt-6 flex flex-wrap justify-end gap-2"><button type="button" onClick={onClose} disabled={pending} className="rounded border px-4 py-2">Cancel</button>{step > 0 && <button type="button" onClick={() => setStep(step - 1)} disabled={pending} className="rounded border px-4 py-2">Back</button>}{step < steps.length - 1 ? <button type="button" disabled={nextDisabled} onClick={() => setStep(step + 1)} className="rounded bg-blue-600 px-4 py-2 text-white disabled:opacity-50">Next</button> : <button type="button" disabled={nextDisabled || !!validation || gpuBlocked} onClick={submit} className="rounded bg-blue-600 px-4 py-2 text-white disabled:opacity-50">{pending ? 'Queueing…' : 'Confirm install'}</button>}</div>
  </div>
}

export function InstallWizard({ appName, onClose, onSubmit, pending, error }: { appName: string; onClose: () => void; onSubmit: (request: DeploymentRequest) => void; pending: boolean; error: string | null }) {
  const { hasPermission } = useAuth()
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null
    return () => previous?.focus()
  }, [])
  useEffect(() => {
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape' && !pending) onClose() }
    window.addEventListener('keydown', escape)
    return () => window.removeEventListener('keydown', escape)
  }, [onClose, pending])
  const options = useQuery({ queryKey: ['apps', 'install-options', appName], queryFn: () => appsApi.getInstallOptions(appName), enabled: hasPermission('apps.install'), retry: false })
  return <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-2 sm:p-4">
    {!hasPermission('apps.install') ? <div role="alert" className="rounded bg-white p-6">apps.install permission required <button type="button" onClick={onClose}>Close</button></div> : options.isPending ? <div role="status" className="rounded bg-white p-6">Loading chart install options… <button type="button" onClick={onClose}>Cancel</button></div> : options.isError ? <div role="alert" className="rounded bg-white p-6">Could not load install options. <button type="button" onClick={() => void options.refetch()}>Retry</button><button type="button" onClick={onClose} className="ml-3">Close</button></div> : options.data && <WizardBody key={`${appName}:${options.data.chart_version}`} options={options.data} onClose={onClose} onSubmit={onSubmit} pending={pending} submitError={error} />}
  </div>
}
