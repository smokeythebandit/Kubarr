import { useMemo, useRef, useState } from 'react'
import {
  FLOW_CLASS_NAMES,
  FlowClassName,
  FlowSeries,
  formatBandwidth,
} from '../../api/networking'
import { flowClassStyle } from './flowClasses'

interface FlowClassChartProps {
  series: FlowSeries
  height?: number
}

/**
 * The classes differ by orders of magnitude — storage ran at 38 MB/s while the
 * internet managed kilobytes — so on one shared axis the small ones flatten
 * into slivers at the baseline. Hiding a class rescales the axis to what is
 * left, which is the only way to read them against each other.
 */
type HiddenClasses = Partial<Record<FlowClassName, boolean>>

interface Column {
  t: number
  values: Record<FlowClassName, number>
  total: number
}

const PAD_LEFT = 78
const PAD_RIGHT = 8
const PAD_TOP = 8
const PAD_BOTTOM = 20
const VIEW_WIDTH = 1000

/**
 * Align the three series onto one time axis.
 *
 * The classes are separate PromQL queries, so a class with no traffic in a
 * bucket is simply absent rather than zero. Stacking needs every column to
 * carry all three, so a missing sample becomes a real zero here — the one place
 * it's safe to invent, because absence of a counted event *is* zero.
 */
function toColumns(series: FlowSeries): Column[] {
  const byTime = new Map<number, Record<FlowClassName, number>>()

  FLOW_CLASS_NAMES.forEach(name => {
    ;(series[name] ?? []).forEach(point => {
      if (!Number.isFinite(point.bytes_per_sec)) return
      const existing = byTime.get(point.t)
      if (existing) {
        existing[name] = point.bytes_per_sec
      } else {
        byTime.set(point.t, {
          internet: 0,
          lan: 0,
          cluster: 0,
          storage: 0,
          [name]: point.bytes_per_sec,
        } as Record<FlowClassName, number>)
      }
    })
  })

  return [...byTime.entries()]
    .sort(([a], [b]) => a - b)
    .map(([t, values]) => ({
      t,
      values,
      total: FLOW_CLASS_NAMES.reduce((sum, name) => sum + values[name], 0),
    }))
}

function formatClock(unixSeconds: number): string {
  return new Date(unixSeconds * 1000).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
  })
}

/**
 * Stacked byte rate over the selected window. Stacked rather than overlaid
 * because the question is "how is the total split", and the total is the top
 * edge of the stack.
 */
