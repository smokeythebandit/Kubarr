import { useState } from 'react'
import { RefreshCw } from 'lucide-react'
import type { StorageUsage, StorageUsageNode } from '../../api/storage'
import { flattenUsage } from './storageUtils'
import { StatisticsDirectoryTree } from './StatisticsDirectoryTree'
import { StatisticsFileTypes } from './StatisticsFileTypes'
import { StatisticsResizeHandle, StatisticsVerticalResizeHandle } from './StatisticsResizeHandle'
import { StatisticsTreemap } from './StatisticsTreemap'
import { buildDirectoryRects, buildExtensionStats, buildTreemapRects, buildTreeRows, expandAncestors } from './storageStatistics'

interface StatisticsViewProps {
  usage?: StorageUsage
  isLoading: boolean
  error: unknown
  onOpenDirectory: (path: string) => void
}

export function StatisticsView({ usage, isLoading, error, onOpenDirectory }: StatisticsViewProps) {
  const [selectedPath, setSelectedPath] = useState<string | null>(null)
  const [selectedExtension, setSelectedExtension] = useState<string | null>(null)
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(() => new Set(['']))
  const [treeWidth, setTreeWidth] = useState(68)
  const [topHeight, setTopHeight] = useState(32)

  if (isLoading) {
    return (
      <div className="flex h-96 items-center justify-center rounded-3xl border border-gray-200 bg-white text-gray-500 shadow-sm dark:border-gray-700 dark:bg-gray-800 dark:text-gray-400">
        <RefreshCw size={20} className="mr-3 animate-spin" />
        Scanning the full NFS tree...
      </div>
    )
  }

  if (error) {
    return (
      <div className="rounded-3xl border border-red-200 bg-red-50 p-6 text-red-700 dark:border-red-500/30 dark:bg-red-950/40 dark:text-red-200">
        {(error as Error).message || 'Failed to scan storage usage'}
      </div>
    )
  }

  if (!usage) return null

  const allNodes = flattenUsage(usage.root).filter((node) => node.path)
  const files = allNodes.filter((node) => node.type === 'file').sort((a, b) => b.size - a.size)
  const selectedNode = allNodes.find((node) => node.path === selectedPath) || usage.root
  const extensionStats = buildExtensionStats(files)
  const extensionColor = new Map(extensionStats.map((stat) => [stat.extension, stat.colorIndex]))
  const treeRows = buildTreeRows(usage.root, expandedPaths)
  const treemapRects = buildTreemapRects(usage.root)
  const directoryRects = buildDirectoryRects(usage.root)

  const openDirectory = (node: StorageUsageNode) => {
    if (node.type === 'directory') onOpenDirectory(node.path)
  }

  const selectPath = (path: string) => {
    setSelectedPath(path)
    setExpandedPaths((current) => expandAncestors(path, current))
  }

  const toggleExpanded = (path: string) => {
    setExpandedPaths((current) => {
      const next = new Set(current)
      if (next.has(path)) next.delete(path)
      else next.add(path)
      next.add('')
      return next
    })
  }

  return (
    <div className="max-w-full space-y-5 overflow-hidden">
      <div className="h-[calc(100vh-13rem)] min-h-[640px] min-w-0 overflow-hidden rounded-3xl border border-gray-200 bg-white shadow-sm dark:border-gray-700 dark:bg-gray-800">
        <div className="flex min-h-[170px] min-w-0" style={{ height: `${topHeight}%` }}>
          <div className="min-w-0" style={{ width: `${treeWidth}%` }}>
            <StatisticsDirectoryTree rows={treeRows} expandedPaths={expandedPaths} selectedPath={selectedNode.path} totalSize={usage.total_size} onSelect={selectPath} onToggleExpanded={toggleExpanded} />
          </div>
          <StatisticsResizeHandle value={treeWidth} onChange={setTreeWidth} />
          <div className="min-w-0 flex-1">
            <StatisticsFileTypes stats={extensionStats} selectedExtension={selectedExtension} onSelect={setSelectedExtension} />
          </div>
        </div>

        <StatisticsVerticalResizeHandle value={topHeight} onChange={setTopHeight} />

        <div className="min-h-0 bg-slate-950 p-2" style={{ height: `calc(${100 - topHeight}% - 0.5rem)` }}>
          <StatisticsTreemap
            rects={treemapRects}
            directoryRects={directoryRects}
            extensionColor={extensionColor}
            selectedPath={selectedNode.path}
            selectedExtension={selectedExtension}
            totalSize={usage.total_size}
            onSelectPath={selectPath}
            onOpenDirectory={openDirectory}
          />
        </div>
      </div>

      {usage.warnings.length > 0 && (
        <div className="rounded-3xl border border-amber-200 bg-amber-50 px-4 py-3 text-sm text-amber-900 dark:border-amber-500/30 dark:bg-amber-950/30 dark:text-amber-200">
          Scan warnings: {usage.warnings.slice(0, 3).join(' · ')}
        </div>
      )}
    </div>
  )
}
