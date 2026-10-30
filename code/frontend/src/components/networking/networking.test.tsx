import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import {
  AppClassRates,
  ClassTotals,
  FlowSeries,
  NetworkStats,
  formatFlowRate,
  totalByteRate,
  totalFlowRate,
} from '../../api/networking'
import { FlowClassCards } from './FlowClassCards'
import { FlowClassChart } from './FlowClassChart'
import { FlowSplitBar } from './FlowSplitBar'
import { NetworkStatsTable } from './NetworkStatsTable'

type ClassName = 'internet' | 'lan' | 'cluster' | 'storage'

/**
 * Bytes are what the page leads with, so they are what these helpers set. A
 * flow count is a separate, optional measure: storage has none at all.
 */
function totals(
  bytes: Partial<Record<ClassName, number>>,
  flows: Partial<Record<ClassName, number>> = {},
  drops: Partial<Record<ClassName, number>> = {}
): ClassTotals {
  const rates = (name: ClassName) => ({
    bytes_per_sec: bytes[name] ?? 0,
    flows_per_sec: flows[name] ?? 0,
    drops_per_sec: drops[name] ?? 0,
  })
  return {
    internet: rates('internet'),
    lan: rates('lan'),
    cluster: rates('cluster'),
    storage: rates('storage'),
  }
}

function stat(overrides: Partial<NetworkStats> & { namespace: string }): NetworkStats {
  return {
    app_name: overrides.namespace,
    rx_bytes_per_sec: 0,
    tx_bytes_per_sec: 0,
    rx_packets_per_sec: 0,
    tx_packets_per_sec: 0,
    rx_errors_per_sec: 0,
    tx_errors_per_sec: 0,
    rx_dropped_per_sec: 0,
    tx_dropped_per_sec: 0,
    pod_count: 1,
    ...overrides,
  }
}

function appRates(app: string, internet: number, lan: number, cluster: number): AppClassRates {
  return { app, ...totals({}, { internet, lan, cluster }) }
}

describe('formatFlowRate', () => {
  it('keeps small rates readable instead of rounding them to zero', () => {
    // Measured on the cluster: LAN sat at 0.40/s while internet ran at 3.78/s.
    // Rounding that to "0" would report no LAN traffic at all.
    expect(formatFlowRate(0.4)).toBe('0.40/s')
    expect(formatFlowRate(0.004)).toBe('0.00/s')
    expect(formatFlowRate(0)).toBe('0/s')
  })

  it('loses precision only once the number is large enough not to need it', () => {
    expect(formatFlowRate(3.783)).toBe('3.78/s')
    expect(formatFlowRate(61.417)).toBe('61.4/s')
    expect(formatFlowRate(1500)).toBe('1.50k/s')
  })
})

describe('totalFlowRate', () => {
  it('sums all three classes', () => {
    // Storage carries no connection count, so it must not be summed in here.
    expect(
      totalFlowRate(totals({}, { internet: 2.85, lan: 3.43, cluster: 61.42, storage: 0 }))
    ).toBeCloseTo(67.7, 5)
    // Storage is excluded on purpose. It is the same bytes one step later --
    // a download arrives from the internet and is then written to NFS -- so
    // adding it reports more traffic than actually moved. Measured on a live
    // cluster, including it turned a ~330 Mbps download into 697 Mbps.
    expect(
      totalByteRate(totals({ internet: 100, lan: 200, cluster: 300, storage: 400 }))
    ).toBe(600)
  })
})

describe('FlowSplitBar', () => {
  it('gives a real but tiny class a visible sliver', () => {
    const { container } = render(
      <FlowSplitBar totals={totals({ internet: 100, lan: 0.4, cluster: 100 })} />
    )

    const segments = container.querySelectorAll('[title]')
    const lan = [...segments].find(node => node.getAttribute('title')?.startsWith('LAN'))
    expect(lan).toBeTruthy()
    // 0.4 against 200 is 0.2% of the total; drawn at that width it would be
    // invisible, and "no LAN traffic" is a different answer from "a little".
    const width = Number.parseFloat((lan as HTMLElement).style.width)
    expect(width).toBeGreaterThan(0.5)
  })

  it('draws nothing for a class with no traffic', () => {
    const { container } = render(
      <FlowSplitBar totals={totals({ internet: 10, lan: 0, cluster: 10 })} />
    )

    const titles = [...container.querySelectorAll('[title]')].map(node =>
      node.getAttribute('title')
    )
    expect(titles.some(title => title?.startsWith('LAN'))).toBe(false)
    expect(titles.some(title => title?.startsWith('Internet'))).toBe(true)
  })

  it('reports every class in its accessible label, including the silent one', () => {
    render(
      <FlowSplitBar
        totals={totals({ internet: 10240, lan: 0, cluster: 10240, storage: 99999 })}
      />
    )

    // Three destinations, and storage is not one of them however busy it is.
    expect(screen.getByRole('img')).toHaveAttribute(
      'aria-label',
      'Internet 10 KB/s, LAN 0 B/s, In-cluster 10 KB/s'
    )
  })
})

