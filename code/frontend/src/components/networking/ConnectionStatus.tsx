import { Radio, WifiOff } from 'lucide-react'
import { ConnectionMode } from '../../hooks/useNetworkMetrics'

interface ConnectionStatusProps {
  mode: ConnectionMode
}

export function ConnectionStatus({ mode }: ConnectionStatusProps) {
  const config = {
    live: {
      icon: Radio,
      label: 'Live',
      className: 'bg-green-100 dark:bg-green-900/30 text-green-600 dark:text-green-400',
      iconClassName: 'text-green-500',
    },
    disconnected: {
      icon: WifiOff,
      label: 'Disconnected',
      className: 'bg-gray-100 dark:bg-gray-700 text-gray-500 dark:text-gray-400',
      iconClassName: 'text-gray-400',
    },
  }

  const { icon: Icon, label, className, iconClassName } = config[mode]

  return (
    <div className={`flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-medium ${className}`}>
      <Icon size={12} className={iconClassName} />
      <span>{label}</span>
    </div>
  )
}
