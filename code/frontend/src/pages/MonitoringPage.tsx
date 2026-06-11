import React, { useState } from 'react'
import { useSearchParams } from 'react-router-dom'
import { useQuery } from '@tanstack/react-query'
import { monitoringApi, TimeSeriesPoint } from '../api/monitoring'
import { logsApi } from '../api/logs'
import { AppIcon } from '../components/AppIcon'
import {
  Activity,
  Cpu,
  HardDrive,
  Server,
  Box,
  RefreshCw,
  AlertCircle,
  TrendingUp,
  Gauge,
  X,
  Clock,
  CheckCircle,
  XCircle,
  RotateCcw,
  FileText,
  ArrowDownToLine,
  ArrowUpFromLine,
  Network
} from 'lucide-react'

// Format bytes to human readable
function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B'
  const k = 1024
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.floor(Math.log(bytes) / Math.log(k))
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(2))} ${sizes[i]}`
}

// Format bytes per second to human readable bandwidth
function formatBandwidth(bytesPerSec: number): string {
  if (bytesPerSec === 0) return '0 B/s'
  const k = 1024
  const sizes = ['B/s', 'KB/s', 'MB/s', 'GB/s']
  const i = Math.floor(Math.log(bytesPerSec) / Math.log(k))
  return `${parseFloat((bytesPerSec / Math.pow(k, i)).toFixed(2))} ${sizes[i]}`
}

// Format CPU cores to millicores or cores
function formatCpu(cores: number): string {
  if (cores < 0.001) return '< 1m'
  if (cores < 1) return `${Math.round(cores * 1000)}m`
  return `${cores.toFixed(2)} cores`
}

// Format timestamp
function formatTime(timestamp: number): string {
  return new Date(timestamp * 1000).toLocaleTimeString('en-US', {
    hour: '2-digit',
    minute: '2-digit',
  })
}

function formatPercent(value: number): string {
  return `${value.toFixed(1)}%`
}

function formatCount(value: number): string {
  return String(Math.round(value))
}

// Progress bar component
function ProgressBar({ value, max, color = 'blue' }: { value: number; max: number; color?: string }) {
  const percentage = max > 0 ? Math.min((value / max) * 100, 100) : 0
  const colorClasses: Record<string, string> = {
    blue: 'bg-blue-500',
    green: 'bg-green-500',
    yellow: 'bg-yellow-500',
    red: 'bg-red-500',
  }

  return (
    <div className="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-2">
      <div
        className={`h-2 rounded-full transition-all duration-300 ${colorClasses[color] || colorClasses.blue}`}
        style={{ width: `${percentage}%` }}
      />
    </div>
  )
}

// Interactive line chart component with smooth curves, grid lines, and tooltips
function SimpleChart({
  data,
  color = 'blue',
  height = 120,
  formatValue,
}: {
  data: TimeSeriesPoint[];
  color?: string;
  height?: number;
  formatValue: (v: number) => string;
}) {
  const [hoverInfo, setHoverInfo] = useState<{ x: number; y: number; value: number; time: string; clientX: number } | null>(null)
  const chartRef = React.useRef<HTMLDivElement>(null)

  if (!data || data.length === 0) {
    return (
      <div className="flex items-center justify-center text-gray-500" style={{ height: height + 20 }}>
        No data available
      </div>
    )
  }

  // Filter out zero/invalid values at the edges that might be artifacts
  const filteredData = data.filter((d, i) => {
    // Keep all middle points
    if (i > 0 && i < data.length - 1) return true
    // For edge points, only keep if they're not suspiciously zero when neighbors aren't
    if (i === 0 && data.length > 1) {
      return d.value > 0 || data[1].value === 0
    }
    if (i === data.length - 1 && data.length > 1) {
      return d.value > 0 || data[data.length - 2].value === 0
    }
    return true
  })

  if (filteredData.length < 2) {
    return (
      <div className="flex items-center justify-center text-gray-500" style={{ height: height + 20 }}>
        Not enough data
      </div>
    )
  }

  const values = filteredData.map(d => d.value)
  const maxValue = Math.max(...values)
  const minValue = Math.min(...values)
  // Add padding to prevent line from touching edges
  const range = maxValue - minValue || maxValue * 0.1 || 1
  const paddedMin = minValue - range * 0.1
  const paddedMax = maxValue + range * 0.1

  const colorMap: Record<string, { stroke: string; fill: string }> = {
    blue: { stroke: '#3b82f6', fill: 'rgba(59, 130, 246, 0.15)' },
    green: { stroke: '#22c55e', fill: 'rgba(34, 197, 94, 0.15)' },
  }
  const colors = colorMap[color] || colorMap.blue

  // Generate smooth curve using cubic bezier
  const width = 100
  const chartHeight = height - 10
  const points = filteredData.map((d, i) => {
    const x = (i / (filteredData.length - 1)) * width
    const y = chartHeight - ((d.value - paddedMin) / (paddedMax - paddedMin)) * chartHeight
    return { x, y, value: d.value, timestamp: d.timestamp }
  })

  // Create smooth bezier curve
  let linePath = `M ${points[0].x},${points[0].y}`
  for (let i = 1; i < points.length; i++) {
    const prev = points[i - 1]
    const curr = points[i]
    const tension = 0.3
    const cp1x = prev.x + (curr.x - prev.x) * tension
    const cp1y = prev.y
    const cp2x = curr.x - (curr.x - prev.x) * tension
    const cp2y = curr.y
    linePath += ` C ${cp1x},${cp1y} ${cp2x},${cp2y} ${curr.x},${curr.y}`
  }

  const areaPath = `${linePath} L ${width},${chartHeight} L 0,${chartHeight} Z`

  // Get first and last timestamps for x-axis labels
  const startTime = formatTime(filteredData[0].timestamp)
  const endTime = formatTime(filteredData[filteredData.length - 1].timestamp)

  // Generate grid lines (4 horizontal lines)
  const gridLines = [0.25, 0.5, 0.75].map(ratio => chartHeight * ratio)

  // Generate Y-axis labels
  const yAxisLabels = [0.25, 0.5, 0.75].map(ratio => {
    const value = paddedMax - (paddedMax - paddedMin) * ratio
    return { y: chartHeight * ratio, label: formatValue(value) }
  })

  // Handle mouse move for tooltip
  const handleMouseMove = (e: React.MouseEvent<HTMLDivElement>) => {
    if (!chartRef.current) return
    const rect = chartRef.current.getBoundingClientRect()
    const relativeX = (e.clientX - rect.left) / rect.width
    const dataIndex = Math.round(relativeX * (filteredData.length - 1))
    const clampedIndex = Math.max(0, Math.min(dataIndex, filteredData.length - 1))
    const point = points[clampedIndex]

    if (point) {
      setHoverInfo({
        x: point.x,
        y: point.y,
        value: point.value,
        time: formatTime(point.timestamp),
        clientX: e.clientX - rect.left,
      })
    }
  }

  const handleMouseLeave = () => {
    setHoverInfo(null)
  }

  return (
    <div className="relative" style={{ height: height + 20 }}>
      <div className="flex">
        {/* Y-axis labels column */}
        <div className="flex flex-col justify-between text-[9px] text-gray-500 dark:text-gray-400 pr-2 shrink-0" style={{ height, width: 45 }}>
          <span className="text-right">{formatValue(paddedMax)}</span>
          {yAxisLabels.map((label, i) => (
            <span key={i} className="text-right">{label.label}</span>
          ))}
          <span className="text-right">{formatValue(paddedMin)}</span>
        </div>

        {/* Chart area */}
        <div
          ref={chartRef}
          className="relative cursor-crosshair flex-1"
          onMouseMove={handleMouseMove}
          onMouseLeave={handleMouseLeave}
        >
          <svg
            viewBox={`0 0 ${width} ${chartHeight}`}
            preserveAspectRatio="none"
            className="w-full"
            style={{ height }}
          >
            <defs>
              <linearGradient id={`gradient-${color}`} x1="0%" y1="0%" x2="0%" y2="100%">
                <stop offset="0%" stopColor={colors.fill.replace('0.15', '0.3')} />
                <stop offset="100%" stopColor={colors.fill.replace('0.15', '0.05')} />
              </linearGradient>
            </defs>

            {/* Grid lines */}
            {gridLines.map((y, i) => (
              <line
                key={i}
                x1="0"
                y1={y}
                x2={width}
                y2={y}
                stroke="rgba(75, 85, 99, 0.3)"
                strokeWidth="0.3"
                strokeDasharray="2,2"
              />
            ))}

            {/* Area fill */}
            <path d={areaPath} fill={`url(#gradient-${color})`} />

            {/* Main line - thinner stroke */}
            <path
              d={linePath}
              stroke={colors.stroke}
              fill="none"
              strokeWidth="1.2"
              strokeLinecap="round"
              strokeLinejoin="round"
              vectorEffect="non-scaling-stroke"
            />

            {/* Hover indicator line */}
            {hoverInfo && (
              <>
                <line
                  x1={hoverInfo.x}
                  y1="0"
                  x2={hoverInfo.x}
                  y2={chartHeight}
                  stroke="rgba(156, 163, 175, 0.5)"
                  strokeWidth="0.5"
                  strokeDasharray="2,2"
                />
                <circle
                  cx={hoverInfo.x}
                  cy={hoverInfo.y}
                  r="2"
                  fill={colors.stroke}
                  stroke="white"
                  strokeWidth="1"
                />
              </>
            )}
          </svg>

          {/* Tooltip */}
          {hoverInfo && (
            <div
              className="absolute z-10 bg-gray-900 dark:bg-gray-800 border border-gray-600 rounded px-2 py-1 text-xs pointer-events-none shadow-lg"
              style={{
                left: Math.min(hoverInfo.clientX + 10, chartRef.current ? chartRef.current.offsetWidth - 100 : 0),
                top: -30,
              }}
            >
              <div className="text-gray-400">{hoverInfo.time}</div>
              <div className="font-medium" style={{ color: colors.stroke }}>{formatValue(hoverInfo.value)}</div>
            </div>
          )}
        </div>
      </div>

      {/* X-axis time labels */}
      <div className="flex text-xs text-gray-500 dark:text-gray-400 mt-1" style={{ paddingLeft: 45 }}>
        <span>{startTime}</span>
        <span className="flex-1 text-center text-gray-400">{formatValue(values[values.length - 1])}</span>
        <span>{endTime}</span>
      </div>
    </div>
  )
}

