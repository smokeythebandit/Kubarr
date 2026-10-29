import { Globe, Router, Boxes } from 'lucide-react'
import type { LucideIcon } from 'lucide-react'
import { FlowClassName } from '../../api/networking'

/**
 * One place that decides how each traffic class looks and reads, so the tile,
 * the cards, the chart and the table can never disagree about which colour
 * means "internet".
 *
 * The palette deliberately avoids the green/orange this page already uses for
 * receive/transmit bytes: a class is a *destination*, not a direction, and
 * sharing colours between the two would suggest they are the same axis.
 */
export interface FlowClassStyle {
  name: FlowClassName
  label: string
  /** What the class actually means, for tooltips and empty states. */
  description: string
  icon: LucideIcon
  /** Raw hex, for inline SVG where Tailwind classes don't reach. */
  stroke: string
  fill: string
  text: string
  bg: string
  border: string
  /** Solid swatch for bars and legends. */
  swatch: string
}

export const FLOW_CLASS_STYLES: Record<FlowClassName, FlowClassStyle> = {
  internet: {
    name: 'internet',
    label: 'Internet',
    description: 'Leaving your network entirely',
    icon: Globe,
    stroke: '#0ea5e9',
    fill: 'rgba(14, 165, 233, 0.18)',
    text: 'text-sky-600 dark:text-sky-400',
    bg: 'bg-sky-500/10',
    border: 'border-sky-500/30',
    swatch: 'bg-sky-500',
  },
  lan: {
    name: 'lan',
    label: 'LAN',
    description: 'Staying on your local network',
    icon: Router,
    stroke: '#f59e0b',
    fill: 'rgba(245, 158, 11, 0.18)',
    text: 'text-amber-600 dark:text-amber-400',
    bg: 'bg-amber-500/10',
    border: 'border-amber-500/30',
    swatch: 'bg-amber-500',
  },
  cluster: {
    name: 'cluster',
    label: 'In-cluster',
    description: 'Between pods, never leaving the node',
    icon: Boxes,
    stroke: '#8b5cf6',
    fill: 'rgba(139, 92, 246, 0.18)',
    text: 'text-violet-600 dark:text-violet-400',
    bg: 'bg-violet-500/10',
    border: 'border-violet-500/30',
    swatch: 'bg-violet-500',
  },
}

export function flowClassStyle(name: FlowClassName): FlowClassStyle {
  return FLOW_CLASS_STYLES[name]
}
