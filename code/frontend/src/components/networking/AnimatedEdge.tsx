import { BaseEdge, Edge, EdgeProps, getBezierPath } from '@xyflow/react'

export interface AnimatedEdgeData {
  /** Measured bytes per second, 0 when nothing was seen on this link. */
  bytesPerSec: number
  /** Busiest measured edge in the graph, for relative weighting. */
  maxBytesPerSec: number
  /** False for a Service-derived edge with no traffic behind it. */
  observed: boolean
  isHighlighted: boolean
  isFaded: boolean
  [key: string]: unknown
}

/**
 * An edge now says two different things.
 *
 * An *observed* edge was measured: its thickness and particle speed come from
 * the real byte rate. An *unobserved* edge exists only because a
 * Kubernetes Service says those two could talk — it stays on the graph, because
 * knowing a link is configured is useful, but it is drawn as a thin dashed line
 * with nothing moving along it. Animating it would claim traffic that never
 * happened.
 */
export function AnimatedEdge({
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

  const observed = data?.observed ?? false
  const bytesPerSec = data?.bytesPerSec ?? 0
  const maxBytesPerSec = Math.max(data?.maxBytesPerSec ?? 1, 1)

  // Weighted by bytes rather than connections: a chatty link moving very
  // little should not look busier than a quiet one carrying a download.
  // Byte rates span orders of magnitude (kilobytes of LAN chatter beside tens
  // of megabytes to the internet), so weight on a log scale or every quiet
  // edge collapses to the minimum.
  const intensity = observed
    ? Math.min(1, Math.log1p(bytesPerSec) / Math.log1p(maxBytesPerSec))
    : 0

  const strokeWidth = observed ? 1.5 + intensity * 6.5 : 1.5
  const animationDuration = Math.max(0.8, 4 - intensity * 3.2)

  const isFaded = data?.isFaded ?? false
  const isHighlighted = data?.isHighlighted ?? false

  // An unobserved edge must read as a deliberate "nothing here", not as a
  // half-rendered line, so it stays clearly visible — it is the dash and the
  // still particles that carry the meaning, not faintness.
  const opacity = isFaded ? 0.1 : isHighlighted ? 1 : observed ? 0.7 : 0.5
  const strokeColor = isHighlighted
    ? 'rgba(59, 130, 246, 0.7)'
    : observed
      ? 'rgba(59, 130, 246, 0.45)'
      : 'rgba(100, 116, 139, 0.85)'
  const particleColor = isHighlighted ? '#3b82f6' : '#94a3b8'

  const particleOffsets = [0, 1 / 3, 2 / 3]

  return (
    <g style={{ opacity, transition: 'opacity 0.3s ease-out' }}>
      <BaseEdge
        id={id}
        path={edgePath}
        style={{
          ...style,
          strokeWidth: isHighlighted ? strokeWidth + 1 : strokeWidth,
          stroke: strokeColor,
          strokeDasharray: observed ? undefined : '4 4',
        }}
      />

      {/* Particles only ever ride a measured edge. */}
      {observed && !isFaded && (
        <>
          {particleOffsets.map((offset, index) => (
            <circle
              key={`forward-${index}`}
              r="4"
              fill={particleColor}
              style={{ opacity: 1 - index * 0.35 }}
            >
              <animateMotion
                dur={`${animationDuration}s`}
                repeatCount="indefinite"
                path={edgePath}
                begin={`${animationDuration * offset}s`}
              />
            </circle>
          ))}
          {particleOffsets.map((offset, index) => (
            <circle
              key={`reverse-${index}`}
              r="4"
              fill={particleColor}
              style={{ opacity: 1 - index * 0.35 }}
            >
              <animateMotion
                dur={`${animationDuration}s`}
                repeatCount="indefinite"
                path={edgePath}
                keyPoints="1;0"
                keyTimes="0;1"
                begin={`${animationDuration * (offset + 1 / 6)}s`}
              />
            </circle>
          ))}
        </>
      )}
    </g>
  )
}