// Mini sparkline chart for inline card use
export function MiniSparkline({
  data,
  color = 'green',
  height = 40,
  interactive = false,
  secondaryData,
  secondaryColor,
  formatValue,
}: {
  data: TimeSeriesPoint[];
  color?: string;
  height?: number;
  interactive?: boolean;
  secondaryData?: TimeSeriesPoint[];
  secondaryColor?: string;
  formatValue?: (v: number) => string;
}) {
  const [hoverInfo, setHoverInfo] = useState<{ x: number; primaryY: number; primaryVal: number; secondaryY?: number; secondaryVal?: number; time: string; pxX: number } | null>(null)
  const containerRef = React.useRef<HTMLDivElement>(null)

  if (!data || data.length < 2) return null

  const colorMap: Record<string, { stroke: string; fill: string }> = {
    blue: { stroke: '#3b82f6', fill: 'rgba(59, 130, 246, 0.15)' },
    green: { stroke: '#22c55e', fill: 'rgba(34, 197, 94, 0.15)' },
    purple: { stroke: '#a855f7', fill: 'rgba(168, 85, 247, 0.15)' },
    red: { stroke: '#ef4444', fill: 'rgba(239, 68, 68, 0.15)' },
    cyan: { stroke: '#06b6d4', fill: 'rgba(6, 182, 212, 0.15)' },
    yellow: { stroke: '#eab308', fill: 'rgba(234, 179, 8, 0.15)' },
  }
  const colors = colorMap[color] || colorMap.green
  const secColors = secondaryColor ? (colorMap[secondaryColor] || colorMap.red) : null

  // Compute shared scale across both series
  const allValues = [...data.map(d => d.value), ...(secondaryData || []).map(d => d.value)]
  const maxVal = Math.max(...allValues)
  const minVal = Math.min(...allValues)
  const range = maxVal - minVal || maxVal * 0.1 || 1
  const paddedMin = minVal - range * 0.1
  const paddedMax = maxVal + range * 0.1

  const width = 100
  const chartHeight = height

  const buildSeries = (series: TimeSeriesPoint[]) => {
    const pts = series.map((d, i) => {
      const x = (i / (series.length - 1)) * width
      const y = chartHeight - ((d.value - paddedMin) / (paddedMax - paddedMin)) * chartHeight
      return { x, y, value: d.value, timestamp: d.timestamp }
    })
    let line = `M ${pts[0].x},${pts[0].y}`
    for (let i = 1; i < pts.length; i++) {
      const prev = pts[i - 1]
      const curr = pts[i]
      const tension = 0.3
      const cp1x = prev.x + (curr.x - prev.x) * tension
      const cp2x = curr.x - (curr.x - prev.x) * tension
      line += ` C ${cp1x},${prev.y} ${cp2x},${curr.y} ${curr.x},${curr.y}`
    }
    const area = `${line} L ${width},${chartHeight} L 0,${chartHeight} Z`
    return { pts, line, area }
  }

  const primary = buildSeries(data)
  const secondary = secondaryData && secondaryData.length >= 2 ? buildSeries(secondaryData) : null

  const handleMouseMove = interactive ? (e: React.MouseEvent<HTMLDivElement>) => {
    if (!containerRef.current) return
    const rect = containerRef.current.getBoundingClientRect()
    const relativeX = (e.clientX - rect.left) / rect.width
    const idx = Math.max(0, Math.min(Math.round(relativeX * (primary.pts.length - 1)), primary.pts.length - 1))
    const pt = primary.pts[idx]
    const secPt = secondary ? secondary.pts[Math.max(0, Math.min(idx, secondary.pts.length - 1))] : undefined
    setHoverInfo({
      x: pt.x,
      primaryY: pt.y,
      primaryVal: pt.value,
      secondaryY: secPt?.y,
      secondaryVal: secPt?.value,
      time: formatTime(pt.timestamp),
      pxX: e.clientX - rect.left,
    })
  } : undefined

  return (
    <div
      ref={containerRef}
      className={`relative ${interactive ? 'cursor-crosshair' : ''}`}
      onMouseMove={handleMouseMove}
      onMouseLeave={interactive ? () => setHoverInfo(null) : undefined}
    >
      <svg
        viewBox={`0 0 ${width} ${chartHeight}`}
        preserveAspectRatio="none"
        className="w-full"
        style={{ height }}
      >
        <defs>
          <linearGradient id={`sparkline-grad-${color}`} x1="0%" y1="0%" x2="0%" y2="100%">
            <stop offset="0%" stopColor={colors.fill.replace('0.15', '0.3')} />
            <stop offset="100%" stopColor={colors.fill.replace('0.15', '0.02')} />
          </linearGradient>
          {secColors && (
            <linearGradient id={`sparkline-grad-${secondaryColor}`} x1="0%" y1="0%" x2="0%" y2="100%">
              <stop offset="0%" stopColor={secColors.fill.replace('0.15', '0.3')} />
              <stop offset="100%" stopColor={secColors.fill.replace('0.15', '0.02')} />
            </linearGradient>
          )}
        </defs>
        <path d={primary.area} fill={`url(#sparkline-grad-${color})`} />
        <path d={primary.line} stroke={colors.stroke} fill="none" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" vectorEffect="non-scaling-stroke" />
        {secondary && secColors && (
          <>
            <path d={secondary.area} fill={`url(#sparkline-grad-${secondaryColor})`} />
            <path d={secondary.line} stroke={secColors.stroke} fill="none" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" vectorEffect="non-scaling-stroke" />
          </>
        )}
        {hoverInfo && (
          <>
            <line x1={hoverInfo.x} y1="0" x2={hoverInfo.x} y2={chartHeight} stroke="rgba(156,163,175,0.5)" strokeWidth="0.5" strokeDasharray="2,2" />
            <circle cx={hoverInfo.x} cy={hoverInfo.primaryY} r="1" fill={colors.stroke} />
            {hoverInfo.secondaryY !== undefined && secColors && (
              <circle cx={hoverInfo.x} cy={hoverInfo.secondaryY} r="1" fill={secColors.stroke} />
            )}
          </>
        )}
      </svg>
      {hoverInfo && (
        <div
          className="absolute z-50 bg-gray-900 dark:bg-gray-800 border border-gray-600 rounded px-2 py-1 text-[10px] pointer-events-none shadow-lg whitespace-nowrap"
          style={{
            left: Math.min(hoverInfo.pxX + 8, (containerRef.current?.offsetWidth || 200) - 120),
            top: -38,
          }}
        >
          <div className="text-gray-400">{hoverInfo.time}</div>
          <div className="flex items-center gap-2">
            <span className="font-medium" style={{ color: colors.stroke }}>{secondaryData ? '↓ ' : ''}{(formatValue || formatBandwidth)(hoverInfo.primaryVal)}</span>
            {hoverInfo.secondaryVal !== undefined && secColors && (
              <span className="font-medium" style={{ color: secColors.stroke }}>↑ {(formatValue || formatBandwidth)(hoverInfo.secondaryVal)}</span>
            )}
          </div>
        </div>
      )}
    </div>
  )
}

