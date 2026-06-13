import { formatBytes } from '../../api/storage'
import type { ExtensionStat } from './storageStatistics'
import { cushionPalette, cushionPreview, extensionDescription } from './storageStatistics'

export function StatisticsFileTypes({ stats, selectedExtension, onSelect }: {
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
              className={`grid min-w-[330px] w-full grid-cols-[58px_48px_minmax(90px,1fr)_86px_54px] text-left transition ${selected ? 'bg-blue-600 text-white hover:bg-blue-600 dark:hover:bg-blue-600' : 'text-gray-800 hover:bg-blue-100/80 dark:text-gray-100 dark:hover:bg-blue-950/50'}`}
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
