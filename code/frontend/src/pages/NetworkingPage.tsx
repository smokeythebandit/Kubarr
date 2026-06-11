import { useState, useMemo, useEffect, useCallback, useRef } from 'react'
import { NetworkTopology, formatBandwidth, formatPackets } from '../api/networking'
import { AppVpnConfig, appVpnApi } from '../api/vpn'
import { useNetworkMetricsWs, ConnectionMode } from '../hooks/useNetworkMetricsWs'
import { AppIcon } from '../components/AppIcon'
import {
  ReactFlow,
  Node,
  Edge,
  Background,
  Controls,
  MiniMap,
  useNodesState,
  useEdgesState,
  useReactFlow,
  Position,
  Handle,
  BaseEdge,
  EdgeProps,
  getBezierPath,
  ReactFlowProvider,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import dagre from 'dagre'
import {
  Network,
  RefreshCw,
  AlertCircle,
  ArrowDownToLine,
  ArrowUpFromLine,
  Globe,
  Wifi,
  WifiOff,
  Zap,
  Maximize2,
  Minimize2,
  Radio,
  RotateCw,
  Shield,
} from 'lucide-react'

// ============================================================================
// Custom Node Component
// ============================================================================

interface TrafficNodeData {
  label: string
  type: 'external' | 'app'
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

function TrafficNode({ data }: { data: TrafficNodeData }) {
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

// ============================================================================
// Animated Edge Component
// ============================================================================

interface AnimatedEdgeData {
  traffic: number
  maxTraffic: number
  isHighlighted: boolean
  isFaded: boolean
  [key: string]: unknown
}

function AnimatedEdge({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  data,
  style,
}: EdgeProps<Edge<AnimatedEdgeData>>) {
  const [edgePath] = getBezierPath({
    sourceX,
    sourceY,
    sourcePosition,
    targetX,
    targetY,
    targetPosition,
  })

  // Calculate line width based on traffic (2-8px)
  const traffic = data?.traffic || 0
  const maxTraffic = data?.maxTraffic || 1
  const strokeWidth = 2 + (traffic / maxTraffic) * 6

  // Animation speed based on traffic (faster = more traffic)
  const animationDuration = Math.max(1, 4 - (traffic / maxTraffic) * 3)

  // Handle fading
  const isFaded = data?.isFaded ?? false
  const isHighlighted = data?.isHighlighted ?? false
  const opacity = isFaded ? 0.1 : isHighlighted ? 1 : 0.6
  const strokeColor = isHighlighted ? 'rgba(59, 130, 246, 0.6)' : 'rgba(59, 130, 246, 0.2)'
  const particleColor = isHighlighted ? '#3b82f6' : '#94a3b8'

  return (
    <g style={{ opacity, transition: 'opacity 0.3s ease-out' }}>
      {/* Background path */}
      <BaseEdge
        id={id}
        path={edgePath}
        style={{
          ...style,
          strokeWidth: isHighlighted ? strokeWidth + 1 : strokeWidth,
          stroke: strokeColor,
        }}
      />

      {/* Animated particles - bidirectional flow */}
      {!isFaded && (
        <>
          {/* Forward direction (source -> target) */}
          <circle r="4" fill={particleColor}>
            <animateMotion
              dur={`${animationDuration}s`}
              repeatCount="indefinite"
              path={edgePath}
            />
          </circle>
          <circle r="4" fill={particleColor} style={{ opacity: 0.6 }}>
            <animateMotion
              dur={`${animationDuration}s`}
              repeatCount="indefinite"
              path={edgePath}
              begin={`${animationDuration / 3}s`}
            />
          </circle>
          <circle r="4" fill={particleColor} style={{ opacity: 0.3 }}>
            <animateMotion
              dur={`${animationDuration}s`}
              repeatCount="indefinite"
              path={edgePath}
              begin={`${(animationDuration / 3) * 2}s`}
            />
          </circle>
          {/* Reverse direction (target -> source) */}
          <circle r="4" fill={particleColor}>
            <animateMotion
              dur={`${animationDuration}s`}
              repeatCount="indefinite"
              path={edgePath}
              keyPoints="1;0"
              keyTimes="0;1"
              begin={`${animationDuration / 6}s`}
            />
          </circle>
          <circle r="4" fill={particleColor} style={{ opacity: 0.6 }}>
            <animateMotion
              dur={`${animationDuration}s`}
              repeatCount="indefinite"
              path={edgePath}
              keyPoints="1;0"
              keyTimes="0;1"
              begin={`${animationDuration / 2}s`}
            />
          </circle>
          <circle r="4" fill={particleColor} style={{ opacity: 0.3 }}>
            <animateMotion
              dur={`${animationDuration}s`}
              repeatCount="indefinite"
              path={edgePath}
              keyPoints="1;0"
              keyTimes="0;1"
              begin={`${(animationDuration / 6) * 5}s`}
            />
          </circle>
        </>
      )}
    </g>
  )
}

// ============================================================================
// Dagre Layout
// ============================================================================

const nodeWidth = 120
const nodeHeight = 100

function getLayoutedElements(
  nodes: Node[],
  edges: Edge[],
  direction: 'TB' | 'LR' = 'TB'
) {
  const dagreGraph = new dagre.graphlib.Graph()
  dagreGraph.setDefaultEdgeLabel(() => ({}))

  const isHorizontal = direction === 'LR'
  dagreGraph.setGraph({ rankdir: direction, nodesep: 80, ranksep: 100 })

  nodes.forEach((node) => {
    dagreGraph.setNode(node.id, { width: nodeWidth, height: nodeHeight })
  })

  edges.forEach((edge) => {
    dagreGraph.setEdge(edge.source, edge.target)
  })

  dagre.layout(dagreGraph)

  const layoutedNodes = nodes.map((node) => {
    const nodeWithPosition = dagreGraph.node(node.id)
    return {
      ...node,
      position: {
        x: nodeWithPosition.x - nodeWidth / 2,
        y: nodeWithPosition.y - nodeHeight / 2,
      },
      targetPosition: isHorizontal ? Position.Left : Position.Top,
      sourcePosition: isHorizontal ? Position.Right : Position.Bottom,
    }
  })

  return { nodes: layoutedNodes, edges }
}

// ============================================================================
// Network Flow Visualization with React Flow
// ============================================================================

const nodeTypes = { traffic: TrafficNode }
const edgeTypes = { animated: AnimatedEdge }

interface FlowVisualizationProps {
  topology: NetworkTopology
  isFullscreen?: boolean
  vpnConfigs?: AppVpnConfig[]
}

function NetworkFlowVisualizationInner({ topology, isFullscreen, vpnConfigs = [] }: FlowVisualizationProps) {
  // State for selected and hovered nodes
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null)
  const [hoveredNodeId, setHoveredNodeId] = useState<string | null>(null)

  // Get ReactFlow instance for fitView
  const { fitView } = useReactFlow()

  // Fit to screen when entering/exiting fullscreen
  useEffect(() => {
    // Small delay to allow the container to resize
    const timer = setTimeout(() => {
      fitView({ padding: 0.2, duration: 200 })
    }, 100)
    return () => clearTimeout(timer)
  }, [isFullscreen, fitView])

  // Helper to get connected node IDs for a given node
  const getConnectedNodeIds = useMemo(() => {
    const connectionMap = new Map<string, Set<string>>()

    topology.edges.forEach(edge => {
      if (!connectionMap.has(edge.source)) {
        connectionMap.set(edge.source, new Set())
      }
      if (!connectionMap.has(edge.target)) {
        connectionMap.set(edge.target, new Set())
      }
      connectionMap.get(edge.source)!.add(edge.target)
      connectionMap.get(edge.target)!.add(edge.source)
    })

    return (nodeId: string): Set<string> => {
      const connected = connectionMap.get(nodeId) || new Set()
      connected.add(nodeId) // Include the node itself
      return connected
    }
  }, [topology.edges])

  // Convert topology to React Flow nodes and edges
  const { initialNodes, initialEdges } = useMemo(() => {
    const maxTraffic = Math.max(...topology.nodes.map(n => n.total_traffic), 1)

    const nodes: Node<TrafficNodeData>[] = topology.nodes.map((node) => {
      // Find VPN config for this app (match by node id which is the app name/namespace)
      const vpnConfig = vpnConfigs.find(c => c.app_name === node.id)

      return {
        id: node.id,
        type: 'traffic',
        position: { x: 0, y: 0 }, // Will be set by dagre
        data: {
          label: node.name,
          type: node.type,
          rx: node.rx_bytes_per_sec,
          tx: node.tx_bytes_per_sec,
          total: node.total_traffic,
          podCount: node.pod_count,
          appId: node.id,
          isHighlighted: false,
          isFaded: false,
          isSelected: false,
          vpnConfig,
        },
      }
    })

    const edges: Edge<AnimatedEdgeData>[] = topology.edges.map((edge, i) => {
      const sourceNode = topology.nodes.find(n => n.id === edge.source)
      const targetNode = topology.nodes.find(n => n.id === edge.target)
      const traffic = Math.min(sourceNode?.total_traffic || 0, targetNode?.total_traffic || 0)

      return {
        id: `e${i}-${edge.source}-${edge.target}`,
        source: edge.source,
        target: edge.target,
        type: 'animated',
        data: { traffic, maxTraffic, isHighlighted: false, isFaded: false },
      }
    })

    return { initialNodes: nodes, initialEdges: edges, maxTraffic }
  }, [topology, vpnConfigs])

  // Apply dagre layout
  const { nodes: layoutedNodes, edges: layoutedEdges } = useMemo(
    () => getLayoutedElements(initialNodes, initialEdges, 'TB'),
    [initialNodes, initialEdges]
  )

  const [nodes, setNodes, onNodesChange] = useNodesState(layoutedNodes)
  const [edges, setEdges, onEdgesChange] = useEdgesState(layoutedEdges)

  // Track the structure (node IDs and edge connections) to detect when layout needs recalculating
  const structureKey = useMemo(() => {
    const nodeIds = topology.nodes.map(n => n.id).sort().join(',')
    const edgeKeys = topology.edges.map(e => `${e.source}->${e.target}`).sort().join(',')
    return `${nodeIds}|${edgeKeys}`
  }, [topology])

  // Update nodes when topology changes - only recalculate layout if structure changed
  const prevStructureRef = useRef(structureKey)
  useEffect(() => {
    const structureChanged = prevStructureRef.current !== structureKey
    prevStructureRef.current = structureKey

    if (structureChanged) {
      // Structure changed - recalculate layout
      const { nodes: newLayoutedNodes, edges: newLayoutedEdges } = getLayoutedElements(
        initialNodes,
        initialEdges,
        'TB'
      )
      setNodes(newLayoutedNodes)
      setEdges(newLayoutedEdges)
    } else {
      // Only data changed - update data without changing positions
      setNodes(nodes =>
        nodes.map(node => {
          const newNodeData = initialNodes.find(n => n.id === node.id)
          if (newNodeData) {
            return {
              ...node,
              data: {
                ...node.data,
                rx: newNodeData.data.rx,
                tx: newNodeData.data.tx,
                total: newNodeData.data.total,
                podCount: newNodeData.data.podCount,
              },
            }
          }
          return node
        })
      )
      setEdges(edges =>
        edges.map(edge => {
          const newEdgeData = initialEdges.find(e => e.id === edge.id)
          if (newEdgeData) {
            return {
              ...edge,
              data: {
                ...edge.data,
                traffic: newEdgeData.data?.traffic || 0,
                maxTraffic: newEdgeData.data?.maxTraffic || 1,
              },
            }
          }
          return edge
        })
      )
    }
  }, [initialNodes, initialEdges, structureKey, setNodes, setEdges])

  // Update highlight/fade state based on selected/hovered node
  useEffect(() => {
    const activeNodeId = selectedNodeId || hoveredNodeId
    const isClickMode = selectedNodeId !== null

    if (!activeNodeId) {
      // No selection or hover - reset all to normal
      setNodes(nodes =>
        nodes.map(node => ({
          ...node,
          data: {
            ...node.data,
            isHighlighted: false,
            isFaded: false,
            isSelected: false,
          },
        }))
      )
      setEdges(edges =>
        edges.map(edge => ({
          ...edge,
          data: {
            ...edge.data,
            isHighlighted: false,
            isFaded: false,
          },
        }))
      )
      return
    }

    const connectedIds = getConnectedNodeIds(activeNodeId)

    setNodes(nodes =>
      nodes.map(node => {
        const isConnected = connectedIds.has(node.id)
        const isActive = node.id === activeNodeId
        return {
          ...node,
          data: {
            ...node.data,
            isHighlighted: isConnected,
            isFaded: isClickMode && !isConnected, // Only fade in click mode, not hover
            isSelected: isActive && isClickMode,
          },
          hidden: isClickMode && !isConnected, // Hide non-connected nodes in click mode
        }
      })
    )

    setEdges(edges =>
      edges.map(edge => {
        const isConnected = edge.source === activeNodeId || edge.target === activeNodeId
        return {
          ...edge,
          data: {
            ...edge.data,
            isHighlighted: isConnected,
            isFaded: !isConnected,
          },
          hidden: isClickMode && !isConnected, // Hide non-connected edges in click mode
        }
      })
    )
  }, [selectedNodeId, hoveredNodeId, getConnectedNodeIds, setNodes, setEdges])

  // Handle node click
  const onNodeClick = useCallback((_event: React.MouseEvent, node: Node) => {
    setSelectedNodeId(prev => (prev === node.id ? null : node.id)) // Toggle selection
  }, [])

  // Handle node mouse enter/leave for hover effect
  const onNodeMouseEnter = useCallback((_event: React.MouseEvent, node: Node) => {
    if (!selectedNodeId) {
      setHoveredNodeId(node.id)
    }
  }, [selectedNodeId])

  const onNodeMouseLeave = useCallback(() => {
    setHoveredNodeId(null)
  }, [])

  // Handle pane click to clear selection
  const onPaneClick = useCallback(() => {
    setSelectedNodeId(null)
  }, [])

  if (topology.nodes.length === 0) {
    return (
      <div className="flex items-center justify-center h-full text-gray-500 dark:text-gray-400">
        <div className="text-center">
          <Network size={48} className="mx-auto mb-4 opacity-50" />
          <p>No network data available</p>
        </div>
      </div>
    )
  }

  return (
    <div className="relative w-full h-full">
      {/* Selection indicator */}
      {selectedNodeId && (
        <div className="absolute top-4 left-4 z-10 bg-blue-500 text-white px-3 py-1.5 rounded-lg shadow-lg flex items-center gap-2 text-sm">
          <span>Showing connections for: <strong>{selectedNodeId}</strong></span>
          <button
            onClick={() => setSelectedNodeId(null)}
            className="ml-2 hover:bg-blue-600 rounded p-0.5 transition-colors"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <line x1="18" y1="6" x2="6" y2="18" />
              <line x1="6" y1="6" x2="18" y2="18" />
            </svg>
          </button>
        </div>
      )}

      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onNodeClick={onNodeClick}
        onNodeMouseEnter={onNodeMouseEnter}
        onNodeMouseLeave={onNodeMouseLeave}
        onPaneClick={onPaneClick}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
        fitView
        fitViewOptions={{ padding: 0.2 }}
        minZoom={0.5}
        maxZoom={1.5}
        proOptions={{ hideAttribution: true }}
        className="bg-gray-50 dark:bg-gray-900"
      >
        <Background color="#e5e7eb" gap={20} className="dark:!bg-gray-900" />
        <Controls className="!bg-white dark:!bg-gray-800 !border-gray-200 dark:!border-gray-700 !shadow-lg [&>button]:!bg-white [&>button]:dark:!bg-gray-800 [&>button]:!border-gray-200 [&>button]:dark:!border-gray-700 [&>button]:!text-gray-600 [&>button]:dark:!text-gray-300 [&>button:hover]:!bg-gray-100 [&>button:hover]:dark:!bg-gray-700" />
        <MiniMap
          nodeColor={(node) => {
            const data = node.data as TrafficNodeData
            if (data.type === 'external') return '#3b82f6'
            return '#10b981'
          }}
          className="!bg-white dark:!bg-gray-800 !border-gray-200 dark:!border-gray-700"
          maskColor="rgba(0, 0, 0, 0.1)"
        />
      </ReactFlow>
    </div>
  )
}

function NetworkFlowVisualization({ topology, isFullscreen, vpnConfigs = [] }: FlowVisualizationProps) {
  return (
    <ReactFlowProvider>
      <NetworkFlowVisualizationInner topology={topology} isFullscreen={isFullscreen} vpnConfigs={vpnConfigs} />
    </ReactFlowProvider>
  )
}

// ============================================================================
// Stats Table Component
// ============================================================================

function NetworkStatsTable({ stats }: { stats: any[] }) {
  const sortedStats = [...stats].sort((a, b) =>
    (b.rx_bytes_per_sec + b.tx_bytes_per_sec) - (a.rx_bytes_per_sec + a.tx_bytes_per_sec)
  )

  return (
    <div className="bg-white dark:bg-gray-800 rounded-lg overflow-hidden border border-gray-200 dark:border-gray-700">
      <table className="w-full">
        <thead>
          <tr className="border-b border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-700/50">
            <th className="text-left px-4 py-3 text-sm font-medium text-gray-600 dark:text-gray-300">App</th>
            <th className="text-right px-4 py-3 text-sm font-medium text-gray-600 dark:text-gray-300">
              <div className="flex items-center justify-end gap-1">
                <ArrowDownToLine size={14} className="text-green-500" />
                Receive
              </div>
            </th>
            <th className="text-right px-4 py-3 text-sm font-medium text-gray-600 dark:text-gray-300">
              <div className="flex items-center justify-end gap-1">
                <ArrowUpFromLine size={14} className="text-orange-500" />
                Transmit
              </div>
            </th>
            <th className="text-right px-4 py-3 text-sm font-medium text-gray-600 dark:text-gray-300">Packets</th>
            <th className="text-right px-4 py-3 text-sm font-medium text-gray-600 dark:text-gray-300">Errors</th>
            <th className="text-right px-4 py-3 text-sm font-medium text-gray-600 dark:text-gray-300">Dropped</th>
            <th className="text-center px-4 py-3 text-sm font-medium text-gray-600 dark:text-gray-300">Status</th>
          </tr>
        </thead>
        <tbody>
          {sortedStats.map(stat => {
            const hasErrors = stat.rx_errors_per_sec > 0 || stat.tx_errors_per_sec > 0
            const hasDropped = stat.rx_dropped_per_sec > 0 || stat.tx_dropped_per_sec > 0

            return (
              <tr
                key={stat.namespace}
                className="border-b border-gray-200 dark:border-gray-700/50 hover:bg-gray-50 dark:hover:bg-gray-700/30"
              >
                <td className="px-4 py-3">
                  <div className="flex items-center gap-3">
                    <AppIcon appName={stat.namespace} size={32} />
                    <span className="font-medium text-gray-900 dark:text-white">{stat.app_name}</span>
                  </div>
                </td>
                <td className="text-right px-4 py-3">
                  <span className="text-green-500 font-mono text-sm">
                    {formatBandwidth(stat.rx_bytes_per_sec)}
                  </span>
                </td>
                <td className="text-right px-4 py-3">
                  <span className="text-orange-500 font-mono text-sm">
                    {formatBandwidth(stat.tx_bytes_per_sec)}
                  </span>
                </td>
                <td className="text-right px-4 py-3">
                  <div className="text-gray-500 dark:text-gray-400 font-mono text-xs">
                    <div>{formatPackets(stat.rx_packets_per_sec)} in</div>
                    <div>{formatPackets(stat.tx_packets_per_sec)} out</div>
                  </div>
                </td>
                <td className="text-right px-4 py-3">
                  <span className={`font-mono text-xs ${hasErrors ? 'text-red-500' : 'text-gray-400'}`}>
                    {stat.rx_errors_per_sec + stat.tx_errors_per_sec > 0
                      ? (stat.rx_errors_per_sec + stat.tx_errors_per_sec).toFixed(2)
                      : '0'}
                  </span>
                </td>
                <td className="text-right px-4 py-3">
                  <span className={`font-mono text-xs ${hasDropped ? 'text-yellow-500' : 'text-gray-400'}`}>
                    {stat.rx_dropped_per_sec + stat.tx_dropped_per_sec > 0
                      ? (stat.rx_dropped_per_sec + stat.tx_dropped_per_sec).toFixed(2)
                      : '0'}
                  </span>
                </td>
                <td className="text-center px-4 py-3">
                  {hasErrors ? (
                    <span className="inline-flex items-center gap-1 px-2 py-1 rounded-full text-xs bg-red-100 dark:bg-red-900/30 text-red-600 dark:text-red-400">
                      <WifiOff size={12} />
                      Errors
                    </span>
                  ) : hasDropped ? (
                    <span className="inline-flex items-center gap-1 px-2 py-1 rounded-full text-xs bg-yellow-100 dark:bg-yellow-900/30 text-yellow-600 dark:text-yellow-400">
                      <AlertCircle size={12} />
                      Drops
                    </span>
                  ) : (
                    <span className="inline-flex items-center gap-1 px-2 py-1 rounded-full text-xs bg-green-100 dark:bg-green-900/30 text-green-600 dark:text-green-400">
                      <Wifi size={12} />
                      Healthy
                    </span>
                  )}
                </td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}

// ============================================================================
// Connection Status Indicator
// ============================================================================

interface ConnectionStatusProps {
  mode: ConnectionMode
}

function ConnectionStatus({ mode }: ConnectionStatusProps) {
  const config = {
    websocket: {
      icon: Radio,
      label: 'Live',
      className: 'bg-green-100 dark:bg-green-900/30 text-green-600 dark:text-green-400',
      iconClassName: 'text-green-500',
    },
    polling: {
      icon: RotateCw,
      label: 'Polling',
      className: 'bg-yellow-100 dark:bg-yellow-900/30 text-yellow-600 dark:text-yellow-400',
      iconClassName: 'text-yellow-500',
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

// ============================================================================
// Main Page Component
// ============================================================================

// Infrastructure namespaces to hide by default
const INFRA_NAMESPACES = ['fluent-bit', 'victoriametrics', 'victorialogs']

export default function NetworkingPage() {
  const [isFullscreen, setIsFullscreen] = useState(false)
  const [showInfra, setShowInfra] = useState(false)
  const [vpnConfigs, setVpnConfigs] = useState<AppVpnConfig[]>([])
  const networkFlowRef = useRef<HTMLDivElement>(null)

  // Use WebSocket hook for real-time updates with HTTP polling fallback
  const {
    topology,
    stats,
    connectionMode,
    error: wsError,
  } = useNetworkMetricsWs()

  // Fetch VPN configs for app indicators
  useEffect(() => {
    const fetchVpnConfigs = async () => {
      try {
        const configs = await appVpnApi.listConfigs()
        setVpnConfigs(configs)
      } catch (err) {
        console.error('Failed to fetch VPN configs:', err)
      }
    }
    fetchVpnConfigs()
    // Refresh VPN configs every 30 seconds
    const interval = setInterval(fetchVpnConfigs, 30000)
    return () => clearInterval(interval)
  }, [])

  const isLoading = !topology && !stats
  const hasError = wsError && connectionMode === 'disconnected'

  // Fullscreen toggle
  const toggleFullscreen = useCallback(() => {
    if (!networkFlowRef.current) return

    if (!document.fullscreenElement) {
      networkFlowRef.current.requestFullscreen().then(() => {
        setIsFullscreen(true)
      }).catch((err: unknown) => {
        console.error('Failed to enter fullscreen:', err)
      })
    } else {
      document.exitFullscreen().then(() => {
        setIsFullscreen(false)
      }).catch((err: unknown) => {
        console.error('Failed to exit fullscreen:', err)
      })
    }
  }, [])

  // Listen for fullscreen change events (e.g., Escape key)
  useEffect(() => {
    const handleFullscreenChange = () => {
      setIsFullscreen(!!document.fullscreenElement)
    }
    document.addEventListener('fullscreenchange', handleFullscreenChange)
    return () => document.removeEventListener('fullscreenchange', handleFullscreenChange)
  }, [])

  // Calculate totals
  const totalRx = stats.reduce((sum, s) => sum + s.rx_bytes_per_sec, 0)
  const totalTx = stats.reduce((sum, s) => sum + s.tx_bytes_per_sec, 0)
  const totalErrors = stats.reduce((sum, s) => sum + s.rx_errors_per_sec + s.tx_errors_per_sec, 0)
  const totalDropped = stats.reduce((sum, s) => sum + s.rx_dropped_per_sec + s.tx_dropped_per_sec, 0)

  // Get internet traffic from the external node
  const externalNode = topology?.nodes.find(n => n.type === 'external')
  const internetTraffic = externalNode ? externalNode.rx_bytes_per_sec + externalNode.tx_bytes_per_sec : totalRx + totalTx

  // Filter topology based on showInfra toggle
  const filteredTopology = useMemo(() => {
    if (!topology) return null
    if (showInfra) return topology

    const filteredNodes = topology.nodes.filter(
      node => !INFRA_NAMESPACES.includes(node.id)
    )
    const filteredNodeIds = new Set(filteredNodes.map(n => n.id))
    const filteredEdges = topology.edges.filter(
      edge => filteredNodeIds.has(edge.source) && filteredNodeIds.has(edge.target)
    )

    return { nodes: filteredNodes, edges: filteredEdges }
  }, [topology, showInfra])

  if (hasError) {
    return (
      <div className="flex flex-col items-center justify-center h-[60vh] text-center">
        <AlertCircle size={64} className="text-red-500 mb-4" />
        <h2 className="text-2xl font-bold mb-2 text-gray-900 dark:text-white">Error Loading Network Data</h2>
        <p className="text-gray-500 dark:text-gray-400 max-w-md">
          Could not connect to network metrics. Check your connection and try again.
        </p>
        <button
          onClick={() => window.location.reload()}
          className="mt-4 px-4 py-2 bg-blue-600 hover:bg-blue-700 rounded-lg text-white"
        >
          Retry
        </button>
      </div>
    )
  }

  return (
    <div className="space-y-8">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold flex items-center gap-2 text-gray-900 dark:text-white">
            <Network className="text-blue-500 dark:text-blue-400" />
            Networking
          </h1>
          <p className="text-gray-500 dark:text-gray-400 mt-1">
            Network topology and traffic flow
          </p>
        </div>
        <div className="flex items-center gap-3">
          <ConnectionStatus mode={connectionMode} />
          <button
            onClick={() => window.location.reload()}
            disabled={isLoading}
            className="flex items-center gap-2 px-4 py-2 bg-blue-600 hover:bg-blue-700 rounded-lg transition-colors disabled:opacity-50 text-white"
          >
            <RefreshCw size={18} className={isLoading ? 'animate-spin' : ''} />
            Refresh
          </button>
        </div>
      </div>

      {/* Summary Cards */}
      <div className="grid grid-cols-2 md:grid-cols-5 gap-4">
        <div className="bg-white dark:bg-gray-800 rounded-lg p-4 border border-gray-200 dark:border-gray-700">
          <div className="flex items-center gap-2 text-gray-500 dark:text-gray-400 mb-2">
            <Globe size={18} className="text-blue-500" />
            <span className="text-sm">Internet Traffic</span>
          </div>
          <div className="text-2xl font-bold text-blue-500">{formatBandwidth(internetTraffic)}</div>
        </div>
        <div className="bg-white dark:bg-gray-800 rounded-lg p-4 border border-gray-200 dark:border-gray-700">
          <div className="flex items-center gap-2 text-gray-500 dark:text-gray-400 mb-2">
            <ArrowDownToLine size={18} className="text-green-500" />
            <span className="text-sm">Total Receive</span>
          </div>
          <div className="text-2xl font-bold text-green-500">{formatBandwidth(totalRx)}</div>
        </div>
        <div className="bg-white dark:bg-gray-800 rounded-lg p-4 border border-gray-200 dark:border-gray-700">
          <div className="flex items-center gap-2 text-gray-500 dark:text-gray-400 mb-2">
            <ArrowUpFromLine size={18} className="text-orange-500" />
            <span className="text-sm">Total Transmit</span>
          </div>
          <div className="text-2xl font-bold text-orange-500">{formatBandwidth(totalTx)}</div>
        </div>
        <div className="bg-white dark:bg-gray-800 rounded-lg p-4 border border-gray-200 dark:border-gray-700">
          <div className="flex items-center gap-2 text-gray-500 dark:text-gray-400 mb-2">
            <Zap size={18} className={totalErrors > 0 ? 'text-red-500' : 'text-gray-400'} />
            <span className="text-sm">Errors/sec</span>
          </div>
          <div className={`text-2xl font-bold ${totalErrors > 0 ? 'text-red-500' : 'text-gray-400'}`}>
            {totalErrors.toFixed(2)}
          </div>
        </div>
        <div className="bg-white dark:bg-gray-800 rounded-lg p-4 border border-gray-200 dark:border-gray-700">
          <div className="flex items-center gap-2 text-gray-500 dark:text-gray-400 mb-2">
            <AlertCircle size={18} className={totalDropped > 0 ? 'text-yellow-500' : 'text-gray-400'} />
            <span className="text-sm">Dropped/sec</span>
          </div>
          <div className={`text-2xl font-bold ${totalDropped > 0 ? 'text-yellow-500' : 'text-gray-400'}`}>
            {totalDropped.toFixed(2)}
          </div>
        </div>
      </div>

      {/* Network Flow Visualization */}
      <div>
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-xl font-semibold flex items-center gap-2 text-gray-900 dark:text-white">
            <Globe size={20} />
            Network Flow
            <span className="text-sm font-normal text-gray-500 dark:text-gray-400 ml-2">
              Click app to focus • Hover to highlight • Drag to pan
            </span>
          </h2>
          <div className="flex items-center gap-2">
            <button
              onClick={() => setShowInfra(!showInfra)}
              className={`flex items-center gap-2 px-3 py-1.5 rounded-lg transition-colors text-sm ${
                showInfra
                  ? 'bg-purple-100 dark:bg-purple-900/30 text-purple-700 dark:text-purple-300'
                  : 'bg-gray-100 dark:bg-gray-700 text-gray-500 dark:text-gray-400 hover:bg-gray-200 dark:hover:bg-gray-600'
              }`}
              title={showInfra ? 'Hide infrastructure' : 'Show infrastructure (fluent-bit, victorialogs, victoriametrics)'}
            >
              <span>{showInfra ? 'Infra: ON' : 'Infra: OFF'}</span>
            </button>
            <button
              onClick={toggleFullscreen}
              className="flex items-center gap-2 px-3 py-1.5 bg-gray-100 dark:bg-gray-700 hover:bg-gray-200 dark:hover:bg-gray-600 rounded-lg transition-colors text-gray-700 dark:text-gray-300"
              title={isFullscreen ? 'Exit fullscreen' : 'Enter fullscreen'}
            >
              {isFullscreen ? <Minimize2 size={18} /> : <Maximize2 size={18} />}
              <span className="text-sm">{isFullscreen ? 'Exit' : 'Fullscreen'}</span>
            </button>
          </div>
        </div>
        <div
          ref={networkFlowRef}
          className={`bg-white dark:bg-gray-800 rounded-lg border border-gray-200 dark:border-gray-700 overflow-hidden ${
            isFullscreen ? 'fixed inset-0 z-50 rounded-none border-0' : ''
          }`}
          style={{ height: isFullscreen ? '100vh' : '665px' }}
        >
          {/* Fullscreen header */}
          {isFullscreen && (
            <div className="absolute top-4 right-4 z-20">
              <button
                onClick={toggleFullscreen}
                className="flex items-center gap-2 px-3 py-2 bg-gray-900/80 hover:bg-gray-900 text-white rounded-lg transition-colors shadow-lg"
              >
                <Minimize2 size={18} />
                <span className="text-sm">Exit Fullscreen</span>
              </button>
            </div>
          )}
          {isLoading ? (
            <div className="flex items-center justify-center h-full">
              <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-500"></div>
            </div>
          ) : filteredTopology ? (
            <NetworkFlowVisualization topology={filteredTopology} isFullscreen={isFullscreen} vpnConfigs={vpnConfigs} />
          ) : null}
        </div>
      </div>

      {/* Network Statistics Table */}
      <div>
        <h2 className="text-xl font-semibold mb-4 flex items-center gap-2 text-gray-900 dark:text-white">
          <Network size={20} />
          Network Statistics
        </h2>
        {isLoading ? (
          <div className="flex items-center justify-center py-12">
            <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500"></div>
          </div>
        ) : stats.length > 0 ? (
          <NetworkStatsTable stats={stats} />
        ) : (
          <div className="bg-white dark:bg-gray-800 rounded-lg p-8 text-center text-gray-500 dark:text-gray-400 border border-gray-200 dark:border-gray-700">
            <Network size={48} className="mx-auto mb-4 opacity-50" />
            <p>No network statistics available</p>
            <p className="text-sm mt-1">Install some apps to see their network usage</p>
          </div>
        )}
      </div>

      {/* Connection mode indicator */}
      <div className="text-center text-sm text-gray-500 dark:text-gray-500">
        {connectionMode === 'websocket' && 'Real-time updates via WebSocket'}
        {connectionMode === 'polling' && 'Updating via HTTP polling (reconnecting...)'}
        {connectionMode === 'disconnected' && 'Disconnected'}
      </div>
    </div>
  )
}
