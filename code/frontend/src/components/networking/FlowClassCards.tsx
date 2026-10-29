import { ArrowDownToLine, ArrowUpFromLine, ShieldAlert } from 'lucide-react'
import {
  ClassTotals,
  FLOW_CLASS_NAMES,
  formatBandwidth,
  formatFlowRate,
  totalFlowRate,
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
 * The page's headline row: where traffic is going (three measured classes, in
 * flows per second) next to how much of it there is (bytes, from cAdvisor).
 *
 * The two halves are deliberately separate cards rather than one blended
 * number. Hubble has no byte counters and cAdvisor has no destinations, so a
 * per-class byte figure would have to be invented.
 */
export function FlowClassCards({ totals, stale, totalRx, totalTx }: FlowClassCardsProps) {
  const total = totalFlowRate(totals)

  return (
    <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-5">
      {FLOW_CLASS_NAMES.map(name => {
        const style = flowClassStyle(name)
        const Icon = style.icon
        const rate = totals[name].flows_per_sec
        const drops = totals[name].drops_per_sec
        const share = total > 0 ? (rate / total) * 100 : 0

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
                  {formatFlowRate(rate)}
                </div>
                <div className="mt-1 flex items-center gap-2 text-xs text-gray-500 dark:text-gray-400">
                  <span>{share.toFixed(0)}% of flows</span>
                  {drops > 0 && (
                    <span
                      className="flex items-center gap-1 text-red-500"
                      title="Flows the network policy denied"
                    >
                      <ShieldAlert size={12} />
                      {formatFlowRate(drops)} dropped
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
