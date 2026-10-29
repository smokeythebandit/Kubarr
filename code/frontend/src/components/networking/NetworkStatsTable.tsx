import { useMemo } from 'react'
import {
  AlertCircle,
  ArrowDownToLine,
  ArrowUpFromLine,
  Wifi,
  WifiOff,
} from 'lucide-react'
import {
  AppClassRates,
  ClassRates,
  ClassTotals,
  FLOW_CLASS_NAMES,
  NetworkStats,
  formatBandwidth,
  formatFlowRate,
  formatPackets,
} from '../../api/networking'
import { AppIcon } from '../AppIcon'
import { flowClassStyle } from './flowClasses'

const EMPTY_RATES: ClassRates = { flows_per_sec: 0, drops_per_sec: 0 }
const EMPTY_TOTALS: ClassTotals = {
  internet: EMPTY_RATES,
  lan: EMPTY_RATES,
  cluster: EMPTY_RATES,
}

interface Row {
  key: string
  name: string
  /** Absent for a source Hubble knows but cAdvisor has no pod for, e.g. the node itself. */
  stats?: NetworkStats
  classes: ClassTotals
}

interface NetworkStatsTableProps {
  stats: NetworkStats[]
  byApp: AppClassRates[]
  /** True when Hubble reported nothing; the class columns then show nothing. */
  stale: boolean
}

/**
 * Per-app detail, now in two measures side by side: how much (bytes, cAdvisor)
 * and where to (flows per class, Hubble).
 *
 * Rows are the union of both sources. Hubble sees senders cAdvisor has no pod
 * for — the node's own kubelet, which is what performs the NFS mounts — and
 * dropping those rows would hide where the LAN traffic actually comes from.
 */
