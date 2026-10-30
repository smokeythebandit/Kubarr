import { ArrowDownToLine, ArrowUpFromLine, ShieldAlert } from 'lucide-react'
import {
  ClassTotals,
  CLASSES_WITH_FLOW_COUNTS,
  FLOW_CLASS_NAMES,
  formatBandwidth,
  formatFlowRate,
  totalByteRate,
} from '../../api/networking'
import { flowClassStyle } from './flowClasses'

interface FlowClassCardsProps {
  totals: ClassTotals
  /** True when Hubble reported no series at all. */
  stale: boolean
  totalRx: number
  totalTx: number
}

const cardShell =
  'rounded-xl border p-4 bg-white dark:bg-gray-800 border-gray-200 dark:border-gray-700'

/**
 * The page's headline row: how much traffic went where.
 *
 * Every figure here is measured, but not all by the same thing. The three
 * destination classes are counted by Cilium's conntrack map, storage by the
 * kernel's NFS client, and the two totals on the right by cAdvisor. Nothing is
 * apportioned from one source to another.
 *
 * Storage sits alongside the destinations rather than among them. It is the
 * same bytes one step later -- a download arrives from the internet and is
 * then written to NFS -- so it carries no share of the destination total.
 */
export function FlowClassCards({ totals, stale, totalRx, totalTx }: FlowClassCardsProps) {
  const total = totalByteRate(totals)

  return (
    <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-6">
      {FLOW_CLASS_NAMES.map(name => {
        const style = flowClassStyle(name)
        const Icon = style.icon
        const bytes = totals[name].bytes_per_sec
        const flows = totals[name].flows_per_sec
        const drops = totals[name].drops_per_sec
        const share = total > 0 ? (bytes / total) * 100 : 0
        const hasFlowCount = CLASSES_WITH_FLOW_COUNTS.includes(name)

        return (
          <div key={name} className={cardShell} title={style.description}>
            <div className="mb-2 flex items-center gap-2">
              <span className={`rounded-lg p-1.5 ${style.bg}`}>
                <Icon size={16} className={style.text} />
              </span>
              <span className="text-sm text-gray-500 dark:text-gray-400">{style.label}</span>
            </div>

            {stale ? (
              <div className="text-xl font-semibold text-gray-400 dark:text-gray-500">
                No data
              </div>
            ) : (
              <>
                <div className={`font-mono text-2xl font-bold ${style.text}`}>
                  {formatBandwidth(bytes)}
                </div>
                <div className="mt-1 flex flex-wrap items-center gap-x-2 text-xs text-gray-500 dark:text-gray-400">
                  {/* Storage is a second stage, not a fourth destination: the
                      same bytes arrive from the internet and are then written
                      down. It gets no share of the destination total, and no
                      connection count -- the kernel reports bytes per mount. */}
                  {hasFlowCount ? (
                    <>
                      <span>{share.toFixed(0)}% of traffic</span>
                      <span>{formatFlowRate(flows)} conn</span>
                    </>
                  ) : (
                    <span>read + written</span>
                  )}
                  {drops > 0 && (
                    <span
                      className="flex items-center gap-1 text-red-500"
                      title="Connections the network policy denied"
                    >
                      <ShieldAlert size={12} />
                      {formatFlowRate(drops)} denied
                    </span>
                  )}
                </div>
              </>
            )}
          </div>
        )
      })}

      <div className={cardShell}>
        <div className="mb-2 flex items-center gap-2">
          <span className="rounded-lg bg-green-500/10 p-1.5">
            <ArrowDownToLine size={16} className="text-green-500" />
          </span>
          <span className="text-sm text-gray-500 dark:text-gray-400">Total Receive</span>
        </div>
        <div className="font-mono text-2xl font-bold text-green-500">
          {formatBandwidth(totalRx)}
        </div>
        <div className="mt-1 text-xs text-gray-500 dark:text-gray-400">all pods, measured</div>
      </div>

      <div className={cardShell}>
        <div className="mb-2 flex items-center gap-2">
          <span className="rounded-lg bg-orange-500/10 p-1.5">
            <ArrowUpFromLine size={16} className="text-orange-500" />
          </span>
          <span className="text-sm text-gray-500 dark:text-gray-400">Total Transmit</span>
        </div>
        <div className="font-mono text-2xl font-bold text-orange-500">
          {formatBandwidth(totalTx)}
        </div>
        <div className="mt-1 text-xs text-gray-500 dark:text-gray-400">all pods, measured</div>
      </div>
    </div>
  )
}
