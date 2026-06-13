type StorageTab = 'browser' | 'statistics'

interface StorageTabsProps {
  activeTab: StorageTab
  onChange: (tab: StorageTab) => void
  compact?: boolean
}

export function StorageTabs({ activeTab, onChange, compact = false }: StorageTabsProps) {
  return (
    <div className={`flex w-fit border border-gray-200 bg-white shadow-sm dark:border-gray-700 dark:bg-gray-800 ${compact ? 'rounded-xl p-0.5' : 'rounded-2xl p-1'}`}>
      {[
        ['browser', 'Browser'],
        ['statistics', 'Statistics'],
      ].map(([value, label]) => (
        <button
          key={value}
          onClick={() => onChange(value as StorageTab)}
          className={`${compact ? 'rounded-lg px-3 py-1.5 text-xs' : 'rounded-xl px-4 py-2 text-sm'} font-bold transition ${
            activeTab === value
              ? 'bg-blue-600 text-white shadow-lg shadow-blue-600/20'
              : 'text-gray-500 hover:bg-gray-100 hover:text-gray-900 dark:text-gray-400 dark:hover:bg-gray-700 dark:hover:text-white'
          }`}
        >
          {label}
        </button>
      ))}
    </div>
  )
}