// Cluster stats card
function ClusterStatsCard({
  icon: Icon,
  label,
  value,
  detailValue,
  detailSub,
  color = 'blue',
  sparklineData,
  sparklineFormatValue,
}: {
  icon: React.ElementType;
  label: string;
  value: string;
  detailValue?: string;
  detailSub?: string;
  color?: string;
  sparklineData?: TimeSeriesPoint[];
  sparklineFormatValue?: (v: number) => string;
}) {
  const iconBgClasses: Record<string, string> = {
    blue: 'from-blue-500/20 to-blue-600/10',
    green: 'from-green-500/20 to-green-600/10',
    yellow: 'from-yellow-500/20 to-yellow-600/10',
    purple: 'from-purple-500/20 to-purple-600/10',
    cyan: 'from-cyan-500/20 to-cyan-600/10',
  }
  const iconColorClasses: Record<string, string> = {
    blue: 'text-blue-500 dark:text-blue-400',
    green: 'text-green-500 dark:text-green-400',
    yellow: 'text-yellow-500 dark:text-yellow-400',
    purple: 'text-purple-500 dark:text-purple-400',
    cyan: 'text-cyan-500 dark:text-cyan-400',
  }
  const detailColorClasses: Record<string, string> = {
    blue: 'text-blue-500 dark:text-blue-400',
    green: 'text-green-500 dark:text-green-400',
    yellow: 'text-yellow-500 dark:text-yellow-400',
    purple: 'text-purple-500 dark:text-purple-400',
    cyan: 'text-cyan-500 dark:text-cyan-400',
  }
  const hasSparkline = sparklineData && sparklineData.length >= 2

  return (
    <div className="relative bg-gradient-to-br from-white to-gray-50 dark:from-gray-800 dark:to-gray-900 rounded-xl p-5 h-[125px] border border-gray-200/60 dark:border-gray-700/60 shadow-[0_4px_12px_rgba(0,0,0,0.05),0_1px_3px_rgba(0,0,0,0.1)] dark:shadow-[0_4px_12px_rgba(0,0,0,0.3),inset_0_1px_0_rgba(255,255,255,0.05)] overflow-hidden flex flex-col justify-between">
      <div className="relative flex items-center gap-3">
        <div className={`p-2.5 bg-gradient-to-br ${iconBgClasses[color] || iconBgClasses.blue} rounded-xl shadow-inner`}>
          <Icon className={`w-5 h-5 ${iconColorClasses[color] || iconColorClasses.blue}`} />
        </div>
        <div className="flex-1">
          <div className="text-gray-500 dark:text-gray-400 text-sm font-medium">{label}</div>
          <div className="text-xl font-bold text-gray-900 dark:text-white">{value}</div>
        </div>
        {detailValue && (
          <div className="text-right">
            <div className={`text-lg font-bold ${detailColorClasses[color] || detailColorClasses.blue}`}>
              {detailValue}
            </div>
            {detailSub && (
              <div className="text-xs text-gray-500 dark:text-gray-500">{detailSub}</div>
            )}
          </div>
        )}
      </div>
      {hasSparkline && (
        <div className="relative -mx-5 -mb-5">
          <MiniSparkline data={sparklineData} color={color} height={45} interactive formatValue={sparklineFormatValue} />
        </div>
      )}
    </div>
  )
}

