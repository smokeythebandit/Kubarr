import { useState, useEffect, useCallback, useRef, useMemo } from 'react'
import { logsApi, LokiStream, LokiLogEntry } from '../api/logs'
import { AppIcon } from '../components/AppIcon'
import {
  Search,
  RefreshCw,
  Clock,
  Filter,
  ChevronDown,
  ChevronUp,
  AlertCircle,
  FileText,
  Play,
  Pause,
  Check,
  AlertTriangle
} from 'lucide-react'

// Available log levels for filtering
const LOG_LEVELS = ['DEBUG', 'INFO', 'WARN', 'ERROR', 'FATAL', 'TRACE'] as const
type LogLevel = typeof LOG_LEVELS[number]

// Sort configuration
type SortField = 'timestamp' | 'level' | 'app' | 'message'
type SortDirection = 'asc' | 'desc'

// Cookie name for storing filter preferences
const LOGS_FILTER_COOKIE = 'kubarr_logs_filters'

// Cookie utility functions
function setCookie(name: string, value: string, days: number = 365) {
  const expires = new Date(Date.now() + days * 864e5).toUTCString()
  document.cookie = `${name}=${encodeURIComponent(value)}; expires=${expires}; path=/; SameSite=Lax`
}

function getCookie(name: string): string | null {
  const match = document.cookie.match(new RegExp('(^| )' + name + '=([^;]+)'))
  return match ? decodeURIComponent(match[2]) : null
}

// Filter preferences interface
interface LogsFilterPrefs {
  selectedLevels: LogLevel[]
  selectedApps: string[]
  timeRangeValue: string
  sortField: SortField
  sortDirection: SortDirection
}

// Load filter preferences from cookie
function loadFilterPrefs(): Partial<LogsFilterPrefs> {
  try {
    const cookie = getCookie(LOGS_FILTER_COOKIE)
    if (cookie) {
      return JSON.parse(cookie)
    }
  } catch (e) {
    console.error('Failed to load logs filter preferences:', e)
  }
  return {}
}

// Save filter preferences to cookie
function saveFilterPrefs(prefs: LogsFilterPrefs) {
  setCookie(LOGS_FILTER_COOKIE, JSON.stringify(prefs))
}

// Get display label for app (capitalize first letter)
function getAppLabel(app: string): string {
  return app.charAt(0).toUpperCase() + app.slice(1).replace(/-/g, ' ')
}

// Time range options
const TIME_RANGES = [
  { label: 'Last 15 minutes', value: '15m', ms: 15 * 60 * 1000 },
  { label: 'Last 1 hour', value: '1h', ms: 60 * 60 * 1000 },
  { label: 'Last 3 hours', value: '3h', ms: 3 * 60 * 60 * 1000 },
  { label: 'Last 6 hours', value: '6h', ms: 6 * 60 * 60 * 1000 },
  { label: 'Last 12 hours', value: '12h', ms: 12 * 60 * 60 * 1000 },
  { label: 'Last 24 hours', value: '24h', ms: 24 * 60 * 60 * 1000 },
  { label: 'Last 7 days', value: '7d', ms: 7 * 24 * 60 * 60 * 1000 },
]

// Format timestamp for display
function formatTimestamp(timestamp: string): string {
  const date = new Date(timestamp)
  return date.toLocaleString('en-US', {
    month: 'short',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  })
}

// Log level row colors (subtle backgrounds for full row)
const LOG_LEVEL_ROW_COLORS: Record<string, string> = {
  DEBUG: 'bg-gray-50 dark:bg-gray-900/30',
  INFO: '', // default, no special color
  WARN: 'bg-yellow-50 dark:bg-yellow-900/20',
  WARNING: 'bg-yellow-50 dark:bg-yellow-900/20',
  ERROR: 'bg-red-50 dark:bg-red-900/30',
  FATAL: 'bg-red-100 dark:bg-red-900/50',
  TRACE: 'bg-purple-50 dark:bg-purple-900/20',
}

// Flatten streams into a single sorted list of log entries
function flattenStreams(streams: LokiStream[] | null | undefined): Array<LokiLogEntry & { labels: Record<string, string> }> {
  const entries: Array<LokiLogEntry & { labels: Record<string, string> }> = []

  if (!streams || !Array.isArray(streams)) {
    return entries
  }

  for (const stream of streams) {
    if (!stream || !Array.isArray(stream.entries)) continue
    for (const entry of stream.entries) {
      entries.push({
        ...entry,
        labels: stream.labels || {},
      })
    }
  }

  // Sort by timestamp (newest first)
  entries.sort((a, b) => new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime())

  return entries
}

