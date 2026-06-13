import { useEffect, useRef, useState } from 'react'
import type { PointerEvent as ReactPointerEvent } from 'react'
import { File, Folder, RefreshCw } from 'lucide-react'
import type { StorageUsage, StorageUsageNode } from '../../api/storage'
import { formatBytes } from '../../api/storage'
import { flattenUsage, usagePercent } from './storageUtils'

interface WinDirStatViewProps {
  usage?: StorageUsage
  isLoading: boolean
  error: unknown
  onOpenDirectory: (path: string) => void
}

interface ExtensionStat {
  extension: string
  size: number
  files: number
  colorIndex: number
}

interface TreeRow {
  node: StorageUsageNode
  depth: number
}

interface TreemapRect {
  node: StorageUsageNode
  x: number
  y: number
  width: number
  height: number
  depth: number
  ridgeHeight: number
}

const cushionPalette = [
  '#0000ff', '#ff0000', '#00ff00', '#ffff00', '#00ffff', '#ff00ff',
  '#ffaa00', '#0055ff', '#ff0055', '#55ff00', '#aa00ff', '#00ff55',
  '#ff00aa', '#00aaff', '#ff5500', '#00ffaa', '#5500ff', '#ffffff',
]

export function WinDirStatView({ usage, isLoading, error, onOpenDirectory }: WinDirStatViewProps) {
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
              <FileTreePane rows={treeRows} expandedPaths={expandedPaths} selectedPath={selectedNode.path} totalSize={usage.total_size} onSelect={selectPath} onToggleExpanded={toggleExpanded} />
            </div>
            <ResizeHandle value={treeWidth} onChange={setTreeWidth} />
            <div className="min-w-0 flex-1">
              <ExtensionPane stats={extensionStats} selectedExtension={selectedExtension} onSelect={setSelectedExtension} />
            </div>
          </div>

          <VerticalResizeHandle value={topHeight} onChange={setTopHeight} />

          <div className="min-h-0 bg-slate-950 p-2" style={{ height: `calc(${100 - topHeight}% - 0.5rem)` }}>
            <TreemapPane
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

function FileTreePane({ rows, expandedPaths, selectedPath, totalSize, onSelect, onToggleExpanded }: {
  rows: TreeRow[]
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
              className={`grid min-w-[520px] w-full grid-cols-[minmax(220px,1fr)_120px_100px_70px] text-left transition hover:bg-blue-50 dark:hover:bg-blue-950/30 ${selected ? 'bg-blue-600 text-white hover:bg-blue-600' : 'text-gray-800 dark:text-gray-100'}`}
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

function ResizeHandle({ value, onChange }: { value: number; onChange: (value: number) => void }) {
  const startDrag = (event: ReactPointerEvent<HTMLButtonElement>) => {
    event.preventDefault()
    const container = event.currentTarget.parentElement
    if (!container) return

    const onMove = (moveEvent: PointerEvent) => {
      const rect = container.getBoundingClientRect()
      const percent = ((moveEvent.clientX - rect.left) / rect.width) * 100
      onChange(Math.min(82, Math.max(35, percent)))
    }

    const stopDrag = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', stopDrag)
    }

    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', stopDrag)
  }

  return (
    <button
      type="button"
      aria-label="Resize directory tree"
      title="Resize directory tree"
      onPointerDown={startDrag}
      className="group relative w-2 shrink-0 cursor-col-resize bg-gray-100 transition hover:bg-blue-100 dark:bg-gray-800 dark:hover:bg-blue-950"
      data-width={value}
    >
      <span className="absolute left-1/2 top-1/2 h-12 w-1 -translate-x-1/2 -translate-y-1/2 rounded-full bg-gray-300 transition group-hover:bg-blue-500 dark:bg-gray-600" />
    </button>
  )
}

function VerticalResizeHandle({ value, onChange }: { value: number; onChange: (value: number) => void }) {
  const startDrag = (event: ReactPointerEvent<HTMLButtonElement>) => {
    event.preventDefault()
    const container = event.currentTarget.parentElement
    if (!container) return

    const onMove = (moveEvent: PointerEvent) => {
      const rect = container.getBoundingClientRect()
      const percent = ((moveEvent.clientY - rect.top) / rect.height) * 100
      onChange(Math.min(65, Math.max(20, percent)))
    }

    const stopDrag = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', stopDrag)
    }

    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', stopDrag)
  }

  return (
    <button
      type="button"
      aria-label="Resize statistics panes"
      title="Resize statistics panes"
      onPointerDown={startDrag}
      className="group relative h-2 w-full cursor-row-resize bg-gray-100 transition hover:bg-blue-100 dark:bg-gray-800 dark:hover:bg-blue-950"
      data-height={value}
    >
      <span className="absolute left-1/2 top-1/2 h-1 w-16 -translate-x-1/2 -translate-y-1/2 rounded-full bg-gray-300 transition group-hover:bg-blue-500 dark:bg-gray-600" />
    </button>
  )
}

function ExtensionPane({ stats, selectedExtension, onSelect }: {
  stats: ExtensionStat[]
  selectedExtension: string | null
  onSelect: (extension: string | null) => void
}) {
  return (
    <div className="h-full min-h-0 min-w-0 overflow-hidden bg-white dark:bg-gray-800">
      <div className="border-b border-gray-200 px-4 py-3 dark:border-gray-700">
        <h3 className="font-black text-gray-950 dark:text-white">File Types</h3>
      </div>
      <div className="grid min-w-[330px] grid-cols-[58px_48px_minmax(90px,1fr)_86px_54px] border-b border-gray-200 bg-gray-50 text-xs font-bold text-gray-500 dark:border-gray-700 dark:bg-gray-900/40 dark:text-gray-400">
        <div className="px-3 py-2">Ext</div>
        <div className="px-3 py-2">Color</div>
        <div className="px-3 py-2">Type</div>
        <div className="px-3 py-2 text-right">Bytes</div>
        <div className="px-3 py-2 text-right">Files</div>
      </div>
      <div className="h-[calc(100%-78px)] overflow-auto text-xs">
        {stats.map((stat) => {
          const selected = selectedExtension === stat.extension
          return (
            <button
              key={`ext-${stat.extension}`}
              type="button"
              onClick={() => onSelect(selected ? null : stat.extension)}
              className={`grid min-w-[330px] w-full grid-cols-[58px_48px_minmax(90px,1fr)_86px_54px] text-left transition hover:bg-blue-50 dark:hover:bg-blue-950/30 ${selected ? 'bg-blue-600 text-white hover:bg-blue-600' : 'text-gray-800 dark:text-gray-100'}`}
            >
              <div className="truncate px-3 py-1.5 font-mono">{stat.extension}</div>
              <div className="px-3 py-1.5">
                <span className="block h-4 w-8 rounded border border-black/20" style={{ background: cushionPreview(cushionPalette[stat.colorIndex % cushionPalette.length]) }} />
              </div>
              <div className="truncate px-3 py-1.5">{extensionDescription(stat.extension)}</div>
              <div className="px-3 py-1.5 text-right font-mono">{formatBytes(stat.size, 1)}</div>
              <div className="px-3 py-1.5 text-right font-mono">{stat.files}</div>
            </button>
          )
        })}
      </div>
    </div>
  )
}

function TreemapPane({ rects, directoryRects, extensionColor, selectedPath, selectedExtension, totalSize, onSelectPath, onOpenDirectory }: {
  rects: TreemapRect[]
  directoryRects: TreemapRect[]
  extensionColor: Map<string, number>
  selectedPath: string
  selectedExtension: string | null
  totalSize: number
  onSelectPath: (path: string) => void
  onOpenDirectory: (node: StorageUsageNode) => void
}) {
  const selectedDirectoryRect = directoryRects.find((rect) => rect.node.path === selectedPath)

  return (
    <div className="relative h-full min-h-0 overflow-hidden rounded-2xl bg-black">
      {rects.length === 0 ? (
        <div className="flex h-full items-center justify-center text-slate-400">No files found.</div>
      ) : (
        <>
          {directoryRects.map((rect) => (
            <button
              key={`dir-${rect.node.path || 'root'}`}
              type="button"
              onClick={() => onSelectPath(rect.node.path)}
              onDoubleClick={() => onOpenDirectory(rect.node)}
              className="absolute z-0 border border-white/20 text-left hover:border-white/70"
              style={rectStyle(rect)}
              title={`${rect.node.path || 'Root'} - ${formatBytes(rect.node.size)} (${usagePercent(rect.node.size, totalSize)})`}
            />
          ))}
          {rects.map((rect) => {
            const extension = extensionFor(rect.node.name)
            const colorIndex = extensionColor.get(extension) || 0
            const selected = selectedPath === rect.node.path
            const outsideSelectedDirectory = selectedDirectoryRect && selectedDirectoryRect.node.path !== '' && !isDescendantPath(rect.node.path, selectedDirectoryRect.node.path)
            const dimmed = (selectedExtension !== null && selectedExtension !== extension) || outsideSelectedDirectory
            return (
              <button
                key={rect.node.path}
                type="button"
                onClick={() => onSelectPath(rect.node.path)}
                onDoubleClick={() => onOpenDirectory(parentDirectory(rect.node))}
                className={`absolute overflow-hidden rounded-[1px] text-left shadow-[inset_1px_1px_0_rgba(255,255,255,0.22),inset_-1px_-1px_0_rgba(0,0,0,0.62)] transition ${selected ? 'z-30 ring-2 ring-white' : 'z-10 hover:z-20 hover:brightness-125'} ${dimmed ? 'opacity-25 saturate-50' : ''}`}
                style={{ ...fileRectStyle(rect), ...cushionStyle(cushionPalette[colorIndex % cushionPalette.length], rect) }}
                title={`${rect.node.path} - ${formatBytes(rect.node.size)} (${usagePercent(rect.node.size, totalSize)})`}
              >
                {rect.width > 7 && rect.height > 5 && extension !== '[none]' && (
                  <span className="relative flex h-full items-center justify-center truncate px-1 text-[10px] font-black text-white drop-shadow-[0_1px_2px_rgba(0,0,0,0.9)]">{extension}</span>
                )}
              </button>
            )
          })}
          {selectedDirectoryRect && (
            <div
              className="pointer-events-none absolute z-40 rounded-sm border-2 border-white shadow-[0_0_0_2px_rgba(37,99,235,0.95),0_0_22px_rgba(59,130,246,0.75)]"
              style={rectStyle(selectedDirectoryRect)}
            >
              <div className="absolute left-2 top-2 max-w-[calc(100%-1rem)] truncate rounded-lg bg-blue-600/90 px-2 py-1 text-xs font-black text-white shadow-lg">
                {selectedDirectoryRect.node.path || '/data'} · {formatBytes(selectedDirectoryRect.node.size)}
              </div>
            </div>
          )}
        </>
      )}
    </div>
  )
}

function buildTreeRows(root: StorageUsageNode, expandedPaths: Set<string>): TreeRow[] {
  const rows: TreeRow[] = []
  const visit = (node: StorageUsageNode, depth: number) => {
    rows.push({ node, depth })
    if (!expandedPaths.has(node.path)) return
    const children = node.children.filter((child) => child.size > 0).sort((a, b) => b.size - a.size)
    for (const child of children) visit(child, depth + 1)
  }
  visit(root, 0)
  return rows
}

function expandAncestors(path: string, current: Set<string>) {
  const next = new Set(current)
  next.add('')
  const parts = path.split('/').filter(Boolean)
  for (let i = 1; i < parts.length; i += 1) {
    next.add(parts.slice(0, i).join('/'))
  }
  return next
}

function buildExtensionStats(files: StorageUsageNode[]): ExtensionStat[] {
  const stats = new Map<string, Omit<ExtensionStat, 'colorIndex'>>()
  for (const file of files) {
    const extension = extensionFor(file.name)
    const existing = stats.get(extension) || { extension, size: 0, files: 0 }
    existing.size += file.size
    existing.files += 1
    stats.set(extension, existing)
  }
  return [...stats.values()].sort((a, b) => b.size - a.size).map((stat, colorIndex) => ({ ...stat, colorIndex }))
}

function buildTreemapRects(root: StorageUsageNode): TreemapRect[] {
  const rects: TreemapRect[] = []
  layoutNode(root, { node: root, x: 0, y: 0, width: 100, height: 100, depth: 0, ridgeHeight: 0.38 }, rects, false)
  return rects.filter((rect) => rect.width > 0 && rect.height > 0)
}

function buildDirectoryRects(root: StorageUsageNode): TreemapRect[] {
  const rects: TreemapRect[] = []
  layoutDirectories(root, { node: root, x: 0, y: 0, width: 100, height: 100, depth: 0, ridgeHeight: 0.38 }, rects)
  return rects.filter((rect) => rect.width >= 4 && rect.height >= 4).slice(0, 220)
}

function layoutNode(node: StorageUsageNode, rect: TreemapRect, output: TreemapRect[], includeSelf: boolean) {
  if (node.size <= 0 || rect.width <= 0 || rect.height <= 0) return
  if (node.type === 'file') {
    if (includeSelf) output.push({ ...rect, node })
    return
  }

  const children = node.children.filter((child) => child.size > 0).sort((a, b) => b.size - a.size)
  for (const childRect of arrangeKDirStat(children, rect)) layoutNode(childRect.node, childRect, output, true)
}

function layoutDirectories(node: StorageUsageNode, rect: TreemapRect, output: TreemapRect[]) {
  if (node.type !== 'directory' || node.size <= 0) return
  output.push({ ...rect, node })
  const children = node.children.filter((child) => child.size > 0).sort((a, b) => b.size - a.size)
  for (const childRect of arrangeKDirStat(children, rect)) {
    if (childRect.node.type === 'directory') layoutDirectories(childRect.node, childRect, output)
  }
}

function arrangeKDirStat(children: StorageUsageNode[], rect: TreemapRect): TreemapRect[] {
  const total = children.reduce((sum, child) => sum + child.size, 0)
  if (!total) return []

  const horizontalRows = rect.width >= rect.height
  const widthRatio = horizontalRows ? rect.width / Math.max(rect.height, 0.001) : rect.height / Math.max(rect.width, 0.001)
  const rows: Array<{ height: number; count: number }> = []
  const childWidths = new Array(children.length).fill(0)

  for (let nextChild = 0; nextChild < children.length;) {
    const row = calculateKDirStatRow(children, nextChild, Math.max(1, widthRatio), total, childWidths)
    rows.push({ height: row.height, count: row.childrenUsed })
    nextChild += row.childrenUsed
  }

  const output: TreemapRect[] = []
  let top = horizontalRows ? rect.y : rect.x
  let childIndex = 0
  rows.forEach((row, rowIndex) => {
    const rowExtent = horizontalRows ? rect.height : rect.width
    const columnExtent = horizontalRows ? rect.width : rect.height
    const fBottom = top + row.height * rowExtent
    const bottom = rowIndex + 1 === rows.length ? (horizontalRows ? rect.y + rect.height : rect.x + rect.width) : fBottom

    let left = horizontalRows ? rect.x : rect.y
    for (let childInRow = 0; childInRow < row.count; childInRow += 1, childIndex += 1) {
      const child = children[childIndex]
      const fRight = left + childWidths[childIndex] * columnExtent
      const lastChild = childInRow + 1 === row.count || childIndex + 1 === childWidths.length || childWidths[childIndex + 1] === 0
      const right = lastChild ? (horizontalRows ? rect.x + rect.width : rect.y + rect.height) : fRight

      output.push(horizontalRows ? {
        node: child, x: left, y: top, width: Math.max(0, right - left), height: Math.max(0, bottom - top), depth: rect.depth + 1, ridgeHeight: rect.ridgeHeight * 0.91,
      } : {
        node: child, x: top, y: left, width: Math.max(0, bottom - top), height: Math.max(0, right - left), depth: rect.depth + 1, ridgeHeight: rect.ridgeHeight * 0.91,
      })
      left = fRight
    }
    top = fBottom
  })
  return output
}

function calculateKDirStatRow(children: StorageUsageNode[], nextChild: number, widthRatio: number, total: number, childWidths: number[]) {
  const minProportion = 0.4
  let sizeUsed = 0
  let rowHeight = 0
  let index = nextChild

  for (; index < children.length; index += 1) {
    const childSize = children[index].size
    if (childSize === 0) break
    sizeUsed += childSize
    const virtualRowHeight = sizeUsed / total
    const childWidth = (childSize / total) * widthRatio / virtualRowHeight
    if (index > nextChild && childWidth / virtualRowHeight < minProportion) {
      sizeUsed -= childSize
      break
    }
    rowHeight = virtualRowHeight
  }

  const childrenUsed = Math.max(1, index - nextChild)
  const rowSize = total * rowHeight
  for (let i = 0; i < childrenUsed; i += 1) childWidths[nextChild + i] = rowSize ? children[nextChild + i].size / rowSize : 0
  return { height: rowHeight || 1, childrenUsed }
}

function rectStyle(rect: TreemapRect) {
  return { left: `${rect.x}%`, top: `${rect.y}%`, width: `${rect.width}%`, height: `${rect.height}%` }
}

function visiblePercent(size: number, total: number) {
  if (!total || !size) return '0%'
  const percent = (size / total) * 100
  return percent < 0.1 ? '<0.1%' : `${percent.toFixed(1)}%`
}

function usageBarStyle(size: number, total: number) {
  if (!total || !size) return { width: '0%' }
  const percent = (size / total) * 100
  return {
    width: `${Math.max(percent, 0.8)}%`,
  }
}

function fileRectStyle(rect: TreemapRect) {
  const style = rectStyle(rect)
  return {
    ...style,
    minWidth: rect.width < 0.35 ? '2px' : undefined,
    minHeight: rect.height < 0.35 ? '2px' : undefined,
  }
}

function cushionPreview(hex: string) {
  return `radial-gradient(ellipse at 30% 22%, ${adjustColor(hex, 0.55)} 0%, ${makeBrightColor(hex, 0.6)} 45%, ${adjustColor(hex, -0.3)} 100%)`
}

function cushionStyle(hex: string, rect: TreemapRect) {
  return { background: `radial-gradient(ellipse at 30% 22%, ${adjustColor(hex, 0.55 + rect.ridgeHeight * 0.35)} 0%, ${makeBrightColor(hex, 0.6)} 38%, ${adjustColor(hex, -0.28 - rect.depth * 0.018)} 100%)` }
}

function makeBrightColor(hex: string, brightness: number) {
  const rgb = hexToRgb(hex)
  const sum = rgb.r + rgb.g + rgb.b || 1
  const factor = (3 * 255 * brightness) / sum
  return rgbToCss(normalizeRgb(rgb.r * factor, rgb.g * factor, rgb.b * factor))
}

function adjustColor(hex: string, amount: number) {
  const rgb = hexToRgb(hex)
  const target = amount >= 0 ? 255 : 0
  const factor = Math.abs(amount)
  return rgbToCss({ r: rgb.r + (target - rgb.r) * factor, g: rgb.g + (target - rgb.g) * factor, b: rgb.b + (target - rgb.b) * factor })
}

function hexToRgb(hex: string) {
  const value = Number.parseInt(hex.slice(1), 16)
  return { r: (value >> 16) & 255, g: (value >> 8) & 255, b: value & 255 }
}

function normalizeRgb(r: number, g: number, b: number) {
  const max = Math.max(r, g, b)
  if (max <= 255) return { r, g, b }
  const scale = 255 / max
  return { r: r * scale, g: g * scale, b: b * scale }
}

function rgbToCss({ r, g, b }: { r: number; g: number; b: number }) {
  return `rgb(${Math.round(r)}, ${Math.round(g)}, ${Math.round(b)})`
}

function extensionFor(name: string) {
  const index = name.lastIndexOf('.')
  if (index <= 0 || index === name.length - 1) return '[none]'
  return name.slice(index).toLowerCase()
}

function extensionDescription(extension: string) {
  if (extension === '[none]') return 'Files without extension'
  const value = extension.slice(1).toUpperCase()
  return `${value} file`
}

function isDescendantPath(path: string, directoryPath: string) {
  return path === directoryPath || path.startsWith(`${directoryPath}/`)
}

function parentDirectory(node: StorageUsageNode): StorageUsageNode {
  const parentPath = node.path.split('/').slice(0, -1).join('/')
  return { name: parentPath || 'Root', path: parentPath, type: 'directory', size: node.size, children: [] }
}
