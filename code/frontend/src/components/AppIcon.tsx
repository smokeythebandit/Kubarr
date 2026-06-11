import { useState, useEffect, memo } from 'react'

interface AppIconProps {
  appName: string
  size?: number
  className?: string
}

// Global in-memory cache for icon URLs and their loaded status
const iconCache = new Map<string, { loaded: boolean; dataUrl?: string; error?: boolean; colors?: string[] }>()

// Parse hex color to RGB
function hexToRgb(hex: string): { r: number; g: number; b: number } | null {
  // Handle shorthand hex (#0cf -> #00ccff)
  const shorthandRegex = /^#?([a-f\d])([a-f\d])([a-f\d])$/i
  hex = hex.replace(shorthandRegex, (_, r, g, b) => r + r + g + g + b + b)

  const result = /^#?([a-f\d]{2})([a-f\d]{2})([a-f\d]{2})$/i.exec(hex)
  return result ? {
    r: parseInt(result[1], 16),
    g: parseInt(result[2], 16),
    b: parseInt(result[3], 16)
  } : null
}

// Calculate color "vibrancy" score (prefer saturated, mid-brightness colors)
function getColorScore(r: number, g: number, b: number, allowGrayscale = false): number {
  const brightness = (r + g + b) / 3
  const max = Math.max(r, g, b)
  const min = Math.min(r, g, b)
  const saturation = max === 0 ? 0 : (max - min) / max

  // Penalize very dark or very light colors
  if (brightness < 30 || brightness > 230) return 0

  // For grayscale-only icons, allow low saturation
  if (!allowGrayscale && saturation < 0.1) return 0

  // Prefer saturated colors
  return saturation * 100 + (brightness > 60 && brightness < 200 ? 50 : 0)
}

// Extract multiple colors from SVG content by parsing fill colors
function extractColorsFromSvg(svgText: string): string[] {
  const colors: Array<{ r: number; g: number; b: number; score: number; key: string }> = []

  // Check for embedded base64 image - if found, return empty (will use canvas fallback)
  if (svgText.includes('data:image/png;base64') || svgText.includes('data:image/jpeg;base64')) {
    return []
  }

  // Find all hex colors in fill attributes and style properties
  const hexPattern = /#([0-9a-fA-F]{3}){1,2}\b/g
  const matches = svgText.match(hexPattern) || []

  const seen = new Set<string>()
  for (const hex of matches) {
    const rgb = hexToRgb(hex)
    if (rgb) {
      const key = `${rgb.r},${rgb.g},${rgb.b}`
      if (seen.has(key)) continue
      seen.add(key)

      const score = getColorScore(rgb.r, rgb.g, rgb.b)
      if (score > 0) {
        colors.push({ ...rgb, score, key })
      }
    }
  }

  // Sort by score and return top colors
  colors.sort((a, b) => b.score - a.score)

  // Return up to 3 distinct colors
  return colors.slice(0, 3).map(c => `rgb(${c.r}, ${c.g}, ${c.b})`)
}

// Extract multiple dominant colors from canvas image data
function extractColorsFromCanvas(ctx: CanvasRenderingContext2D, size: number): string[] {
  const imageData = ctx.getImageData(0, 0, size, size)
  const pixels = imageData.data

  const colorCounts = new Map<string, { r: number; g: number; b: number; count: number }>()

  for (let i = 0; i < pixels.length; i += 4) {
    const r = pixels[i]
    const g = pixels[i + 1]
    const b = pixels[i + 2]
    const a = pixels[i + 3]

    if (a < 128) continue

    const score = getColorScore(r, g, b, true)
    if (score === 0) continue

    // Quantize to reduce unique colors
    const qr = Math.round(r / 24) * 24
    const qg = Math.round(g / 24) * 24
    const qb = Math.round(b / 24) * 24
    const key = `${qr},${qg},${qb}`

    const existing = colorCounts.get(key)
    if (existing) {
      existing.count += score
    } else {
      colorCounts.set(key, { r: qr, g: qg, b: qb, count: score })
    }
  }

  // Sort by count and get top colors
  const sorted = Array.from(colorCounts.values()).sort((a, b) => b.count - a.count)

  // Return up to 3 distinct colors (with some color distance)
  const result: string[] = []
  for (const color of sorted) {
    if (result.length >= 3) break

    // Check color distance from existing colors
    const isDifferent = result.every(existing => {
      const [er, eg, eb] = existing.replace('rgb(', '').replace(')', '').split(',').map(Number)
      const distance = Math.sqrt(
        Math.pow(color.r - er, 2) +
        Math.pow(color.g - eg, 2) +
        Math.pow(color.b - eb, 2)
      )
      return distance > 50 // Minimum color distance
    })

    if (isDifferent || result.length === 0) {
      result.push(`rgb(${color.r}, ${color.g}, ${color.b})`)
    }
  }

  return result
}