export function FlowClassChart({ series, height = 220 }: FlowClassChartProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const [hoverIndex, setHoverIndex] = useState<number | null>(null)
  const [hidden, setHidden] = useState<HiddenClasses>({})

  const columns = useMemo(() => toColumns(series), [series])

  const visibleClasses = useMemo(
    () => FLOW_CLASS_NAMES.filter(name => !hidden[name]),
    [hidden]
  )

  const peak = useMemo(
    () =>
      Math.max(
        ...columns.map(column =>
          visibleClasses.reduce((sum, name) => sum + column.values[name], 0)
        ),
        0.001
      ),
    [columns, visibleClasses]
  )

  const toggle = (name: FlowClassName) => {
    setHidden(previous => {
      const next = { ...previous, [name]: !previous[name] }
      // Hiding the last visible class would leave an empty chart with no way
      // back except guessing which legend entry to press.
      if (FLOW_CLASS_NAMES.every(each => next[each])) return previous
      return next
    })
  }

  if (columns.length < 2) {
    return (
      <div
        className="flex items-center justify-center rounded-xl border border-gray-200 bg-white text-sm text-gray-500 dark:border-gray-700 dark:bg-gray-800 dark:text-gray-400"
        style={{ height }}
      >
        Not enough history yet for this window
      </div>
    )
  }

  const plotWidth = VIEW_WIDTH - PAD_LEFT - PAD_RIGHT
  const plotHeight = height - PAD_TOP - PAD_BOTTOM

  const xAt = (index: number) =>
    PAD_LEFT + (index / (columns.length - 1)) * plotWidth
  const yAt = (value: number) => PAD_TOP + plotHeight - (value / peak) * plotHeight

  // Build each class as a band sitting on the one below it.
  const bands = (() => {
    const baseline = new Array(columns.length).fill(0)
    return visibleClasses.map(name => {
      const style = flowClassStyle(name)
      const tops = columns.map((column, index) => {
        const top = baseline[index] + column.values[name]
        return top
      })
      const area = [
        ...tops.map((top, index) => `${index === 0 ? 'M' : 'L'}${xAt(index)},${yAt(top)}`),
        ...baseline
          .map((base, index) => ({ base, index }))
          .reverse()
          .map(({ base, index }) => `L${xAt(index)},${yAt(base)}`),
        'Z',
      ].join(' ')
      const line = tops
        .map((top, index) => `${index === 0 ? 'M' : 'L'}${xAt(index)},${yAt(top)}`)
        .join(' ')
      tops.forEach((top, index) => {
        baseline[index] = top
      })
      return { style, area, line }
    })
  })()

  const hovered = hoverIndex === null ? null : columns[hoverIndex]

  const handleMove = (event: React.MouseEvent<HTMLDivElement>) => {
    const bounds = containerRef.current?.getBoundingClientRect()
    if (!bounds) return
    const ratio = (event.clientX - bounds.left) / bounds.width
    const viewX = ratio * VIEW_WIDTH
    const fraction = (viewX - PAD_LEFT) / plotWidth
    const index = Math.round(fraction * (columns.length - 1))
    setHoverIndex(Math.min(columns.length - 1, Math.max(0, index)))
  }

  const latest = columns[columns.length - 1]

  return (
    <div className="overflow-hidden rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
      <div className="flex flex-wrap items-center gap-2 border-b border-gray-200 px-3 py-2 dark:border-gray-700">
        {FLOW_CLASS_NAMES.map(name => {
          const style = flowClassStyle(name)
          const isHidden = Boolean(hidden[name])
          return (
            <button
              key={name}
              type="button"
              onClick={() => toggle(name)}
              aria-pressed={!isHidden}
              title={`${isHidden ? 'Show' : 'Hide'} ${style.label} — ${style.description}`}
              className={`flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs transition-colors ${
                isHidden
                  ? 'border-gray-200 text-gray-400 dark:border-gray-700 dark:text-gray-600'
                  : `${style.border} ${style.bg} ${style.text}`
              }`}
            >
              <span
                className={`h-2 w-2 rounded-full ${
                  isHidden ? 'bg-gray-300 dark:bg-gray-600' : style.swatch
                }`}
              />
              <span>{style.label}</span>
              <span className="font-mono font-semibold">
                {formatBandwidth(latest.values[name])}
              </span>
            </button>
          )
        })}
        <span className="ml-auto text-xs text-gray-400 dark:text-gray-500">
          Click a class to hide it and rescale
        </span>
      </div>

      <div
        ref={containerRef}
        className="relative"
        onMouseMove={handleMove}
        onMouseLeave={() => setHoverIndex(null)}
      >
      <svg
        viewBox={`0 0 ${VIEW_WIDTH} ${height}`}
        preserveAspectRatio="none"
        className="block w-full"
        style={{ height }}
      >
        {[0, 0.5, 1].map(fraction => {
          const value = peak * (1 - fraction)
          const y = PAD_TOP + fraction * plotHeight
          return (
            <g key={fraction}>
              <line
                x1={PAD_LEFT}
                x2={VIEW_WIDTH - PAD_RIGHT}
                y1={y}
                y2={y}
                stroke="currentColor"
                strokeWidth={1}
                className="text-gray-200 dark:text-gray-700"
              />
              <text
                x={PAD_LEFT - 6}
                y={y + 3}
                textAnchor="end"
                className="fill-gray-400 text-[10px] dark:fill-gray-500"
                style={{ fontSize: 10 }}
              >
                {formatBandwidth(value)}
              </text>
            </g>
          )
        })}

        {bands.map(band => (
          <g key={band.style.name}>
            <path d={band.area} fill={band.style.stroke} opacity={0.35} />
            <path d={band.line} fill="none" stroke={band.style.stroke} strokeWidth={2} />
          </g>
        ))}

        {hoverIndex !== null && (
          <line
            x1={xAt(hoverIndex)}
            x2={xAt(hoverIndex)}
            y1={PAD_TOP}
            y2={PAD_TOP + plotHeight}
            stroke="currentColor"
            strokeWidth={1}
            strokeDasharray="3 3"
            className="text-gray-400 dark:text-gray-500"
          />
        )}

        <text
          x={PAD_LEFT}
          y={height - 6}
          className="fill-gray-400 dark:fill-gray-500"
          style={{ fontSize: 10 }}
        >
          {formatClock(columns[0].t)}
        </text>
        <text
          x={VIEW_WIDTH - PAD_RIGHT}
          y={height - 6}
          textAnchor="end"
          className="fill-gray-400 dark:fill-gray-500"
          style={{ fontSize: 10 }}
        >
          {formatClock(columns[columns.length - 1].t)}
        </text>
      </svg>

      {hovered && (
        <div className="pointer-events-none absolute right-3 top-3 rounded-lg bg-gray-900/90 px-3 py-2 text-xs text-white shadow-lg">
          <div className="mb-1 font-semibold">{formatClock(hovered.t)}</div>
          {visibleClasses.map(name => {
            const style = flowClassStyle(name)
            return (
              <div key={name} className="flex items-center gap-2">
                <span className={`h-2 w-2 rounded-full ${style.swatch}`} />
                <span className="text-gray-300">{style.label}</span>
                <span className="ml-auto font-mono">
                  {formatBandwidth(hovered.values[name])}
                </span>
              </div>
            )
          })}
          <div className="mt-1 flex items-center gap-2 border-t border-white/20 pt-1">
            <span className="text-gray-300">
              {visibleClasses.length === FLOW_CLASS_NAMES.length ? 'Total' : 'Shown'}
            </span>
            <span className="ml-auto font-mono">
              {formatBandwidth(
                visibleClasses.reduce((sum, name) => sum + hovered.values[name], 0)
              )}
            </span>
          </div>
        </div>
      )}
      </div>
    </div>
  )
}
