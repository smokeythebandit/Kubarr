import { QueryClient, QueryClientProvider, useQuery } from '@tanstack/react-query'
import { render, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { monitoringApi } from '../api/monitoring'
import { appsApi } from '../api/apps'

function client() {
  return new QueryClient({ defaultOptions: { queries: { retry: false } } })
}

describe('dashboard data fetching', () => {
  it('asks for every namespace in one request', async () => {
    const batched = vi.spyOn(monitoringApi, 'getPodStatusFor').mockResolvedValue([])

    function Panel() {
      useQuery({
        queryKey: ['pods', 'sonarr,radarr'],
        queryFn: () => monitoringApi.getPodStatusFor(['sonarr', 'radarr']),
      })
      return null
    }

    // Two panels wanting the same data must produce one request, not two.
    render(
      <QueryClientProvider client={client()}>
        <Panel />
        <Panel />
      </QueryClientProvider>,
    )

    await waitFor(() => expect(batched).toHaveBeenCalledTimes(1))
    expect(batched).toHaveBeenCalledWith(['sonarr', 'radarr'])
  })

  it('fetches every catalog icon in one request', async () => {
    const bundle = vi.spyOn(appsApi, 'getIcons').mockResolvedValue({
      sonarr: '<svg id="sonarr"/>',
      radarr: '<svg id="radarr"/>',
    })

    const icons = await appsApi.getIcons()

    expect(bundle).toHaveBeenCalledTimes(1)
    expect(Object.keys(icons)).toEqual(['sonarr', 'radarr'])
  })
})
