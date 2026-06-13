import type { StorageUsageNode } from '../../api/storage'

export interface ExtensionStat {
  extension: string
  size: number
  files: number
  colorIndex: number
}

export interface StatisticsTreeRow {
  node: StorageUsageNode
  depth: number
}

export interface TreemapRect {
  node: StorageUsageNode
  x: number
  y: number
  width: number
  height: number
  depth: number
  ridgeHeight: number
}

export const cushionPalette = [
  '#0000ff', '#ff0000', '#00ff00', '#ffff00', '#00ffff', '#ff00ff',
  '#ffaa00', '#0055ff', '#ff0055', '#55ff00', '#aa00ff', '#00ff55',
  '#ff00aa', '#00aaff', '#ff5500', '#00ffaa', '#5500ff', '#ffffff',
]

export function buildTreeRows(root: StorageUsageNode, expandedPaths: Set<string>): StatisticsTreeRow[] {
  const rows: StatisticsTreeRow[] = []
  const visit = (node: StorageUsageNode, depth: number) => {
    rows.push({ node, depth })
    if (!expandedPaths.has(node.path)) return
    const children = node.children.filter((child) => child.size > 0).sort((a, b) => b.size - a.size)
    for (const child of children) visit(child, depth + 1)
  }
  visit(root, 0)
  return rows
}

export function expandAncestors(path: string, current: Set<string>) {
  const next = new Set(current)
  next.add('')
  const parts = path.split('/').filter(Boolean)
  for (let i = 1; i < parts.length; i += 1) next.add(parts.slice(0, i).join('/'))
  return next
}

export function buildExtensionStats(files: StorageUsageNode[]): ExtensionStat[] {
  const stats = new Map<string, Omit<ExtensionStat, 'colorIndex'>>()
  for (const file of files) {
    const extension = extensionFor(file.name)
    const existing = stats.get(extension) || { extension, size: 0, files: 0 }
    existing.size += file.size
    existing.files += 1
    stats.set(extension, existing)
  }
  return [...stats.values()].sort((a, b) => b.size - a.size).map((stat, colorIndex) => ({ ...stat, colorIndex }))
}

export function buildTreemapRects(root: StorageUsageNode): TreemapRect[] {
  const rects: TreemapRect[] = []
  layoutNode(root, { node: root, x: 0, y: 0, width: 100, height: 100, depth: 0, ridgeHeight: 0.38 }, rects, false)
  return rects.filter((rect) => rect.width > 0 && rect.height > 0)
}

export function buildDirectoryRects(root: StorageUsageNode): TreemapRect[] {
  const rects: TreemapRect[] = []
  layoutDirectories(root, { node: root, x: 0, y: 0, width: 100, height: 100, depth: 0, ridgeHeight: 0.38 }, rects)
  return rects.filter((rect) => rect.width >= 4 && rect.height >= 4).slice(0, 220)
}

function layoutNode(node: StorageUsageNode, rect: TreemapRect, output: TreemapRect[], includeSelf: boolean) {
  if (node.size <= 0 || rect.width <= 0 || rect.height <= 0) return
  if (node.type === 'file') {
    if (includeSelf) output.push({ ...rect, node })
    return
  }

  const children = node.children.filter((child) => child.size > 0).sort((a, b) => b.size - a.size)
  for (const childRect of arrangeKDirStat(children, rect)) layoutNode(childRect.node, childRect, output, true)
}

function layoutDirectories(node: StorageUsageNode, rect: TreemapRect, output: TreemapRect[]) {
  if (node.type !== 'directory' || node.size <= 0) return
  output.push({ ...rect, node })
  const children = node.children.filter((child) => child.size > 0).sort((a, b) => b.size - a.size)
  for (const childRect of arrangeKDirStat(children, rect)) {
    if (childRect.node.type === 'directory') layoutDirectories(childRect.node, childRect, output)
  }
}

function arrangeKDirStat(children: StorageUsageNode[], rect: TreemapRect): TreemapRect[] {
  const total = children.reduce((sum, child) => sum + child.size, 0)
  if (!total) return []

  const horizontalRows = rect.width >= rect.height
  const widthRatio = horizontalRows ? rect.width / Math.max(rect.height, 0.001) : rect.height / Math.max(rect.width, 0.001)
  const rows: Array<{ height: number; count: number }> = []
  const childWidths = new Array(children.length).fill(0)

  for (let nextChild = 0; nextChild < children.length;) {
    const row = calculateKDirStatRow(children, nextChild, Math.max(1, widthRatio), total, childWidths)
    rows.push({ height: row.height, count: row.childrenUsed })
    nextChild += row.childrenUsed
  }

  const output: TreemapRect[] = []
  let top = horizontalRows ? rect.y : rect.x
  let childIndex = 0
  rows.forEach((row, rowIndex) => {
    const rowExtent = horizontalRows ? rect.height : rect.width
    const columnExtent = horizontalRows ? rect.width : rect.height
    const fBottom = top + row.height * rowExtent
    const bottom = rowIndex + 1 === rows.length ? (horizontalRows ? rect.y + rect.height : rect.x + rect.width) : fBottom

    let left = horizontalRows ? rect.x : rect.y
    for (let childInRow = 0; childInRow < row.count; childInRow += 1, childIndex += 1) {
      const child = children[childIndex]
      const fRight = left + childWidths[childIndex] * columnExtent
      const lastChild = childInRow + 1 === row.count || childIndex + 1 === childWidths.length || childWidths[childIndex + 1] === 0
      const right = lastChild ? (horizontalRows ? rect.x + rect.width : rect.y + rect.height) : fRight

      output.push(horizontalRows ? {
        node: child, x: left, y: top, width: Math.max(0, right - left), height: Math.max(0, bottom - top), depth: rect.depth + 1, ridgeHeight: rect.ridgeHeight * 0.91,
      } : {
        node: child, x: top, y: left, width: Math.max(0, bottom - top), height: Math.max(0, right - left), depth: rect.depth + 1, ridgeHeight: rect.ridgeHeight * 0.91,
      })
      left = fRight
    }
    top = fBottom
  })
  return output
}

