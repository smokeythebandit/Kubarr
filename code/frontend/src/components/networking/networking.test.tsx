import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import {
  AppClassRates,
  ClassTotals,
  FlowSeries,
  NetworkStats,
  formatFlowRate,
  totalFlowRate,
} from '../../api/networking'
import { FlowClassCards } from './FlowClassCards'
import { FlowClassChart } from './FlowClassChart'
import { FlowSplitBar } from './FlowSplitBar'
import { NetworkStatsTable } from './NetworkStatsTable'

function totals(
  internet: number,
  lan: number,
  cluster: number,
  drops: Partial<Record<'internet' | 'lan' | 'cluster', number>> = {}
): ClassTotals {
  return {
    internet: { flows_per_sec: internet, drops_per_sec: drops.internet ?? 0 },
    lan: { flows_per_sec: lan, drops_per_sec: drops.lan ?? 0 },
    cluster: { flows_per_sec: cluster, drops_per_sec: drops.cluster ?? 0 },
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
  return { app, ...totals(internet, lan, cluster) }
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
    expect(totalFlowRate(totals(2.85, 3.43, 61.42))).toBeCloseTo(67.7, 5)
  })
})

describe('FlowSplitBar', () => {
  it('gives a real but tiny class a visible sliver', () => {
    const { container } = render(<FlowSplitBar totals={totals(100, 0.4, 100)} />)

    const segments = container.querySelectorAll('[title]')
    const lan = [...segments].find(node => node.getAttribute('title')?.startsWith('LAN'))
    expect(lan).toBeTruthy()
    // 0.4 against 200 is 0.2% of the total; drawn at that width it would be
    // invisible, and "no LAN traffic" is a different answer from "a little".
    const width = Number.parseFloat((lan as HTMLElement).style.width)
    expect(width).toBeGreaterThan(0.5)
  })

  it('draws nothing for a class with no traffic', () => {
    const { container } = render(<FlowSplitBar totals={totals(10, 0, 10)} />)

    const titles = [...container.querySelectorAll('[title]')].map(node =>
      node.getAttribute('title')
    )
    expect(titles.some(title => title?.startsWith('LAN'))).toBe(false)
    expect(titles.some(title => title?.startsWith('Internet'))).toBe(true)
  })

  it('reports every class in its accessible label, including the silent one', () => {
    render(<FlowSplitBar totals={totals(10, 0, 10)} />)

    expect(screen.getByRole('img')).toHaveAttribute(
      'aria-label',
      'Internet 10.0/s, LAN 0/s, In-cluster 10.0/s'
    )
  })
})

describe('FlowClassCards', () => {
  it('shows each class rate and its share of the total', () => {
    render(
      <FlowClassCards totals={totals(2.85, 3.43, 61.42)} stale={false} totalRx={0} totalTx={0} />
    )

    expect(screen.getByText('2.85/s')).toBeInTheDocument()
    expect(screen.getByText('3.43/s')).toBeInTheDocument()
    expect(screen.getByText('61.4/s')).toBeInTheDocument()
    // 61.42 of 67.7 is 91%.
    expect(screen.getByText('91% of flows')).toBeInTheDocument()
  })

  it('says there is no data rather than drawing a convincing zero', () => {
    render(<FlowClassCards totals={totals(0, 0, 0)} stale totalRx={1024} totalTx={0} />)

    expect(screen.getAllByText('No data')).toHaveLength(3)
    expect(screen.queryByText('0/s')).not.toBeInTheDocument()
    // Byte volumes come from cAdvisor and are unaffected by Hubble being down.
    expect(screen.getByText('1 KB/s')).toBeInTheDocument()
  })

  it('surfaces denied flows next to the class that was denied', () => {
    render(
      <FlowClassCards
        totals={totals(5, 0, 0, { internet: 1.25 })}
        stale={false}
        totalRx={0}
        totalTx={0}
      />
    )

    expect(screen.getByText('1.25/s dropped')).toBeInTheDocument()
  })
})

describe('FlowClassChart', () => {
  const series = (points: Array<[number, number]>) =>
    points.map(([t, flows_per_sec]) => ({ t, flows_per_sec }))

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
        byApp={[{ app: 'sonarr', ...totals(1, 0, 0, { internet: 0.5 }) }]}
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