export function NetworkStatsTable({ stats, byApp, stale }: NetworkStatsTableProps) {
  const rows = useMemo<Row[]>(() => {
    const classesByApp = new Map(byApp.map(entry => [entry.app, entry]))
    const merged = new Map<string, Row>()

    stats.forEach(stat => {
      merged.set(stat.namespace, {
        key: stat.namespace,
        name: stat.app_name,
        stats: stat,
        classes: classesByApp.get(stat.namespace) ?? EMPTY_TOTALS,
      })
    })

    byApp.forEach(entry => {
      if (merged.has(entry.app)) return
      merged.set(entry.app, {
        key: entry.app,
        name: entry.app,
        classes: entry,
      })
    })

    const weight = (row: Row) =>
      (row.stats ? row.stats.rx_bytes_per_sec + row.stats.tx_bytes_per_sec : 0) * 1e6 +
      FLOW_CLASS_NAMES.reduce((sum, name) => sum + row.classes[name].flows_per_sec, 0)

    return [...merged.values()].sort((a, b) => weight(b) - weight(a))
  }, [stats, byApp])

  const headerCell =
    'px-4 py-3 text-sm font-medium text-gray-600 dark:text-gray-300 whitespace-nowrap'

  return (
    <div className="overflow-hidden rounded-lg border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
      <div className="overflow-x-auto">
        <table className="w-full">
          <thead>
            <tr className="border-b border-gray-200 bg-gray-50 dark:border-gray-700 dark:bg-gray-700/50">
              <th className={`text-left ${headerCell}`}>App</th>
              <th className={`text-right ${headerCell}`}>
                <div className="flex items-center justify-end gap-1">
                  <ArrowDownToLine size={14} className="text-green-500" />
                  Receive
                </div>
              </th>
              <th className={`text-right ${headerCell}`}>
                <div className="flex items-center justify-end gap-1">
                  <ArrowUpFromLine size={14} className="text-orange-500" />
                  Transmit
                </div>
              </th>
              <th className={`text-right ${headerCell}`}>Packets</th>
              {FLOW_CLASS_NAMES.map(name => {
                const style = flowClassStyle(name)
                const Icon = style.icon
                return (
                  <th key={name} className={`text-right ${headerCell}`} title={style.description}>
                    <div className="flex items-center justify-end gap-1">
                      <Icon size={14} className={style.text} />
                      {style.label}
                    </div>
                  </th>
                )
              })}
              <th className={`text-center ${headerCell}`}>Status</th>
            </tr>
          </thead>
          <tbody>
            {rows.map(row => {
              const hasErrors = row.stats
                ? row.stats.rx_errors_per_sec > 0 || row.stats.tx_errors_per_sec > 0
                : false
              const hasDropped = row.stats
                ? row.stats.rx_dropped_per_sec > 0 || row.stats.tx_dropped_per_sec > 0
                : false
              const deniedFlows = FLOW_CLASS_NAMES.reduce(
                (sum, name) => sum + row.classes[name].drops_per_sec,
                0
              )

              return (
                <tr
                  key={row.key}
                  className="border-b border-gray-200 hover:bg-gray-50 dark:border-gray-700/50 dark:hover:bg-gray-700/30"
                >
                  <td className="px-4 py-3">
                    <div className="flex items-center gap-3">
                      <AppIcon appName={row.key} size={32} />
                      <span className="font-medium text-gray-900 dark:text-white">{row.name}</span>
                    </div>
                  </td>
                  <td className="px-4 py-3 text-right">
                    {row.stats ? (
                      <span className="font-mono text-sm text-green-500">
                        {formatBandwidth(row.stats.rx_bytes_per_sec)}
                      </span>
                    ) : (
                      <span className="text-sm text-gray-400">—</span>
                    )}
                  </td>
                  <td className="px-4 py-3 text-right">
                    {row.stats ? (
                      <span className="font-mono text-sm text-orange-500">
                        {formatBandwidth(row.stats.tx_bytes_per_sec)}
                      </span>
                    ) : (
                      <span className="text-sm text-gray-400">—</span>
                    )}
                  </td>
                  <td className="px-4 py-3 text-right">
                    {row.stats ? (
                      <div className="font-mono text-xs text-gray-500 dark:text-gray-400">
                        <div>{formatPackets(row.stats.rx_packets_per_sec)} in</div>
                        <div>{formatPackets(row.stats.tx_packets_per_sec)} out</div>
                      </div>
                    ) : (
                      <span className="text-sm text-gray-400">—</span>
                    )}
                  </td>
                  {FLOW_CLASS_NAMES.map(name => {
                    const style = flowClassStyle(name)
                    const rate = row.classes[name].flows_per_sec
                    return (
                      <td key={name} className="px-4 py-3 text-right">
                        {stale ? (
                          <span className="text-sm text-gray-400">—</span>
                        ) : (
                          <span
                            className={`font-mono text-sm ${
                              rate > 0 ? style.text : 'text-gray-400 dark:text-gray-600'
                            }`}
                          >
                            {rate > 0 ? formatFlowRate(rate) : '0'}
                          </span>
                        )}
                      </td>
                    )
                  })}
                  <td className="px-4 py-3 text-center">
                    {deniedFlows > 0 ? (
                      <span
                        className="inline-flex items-center gap-1 rounded-full bg-red-100 px-2 py-1 text-xs text-red-600 dark:bg-red-900/30 dark:text-red-400"
                        title="Flows the network policy denied"
                      >
                        <AlertCircle size={12} />
                        Denied
                      </span>
                    ) : hasErrors ? (
                      <span className="inline-flex items-center gap-1 rounded-full bg-red-100 px-2 py-1 text-xs text-red-600 dark:bg-red-900/30 dark:text-red-400">
                        <WifiOff size={12} />
                        Errors
                      </span>
                    ) : hasDropped ? (
                      <span className="inline-flex items-center gap-1 rounded-full bg-yellow-100 px-2 py-1 text-xs text-yellow-600 dark:bg-yellow-900/30 dark:text-yellow-400">
                        <AlertCircle size={12} />
                        Drops
                      </span>
                    ) : (
                      <span className="inline-flex items-center gap-1 rounded-full bg-green-100 px-2 py-1 text-xs text-green-600 dark:bg-green-900/30 dark:text-green-400">
                        <Wifi size={12} />
                        Healthy
                      </span>
                    )}
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>

      <p className="border-t border-gray-200 px-4 py-2 text-xs text-gray-500 dark:border-gray-700 dark:text-gray-400">
        Receive and transmit are bytes per second per pod. The three class columns are flow
        events per second — Hubble counts connections, not bytes. LAN traffic is usually
        attributed to the node rather than an app, because the kubelet performs the NFS
        mounts on the pods' behalf.
      </p>
    </div>
  )
}