function calculateKDirStatRow(children: StorageUsageNode[], nextChild: number, widthRatio: number, total: number, childWidths: number[]) {
  const minProportion = 0.4
  let sizeUsed = 0
  let rowHeight = 0
  let index = nextChild

  for (; index < children.length; index += 1) {
    const childSize = children[index].size
    if (childSize === 0) break
    sizeUsed += childSize
    const virtualRowHeight = sizeUsed / total
    const childWidth = (childSize / total) * widthRatio / virtualRowHeight
    if (index > nextChild && childWidth / virtualRowHeight < minProportion) {
      sizeUsed -= childSize
      break
    }
    rowHeight = virtualRowHeight
  }

  const childrenUsed = Math.max(1, index - nextChild)
  const rowSize = total * rowHeight
  for (let i = 0; i < childrenUsed; i += 1) childWidths[nextChild + i] = rowSize ? children[nextChild + i].size / rowSize : 0
  return { height: rowHeight || 1, childrenUsed }
}

export function rectStyle(rect: TreemapRect) {
  return { left: `${rect.x}%`, top: `${rect.y}%`, width: `${rect.width}%`, height: `${rect.height}%` }
}

export function visiblePercent(size: number, total: number) {
  if (!total || !size) return '0%'
  const percent = (size / total) * 100
  return percent < 0.1 ? '<0.1%' : `${percent.toFixed(1)}%`
}

export function usageBarStyle(size: number, total: number) {
  if (!total || !size) return { width: '0%' }
  const percent = (size / total) * 100
  return { width: `${Math.max(percent, 0.8)}%` }
}

export function fileRectStyle(rect: TreemapRect) {
  return {
    ...rectStyle(rect),
    minWidth: rect.width < 0.35 ? '2px' : undefined,
    minHeight: rect.height < 0.35 ? '2px' : undefined,
  }
}

export function cushionPreview(hex: string) {
  return `radial-gradient(ellipse at 30% 22%, ${adjustColor(hex, 0.55)} 0%, ${makeBrightColor(hex, 0.6)} 45%, ${adjustColor(hex, -0.3)} 100%)`
}

export function cushionStyle(hex: string, rect: TreemapRect) {
  return { background: `radial-gradient(ellipse at 30% 22%, ${adjustColor(hex, 0.55 + rect.ridgeHeight * 0.35)} 0%, ${makeBrightColor(hex, 0.6)} 38%, ${adjustColor(hex, -0.28 - rect.depth * 0.018)} 100%)` }
}

function makeBrightColor(hex: string, brightness: number) {
  const rgb = hexToRgb(hex)
  const sum = rgb.r + rgb.g + rgb.b || 1
  const factor = (3 * 255 * brightness) / sum
  return rgbToCss(normalizeRgb(rgb.r * factor, rgb.g * factor, rgb.b * factor))
}

function adjustColor(hex: string, amount: number) {
  const rgb = hexToRgb(hex)
  const target = amount >= 0 ? 255 : 0
  const factor = Math.abs(amount)
  return rgbToCss({ r: rgb.r + (target - rgb.r) * factor, g: rgb.g + (target - rgb.g) * factor, b: rgb.b + (target - rgb.b) * factor })
}

function hexToRgb(hex: string) {
  const value = Number.parseInt(hex.slice(1), 16)
  return { r: (value >> 16) & 255, g: (value >> 8) & 255, b: value & 255 }
}

function normalizeRgb(r: number, g: number, b: number) {
  const max = Math.max(r, g, b)
  if (max <= 255) return { r, g, b }
  const scale = 255 / max
  return { r: r * scale, g: g * scale, b: b * scale }
}

function rgbToCss({ r, g, b }: { r: number; g: number; b: number }) {
  return `rgb(${Math.round(r)}, ${Math.round(g)}, ${Math.round(b)})`
}

export function extensionFor(name: string) {
  const index = name.lastIndexOf('.')
  if (index <= 0 || index === name.length - 1) return '[none]'
  return name.slice(index).toLowerCase()
}

export function extensionDescription(extension: string) {
  if (extension === '[none]') return 'Files without extension'
  return `${extension.slice(1).toUpperCase()} file`
}

export function isDescendantPath(path: string, directoryPath: string) {
  return path === directoryPath || path.startsWith(`${directoryPath}/`)
}

export function parentDirectory(node: StorageUsageNode): StorageUsageNode {
  const parentPath = node.path.split('/').slice(0, -1).join('/')
  return { name: parentPath || 'Root', path: parentPath, type: 'directory', size: node.size, children: [] }
}