// App Detail Modal
function AppDetailModal({
  appName,
  onClose,
}: {
  appName: string;
  onClose: () => void;
}) {
  const [duration, setDuration] = useState('1h')
  const [activeTab, setActiveTab] = useState<'metrics' | 'pods' | 'logs'>('metrics')

  const { data: detailMetrics, isLoading, isFetching } = useQuery({
    queryKey: ['monitoring', 'app', appName, duration],
    queryFn: () => monitoringApi.getAppDetailMetrics(appName, duration),
    refetchInterval: 30000,
    placeholderData: (previousData) => previousData, // Keep previous data while fetching
    staleTime: 10000, // Consider data fresh for 10 seconds
  })

  const { data: logsData, isLoading: logsLoading } = useQuery({
    queryKey: ['logs', appName],
    queryFn: async () => {
      const now = Date.now()
      const start = String((now - 60 * 60 * 1000) * 1e6) // Last hour
      const end = String(now * 1e6)
      return logsApi.queryLogs({
        query: `{namespace="${appName}"}`,
        start,
        end,
        limit: 100,
        direction: 'backward',
      })
    },
    enabled: activeTab === 'logs',
  })

  const durations = [
    { label: '15m', value: '15m' },
    { label: '1h', value: '1h' },
    { label: '3h', value: '3h' },
    { label: '6h', value: '6h' },
    { label: '12h', value: '12h' },
    { label: '24h', value: '24h' },
  ]

  // Flatten logs
  const logEntries = logsData?.streams?.flatMap(stream =>
    stream.entries.map(entry => ({
      ...entry,
      labels: stream.labels,
    }))
  ).sort((a, b) => new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime()) || []

  return (
    <div className="fixed inset-0 bg-black/70 flex items-center justify-center z-50 p-4">
      <div className="bg-white dark:bg-gray-800 rounded-xl w-full max-w-4xl max-h-[90vh] overflow-hidden flex flex-col border border-gray-200 dark:border-gray-700">
        {/* Header */}
        <div className="flex items-center justify-between p-6 border-b border-gray-200 dark:border-gray-700">
          <div className="flex items-center gap-4">
            <AppIcon appName={appName} size={48} />
            <div>
              <h2 className="text-2xl font-bold capitalize text-gray-900 dark:text-white">{appName}</h2>
              <p className="text-gray-500 dark:text-gray-400 text-sm">Detailed metrics and information</p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-2 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg transition-colors text-gray-600 dark:text-gray-300"
          >
            <X size={24} />
          </button>
        </div>

        {/* Tabs */}
        <div className="flex border-b border-gray-200 dark:border-gray-700">
          <button
            onClick={() => setActiveTab('metrics')}
            className={`px-6 py-3 font-medium transition-colors ${
              activeTab === 'metrics'
                ? 'text-blue-600 dark:text-blue-400 border-b-2 border-blue-500 dark:border-blue-400'
                : 'text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white'
            }`}
          >
            <div className="flex items-center gap-2">
              <TrendingUp size={18} />
              Metrics
            </div>
          </button>
          <button
            onClick={() => setActiveTab('pods')}
            className={`px-6 py-3 font-medium transition-colors ${
              activeTab === 'pods'
                ? 'text-blue-600 dark:text-blue-400 border-b-2 border-blue-500 dark:border-blue-400'
                : 'text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white'
            }`}
          >
            <div className="flex items-center gap-2">
              <Box size={18} />
              Pods ({detailMetrics?.pods?.length || 0})
            </div>
          </button>
          <button
            onClick={() => setActiveTab('logs')}
            className={`px-6 py-3 font-medium transition-colors ${
              activeTab === 'logs'
                ? 'text-blue-600 dark:text-blue-400 border-b-2 border-blue-500 dark:border-blue-400'
                : 'text-gray-500 dark:text-gray-400 hover:text-gray-900 dark:hover:text-white'
            }`}
          >
            <div className="flex items-center gap-2">
              <FileText size={18} />
              Logs
            </div>
          </button>
        </div>

        {/* Content */}
        <div className="flex-1 overflow-auto p-6 min-h-[400px]">
          {isLoading && !detailMetrics ? (
            <div className="flex items-center justify-center py-12">
              <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500"></div>
            </div>
          ) : activeTab === 'metrics' ? (
            <div className="space-y-6">
              {/* Duration selector */}
              <div className="flex items-center gap-2">
                <Clock size={16} className="text-gray-500 dark:text-gray-400" />
                <span className="text-sm text-gray-500 dark:text-gray-400">Time range:</span>
                <div className="flex gap-1">
                  {durations.map(d => (
                    <button
                      key={d.value}
                      onClick={() => setDuration(d.value)}
                      className={`px-3 py-1 text-sm rounded transition-colors ${
                        duration === d.value
                          ? 'bg-blue-600 text-white'
                          : 'bg-gray-200 dark:bg-gray-700 text-gray-600 dark:text-gray-300 hover:bg-gray-300 dark:hover:bg-gray-600'
                      }`}
                    >
                      {d.label}
                    </button>
                  ))}
                </div>
                {isFetching && (
                  <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-blue-500 ml-2"></div>
                )}
              </div>

              {/* Current stats */}
              <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
                <div className="bg-gray-100 dark:bg-gray-900 rounded-lg p-4">
                  <div className="flex items-center gap-2 text-gray-500 dark:text-gray-400 mb-2">
                    <Cpu size={16} />
                    <span className="text-sm">CPU Usage</span>
                  </div>
                  <div className="text-2xl font-bold text-blue-500 dark:text-blue-400">
                    {formatCpu(detailMetrics?.historical?.cpu_usage_cores || 0)}
                  </div>
                </div>
                <div className="bg-gray-100 dark:bg-gray-900 rounded-lg p-4">
                  <div className="flex items-center gap-2 text-gray-500 dark:text-gray-400 mb-2">
                    <HardDrive size={16} />
                    <span className="text-sm">Memory Usage</span>
                  </div>
                  <div className="text-2xl font-bold text-green-500 dark:text-green-400">
                    {formatBytes(detailMetrics?.historical?.memory_usage_bytes || 0)}
                  </div>
                </div>
                <div className="bg-gray-100 dark:bg-gray-900 rounded-lg p-4">
                  <div className="flex items-center gap-2 text-gray-500 dark:text-gray-400 mb-2">
                    <ArrowDownToLine size={16} className="text-green-500 dark:text-green-400" />
                    <span className="text-sm">Network In</span>
                  </div>
                  <div className="text-2xl font-bold text-green-500 dark:text-green-400">
                    {formatBandwidth(detailMetrics?.historical?.network_receive_bytes_per_sec || 0)}
                  </div>
                </div>
                <div className="bg-gray-100 dark:bg-gray-900 rounded-lg p-4">
                  <div className="flex items-center gap-2 text-gray-500 dark:text-gray-400 mb-2">
                    <ArrowUpFromLine size={16} className="text-orange-500 dark:text-orange-400" />
                    <span className="text-sm">Network Out</span>
                  </div>
                  <div className="text-2xl font-bold text-orange-500 dark:text-orange-400">
                    {formatBandwidth(detailMetrics?.historical?.network_transmit_bytes_per_sec || 0)}
                  </div>
                </div>
              </div>

              {/* Charts */}
              <div className="space-y-6">
                <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
                  <div className="bg-gray-100 dark:bg-gray-900 rounded-lg p-4">
                    <h3 className="text-sm font-medium text-gray-500 dark:text-gray-400 mb-4 flex items-center gap-2">
                      <Cpu size={16} className="text-blue-500 dark:text-blue-400" />
                      CPU Usage History
                    </h3>
                    <SimpleChart
                      data={detailMetrics?.historical?.cpu_series || []}
                      color="blue"
                      height={100}
                      formatValue={formatCpu}
                    />
                  </div>

                  <div className="bg-gray-100 dark:bg-gray-900 rounded-lg p-4">
                    <h3 className="text-sm font-medium text-gray-500 dark:text-gray-400 mb-4 flex items-center gap-2">
                      <HardDrive size={16} className="text-green-500 dark:text-green-400" />
                      Memory Usage History
                    </h3>
                    <SimpleChart
                      data={detailMetrics?.historical?.memory_series || []}
                      color="green"
                      height={100}
                      formatValue={formatBytes}
                    />
                  </div>
                </div>

                <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
                  <div className="bg-gray-100 dark:bg-gray-900 rounded-lg p-4">
                    <h3 className="text-sm font-medium text-gray-500 dark:text-gray-400 mb-4 flex items-center gap-2">
                      <ArrowDownToLine size={16} className="text-green-500 dark:text-green-400" />
                      Network Receive History
                    </h3>
                    <SimpleChart
                      data={detailMetrics?.historical?.network_rx_series || []}
                      color="green"
                      height={100}
                      formatValue={formatBandwidth}
                    />
                  </div>

                  <div className="bg-gray-100 dark:bg-gray-900 rounded-lg p-4">
                    <h3 className="text-sm font-medium text-gray-500 dark:text-gray-400 mb-4 flex items-center gap-2">
                      <ArrowUpFromLine size={16} className="text-orange-500 dark:text-orange-400" />
                      Network Transmit History
                    </h3>
                    <SimpleChart
                      data={detailMetrics?.historical?.network_tx_series || []}
                      color="blue"
                      height={100}
                      formatValue={formatBandwidth}
                    />
                  </div>
                </div>
              </div>
            </div>
          ) : activeTab === 'pods' ? (
            <div className="space-y-4">
              {detailMetrics?.pods?.length === 0 ? (
                <div className="text-center py-12 text-gray-500 dark:text-gray-400">
                  <Box size={48} className="mx-auto mb-4 opacity-50" />
                  <p>No pods found</p>
                </div>
              ) : (
                <div className="bg-gray-100 dark:bg-gray-900 rounded-lg overflow-hidden">
                  <table className="w-full">
                    <thead>
                      <tr className="border-b border-gray-200 dark:border-gray-700">
                        <th className="text-left px-4 py-3 text-sm font-medium text-gray-500 dark:text-gray-400">Pod</th>
                        <th className="text-left px-4 py-3 text-sm font-medium text-gray-500 dark:text-gray-400">Status</th>
                        <th className="text-left px-4 py-3 text-sm font-medium text-gray-500 dark:text-gray-400">Ready</th>
                        <th className="text-right px-4 py-3 text-sm font-medium text-gray-500 dark:text-gray-400">CPU</th>
                        <th className="text-right px-4 py-3 text-sm font-medium text-gray-500 dark:text-gray-400">Memory</th>
                        <th className="text-left px-4 py-3 text-sm font-medium text-gray-500 dark:text-gray-400">Restarts</th>
                        <th className="text-left px-4 py-3 text-sm font-medium text-gray-500 dark:text-gray-400">Age</th>
                      </tr>
                    </thead>
                    <tbody>
                      {detailMetrics?.pods?.map((pod) => (
                        <tr key={pod.name} className="border-b border-gray-200 dark:border-gray-800 hover:bg-gray-200 dark:hover:bg-gray-800/50">
                          <td className="px-4 py-3">
                            <div className="font-mono text-sm text-gray-900 dark:text-white">{pod.name}</div>
                            <div className="text-xs text-gray-500">{pod.ip || 'No IP'}</div>
                          </td>
                          <td className="px-4 py-3">
                            <span className={`inline-flex items-center gap-1 px-2 py-1 rounded text-xs font-medium ${
                              pod.status === 'Running'
                                ? 'bg-green-900/50 text-green-400'
                                : pod.status === 'Pending'
                                ? 'bg-yellow-900/50 text-yellow-400'
                                : 'bg-red-900/50 text-red-400'
                            }`}>
                              {pod.status}
                            </span>
                          </td>
                          <td className="px-4 py-3">
                            {pod.ready ? (
                              <CheckCircle size={18} className="text-green-400" />
                            ) : (
                              <XCircle size={18} className="text-red-400" />
                            )}
                          </td>
                          <td className="px-4 py-3 text-right">
                            <span className="text-blue-400 font-mono text-sm">
                              {pod.cpu_usage != null ? formatCpu(pod.cpu_usage) : '-'}
                            </span>
                          </td>
                          <td className="px-4 py-3 text-right">
                            <span className="text-green-400 font-mono text-sm">
                              {pod.memory_usage != null ? formatBytes(pod.memory_usage) : '-'}
                            </span>
                          </td>
                          <td className="px-4 py-3">
                            <span className={`flex items-center gap-1 ${pod.restarts > 0 ? 'text-yellow-500 dark:text-yellow-400' : 'text-gray-500 dark:text-gray-400'}`}>
                              {pod.restarts > 0 && <RotateCcw size={14} />}
                              {pod.restarts}
                            </span>
                          </td>
                          <td className="px-4 py-3 text-gray-500 dark:text-gray-400 text-sm">{pod.age}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </div>
          ) : (
            <div className="space-y-4">
              {logsLoading ? (
                <div className="flex items-center justify-center py-12">
                  <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500"></div>
                </div>
              ) : logEntries.length === 0 ? (
                <div className="text-center py-12 text-gray-500 dark:text-gray-400">
                  <FileText size={48} className="mx-auto mb-4 opacity-50" />
                  <p>No logs found</p>
                  <p className="text-sm mt-1">Logs from the last hour will appear here</p>
                </div>
              ) : (
                <div className="bg-gray-100 dark:bg-gray-900 rounded-lg overflow-hidden">
                  <div className="max-h-96 overflow-y-auto font-mono text-sm">
                    {logEntries.slice(0, 100).map((entry, i) => (
                      <div
                        key={i}
                        className="px-4 py-2 border-b border-gray-200 dark:border-gray-800 hover:bg-gray-200 dark:hover:bg-gray-800/50"
                      >
                        <div className="flex items-start gap-3">
                          <span className="text-gray-500 text-xs whitespace-nowrap">
                            {new Date(entry.timestamp).toLocaleTimeString()}
                          </span>
                          <span className="text-gray-700 dark:text-gray-300 break-all">{entry.line}</span>
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

export default function MonitoringPage() {
  const [searchParams, setSearchParams] = useSearchParams()
  const [autoRefresh, setAutoRefresh] = useState(true)

  // Get selected app from URL params
  const selectedApp = searchParams.get('app')
  const setSelectedApp = (app: string | null) => {
    if (app) {
      setSearchParams({ app })
    } else {
      setSearchParams({})
    }
  }

  // Check if VictoriaMetrics is available
  const { data: metricsStatus, isLoading: metricsLoading } = useQuery({
    queryKey: ['monitoring', 'vm', 'available'],
    queryFn: monitoringApi.checkMetricsAvailable,
    refetchInterval: 30000,
  })

  // Get cluster metrics
  const {
    data: clusterMetrics,
    isLoading: clusterLoading,
    refetch: refetchCluster,
  } = useQuery({
    queryKey: ['monitoring', 'vm', 'cluster'],
    queryFn: monitoringApi.getClusterMetrics,
    refetchInterval: autoRefresh ? 10000 : false,
    enabled: metricsStatus?.available,
  })

  // Get app metrics
  const {
    data: appMetrics,
    isLoading: appsLoading,
    refetch: refetchApps,
  } = useQuery({
    queryKey: ['monitoring', 'vm', 'apps'],
    queryFn: monitoringApi.getAppMetrics,
    refetchInterval: autoRefresh ? 10000 : false,
    enabled: metricsStatus?.available,
  })

  // Get cluster network history for sparkline
  const {
    data: networkHistory,
    refetch: refetchNetworkHistory,
  } = useQuery({
    queryKey: ['monitoring', 'vm', 'cluster', 'network-history'],
    queryFn: () => monitoringApi.getClusterNetworkHistory('15m'),
    refetchInterval: autoRefresh ? 10000 : false,
    enabled: metricsStatus?.available,
  })

  // Get cluster metrics history for sparklines on all KPI cards
  const {
    data: metricsHistory,
    refetch: refetchMetricsHistory,
  } = useQuery({
    queryKey: ['monitoring', 'vm', 'cluster', 'metrics-history'],
    queryFn: () => monitoringApi.getClusterMetricsHistory('15m'),
    refetchInterval: autoRefresh ? 10000 : false,
    enabled: metricsStatus?.available,
  })

  const handleRefresh = () => {
    refetchCluster()
    refetchApps()
    refetchNetworkHistory()
    refetchMetricsHistory()
  }

  // Sort apps by memory usage (descending) - backend already returns only relevant apps
  const sortedApps = [...(appMetrics || [])]
    .sort((a, b) => b.memory_usage_bytes - a.memory_usage_bytes)

  // Calculate totals for apps
  const totalAppMemory = sortedApps.reduce((sum, app) => sum + app.memory_usage_bytes, 0)
  const totalAppCpu = sortedApps.reduce((sum, app) => sum + app.cpu_usage_cores, 0)
  const totalAppNetworkRx = sortedApps.reduce((sum, app) => sum + (app.network_receive_bytes_per_sec || 0), 0)
  const totalAppNetworkTx = sortedApps.reduce((sum, app) => sum + (app.network_transmit_bytes_per_sec || 0), 0)

  if (metricsLoading) {
    return (
      <div className="flex items-center justify-center h-[60vh]">
        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-500"></div>
      </div>
    )
  }

  if (!metricsStatus?.available) {
    return (
      <div className="flex flex-col items-center justify-center h-[60vh] text-center">
        <AlertCircle size={64} className="text-yellow-500 mb-4" />
        <h2 className="text-2xl font-bold mb-2 text-gray-900 dark:text-white">Metrics Not Available</h2>
        <p className="text-gray-500 dark:text-gray-400 max-w-md">
          {metricsStatus?.message || 'Cannot connect to VictoriaMetrics. It may be starting up.'}
        </p>
      </div>
    )
  }

  return (
    <div className="space-y-8">
      {/* App Detail Modal */}
      {selectedApp && (
        <AppDetailModal appName={selectedApp} onClose={() => setSelectedApp(null)} />
      )}

      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold flex items-center gap-2 text-gray-900 dark:text-white">
            <Activity className="text-blue-500 dark:text-blue-400" />
            Resources
          </h1>
          <p className="text-gray-500 dark:text-gray-400 mt-1">CPU, memory, and resource usage metrics</p>
        </div>
        <div className="flex items-center gap-3">
          <button
            onClick={() => setAutoRefresh(!autoRefresh)}
            className={`flex items-center gap-2 px-4 py-2 rounded-lg transition-colors ${
              autoRefresh
                ? 'bg-green-600 hover:bg-green-700 text-white'
                : 'bg-gray-200 dark:bg-gray-700 hover:bg-gray-300 dark:hover:bg-gray-600 text-gray-700 dark:text-white'
            }`}
          >
            <Gauge size={18} />
            {autoRefresh ? 'Live' : 'Paused'}
          </button>
          <button
            onClick={handleRefresh}
            disabled={clusterLoading || appsLoading}
            className="flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-700 rounded-lg transition-colors disabled:opacity-50"
          >
            <RefreshCw size={18} className={clusterLoading || appsLoading ? 'animate-spin' : ''} />
            Refresh
          </button>
        </div>
      </div>

      {/* Cluster Overview */}
      <div>
        <h2 className="text-xl font-semibold mb-4 flex items-center gap-2 text-gray-900 dark:text-white">
          <Server size={20} />
          Cluster Overview
        </h2>
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 xl:grid-cols-5 gap-4">
          <ClusterStatsCard
            icon={Cpu}
            label="CPU Usage"
            value={`${clusterMetrics?.cpu_usage_percent?.toFixed(1) || 0}%`}
            detailValue={formatCpu(clusterMetrics?.used_cpu_cores || 0)}
            detailSub={`/ ${clusterMetrics?.total_cpu_cores || 0} cores`}
            color="blue"
            sparklineData={metricsHistory?.cpu_series}
            sparklineFormatValue={formatPercent}
          />
          <ClusterStatsCard
            icon={HardDrive}
            label="Memory Usage"
            value={`${clusterMetrics?.memory_usage_percent?.toFixed(1) || 0}%`}
            detailValue={formatBytes(clusterMetrics?.used_memory_bytes || 0)}
            detailSub={`/ ${formatBytes(clusterMetrics?.total_memory_bytes || 0)}`}
            color="purple"
            sparklineData={metricsHistory?.memory_series}
            sparklineFormatValue={formatPercent}
          />
          <div className="relative bg-gradient-to-br from-white to-gray-50 dark:from-gray-800 dark:to-gray-900 rounded-xl p-5 h-[125px] border border-gray-200/60 dark:border-gray-700/60 shadow-[0_4px_12px_rgba(0,0,0,0.05),0_1px_3px_rgba(0,0,0,0.1)] dark:shadow-[0_4px_12px_rgba(0,0,0,0.3),inset_0_1px_0_rgba(255,255,255,0.05)] overflow-hidden flex flex-col justify-between">
            <div className="relative flex items-center gap-3">
              <div className="p-2.5 bg-gradient-to-br from-green-500/20 to-green-600/10 rounded-xl shadow-inner">
                <Network className="w-5 h-5 text-green-500 dark:text-green-400" />
              </div>
              <div className="flex-1">
                <div className="text-gray-500 dark:text-gray-400 text-sm font-medium">Network I/O</div>
              </div>
              <div className="text-right">
                <div className="text-sm font-bold text-green-500 dark:text-green-400">↓ {formatBandwidth(clusterMetrics?.network_receive_bytes_per_sec || 0)}</div>
                <div className="text-sm font-bold text-red-500 dark:text-red-400">↑ {formatBandwidth(clusterMetrics?.network_transmit_bytes_per_sec || 0)}</div>
              </div>
            </div>
            <div className="relative -mx-5 -mb-5">
              <MiniSparkline data={networkHistory?.rx_series || []} color="green" secondaryData={networkHistory?.tx_series} secondaryColor="red" height={45} interactive />
            </div>
          </div>
          <ClusterStatsCard
            icon={Box}
            label="Containers"
            value={String(clusterMetrics?.container_count || 0)}
            color="yellow"
            sparklineData={metricsHistory?.container_series}
            sparklineFormatValue={formatCount}
          />
          <ClusterStatsCard
            icon={Server}
            label="Pods"
            value={String(clusterMetrics?.pod_count || 0)}
            color="cyan"
            sparklineData={metricsHistory?.pod_series}
            sparklineFormatValue={formatCount}
          />
        </div>
      </div>

      {/* Per-App Metrics */}
      <div>
        <h2 className="text-xl font-semibold mb-4 flex items-center gap-2 text-gray-900 dark:text-white">
          <TrendingUp size={20} />
          App Resource Usage
          <span className="text-sm font-normal text-gray-500 dark:text-gray-400 ml-2">Click an app for details</span>
        </h2>

        {appsLoading ? (
          <div className="flex items-center justify-center py-12">
            <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500"></div>
          </div>
        ) : sortedApps.length === 0 ? (
          <div className="bg-white dark:bg-gray-800 rounded-lg p-8 text-center text-gray-500 dark:text-gray-400 border border-gray-200 dark:border-gray-700">
            <Box size={48} className="mx-auto mb-4 opacity-50" />
            <p>No app metrics available</p>
            <p className="text-sm mt-1">Install some apps to see their resource usage</p>
          </div>
        ) : (
          <div className="bg-white dark:bg-gray-800 rounded-lg overflow-hidden border border-gray-200 dark:border-gray-700">
            <table className="w-full">
              <thead>
                <tr className="border-b border-gray-200 dark:border-gray-700">
                  <th className="text-left px-6 py-4 text-sm font-medium text-gray-500 dark:text-gray-400">App</th>
                  <th className="text-right px-6 py-4 text-sm font-medium text-gray-500 dark:text-gray-400">CPU</th>
                  <th className="text-right px-6 py-4 text-sm font-medium text-gray-500 dark:text-gray-400">Memory</th>
                  <th className="text-right px-6 py-4 text-sm font-medium text-gray-500 dark:text-gray-400">Network</th>
                  <th className="text-left px-6 py-4 text-sm font-medium text-gray-500 dark:text-gray-400 w-1/4">Usage</th>
                </tr>
              </thead>
              <tbody>
                {sortedApps.map((app) => {
                  const memoryPercent = totalAppMemory > 0
                    ? (app.memory_usage_bytes / totalAppMemory) * 100
                    : 0

                  return (
                    <tr
                      key={app.namespace}
                      onClick={() => setSelectedApp(app.app_name)}
                      className="border-b border-gray-200 dark:border-gray-700/50 hover:bg-gray-100 dark:hover:bg-gray-700/30 transition-colors cursor-pointer"
                    >
                      <td className="px-6 py-4">
                        <div className="flex items-center gap-3">
                          <AppIcon appName={app.app_name} size={32} />
                          <div className="font-medium capitalize text-gray-900 dark:text-white">{app.app_name}</div>
                        </div>
                      </td>
                      <td className="text-right px-6 py-4">
                        <span className="text-blue-400 font-mono">
                          {formatCpu(app.cpu_usage_cores)}
                        </span>
                      </td>
                      <td className="text-right px-6 py-4">
                        <span className="text-green-400 font-mono">
                          {formatBytes(app.memory_usage_bytes)}
                        </span>
                      </td>
                      <td className="text-right px-6 py-4">
                        <div className="text-purple-400 font-mono text-sm">
                          <div className="flex items-center justify-end gap-1">
                            <ArrowDownToLine size={12} className="text-green-400" />
                            <span className="text-green-400">{formatBandwidth(app.network_receive_bytes_per_sec || 0)}</span>
                          </div>
                          <div className="flex items-center justify-end gap-1">
                            <ArrowUpFromLine size={12} className="text-orange-400" />
                            <span className="text-orange-400">{formatBandwidth(app.network_transmit_bytes_per_sec || 0)}</span>
                          </div>
                        </div>
                      </td>
                      <td className="px-6 py-4">
                        <div className="flex items-center gap-3">
                          <div className="flex-1">
                            <ProgressBar
                              value={app.memory_usage_bytes}
                              max={totalAppMemory}
                              color={memoryPercent > 50 ? 'yellow' : 'green'}
                            />
                          </div>
                          <span className="text-xs text-gray-500 w-12 text-right">
                            {memoryPercent.toFixed(1)}%
                          </span>
                        </div>
                      </td>
                    </tr>
                  )
                })}
              </tbody>
              <tfoot>
                <tr className="bg-gray-100 dark:bg-gray-700/30">
                  <td className="px-6 py-4 font-medium text-gray-900 dark:text-white">Total</td>
                  <td className="text-right px-6 py-4">
                    <span className="text-blue-400 font-mono font-medium">
                      {formatCpu(totalAppCpu)}
                    </span>
                  </td>
                  <td className="text-right px-6 py-4">
                    <span className="text-green-400 font-mono font-medium">
                      {formatBytes(totalAppMemory)}
                    </span>
                  </td>
                  <td className="text-right px-6 py-4">
                    <div className="font-mono text-sm">
                      <div className="flex items-center justify-end gap-1">
                        <ArrowDownToLine size={12} className="text-green-400" />
                        <span className="text-green-400">{formatBandwidth(totalAppNetworkRx)}</span>
                      </div>
                      <div className="flex items-center justify-end gap-1">
                        <ArrowUpFromLine size={12} className="text-orange-400" />
                        <span className="text-orange-400">{formatBandwidth(totalAppNetworkTx)}</span>
                      </div>
                    </div>
                  </td>
                  <td className="px-6 py-4"></td>
                </tr>
              </tfoot>
            </table>
          </div>
        )}
      </div>

      {/* Auto-refresh indicator */}
      {autoRefresh && (
        <div className="text-center text-sm text-gray-500 dark:text-gray-500">
          Auto-refreshing every 10 seconds
        </div>
      )}
    </div>
  )
}