// Extract multiple dominant colors from an image blob
async function extractDominantColors(blob: Blob): Promise<string[]> {
  const text = await blob.text()

  // Check if it's an SVG without embedded images
  if (text.trim().startsWith('<svg') || text.trim().startsWith('<?xml')) {
    const svgColors = extractColorsFromSvg(text)
    if (svgColors.length > 0) {
      return svgColors
    }
    // If SVG has embedded image or no colors, fall through to canvas
  }

  // For non-SVG images or SVGs with embedded images, use canvas approach
  return new Promise((resolve) => {
    const img = new Image()
    img.crossOrigin = 'anonymous'

    img.onload = () => {
      try {
        const canvas = document.createElement('canvas')
        const ctx = canvas.getContext('2d')
        if (!ctx) {
          resolve([])
          return
        }

        const sampleSize = 64
        canvas.width = sampleSize
        canvas.height = sampleSize
        ctx.drawImage(img, 0, 0, sampleSize, sampleSize)

        resolve(extractColorsFromCanvas(ctx, sampleSize))
      } catch {
        resolve([])
      } finally {
        URL.revokeObjectURL(img.src)
      }
    }

    img.onerror = () => resolve([])

    // Handle SVG with embedded base64 or regular blob
    if (text.includes('data:image')) {
      // Extract base64 image from SVG and load it
      const base64Match = text.match(/data:image\/[^;]+;base64,[^"']+/)
      if (base64Match) {
        img.src = base64Match[0]
        return
      }
    }
    img.src = URL.createObjectURL(blob)
  })
}

// Preload an icon and cache it
export async function preloadIcon(appName: string): Promise<void> {
  const cacheKey = appName.toLowerCase()

  // Skip if already cached
  if (iconCache.has(cacheKey)) return

  // Mark as loading
  iconCache.set(cacheKey, { loaded: false })

  try {
    const iconUrl = `/api/apps/catalog/${cacheKey}/icon`
    const response = await fetch(iconUrl)

    if (!response.ok) {
      iconCache.set(cacheKey, { loaded: true, error: true })
      return
    }

    const blob = await response.blob()

    // Clone blob for color extraction (since blob.text() consumes it)
    const blobForColor = blob.slice()

    const dataUrl = URL.createObjectURL(blob)
    const colors = await extractDominantColors(blobForColor)

    iconCache.set(cacheKey, { loaded: true, dataUrl, colors })
  } catch {
    iconCache.set(cacheKey, { loaded: true, error: true })
  }
}

// Preload multiple icons in parallel
export async function preloadIcons(appNames: string[]): Promise<void> {
  await Promise.all(appNames.map(preloadIcon))
}

// Get cached icon URL
function getCachedIcon(appName: string): { url: string | null; loading: boolean; error: boolean } {
  const cacheKey = appName.toLowerCase()
  const cached = iconCache.get(cacheKey)

  if (!cached) {
    // Not in cache, start loading
    preloadIcon(appName)
    return { url: null, loading: true, error: false }
  }

  if (!cached.loaded) {
    return { url: null, loading: true, error: false }
  }

  if (cached.error) {
    return { url: null, loading: false, error: true }
  }

  return { url: cached.dataUrl || null, loading: false, error: false }
}

// Get cached dominant colors for an app icon
export function getIconColors(appName: string): string[] {
  const cacheKey = appName.toLowerCase()
  const cached = iconCache.get(cacheKey)
  return cached?.colors || []
}

// Hook to get dominant colors with automatic re-render when available
export function useIconColors(appName: string): string[] {
  const [colors, setColors] = useState<string[]>(() => {
    const cached = iconCache.get(appName.toLowerCase())
    return cached?.colors || []
  })

  useEffect(() => {
    const cacheKey = appName.toLowerCase()

    const checkColors = () => {
      const cached = iconCache.get(cacheKey)
      if (cached?.loaded && cached.colors && cached.colors.length > 0) {
        setColors(cached.colors)
        return true
      }
      return false
    }

    // Check immediately
    if (checkColors()) return

    // Poll until colors are available
    const interval = setInterval(() => {
      if (checkColors()) {
        clearInterval(interval)
      }
    }, 100)

    return () => clearInterval(interval)
  }, [appName])

  return colors
}

function AppIconComponent({ appName, size = 40, className = '' }: AppIconProps) {
  const [, setRenderTrigger] = useState(0)
  const { url, loading, error } = getCachedIcon(appName)

  // If loading, set up a timer to re-check
  useEffect(() => {
    if (loading) {
      const checkLoaded = setInterval(() => {
        const cached = iconCache.get(appName.toLowerCase())
        if (cached?.loaded) {
          setRenderTrigger(prev => prev + 1)
          clearInterval(checkLoaded)
        }
      }, 50)

      return () => clearInterval(checkLoaded)
    }
  }, [loading, appName])

  // Show fallback if error
  if (error) {
    return (
      <div
        className={`flex items-center justify-center bg-gray-600 rounded-lg text-white font-bold ${className}`}
        style={{ width: size, height: size, fontSize: size * 0.5 }}
      >
        {appName.charAt(0).toUpperCase()}
      </div>
    )
  }

  // Show loading placeholder
  if (loading || !url) {
    return (
      <div
        className={`bg-gray-700 rounded-lg animate-pulse ${className}`}
        style={{ width: size, height: size }}
      />
    )
  }

  return (
    <img
      src={url}
      alt={`${appName} icon`}
      width={size}
      height={size}
      className={`rounded-lg ${className}`}
      loading="eager"
      decoding="async"
    />
  )
}

// Memoize the component to prevent unnecessary re-renders
export const AppIcon = memo(AppIconComponent)

export default AppIcon
