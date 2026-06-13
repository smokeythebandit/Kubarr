import type { PointerEvent as ReactPointerEvent } from 'react'

export function StatisticsResizeHandle({ value, onChange }: { value: number; onChange: (value: number) => void }) {
  const startDrag = (event: ReactPointerEvent<HTMLButtonElement>) => {
    event.preventDefault()
    const container = event.currentTarget.parentElement
    if (!container) return

    const onMove = (moveEvent: PointerEvent) => {
      const rect = container.getBoundingClientRect()
      const percent = ((moveEvent.clientX - rect.left) / rect.width) * 100
      onChange(Math.min(82, Math.max(35, percent)))
    }

    const stopDrag = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', stopDrag)
    }

    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', stopDrag)
  }

  return (
    <button
      type="button"
      aria-label="Resize directory tree"
      title="Resize directory tree"
      onPointerDown={startDrag}
      className="group relative w-2 shrink-0 cursor-col-resize bg-gray-100 transition hover:bg-blue-100 dark:bg-gray-800 dark:hover:bg-blue-950"
      data-width={value}
    >
      <span className="absolute left-1/2 top-1/2 h-12 w-1 -translate-x-1/2 -translate-y-1/2 rounded-full bg-gray-300 transition group-hover:bg-blue-500 dark:bg-gray-600" />
    </button>
  )
}

export function StatisticsVerticalResizeHandle({ value, onChange }: { value: number; onChange: (value: number) => void }) {
  const startDrag = (event: ReactPointerEvent<HTMLButtonElement>) => {
    event.preventDefault()
    const container = event.currentTarget.parentElement
    if (!container) return

    const onMove = (moveEvent: PointerEvent) => {
      const rect = container.getBoundingClientRect()
      const percent = ((moveEvent.clientY - rect.top) / rect.height) * 100
      onChange(Math.min(65, Math.max(20, percent)))
    }

    const stopDrag = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', stopDrag)
    }

    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', stopDrag)
  }

  return (
    <button
      type="button"
      aria-label="Resize statistics panes"
      title="Resize statistics panes"
      onPointerDown={startDrag}
      className="group relative h-2 w-full cursor-row-resize bg-gray-100 transition hover:bg-blue-100 dark:bg-gray-800 dark:hover:bg-blue-950"
      data-height={value}
    >
      <span className="absolute left-1/2 top-1/2 h-1 w-16 -translate-x-1/2 -translate-y-1/2 rounded-full bg-gray-300 transition group-hover:bg-blue-500 dark:bg-gray-600" />
    </button>
  )
}
