import type { AppOperation } from '../types'

interface OperationQueueProps {
  operations: AppOperation[]
  displayNames: Record<string, string>
}

function formatElapsed(startedAt: string | null): string {
  if (!startedAt) return 'Starting'
  const seconds = Math.max(0, Math.floor((Date.now() - new Date(startedAt).getTime()) / 1000))
  if (seconds < 60) return `${seconds}s elapsed`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m elapsed`
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m elapsed`
}

const operationLabels: Record<string, string> = {
  install: 'Installing',
  update: 'Updating',
  delete: 'Removing',
  restart: 'Restarting',
}

export function OperationQueue({ operations, displayNames }: OperationQueueProps) {
  const active = operations
    .filter(operation => operation.status === 'queued' || operation.status === 'running')
    .sort((a, b) => {
      if (a.status === b.status) return new Date(a.created_at).getTime() - new Date(b.created_at).getTime()
      return a.status === 'running' ? -1 : 1
    })
  const recentFailureCutoff = Date.now() - 24 * 60 * 60 * 1000
  const failures = operations
    .filter(operation => operation.status === 'failed' && new Date(operation.updated_at).getTime() >= recentFailureCutoff)
    .slice(0, 3)
  const visibleOperations = [...active, ...failures]

  if (visibleOperations.length === 0) return null

  const queued = active.filter(operation => operation.status === 'queued')
  const runningCount = active.length - queued.length

  return (
    <section className="overflow-hidden rounded-2xl border border-slate-200/80 bg-white/80 shadow-sm backdrop-blur dark:border-slate-700/80 dark:bg-slate-900/70">
      <div className="flex flex-wrap items-center gap-3 border-b border-slate-200/80 px-5 py-4 dark:border-slate-700/80">
        <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-blue-500/10 text-blue-600 dark:text-blue-400">
          <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 6h16M4 12h10M4 18h7" />
          </svg>
        </div>
        <div>
          <h2 className="font-semibold text-slate-950 dark:text-white">Operation queue</h2>
          <p className="text-xs text-slate-500 dark:text-slate-400">Worker activity updates automatically</p>
        </div>
        <div className="ml-auto flex items-center gap-2 text-xs font-medium">
          {runningCount > 0 && <span className="rounded-full bg-blue-500/10 px-2.5 py-1 text-blue-700 dark:text-blue-300">{runningCount} running</span>}
          {queued.length > 0 && <span className="rounded-full bg-amber-500/10 px-2.5 py-1 text-amber-700 dark:text-amber-300">{queued.length} queued</span>}
          {failures.length > 0 && <span className="rounded-full bg-red-500/10 px-2.5 py-1 text-red-700 dark:text-red-300">{failures.length} failed</span>}
        </div>
      </div>
      <div className="divide-y divide-slate-200/80 dark:divide-slate-700/80">
        {visibleOperations.map(operation => {
          const queuePosition = operation.status === 'queued' ? queued.findIndex(item => item.id === operation.id) + 1 : 0
          const isRunning = operation.status === 'running'
          const isFailed = operation.status === 'failed'
          return (
            <div key={operation.id} className="relative grid gap-2 px-5 py-4 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="truncate font-medium text-slate-900 dark:text-white">{displayNames[operation.app_name] || operation.app_name}</span>
                  <span className={`rounded-full px-2 py-0.5 text-[11px] font-semibold uppercase tracking-wide ${isFailed ? 'bg-red-500/10 text-red-700 dark:text-red-300' : isRunning ? 'bg-blue-500/10 text-blue-700 dark:text-blue-300' : 'bg-amber-500/10 text-amber-700 dark:text-amber-300'}`}>
                    {isFailed ? 'Failed' : isRunning ? 'Running' : `Queued #${queuePosition}`}
                  </span>
                </div>
                <p className={`mt-1 truncate text-sm ${isFailed ? 'text-red-600 dark:text-red-300' : 'text-slate-500 dark:text-slate-400'}`}>
                  {isFailed ? operation.error || operation.message || 'Operation failed' : operation.message || `${operationLabels[operation.operation] || operation.operation} app`}
                </p>
              </div>
              <div className="text-xs text-slate-500 dark:text-slate-400 sm:text-right">
                {isRunning ? formatElapsed(operation.started_at) : isFailed ? 'Recent failure' : 'Waiting for worker'}
              </div>
              {isRunning && (
                <div role="progressbar" aria-label={`${displayNames[operation.app_name] || operation.app_name} operation in progress`} className="absolute inset-x-0 bottom-0 h-0.5 overflow-hidden bg-blue-500/10">
                  <div className="h-full w-1/3 animate-[pulse_1.2s_ease-in-out_infinite] rounded-full bg-blue-500" />
                </div>
              )}
            </div>
          )
        })}
      </div>
    </section>
  )
}
