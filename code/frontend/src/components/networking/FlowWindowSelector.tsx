import { FLOW_WINDOWS, FlowWindow } from '../../api/networking'

interface FlowWindowSelectorProps {
  value: FlowWindow
  onChange: (window: FlowWindow) => void
}

const LABELS: Record<FlowWindow, string> = {
  '15m': '15m',
  '1h': '1h',
  '6h': '6h',
  '24h': '24h',
}

export function FlowWindowSelector({ value, onChange }: FlowWindowSelectorProps) {
  return (
    <div
      className="inline-flex rounded-lg border border-gray-200 bg-gray-100 p-0.5 dark:border-gray-700 dark:bg-gray-700/50"
      role="group"
      aria-label="Time window"
    >
      {FLOW_WINDOWS.map(window => (
        <button
          key={window}
          type="button"
          onClick={() => onChange(window)}
          aria-pressed={window === value}
          className={`rounded-md px-3 py-1 text-sm transition-colors ${
            window === value
              ? 'bg-white font-semibold text-gray-900 shadow-sm dark:bg-gray-800 dark:text-white'
              : 'text-gray-500 hover:text-gray-900 dark:text-gray-400 dark:hover:text-white'
          }`}
        >
          {LABELS[window]}
        </button>
      ))}
    </div>
  )
}
