// Package picker runs the gluetun speed round and the degradation monitor.
package picker

import (
	"context"
	"errors"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"strconv"
	"strings"
	"time"

	"github.com/smokeythebandit/kubarr-vpn-picker/servers"
)

// IdleBytesPerSec is the tunnel traffic below which the app counts as idle.
const IdleBytesPerSec = 1e6

const maxDownload = 100 << 20

// Switcher controls which server gluetun is connected to.
type Switcher interface {
	Switch(ctx context.Context, hostname string) error
	WaitRunning(ctx context.Context, timeout time.Duration) error
}

// Meter reads a monotonically increasing byte counter of the tunnel.
type Meter interface{ Bytes() (uint64, error) }

// TunMeter reads the tunnel interface's rx byte counter.
type TunMeter struct{ Path string }

func (m TunMeter) Bytes() (uint64, error) {
	b, err := os.ReadFile(m.Path)
	if err != nil {
		return 0, err
	}
	return strconv.ParseUint(strings.TrimSpace(string(b)), 10, 64)
}

// Tester generates download traffic for up to d.
type Tester interface {
	Run(ctx context.Context, d time.Duration) error
}

// HTTPTester downloads from URLs (in order, next one on error) until d or 100 MB.
type HTTPTester struct {
	URLs   []string
	Client *http.Client
}

func (h HTTPTester) Run(ctx context.Context, d time.Duration) error {
	ctx, cancel := context.WithTimeout(ctx, d)
	defer cancel()
	client := h.Client
	if client == nil {
		client = &http.Client{}
	}
	var last error = errors.New("no test endpoints configured")
	for _, u := range h.URLs {
		err := func() error {
			req, err := http.NewRequestWithContext(ctx, http.MethodGet, u, nil)
			if err != nil {
				return err
			}
			resp, err := client.Do(req)
			if err != nil {
				return err
			}
			defer resp.Body.Close()
			if resp.StatusCode/100 != 2 {
				return fmt.Errorf("%s: %s", u, resp.Status)
			}
			_, err = io.Copy(io.Discard, io.LimitReader(resp.Body, maxDownload))
			return err
		}()
		if err == nil || ctx.Err() != nil {
			// The deadline ending the download is the normal outcome.
			return nil
		}
		last = err
	}
	return last
}

// Config holds the round and monitor tunables.
type Config struct {
	StateDir           string
	TestDuration       time.Duration
	RunTimeout         time.Duration // wait for gluetun "running" after a switch (default 30 s)
	IdleWindow         time.Duration // default TestDuration
	Interval           time.Duration
	Cooldown           time.Duration
	DegradeThreshold   float64
	DegradeChecks      int
	SwitchWhileRunning bool
	Now                func() time.Time
	Sleep              func(ctx context.Context, d time.Duration) error
}

func (c Config) withDefaults() Config {
	if c.RunTimeout <= 0 {
		c.RunTimeout = 30 * time.Second
	}
	if c.IdleWindow <= 0 {
		c.IdleWindow = c.TestDuration
	}
	if c.Now == nil {
		c.Now = time.Now
	}
	if c.Sleep == nil {
		c.Sleep = func(ctx context.Context, d time.Duration) error {
			select {
			case <-ctx.Done():
				return ctx.Err()
			case <-time.After(d):
				return nil
			}
		}
	}
	return c
}

// Result is one candidate's outcome in a round.
type Result struct {
	Hostname    string  `json:"hostname"`
	BytesPerSec float64 `json:"bytes_per_sec"`
	Err         string  `json:"error,omitempty"`
}

// measure returns tunnel bytes per second across run().
func measure(cfg Config, m Meter, run func() error) (float64, error) {
	before, err := m.Bytes()
	if err != nil {
		return 0, err
	}
	t0 := cfg.Now()
	runErr := run()
	after, err := m.Bytes()
	if err != nil {
		return 0, err
	}
	if runErr != nil {
		return 0, runErr
	}
	dt := cfg.Now().Sub(t0).Seconds()
	if dt <= 0 || after < before {
		return 0, errors.New("invalid measurement window")
	}
	return float64(after-before) / dt, nil
}

// Round tests every shortlisted server and leaves gluetun on the fastest one.
// Candidates that fail to switch, never reach "running" or fail the download
// are skipped. If none works, it switches back to short[0] and returns an error.
func Round(ctx context.Context, c Switcher, short []servers.Server, m Meter, t Tester, cfg Config) (servers.Server, []Result, error) {
	cfg = cfg.withDefaults()
	if len(short) == 0 {
		return servers.Server{}, nil, errors.New("empty shortlist")
	}
	var results []Result
	bestIdx := -1
	for _, s := range short {
		if ctx.Err() != nil {
			break
		}
		r := Result{Hostname: s.Hostname}
		if err := c.Switch(ctx, s.Hostname); err != nil {
			r.Err = err.Error()
		} else if err := c.WaitRunning(ctx, cfg.RunTimeout); err != nil {
			r.Err = err.Error()
		} else if bps, err := measure(cfg, m, func() error { return t.Run(ctx, cfg.TestDuration) }); err != nil {
			r.Err = err.Error()
		} else {
			r.BytesPerSec = bps
			if bestIdx < 0 || bps > results[bestIdx].BytesPerSec {
				bestIdx = len(results)
			}
		}
		log.Printf("round: %s -> %.1f Mbit/s %s", s.Hostname, r.BytesPerSec*8/1e6, r.Err)
		results = append(results, r)
	}
	if bestIdx < 0 {
		if err := c.Switch(ctx, short[0].Hostname); err != nil {
			return short[0], results, fmt.Errorf("no candidate worked and switching back failed: %w", err)
		}
		return short[0], results, errors.New("no candidate worked; returned to first shortlist entry")
	}
	best := short[bestIdx]
	if err := c.Switch(ctx, best.Hostname); err != nil {
		return best, results, err
	}
	if err := c.WaitRunning(ctx, cfg.RunTimeout); err != nil {
		return best, results, fmt.Errorf("best server %s did not come back: %w", best.Hostname, err)
	}
	return best, results, nil
}
