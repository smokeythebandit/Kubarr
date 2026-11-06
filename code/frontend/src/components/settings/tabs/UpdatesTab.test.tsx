import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { UpdatesTab } from './UpdatesTab'

const mocks = vi.hoisted(() => ({ getAutoUpdate: vi.fn(), setAutoUpdate: vi.fn() }))

vi.mock('../../../api/apps', () => ({
  appsApi: {
    getAutoUpdate: (...args: unknown[]) => mocks.getAutoUpdate(...args),
    setAutoUpdate: (...args: unknown[]) => mocks.setAutoUpdate(...args),
  },
}))

const saved = {
  config: {
    enabled: true,
    allow_major: false,
    window: { days: ['sat', 'sun'], start: '22:00', end: '02:00', timezone: 'Europe/Amsterdam' },
  },
  in_window: false,
  next_window_start: '2026-11-07T21:00:00Z',
}

function renderTab() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } })
  return render(<QueryClientProvider client={client}><UpdatesTab /></QueryClientProvider>)
}

describe('UpdatesTab', () => {
  beforeEach(() => {
    mocks.getAutoUpdate.mockResolvedValue(saved)
    mocks.setAutoUpdate.mockReset()
  })

  it('shows the saved window in words, overnight included', async () => {
    renderTab()
    expect(await screen.findByText('Sat, Sun, 22:00–02:00 (overnight), Europe/Amsterdam')).toBeInTheDocument()
    expect(screen.getByRole('switch', { name: 'Automatic updates' })).toHaveAttribute('aria-checked', 'true')
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled()
  })

  it('saves an edited window', async () => {
    mocks.setAutoUpdate.mockImplementation(async (config: unknown) => ({ ...saved, config }))
    renderTab()

    fireEvent.click(await screen.findByRole('button', { name: 'Mon' }))
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))

    await waitFor(() => expect(mocks.setAutoUpdate).toHaveBeenCalledTimes(1))
    expect(mocks.setAutoUpdate.mock.calls[0][0].window.days).toEqual(['sat', 'sun', 'mon'])
    expect(await screen.findByText('Saved')).toBeInTheDocument()
  })

  it('refuses to save a window with no days', async () => {
    renderTab()
    fireEvent.click(await screen.findByRole('button', { name: 'Sat' }))
    fireEvent.click(screen.getByRole('button', { name: 'Sun' }))

    expect(screen.getByText('Pick at least one day.')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled()
  })

  it('shows the server error when a save is rejected', async () => {
    mocks.setAutoUpdate.mockRejectedValue({ response: { data: { detail: "Unknown timezone 'Mars'" } }, message: 'Bad Request' })
    renderTab()
    fireEvent.click(await screen.findByRole('button', { name: 'Mon' }))
    fireEvent.click(screen.getByRole('button', { name: 'Save' }))
    expect(await screen.findByText("Unknown timezone 'Mars'")).toBeInTheDocument()
  })
})