export default function LogsPage() {
  // Load saved preferences from cookie
  const savedPrefs = useMemo(() => loadFilterPrefs(), [])

  const [apps, setApps] = useState<string[]>([])
  const [selectedApps, setSelectedApps] = useState<string[]>([])
  const [selectedLevels, setSelectedLevels] = useState<LogLevel[]>(
    savedPrefs.selectedLevels?.filter(l => LOG_LEVELS.includes(l)) || [...LOG_LEVELS]
  )
  const [timeRange, setTimeRange] = useState(
    TIME_RANGES.find(r => r.value === savedPrefs.timeRangeValue) || TIME_RANGES[1]
  )
  const [searchText, setSearchText] = useState('')
  const [streams, setStreams] = useState<LokiStream[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [autoRefresh, setAutoRefresh] = useState(false)
  const [showAppDropdown, setShowAppDropdown] = useState(false)
  const [showTimeDropdown, setShowTimeDropdown] = useState(false)
  const [showLevelDropdown, setShowLevelDropdown] = useState(false)
  const [sortField, setSortField] = useState<SortField>(savedPrefs.sortField || 'timestamp')
  const [sortDirection, setSortDirection] = useState<SortDirection>(savedPrefs.sortDirection || 'desc')

  const logsContainerRef = useRef<HTMLDivElement>(null)
  const autoRefreshRef = useRef<ReturnType<typeof setInterval> | null>(null)

  // Save filter preferences to cookie when they change
  useEffect(() => {
    saveFilterPrefs({
      selectedLevels,
      selectedApps,
      timeRangeValue: timeRange.value,
      sortField,
      sortDirection,
    })
  }, [selectedLevels, selectedApps, timeRange.value, sortField, sortDirection])

  // Load apps (namespaces) on mount
  useEffect(() => {
    const loadApps = async () => {
      try {
        const ns = await logsApi.getNamespaces()
        const appList = Array.isArray(ns) ? ns : []
        setApps(appList)
        // Restore saved apps, filtering to only include apps that still exist
        const savedApps = savedPrefs.selectedApps
        if (savedApps && savedApps.length > 0) {
          const validSavedApps = savedApps.filter(app => appList.includes(app))
          setSelectedApps(validSavedApps.length > 0 ? validSavedApps : appList)
        } else {
          // Select all apps by default
          setSelectedApps(appList)
        }
      } catch (err) {
        console.error('Failed to load apps:', err)
        setError('Failed to connect to VictoriaLogs. Make sure VictoriaLogs is running.')
        setApps([])
        setSelectedApps([])
      }
    }
    loadApps()
  }, [])

  // Build LogQL query
  const buildQuery = useCallback(() => {
    let query = ''

    if (selectedApps.length === 0 || selectedApps.length === apps.length) {
      // All apps or none selected - query everything
      query = '{namespace=~".+"}'
    } else if (selectedApps.length === 1) {
      query = `{namespace="${selectedApps[0]}"}`
    } else {
      query = `{namespace=~"${selectedApps.join('|')}"}`
    }

    // Add search filter if provided
    if (searchText.trim()) {
      query += ` |~ "(?i)${searchText.trim()}"`
    }

    return query
  }, [selectedApps, apps.length, searchText])

  // Fetch logs
  const fetchLogs = useCallback(async () => {
    if (apps.length === 0) {
      return
    }

    setLoading(true)
    setError(null)

    try {
      const now = Date.now()
      const start = String((now - timeRange.ms) * 1e6)
      const end = String(now * 1e6)

      const response = await logsApi.queryLogs({
        query: buildQuery(),
        start,
        end,
        limit: 2000,
        direction: 'backward',
      })

      setStreams(response?.streams || [])
    } catch (err: any) {
      setError(err.response?.data?.detail || err.message || 'Failed to fetch logs')
      setStreams([])
    } finally {
      setLoading(false)
    }
  }, [buildQuery, timeRange.ms, apps.length])

  // Initial fetch and when filters change
  useEffect(() => {
    if (selectedApps.length > 0) {
      fetchLogs()
    }
  }, [fetchLogs, selectedApps.length])

  // Auto-refresh
  useEffect(() => {
    if (autoRefresh) {
      autoRefreshRef.current = setInterval(fetchLogs, 5000)
    } else if (autoRefreshRef.current) {
      clearInterval(autoRefreshRef.current)
      autoRefreshRef.current = null
    }

    return () => {
      if (autoRefreshRef.current) {
        clearInterval(autoRefreshRef.current)
      }
    }
  }, [autoRefresh, fetchLogs])

  // Toggle app selection
  const toggleApp = (app: string) => {
    setSelectedApps(prev =>
      prev.includes(app)
        ? prev.filter(a => a !== app)
        : [...prev, app]
    )
  }

  // Select/deselect all apps
  const toggleAllApps = () => {
    if (selectedApps.length === apps.length) {
      setSelectedApps([])
    } else {
      setSelectedApps([...apps])
    }
  }

  // Toggle level selection
  const toggleLevel = (level: LogLevel) => {
    setSelectedLevels(prev =>
      prev.includes(level)
        ? prev.filter(l => l !== level)
        : [...prev, level]
    )
  }

  // Select/deselect all levels
  const toggleAllLevels = () => {
    if (selectedLevels.length === LOG_LEVELS.length) {
      setSelectedLevels([])
    } else {
      setSelectedLevels([...LOG_LEVELS])
    }
  }

  // Handle column header click for sorting
  const handleSort = (field: SortField) => {
    if (sortField === field) {
      setSortDirection(prev => prev === 'asc' ? 'desc' : 'asc')
    } else {
      setSortField(field)
      setSortDirection(field === 'timestamp' ? 'desc' : 'asc')
    }
  }

  // Flatten, filter, and sort entries
  const entries = useMemo(() => {
    const flattened = flattenStreams(streams)

    // Filter by selected levels
    const filtered = flattened.filter(entry => {
      const level = entry.level?.toUpperCase() as LogLevel
      if (!level) return selectedLevels.length === LOG_LEVELS.length // Show entries without level if all levels selected
      return selectedLevels.includes(level)
    })

    // Sort entries
    return filtered.sort((a, b) => {
      let comparison = 0

      switch (sortField) {
        case 'timestamp':
          comparison = new Date(a.timestamp).getTime() - new Date(b.timestamp).getTime()
          break
        case 'level': {
          const levelA = a.level?.toUpperCase() || ''
          const levelB = b.level?.toUpperCase() || ''
          comparison = levelA.localeCompare(levelB)
          break
        }
        case 'app': {
          const appA = a.labels.namespace || ''
          const appB = b.labels.namespace || ''
          comparison = appA.localeCompare(appB)
          break
        }
        case 'message':
          comparison = (a.line || '').localeCompare(b.line || '')
          break
      }

      return sortDirection === 'asc' ? comparison : -comparison
    })
  }, [streams, selectedLevels, sortField, sortDirection])

  // Get selected apps display text
  const getSelectedAppsText = () => {
    if (selectedApps.length === 0) return 'No apps selected'
    if (selectedApps.length === apps.length) return 'All apps'
    if (selectedApps.length === 1) {
      return getAppLabel(selectedApps[0])
    }
    return `${selectedApps.length} apps`
  }

  // Get selected levels display text
  const getSelectedLevelsText = () => {
    if (selectedLevels.length === 0) return 'No levels'
    if (selectedLevels.length === LOG_LEVELS.length) return 'All levels'
    if (selectedLevels.length === 1) return selectedLevels[0]
    return `${selectedLevels.length} levels`
  }

  // Get level color for badge
  const getLevelColor = (level: string) => {
    switch (level) {
      case 'DEBUG': return 'bg-gray-200 text-gray-700 dark:bg-gray-700 dark:text-gray-300'
      case 'INFO': return 'bg-blue-100 text-blue-700 dark:bg-blue-900/50 dark:text-blue-300'
      case 'WARN': return 'bg-yellow-100 text-yellow-700 dark:bg-yellow-900/50 dark:text-yellow-300'
      case 'ERROR': return 'bg-red-100 text-red-700 dark:bg-red-900/50 dark:text-red-300'
      case 'FATAL': return 'bg-red-200 text-red-800 dark:bg-red-900/70 dark:text-red-200'
      case 'TRACE': return 'bg-purple-100 text-purple-700 dark:bg-purple-900/50 dark:text-purple-300'
      default: return 'bg-gray-200 text-gray-700 dark:bg-gray-700 dark:text-gray-300'
    }
  }

  // Sort icon component
  const SortIcon = ({ field }: { field: SortField }) => {
    if (sortField !== field) return <ChevronDown size={14} className="opacity-30" />
    return sortDirection === 'asc' ? <ChevronUp size={14} /> : <ChevronDown size={14} />
  }

  return (
    <div className="h-full flex flex-col">
      {/* Header */}
      <div className="mb-6">
        <h1 className="text-2xl font-bold flex items-center gap-2 text-gray-900 dark:text-white">
          <FileText className="text-blue-500 dark:text-blue-400" />
          Logs
        </h1>
        <p className="text-gray-500 dark:text-gray-400 mt-1">View application logs from VictoriaLogs</p>
      </div>

      {/* Filters Bar */}
      <div className="flex flex-wrap items-center gap-3 mb-4">
        {/* App Filter */}
        <div className="relative">
          <button
            onClick={() => setShowAppDropdown(!showAppDropdown)}
            className="flex items-center gap-2 px-4 py-2 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg hover:bg-gray-100 dark:hover:bg-gray-750 transition-colors text-gray-900 dark:text-white"
          >
            <Filter size={16} />
            <span>{getSelectedAppsText()}</span>
            <ChevronDown size={16} />
          </button>

          {showAppDropdown && (
            <div className="absolute top-full left-0 mt-1 w-72 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg shadow-xl z-50 max-h-96 overflow-y-auto">
              <div className="p-2 border-b border-gray-200 dark:border-gray-700">
                <button
                  onClick={toggleAllApps}
                  className="w-full flex items-center gap-2 px-3 py-2 text-sm hover:bg-gray-100 dark:hover:bg-gray-700 rounded transition-colors text-gray-900 dark:text-white"
                >
                  <div className={`w-5 h-5 rounded border flex items-center justify-center ${
                    selectedApps.length === apps.length
                      ? 'bg-blue-600 border-blue-600'
                      : 'border-gray-300 dark:border-gray-600'
                  }`}>
                    {selectedApps.length === apps.length && <Check size={14} className="text-white" />}
                  </div>
                  <span className="font-medium">
                    {selectedApps.length === apps.length ? 'Deselect All' : 'Select All'}
                  </span>
                </button>
              </div>
              <div className="p-2 space-y-1">
                {apps.map(app => {
                  const isSelected = selectedApps.includes(app)
                  return (
                    <button
                      key={app}
                      onClick={() => toggleApp(app)}
                      className="w-full flex items-center gap-3 px-3 py-2 hover:bg-gray-100 dark:hover:bg-gray-700 rounded transition-colors text-left text-gray-900 dark:text-white"
                    >
                      <div className={`w-5 h-5 rounded border flex items-center justify-center ${
                        isSelected ? 'bg-blue-600 border-blue-600' : 'border-gray-300 dark:border-gray-600'
                      }`}>
                        {isSelected && <Check size={14} className="text-white" />}
                      </div>
                      <AppIcon appName={app} size={32} />
                      <span className="flex-1 font-medium">{getAppLabel(app)}</span>
                    </button>
                  )
                })}
              </div>
            </div>
          )}
        </div>

        {/* Level Filter */}
        <div className="relative">
          <button
            onClick={() => setShowLevelDropdown(!showLevelDropdown)}
            className="flex items-center gap-2 px-4 py-2 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg hover:bg-gray-100 dark:hover:bg-gray-750 transition-colors text-gray-900 dark:text-white"
          >
            <AlertTriangle size={16} />
            <span>{getSelectedLevelsText()}</span>
            <ChevronDown size={16} />
          </button>

          {showLevelDropdown && (
            <div className="absolute top-full left-0 mt-1 w-48 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg shadow-xl z-50">
              <div className="p-2 border-b border-gray-200 dark:border-gray-700">
                <button
                  onClick={toggleAllLevels}
                  className="w-full flex items-center gap-2 px-3 py-2 text-sm hover:bg-gray-100 dark:hover:bg-gray-700 rounded transition-colors text-gray-900 dark:text-white"
                >
                  <div className={`w-5 h-5 rounded border flex items-center justify-center ${
                    selectedLevels.length === LOG_LEVELS.length
                      ? 'bg-blue-600 border-blue-600'
                      : 'border-gray-300 dark:border-gray-600'
                  }`}>
                    {selectedLevels.length === LOG_LEVELS.length && <Check size={14} className="text-white" />}
                  </div>
                  <span className="font-medium">
                    {selectedLevels.length === LOG_LEVELS.length ? 'Deselect All' : 'Select All'}
                  </span>
                </button>
              </div>
              <div className="p-2 space-y-1">
                {LOG_LEVELS.map(level => {
                  const isSelected = selectedLevels.includes(level)
                  return (
                    <button
                      key={level}
                      onClick={() => toggleLevel(level)}
                      className="w-full flex items-center gap-3 px-3 py-2 hover:bg-gray-100 dark:hover:bg-gray-700 rounded transition-colors text-left text-gray-900 dark:text-white"
                    >
                      <div className={`w-5 h-5 rounded border flex items-center justify-center ${
                        isSelected ? 'bg-blue-600 border-blue-600' : 'border-gray-300 dark:border-gray-600'
                      }`}>
                        {isSelected && <Check size={14} className="text-white" />}
                      </div>
                      <span className={`px-2 py-0.5 rounded text-xs font-medium ${getLevelColor(level)}`}>
                        {level}
                      </span>
                    </button>
                  )
                })}
              </div>
            </div>
          )}
        </div>

        {/* Time Range */}
        <div className="relative">
          <button
            onClick={() => setShowTimeDropdown(!showTimeDropdown)}
            className="flex items-center gap-2 px-4 py-2 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg hover:bg-gray-100 dark:hover:bg-gray-750 transition-colors text-gray-900 dark:text-white"
          >
            <Clock size={16} />
            <span>{timeRange.label}</span>
            <ChevronDown size={16} />
          </button>

          {showTimeDropdown && (
            <div className="absolute top-full left-0 mt-1 w-48 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg shadow-xl z-50">
              {TIME_RANGES.map(range => (
                <button
                  key={range.value}
                  onClick={() => {
                    setTimeRange(range)
                    setShowTimeDropdown(false)
                  }}
                  className={`w-full text-left px-4 py-2 text-sm hover:bg-gray-100 dark:hover:bg-gray-700 transition-colors first:rounded-t-lg last:rounded-b-lg ${
                    timeRange.value === range.value ? 'bg-gray-100 dark:bg-gray-700 text-blue-600 dark:text-blue-400' : 'text-gray-900 dark:text-white'
                  }`}
                >
                  {range.label}
                </button>
              ))}
            </div>
          )}
        </div>

        {/* Search */}
        <div className="flex-1 min-w-[200px] max-w-md relative">
          <Search size={16} className="absolute left-3 top-1/2 -translate-y-1/2 text-gray-400" />
          <input
            type="text"
            value={searchText}
            onChange={(e) => setSearchText(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && fetchLogs()}
            placeholder="Search logs... (press Enter)"
            className="w-full pl-10 pr-4 py-2 bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 rounded-lg focus:outline-none focus:border-blue-500 transition-colors text-gray-900 dark:text-white placeholder-gray-500 dark:placeholder-gray-400"
          />
        </div>

        {/* Actions */}
        <div className="flex items-center gap-2">
          <button
            onClick={() => setAutoRefresh(!autoRefresh)}
            className={`flex items-center gap-2 px-4 py-2 rounded-lg transition-colors ${
              autoRefresh
                ? 'bg-green-600 hover:bg-green-700 text-white'
                : 'bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 hover:bg-gray-100 dark:hover:bg-gray-750 text-gray-900 dark:text-white'
            }`}
            title={autoRefresh ? 'Stop auto-refresh' : 'Start auto-refresh (5s)'}
          >
            {autoRefresh ? <Pause size={16} /> : <Play size={16} />}
            <span className="hidden sm:inline">{autoRefresh ? 'Live' : 'Auto'}</span>
          </button>

          <button
            onClick={fetchLogs}
            disabled={loading}
            className="flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-700 rounded-lg transition-colors disabled:opacity-50"
          >
            <RefreshCw size={16} className={loading ? 'animate-spin' : ''} />
            <span className="hidden sm:inline">Refresh</span>
          </button>
        </div>
      </div>

      {/* Close dropdowns on click outside */}
      {(showAppDropdown || showTimeDropdown || showLevelDropdown) && (
        <div
          className="fixed inset-0 z-40"
          onClick={() => {
            setShowAppDropdown(false)
            setShowTimeDropdown(false)
            setShowLevelDropdown(false)
          }}
        />
      )}

      {/* Error */}
      {error && (
        <div className="mb-4 p-4 bg-red-100 dark:bg-red-900/30 border border-red-300 dark:border-red-700 rounded-lg flex items-center gap-3">
          <AlertCircle className="text-red-500 dark:text-red-400 flex-shrink-0" />
          <span className="text-red-600 dark:text-red-300">{error}</span>
        </div>
      )}

      {/* Stats */}
      <div className="mb-2 text-sm text-gray-500 dark:text-gray-400">
        {loading ? (
          <span>Loading...</span>
        ) : (
          <span>{entries.length.toLocaleString()} log entries</span>
        )}
        {autoRefresh && <span className="ml-2 text-green-600 dark:text-green-400">Auto-refreshing every 5s</span>}
      </div>

      {/* Log Viewer */}
      <div
        ref={logsContainerRef}
        className="flex-1 bg-gray-100 dark:bg-gray-950 border border-gray-200 dark:border-gray-800 rounded-lg overflow-auto font-mono text-sm"
      >
        {entries.length === 0 && !loading ? (
          <div className="flex items-center justify-center h-full text-gray-500">
            <div className="text-center">
              <FileText size={48} className="mx-auto mb-4 opacity-50" />
              <p>No logs found</p>
              <p className="text-sm mt-1">Try adjusting your filters or time range</p>
            </div>
          </div>
        ) : (
          <table className="w-full">
            <thead className="sticky top-0 bg-gray-200 dark:bg-gray-900 border-b border-gray-300 dark:border-gray-700">
              <tr>
                <th
                  onClick={() => handleSort('timestamp')}
                  className="px-3 py-2 text-left text-xs font-semibold text-gray-600 dark:text-gray-300 cursor-pointer hover:bg-gray-300 dark:hover:bg-gray-800 transition-colors whitespace-nowrap"
                >
                  <div className="flex items-center gap-1">
                    Time
                    <SortIcon field="timestamp" />
                  </div>
                </th>
                <th
                  onClick={() => handleSort('level')}
                  className="px-2 py-2 text-left text-xs font-semibold text-gray-600 dark:text-gray-300 cursor-pointer hover:bg-gray-300 dark:hover:bg-gray-800 transition-colors whitespace-nowrap"
                >
                  <div className="flex items-center gap-1">
                    Level
                    <SortIcon field="level" />
                  </div>
                </th>
                <th
                  onClick={() => handleSort('app')}
                  className="px-2 py-2 text-left text-xs font-semibold text-gray-600 dark:text-gray-300 cursor-pointer hover:bg-gray-300 dark:hover:bg-gray-800 transition-colors whitespace-nowrap"
                >
                  <div className="flex items-center gap-1">
                    App
                    <SortIcon field="app" />
                  </div>
                </th>
                <th
                  onClick={() => handleSort('message')}
                  className="px-3 py-2 text-left text-xs font-semibold text-gray-600 dark:text-gray-300 cursor-pointer hover:bg-gray-300 dark:hover:bg-gray-800 transition-colors"
                >
                  <div className="flex items-center gap-1">
                    Message
                    <SortIcon field="message" />
                  </div>
                </th>
              </tr>
            </thead>
            <tbody>
              {entries.map((entry, index) => {
                const appName = entry.labels.namespace || 'unknown'
                const level = entry.level?.toUpperCase()
                const rowColor = level ? LOG_LEVEL_ROW_COLORS[level] || '' : ''

                return (
                  <tr
                    key={`${entry.timestamp}-${index}`}
                    className={`border-b border-gray-200 dark:border-gray-800/50 ${rowColor}`}
                  >
                    <td className="px-3 py-1 text-gray-500 whitespace-nowrap align-top text-xs">
                      {formatTimestamp(entry.timestamp)}
                    </td>
                    <td className="px-2 py-1 whitespace-nowrap align-top">
                      {level && (
                        <span className={`px-1.5 py-0.5 rounded text-xs font-medium ${getLevelColor(level)}`}>
                          {level}
                        </span>
                      )}
                    </td>
                    <td className="px-2 py-1 whitespace-nowrap align-top">
                      <div className="inline-flex items-center gap-1.5 px-1.5 py-0.5 text-xs rounded bg-gray-200 dark:bg-gray-800 text-gray-700 dark:text-gray-200">
                        <AppIcon appName={appName} size={18} />
                        <span>{getAppLabel(appName)}</span>
                      </div>
                    </td>
                    <td className="px-3 py-1 text-gray-700 dark:text-gray-200 whitespace-pre-wrap break-all">
                      {entry.line}
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        )}
      </div>
    </div>
  )
}
