import { AlertTriangle, CheckCircle2, Clock3, LoaderCircle, RotateCcw, ServerCog } from 'lucide-react'
import type { AppOperation } from '../types'

interface OperationQueueProps {
  operations: AppOperation[]
  displayNames: Record<string, string>
}

const operationLabels: Record<string, string> = {
  install: 'Install',
  update: 'Update',
  delete: 'Remove',
  restart: 'Restart',
}

function formatDuration(start: string | null, end: string | null = null): string {
  if (!start) return 'Not started'
  const milliseconds = Math.max(0, (end ? new Date(end).getTime() : Date.now()) - new Date(start).getTime())
  const seconds = Math.floor(milliseconds / 1000)
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`
}

function formatTimestamp(value: string): string {
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(new Date(value))
}

function OperationType({ operation }: { operation: AppOperation }) {
  return (
    <span className="rounded-md bg-slate-100 px-2 py-1 text-[11px] font-semibold uppercase tracking-[0.12em] text-slate-600 dark:bg-slate-800 dark:text-slate-300">
      {operationLabels[operation.operation] || operation.operation}
    </span>
  )
}

export function OperationQueue({ operations, displayNames }: OperationQueueProps) {
  const sorted = [...operations].sort((a, b) => new Date(b.created_at).getTime() - new Date(a.created_at).getTime())
  const running = sorted.filter(operation => operation.status === 'running')
  const queued = sorted
    .filter(operation => operation.status === 'queued')
    .sort((a, b) => new Date(a.created_at).getTime() - new Date(b.created_at).getTime())
  const failed = sorted.filter(operation => operation.status === 'failed')
  const succeeded = sorted.filter(operation => operation.status === 'succeeded')
  const activeCount = running.length + queued.length

  return (
    <div className="space-y-6">
      <section className="overflow-hidden rounded-2xl border border-slate-200 bg-slate-950 text-white shadow-sm dark:border-slate-700">
        <div className="grid gap-px bg-white/10 sm:grid-cols-2 xl:grid-cols-4">
          {[
            { label: 'Active', value: activeCount, detail: `${running.length} running`, icon: LoaderCircle, color: 'text-sky-300' },
            { label: 'Queued', value: queued.length, detail: 'Waiting for worker', icon: Clock3, color: 'text-amber-300' },
            { label: 'Succeeded', value: succeeded.length, detail: 'Recorded operations', icon: CheckCircle2, color: 'text-emerald-300' },
            { label: 'Failed', value: failed.length, detail: 'Needs attention', icon: AlertTriangle, color: 'text-rose-300' },
          ].map(({ label, value, detail, icon: Icon, color }) => (
            <div key={label} className="bg-slate-950 px-5 py-5">
              <div className="flex items-center justify-between">
                <span className="text-xs font-semibold uppercase tracking-[0.18em] text-slate-400">{label}</span>
                <Icon className={color} size={18} />
              </div>
              <div className="mt-3 text-3xl font-semibold tabular-nums">{value}</div>
              <p className="mt-1 text-xs text-slate-400">{detail}</p>
            </div>
          ))}
        </div>
      </section>

      <div className="grid gap-6 xl:grid-cols-[minmax(0,1.35fr)_minmax(320px,0.65fr)]">
        <section className="overflow-hidden rounded-2xl border border-slate-200 bg-white shadow-sm dark:border-slate-700 dark:bg-slate-900">
          <div className="flex items-center gap-3 border-b border-slate-200 px-5 py-4 dark:border-slate-700">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-sky-500/10 text-sky-600 dark:text-sky-400">
              <ServerCog size={20} />
            </div>
            <div>
              <h2 className="font-semibold text-slate-950 dark:text-white">Worker activity</h2>
              <p className="text-xs text-slate-500 dark:text-slate-400">Running work and execution details</p>
            </div>
            <span className="ml-auto rounded-full bg-sky-500/10 px-2.5 py-1 text-xs font-semibold text-sky-700 dark:text-sky-300">Live</span>
          </div>

          {running.length === 0 ? (
            <div className="flex min-h-48 flex-col items-center justify-center px-6 py-10 text-center">
              <CheckCircle2 className="text-emerald-500" size={34} />
              <h3 className="mt-3 font-medium text-slate-900 dark:text-white">Worker is idle</h3>
              <p className="mt-1 text-sm text-slate-500 dark:text-slate-400">New operations will appear here as soon as execution begins.</p>
            </div>
          ) : (
            <div className="divide-y divide-slate-200 dark:divide-slate-700">
              {running.map(operation => (
                <div key={operation.id} className="relative px-5 py-5">
                  <div className="flex flex-wrap items-start gap-3">
                    <div className="min-w-0 flex-1">
                      <div className="flex flex-wrap items-center gap-2">
                        <h3 className="font-semibold text-slate-950 dark:text-white">{displayNames[operation.app_name] || operation.app_name}</h3>
                        <OperationType operation={operation} />
                      </div>
                      <p className="mt-2 text-sm text-slate-600 dark:text-slate-300">{operation.message || `${operationLabels[operation.operation] || operation.operation} in progress`}</p>
                    </div>
                    <div className="text-right">
                      <div className="text-sm font-semibold tabular-nums text-sky-700 dark:text-sky-300">{formatDuration(operation.started_at)}</div>
                      <div className="mt-1 text-xs text-slate-500">Attempt {operation.attempts + 1}</div>
                    </div>
                  </div>
                  <dl className="mt-4 grid gap-3 rounded-xl bg-slate-50 p-3 text-xs dark:bg-slate-800/70 sm:grid-cols-3">
                    <div><dt className="text-slate-500">Created</dt><dd className="mt-1 font-medium text-slate-700 dark:text-slate-200">{formatTimestamp(operation.created_at)}</dd></div>
                    <div><dt className="text-slate-500">Started</dt><dd className="mt-1 font-medium text-slate-700 dark:text-slate-200">{operation.started_at ? formatTimestamp(operation.started_at) : 'Starting'}</dd></div>
                    <div><dt className="text-slate-500">Operation ID</dt><dd className="mt-1 truncate font-mono text-slate-700 dark:text-slate-200" title={operation.id}>{operation.id}</dd></div>
                  </dl>
                  <div role="progressbar" aria-label={`${displayNames[operation.app_name] || operation.app_name} operation in progress`} className="absolute inset-x-0 bottom-0 h-1 overflow-hidden bg-sky-500/10">
                    <div className="h-full w-1/3 animate-[queue-progress_1.4s_ease-in-out_infinite] rounded-full bg-sky-500" />
                  </div>
                </div>
              ))}
            </div>
          )}
        </section>

        <section className="overflow-hidden rounded-2xl border border-slate-200 bg-white shadow-sm dark:border-slate-700 dark:bg-slate-900">
          <div className="border-b border-slate-200 px-5 py-4 dark:border-slate-700">
            <h2 className="font-semibold text-slate-950 dark:text-white">Waiting queue</h2>
            <p className="text-xs text-slate-500 dark:text-slate-400">Processed in submission order</p>
          </div>
          {queued.length === 0 ? (
            <p className="px-5 py-10 text-center text-sm text-slate-500 dark:text-slate-400">No operations are waiting.</p>
          ) : (
            <ol className="divide-y divide-slate-200 dark:divide-slate-700">
              {queued.map((operation, index) => (
                <li key={operation.id} className="flex gap-3 px-5 py-4">
                  <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-amber-500/10 text-sm font-bold text-amber-700 dark:text-amber-300">{index + 1}</span>
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="truncate font-medium text-slate-900 dark:text-white">{displayNames[operation.app_name] || operation.app_name}</span>
                      <OperationType operation={operation} />
                    </div>
                    <p className="mt-1 text-xs text-slate-500">Queued {formatTimestamp(operation.created_at)}</p>
                  </div>
                </li>
              ))}
            </ol>
          )}
        </section>
      </div>

      <section className="overflow-hidden rounded-2xl border border-slate-200 bg-white shadow-sm dark:border-slate-700 dark:bg-slate-900">
        <div className="flex flex-wrap items-center gap-3 border-b border-slate-200 px-5 py-4 dark:border-slate-700">
          <div>
            <h2 className="font-semibold text-slate-950 dark:text-white">Operation history</h2>
            <p className="text-xs text-slate-500 dark:text-slate-400">Completed and failed worker requests</p>
          </div>
          <span className="ml-auto text-xs text-slate-500">{failed.length + succeeded.length} records</span>
        </div>
        {failed.length + succeeded.length === 0 ? (
          <p className="px-5 py-10 text-center text-sm text-slate-500 dark:text-slate-400">No completed operations yet.</p>
        ) : (
          <div className="overflow-x-auto">
            <table className="min-w-full divide-y divide-slate-200 text-left text-sm dark:divide-slate-700">
              <thead className="bg-slate-50 text-xs uppercase tracking-wide text-slate-500 dark:bg-slate-800/60">
                <tr><th className="px-5 py-3 font-semibold">Application</th><th className="px-5 py-3 font-semibold">Action</th><th className="px-5 py-3 font-semibold">Result</th><th className="px-5 py-3 font-semibold">Duration</th><th className="px-5 py-3 font-semibold">Finished</th><th className="px-5 py-3 font-semibold">Detail</th></tr>
              </thead>
              <tbody className="divide-y divide-slate-200 dark:divide-slate-700">
                {[...failed, ...succeeded].sort((a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime()).map(operation => {
                  const isFailed = operation.status === 'failed'
                  return (
                    <tr key={operation.id} className="align-top">
                      <td className="whitespace-nowrap px-5 py-4 font-medium text-slate-900 dark:text-white">{displayNames[operation.app_name] || operation.app_name}</td>
                      <td className="px-5 py-4"><OperationType operation={operation} /></td>
                      <td className="px-5 py-4"><span className={`inline-flex items-center gap-1.5 font-medium ${isFailed ? 'text-rose-600 dark:text-rose-300' : 'text-emerald-600 dark:text-emerald-300'}`}>{isFailed ? <AlertTriangle size={15} /> : <CheckCircle2 size={15} />}{isFailed ? 'Failed' : 'Succeeded'}</span></td>
                      <td className="whitespace-nowrap px-5 py-4 tabular-nums text-slate-600 dark:text-slate-300">{formatDuration(operation.started_at, operation.finished_at || operation.updated_at)}</td>
                      <td className="whitespace-nowrap px-5 py-4 text-slate-500">{formatTimestamp(operation.finished_at || operation.updated_at)}</td>
                      <td className={`max-w-sm px-5 py-4 ${isFailed ? 'text-rose-600 dark:text-rose-300' : 'text-slate-500 dark:text-slate-400'}`}>{operation.error || operation.message || 'Completed successfully'}</td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
          </div>
        )}
      </section>

      {failed.length > 0 && (
        <section className="rounded-2xl border border-rose-200 bg-rose-50 p-5 dark:border-rose-900/60 dark:bg-rose-950/30">
          <div className="flex items-center gap-2 text-rose-800 dark:text-rose-200"><RotateCcw size={18} /><h2 className="font-semibold">Failure summary</h2></div>
          <p className="mt-2 text-sm text-rose-700 dark:text-rose-300">Review the detailed Helm or Kubernetes error in the history table before retrying the application action.</p>
        </section>
      )}
    </div>
  )
}
