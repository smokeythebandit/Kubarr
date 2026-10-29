import {
  ClassTotals,
  FLOW_CLASS_NAMES,
  formatBandwidth,
  totalByteRate,
} from '../../api/networking'
import { flowClassStyle } from './flowClasses'

interface FlowSplitBarProps {
  totals: ClassTotals
  /** Bar thickness in pixels. */
  height?: number
  /** Show the per-class rate under the bar. */
  showLegend?: boolean
  className?: string
}

/**
 * The split as one bar: how much traffic went where, at a glance.
 *
 * Proportions are by bytes, which is what the page leads with. A class with a
 * real but tiny share still gets a visible sliver, because "almost no LAN
 * traffic" and "no LAN traffic" are different answers and the bar is often the
 * only place the difference shows.
 */
const MIN_VISIBLE_SHARE = 0.02

export function FlowSplitBar({
  totals,
  height = 8,
  showLegend = false,
  className = '',
}: FlowSplitBarProps) {
  const total = totalByteRate(totals)

  const segments = FLOW_CLASS_NAMES.map(name => {
    const style = flowClassStyle(name)
    const rate = totals[name].bytes_per_sec
    const share = total > 0 ? rate / total : 0
    return {
      style,
      rate,
      drops: totals[name].drops_per_sec,
      share,
      width: rate > 0 ? Math.max(share, MIN_VISIBLE_SHARE) : 0,
    }
  })

  const widthSum = segments.reduce((sum, s) => sum + s.width, 0)

  return (
    <div className={className}>
      <div
        className="flex w-full overflow-hidden rounded-full bg-gray-200 dark:bg-gray-700"
        style={{ height }}
        role="img"
        aria-label={segments
          .map(s => `${s.style.label} ${formatBandwidth(s.rate)}`)
          .join(', ')}
      >
        {widthSum > 0 &&
          segments.map(segment =>
            segment.width > 0 ? (
              <div
                key={segment.style.name}
                className={`${segment.style.swatch} transition-all duration-500`}
                style={{ width: `${(segment.width / widthSum) * 100}%` }}
                title={`${segment.style.label}: ${formatBandwidth(segment.rate)} (${(
                  segment.share * 100
                ).toFixed(0)}%)`}
              />
            ) : null
          )}
      </div>

      {showLegend && (
        <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1">
          {segments.map(segment => (
            <div key={segment.style.name} className="flex items-center gap-1.5">
              <span
                className={`h-2 w-2 shrink-0 rounded-full ${segment.style.swatch}`}
              />
              <span className="text-xs text-gray-500 dark:text-gray-400">
                {segment.style.label}
              </span>
              <span className="font-mono text-xs font-semibold text-gray-700 dark:text-gray-200">
                {formatBandwidth(segment.rate)}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