describe('FlowClassCards', () => {
  it('shows each class rate and its share of the total', () => {
    render(
      <FlowClassCards
        totals={totals(
          { internet: 1024, lan: 2048, cluster: 4096, storage: 8192 },
          { internet: 2.85, lan: 3.43, cluster: 61.42 }
        )}
        stale={false}
        totalRx={0}
        totalTx={0}
      />
    )

    expect(screen.getByText('1 KB/s')).toBeInTheDocument()
    expect(screen.getByText('2 KB/s')).toBeInTheDocument()
    expect(screen.getByText('4 KB/s')).toBeInTheDocument()
    expect(screen.getByText('8 KB/s')).toBeInTheDocument()
    // 4096 of the 7168 that went somewhere is 57%. Storage's 8192 is not in
    // the denominator: it is those same bytes being written down.
    expect(screen.getByText('57% of traffic')).toBeInTheDocument()
    // Connection counts stay visible, but only where one exists.
    expect(screen.getByText('61.4/s conn')).toBeInTheDocument()
    // Storage says what it is instead of claiming a share.
    expect(screen.getByText('read + written')).toBeInTheDocument()
  })

  it('says there is no data rather than drawing a convincing zero', () => {
    render(<FlowClassCards totals={totals({})} stale totalRx={1024} totalTx={0} />)

    expect(screen.getAllByText('No data')).toHaveLength(4)
    expect(screen.queryByText('0/s')).not.toBeInTheDocument()
    // Byte volumes come from cAdvisor and are unaffected by Hubble being down.
    expect(screen.getByText('1 KB/s')).toBeInTheDocument()
  })

  it('surfaces denied flows next to the class that was denied', () => {
    render(
      <FlowClassCards
        totals={totals({ internet: 5120 }, { internet: 5 }, { internet: 1.25 })}
        stale={false}
        totalRx={0}
        totalTx={0}
      />
    )

    expect(screen.getByText('1.25/s denied')).toBeInTheDocument()
  })
})

describe('FlowClassChart', () => {
  const series = (points: Array<[number, number]>) =>
    points.map(([t, bytes_per_sec]) => ({ t, flows_per_sec: 0, bytes_per_sec }))

  it('treats a class missing from a bucket as zero rather than dropping the column', () => {
    // internet reports at every step; lan only reports once. Stacking needs all
    // three classes at every timestamp, so the gaps become real zeros.
    const flowSeries: FlowSeries = {
      internet: series([
        [100, 1],
        [160, 2],
        [220, 3],
      ]),
      lan: series([[160, 5]]),
      cluster: [],
    }

    const { container } = render(<FlowClassChart series={flowSeries} />)

    // Three timestamps survive as three columns, so two band outlines each span
    // the full plot rather than collapsing to the single lan sample.
    const paths = container.querySelectorAll('path')
    expect(paths.length).toBeGreaterThan(0)
    expect(screen.queryByText('Not enough history yet for this window')).not.toBeInTheDocument()
  })

  it('refuses to draw a trend from a single point', () => {
    render(
      <FlowClassChart
        series={{ internet: series([[100, 1]]), lan: [], cluster: [] }}
      />
    )

    expect(screen.getByText('Not enough history yet for this window')).toBeInTheDocument()
  })

  it('ignores non-finite samples', () => {
    const flowSeries: FlowSeries = {
      internet: series([
        [100, 1],
        [160, Number.NaN],
        [220, 3],
      ]),
      lan: [],
      cluster: [],
    }

    expect(() => render(<FlowClassChart series={flowSeries} />)).not.toThrow()
    expect(screen.queryByText('NaN/s')).not.toBeInTheDocument()
  })
})

describe('NetworkStatsTable', () => {
  it('keeps a flow source that has no pod behind it', () => {
    // Hubble attributes LAN traffic to the node, because the kubelet performs
    // the NFS mounts. cAdvisor has no pod for that, so an inner join would hide
    // where all the LAN traffic comes from.
    render(
      <NetworkStatsTable
        stats={[stat({ namespace: 'qbittorrent', rx_bytes_per_sec: 2048 })]}
        byApp={[appRates('qbittorrent', 5.65, 0, 1), appRates('host', 0, 3.43, 0)]}
        stale={false}
      />
    )

    expect(screen.getByText('qbittorrent')).toBeInTheDocument()
    expect(screen.getByText('host')).toBeInTheDocument()
    expect(screen.getByText('5.65/s')).toBeInTheDocument()
    expect(screen.getByText('3.43/s')).toBeInTheDocument()
  })

  it('shows an app with byte volume but no measured flows as zero, not missing', () => {
    render(
      <NetworkStatsTable
        stats={[stat({ namespace: 'jellyfin', rx_bytes_per_sec: 1024 })]}
        byApp={[]}
        stale={false}
      />
    )

    expect(screen.getByText('jellyfin')).toBeInTheDocument()
    expect(screen.getAllByText('0')).toHaveLength(3)
  })

  it('blanks the class columns when Hubble has nothing to say', () => {
    render(
      <NetworkStatsTable
        stats={[stat({ namespace: 'jellyfin', rx_bytes_per_sec: 1024 })]}
        byApp={[]}
        stale
      />
    )

    expect(screen.getByText('1 KB/s')).toBeInTheDocument()
    expect(screen.getAllByText('—')).toHaveLength(3)
  })

  it('flags an app whose flows the network policy denied', () => {
    render(
      <NetworkStatsTable
        stats={[stat({ namespace: 'sonarr' })]}
        byApp={[{ app: 'sonarr', ...totals({}, { internet: 1 }, { internet: 0.5 }) }]}
        stale={false}
      />
    )

    expect(screen.getByText('Denied')).toBeInTheDocument()
  })

  it('sorts the busiest app first', () => {
    render(
      <NetworkStatsTable
        stats={[
          stat({ namespace: 'quiet', rx_bytes_per_sec: 1 }),
          stat({ namespace: 'busy', rx_bytes_per_sec: 10_000 }),
        ]}
        byApp={[]}
        stale={false}
      />
    )

    const names = screen.getAllByRole('row').slice(1).map(row => row.textContent ?? '')
    expect(names[0]).toContain('busy')
  })
})
