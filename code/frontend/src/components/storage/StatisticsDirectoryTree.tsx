import { useEffect, useRef } from 'react'
import { File, Folder } from 'lucide-react'
import { formatBytes } from '../../api/storage'
import type { StatisticsTreeRow } from './storageStatistics'
import { usageBarStyle, visiblePercent } from './storageStatistics'

export function StatisticsDirectoryTree({ rows, expandedPaths, selectedPath, totalSize, onSelect, onToggleExpanded }: {
  rows: StatisticsTreeRow[]
  expandedPaths: Set<string>
  selectedPath: string
  totalSize: number
  onSelect: (path: string) => void
  onToggleExpanded: (path: string) => void
}) {
  const selectedRowRef = useRef<HTMLButtonElement | null>(null)

  useEffect(() => {
    selectedRowRef.current?.scrollIntoView({ block: 'nearest' })
  }, [selectedPath, rows])

  return (
    <div className="h-full min-h-0 min-w-0 overflow-hidden border-r border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-800">
      <div className="border-b border-gray-200 px-4 py-3 dark:border-gray-700">
        <h3 className="font-black text-gray-950 dark:text-white">Directory Tree</h3>
      </div>
      <div className="grid min-w-[520px] grid-cols-[minmax(220px,1fr)_120px_100px_70px] border-b border-gray-200 bg-gray-50 text-xs font-bold text-gray-500 dark:border-gray-700 dark:bg-gray-900/40 dark:text-gray-400">
        <div className="px-4 py-2">Name</div>
        <div className="px-3 py-2 text-right">Subtree</div>
        <div className="px-3 py-2 text-right">Size</div>
        <div className="px-3 py-2 text-right">Items</div>
      </div>
      <div className="h-[calc(100%-78px)] overflow-auto text-xs">
        {rows.map(({ node, depth }) => {
          const selected = selectedPath === node.path
          const hasChildren = node.type === 'directory' && node.children.some((child) => child.size > 0)
          const expanded = expandedPaths.has(node.path)
          return (
            <button
              key={`tree-${node.path || 'root'}`}
              ref={selected ? selectedRowRef : undefined}
              type="button"
              onClick={() => onSelect(node.path)}
              onDoubleClick={() => {
                if (hasChildren) onToggleExpanded(node.path)
              }}
              className={`grid min-w-[520px] w-full grid-cols-[minmax(220px,1fr)_120px_100px_70px] text-left transition ${selected ? 'bg-blue-600 text-white hover:bg-blue-600 dark:hover:bg-blue-600' : 'text-gray-800 hover:bg-blue-100/80 dark:text-gray-100 dark:hover:bg-blue-950/50'}`}
            >
              <div className="flex min-w-0 items-center gap-2 px-4 py-1.5" style={{ paddingLeft: 16 + depth * 16 }}>
                {hasChildren ? (
                  <span
                    role="button"
                    tabIndex={0}
                    onClick={(event) => {
                      event.stopPropagation()
                      onToggleExpanded(node.path)
                    }}
                    onKeyDown={(event) => {
                      if (event.key === 'Enter' || event.key === ' ') {
                        event.preventDefault()
                        event.stopPropagation()
                        onToggleExpanded(node.path)
                      }
                    }}
                    className={`flex h-4 w-4 shrink-0 items-center justify-center rounded border text-[10px] font-black ${selected ? 'border-white/70 text-white' : 'border-gray-300 text-gray-500 dark:border-gray-600 dark:text-gray-400'}`}
                  >
                    {expanded ? '-' : '+'}
                  </span>
                ) : (
                  <span className="h-4 w-4 shrink-0" />
                )}
                {node.type === 'directory' ? <Folder size={14} className="shrink-0 text-yellow-600" /> : <File size={14} className="shrink-0 text-slate-500" />}
                <span className="truncate">{node.name || 'Root'}</span>
              </div>
              <div className="px-3 py-1.5 text-right">
                <div className="flex items-center gap-2">
                  <span className="min-w-10">{visiblePercent(node.size, totalSize)}</span>
                  <span className="h-1.5 flex-1 overflow-hidden rounded-full bg-gray-200 dark:bg-gray-700">
                    <span className="block h-full rounded-full bg-blue-500" style={usageBarStyle(node.size, totalSize)} />
                  </span>
                </div>
              </div>
              <div className="px-3 py-1.5 text-right font-mono">{formatBytes(node.size, 1)}</div>
              <div className="px-3 py-1.5 text-right font-mono">{node.children.length}</div>
            </button>
          )
        })}
      </div>
    </div>
  )
}
