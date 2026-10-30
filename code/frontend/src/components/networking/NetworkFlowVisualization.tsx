import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
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
  ReactFlowProvider,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'
import dagre from 'dagre'
import { Network } from 'lucide-react'
import { NetworkTopology } from '../../api/networking'
import { AppVpnConfig } from '../../api/vpn'
import { TrafficNode, TrafficNodeData } from './TrafficNode'
import { AnimatedEdge, AnimatedEdgeData } from './AnimatedEdge'

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
    // Edge weight is the link's own measured byte rate, not the endpoints'
    // total volume and not a connection count. A busy app talking to a quiet
    // one used to make that link look busy, and weighting by connections made
    // a chatty link outrank one carrying a download.
    const maxBytesPerSec = Math.max(
      ...topology.edges.map(edge => edge.bytes_per_sec ?? 0),
      1
    )

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

    const edges: Edge<AnimatedEdgeData>[] = topology.edges.map((edge, i) => ({
      id: `e${i}-${edge.source}-${edge.target}`,
      source: edge.source,
      target: edge.target,
      type: 'animated',
      data: {
        bytesPerSec: edge.bytes_per_sec ?? 0,
        maxBytesPerSec,
        observed: edge.observed,
        isHighlighted: false,
        isFaded: false,
      },
    }))

    return { initialNodes: nodes, initialEdges: edges }
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
                bytesPerSec: newEdgeData.data?.bytesPerSec ?? 0,
                maxBytesPerSec: newEdgeData.data?.maxBytesPerSec ?? 1,
                observed: newEdgeData.data?.observed ?? false,
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
          hidden: false,
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
          hidden: false,
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

      {/* What a line means. Without this the dashed edges read as a rendering
          glitch rather than "configured but silent". */}
      <div className="absolute bottom-4 left-16 z-10 flex flex-col gap-1 rounded-lg bg-white/90 px-3 py-2 text-xs shadow-lg dark:bg-gray-800/90">
        <div className="flex items-center gap-2 text-gray-600 dark:text-gray-300">
          <svg width="26" height="6" aria-hidden>
            <line x1="0" y1="3" x2="26" y2="3" stroke="#3b82f6" strokeWidth="4" />
          </svg>
          <span>Measured traffic — thicker moves more data</span>
        </div>
        <div className="flex items-center gap-2 text-gray-500 dark:text-gray-400">
          <svg width="26" height="6" aria-hidden>
            <line
              x1="0"
              y1="3"
              x2="26"
              y2="3"
              stroke="#94a3b8"
              strokeWidth="1.5"
              strokeDasharray="4 4"
            />
          </svg>
          <span>Can talk, nothing seen</span>
        </div>
      </div>

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

export function NetworkFlowVisualization({ topology, isFullscreen, vpnConfigs = [] }: FlowVisualizationProps) {
  return (
    <ReactFlowProvider>
      <NetworkFlowVisualizationInner topology={topology} isFullscreen={isFullscreen} vpnConfigs={vpnConfigs} />
    </ReactFlowProvider>
  )
}
