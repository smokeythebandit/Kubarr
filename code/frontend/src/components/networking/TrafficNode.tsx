import { Position, Handle } from '@xyflow/react'
import { ArrowDownToLine, ArrowUpFromLine, Globe, Shield } from 'lucide-react'
import { formatBandwidth } from '../../api/networking'
import { AppVpnConfig } from '../../api/vpn'
import { AppIcon } from '../AppIcon'

export interface TrafficNodeData {
  label: string
  type: 'external' | 'app' | 'lan' | 'infra'
  rx: number
  tx: number
  total: number
  podCount: number
  appId: string
  isHighlighted: boolean
  isFaded: boolean
  isSelected: boolean
  vpnConfig?: AppVpnConfig
  [key: string]: unknown
}

export function TrafficNode({ data }: { data: TrafficNodeData }) {
  // Determine opacity based on highlight/fade state
  const opacity = data.isFaded ? 0.2 : 1
  const scale = data.isHighlighted ? 1.05 : 1
  const zIndex = data.isHighlighted ? 50 : 1

  return (
    <div
      className="relative transition-all duration-300 ease-out cursor-pointer"
      style={{
        opacity,
        transform: `scale(${scale})`,
        zIndex,
      }}
    >
      {/* Handles for connections */}
      <Handle
        type="target"
        position={Position.Top}
        className="!bg-blue-500 !w-2 !h-2 !border-2 !border-white dark:!border-gray-800"
      />
      <Handle
        type="source"
        position={Position.Bottom}
        className="!bg-blue-500 !w-2 !h-2 !border-2 !border-white dark:!border-gray-800"
      />

      {/* Node Card */}
      <div className={`bg-white dark:bg-gray-800 rounded-xl p-3 border-2 shadow-lg transition-all duration-200 min-w-[100px] ${
        data.isSelected
          ? 'border-blue-500 shadow-blue-500/30 shadow-xl ring-2 ring-blue-500/50'
          : data.isHighlighted
            ? 'border-blue-400 shadow-xl'
            : 'border-gray-200 dark:border-gray-600 hover:shadow-xl hover:border-blue-500'
      }`}>
        {/* Icon */}
        <div className="flex justify-center mb-2">
          {data.type === 'external' ? (
            <div className="w-10 h-10 rounded-xl bg-gradient-to-br from-blue-500 to-blue-600 flex items-center justify-center shadow-md">
              <Globe size={24} className="text-white" />
            </div>
          ) : (
            <AppIcon appName={data.appId} size={40} className="rounded-xl shadow-md" />
          )}
        </div>

        {/* Name */}
        <div className="text-center mb-2">
          <div className="text-xs font-semibold text-gray-900 dark:text-white truncate max-w-[90px]">
            {data.label}
          </div>
        </div>

        {/* Traffic Stats */}
        <div className="flex flex-col gap-0.5 text-[10px]">
          <div className="flex items-center justify-center gap-1 text-green-500">
            <ArrowDownToLine size={10} />
            <span className="font-mono">{formatBandwidth(data.rx)}</span>
          </div>
          <div className="flex items-center justify-center gap-1 text-orange-500">
            <ArrowUpFromLine size={10} />
            <span className="font-mono">{formatBandwidth(data.tx)}</span>
          </div>
        </div>


        {/* VPN indicator badge */}
        {data.vpnConfig && (
          <div className="absolute -top-2 -right-2 group">
            <div className="bg-green-500 text-white rounded-full w-5 h-5 flex items-center justify-center shadow cursor-help">
              <Shield size={12} />
            </div>
            {/* VPN Tooltip */}
            <div className="absolute bottom-full right-0 mb-2 hidden group-hover:block z-50">
              <div className="bg-gray-900 text-white text-xs rounded-lg px-3 py-2 whitespace-nowrap shadow-lg">
                <div className="font-semibold text-green-400 mb-1">VPN Active</div>
                <div className="text-gray-300">Provider: {data.vpnConfig.vpn_provider_name}</div>
                <div className="text-gray-300">
                  Kill Switch: {data.vpnConfig.effective_kill_switch ? 'ON' : 'OFF'}
                </div>
                <div className="absolute bottom-0 right-3 translate-y-full">
                  <div className="border-4 border-transparent border-t-gray-900"></div>
                </div>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
