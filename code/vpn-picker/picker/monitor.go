package picker

import (
	"context"
	"log"
	"time"

	"github.com/smokeythebandit/kubarr-vpn-picker/servers"
)

// Monitor watches the current server and re-runs the round on sustained degradation.
type Monitor struct {
	Sw        Switcher
	M         Meter
	T         Tester
	Cfg       Config
	Short     []servers.Server
	St        *Status
	Baseline  float64 // bytes/s of the last round's winner
	LastRound time.Time

	degraded int
}

// Startup waits for the tunnel, runs the speed round (unless a recent result
// exists) and returns the monitor. It returns (nil, nil) when there is no
// shortlist, in which case gluetun is left untouched.
func Startup(ctx context.Context, sw Switcher, m Meter, t Tester, cfg Config) (*Monitor, error) {
	cfg = cfg.withDefaults()
	short, err := ReadShortlist(cfg.StateDir)
	if err != nil {
		return nil, err
	}
	if len(short) == 0 {
		log.Printf("no shortlist; leaving gluetun's server alone")
		return nil, nil
	}
	mon := &Monitor{Sw: sw, M: m, T: t, Cfg: cfg, Short: short, St: &Status{}}
	if err := sw.WaitRunning(ctx, 5*time.Minute); err != nil {
		return nil, err
	}
	if r := ReadResult(cfg.StateDir); r != nil && cfg.Now().Sub(r.At) < cfg.Cooldown {
		log.Printf("recent round (%s) found, skipping speed round", r.At.Format(time.RFC3339))
		mon.Baseline = r.Mbps * 1e6 / 8
		mon.LastRound = r.At
		mon.St.update(func(d *statusData) { d.Current, d.LastRound = r.Best, r.At })
		return mon, nil
	}
	mon.round(ctx)
	return mon, nil
}

func (m *Monitor) round(ctx context.Context) {
	best, results, err := Round(ctx, m.Sw, m.Short, m.M, m.T, m.Cfg)
	now := m.Cfg.Now()
	m.LastRound = now // also on failure, so we never hammer
	m.degraded = 0
	m.St.update(func(d *statusData) { d.Results, d.LastRound, d.Current, d.Degraded = results, now, best.Hostname, 0 })
	if err != nil {
		log.Printf("round: %v", err)
		return
	}
	for _, r := range results {
		if r.Hostname == best.Hostname {
			m.Baseline = r.BytesPerSec
		}
	}
	if err := WriteResult(m.Cfg.StateDir, ResultFile{Best: best.Hostname, Mbps: m.Baseline * 8 / 1e6, At: now}); err != nil {
		log.Printf("write result: %v", err)
	}
	log.Printf("round: best %s at %.1f Mbit/s", best.Hostname, m.Baseline*8/1e6)
}

// Step performs one periodic check.
func (m *Monitor) Step(ctx context.Context) {
	cfg := m.Cfg.withDefaults()
	rate, err := measure(cfg, m.M, func() error { return m.T.Run(ctx, cfg.TestDuration) })
	if err != nil {
		log.Printf("check: %v", err)
		rate = 0
	}
	now := cfg.Now()
	if rate >= cfg.DegradeThreshold*m.Baseline {
		m.degraded = 0
	} else {
		m.degraded++
	}
	m.St.update(func(d *statusData) {
		d.LastCheck, d.LastMbps, d.Degraded = now, rate*8/1e6, m.degraded
		d.NextCheck = now.Add(cfg.Interval)
	})
	if m.degraded < cfg.DegradeChecks {
		return
	}
	if now.Sub(m.LastRound) < cfg.Cooldown {
		return
	}
	if !cfg.SwitchWhileRunning {
		idle, err := m.idle(ctx, cfg)
		if err != nil || !idle {
			return
		}
	}
	log.Printf("check: %d degraded checks (%.1f Mbit/s vs %.1f), re-running round", m.degraded, rate*8/1e6, m.Baseline*8/1e6)
	m.round(ctx)
}

// idle reports whether tunnel traffic stays below IdleBytesPerSec over the idle window.
func (m *Monitor) idle(ctx context.Context, cfg Config) (bool, error) {
	rate, err := measure(cfg, m.M, func() error { return cfg.Sleep(ctx, cfg.IdleWindow) })
	return err == nil && rate < IdleBytesPerSec, err
}

// Run checks every Interval until ctx is cancelled.
func (m *Monitor) Run(ctx context.Context) {
	cfg := m.Cfg.withDefaults()
	for {
		if cfg.Sleep(ctx, cfg.Interval) != nil {
			return
		}
		m.Step(ctx)
	}
}
