import type { StorageUsageNode } from '../../api/storage'
import { formatBytes } from '../../api/storage'
import { usagePercent } from './storageUtils'
import type { TreemapRect } from './storageStatistics'
import { cushionPalette, cushionStyle, extensionFor, fileRectStyle, isDescendantPath, parentDirectory, rectStyle } from './storageStatistics'

export function StatisticsTreemap({ rects, directoryRects, extensionColor, selectedPath, selectedExtension, totalSize, onSelectPath, onOpenDirectory }: {
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
            const dimmed = !selected && ((selectedExtension !== null && selectedExtension !== extension) || outsideSelectedDirectory)
            return (
              <button
                key={rect.node.path}
                type="button"
                onClick={() => onSelectPath(rect.node.path)}
                onDoubleClick={() => onOpenDirectory(parentDirectory(rect.node))}
                className={`absolute overflow-hidden rounded-[1px] text-left shadow-[inset_1px_1px_0_rgba(255,255,255,0.22),inset_-1px_-1px_0_rgba(0,0,0,0.62)] transition ${selected ? 'z-30 ring-2 ring-white' : 'z-10 hover:z-20 hover:brightness-150 hover:ring-2 hover:ring-white/80'} ${dimmed ? 'opacity-25 saturate-50' : ''}`}
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
