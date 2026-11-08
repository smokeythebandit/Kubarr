package picker

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/smokeythebandit/kubarr-vpn-picker/servers"
)

// world is a fake gluetun + tunnel + clock.
type world struct {
	now      time.Time
	start    time.Time
	cur      string
	switches []string
	dead     map[string]bool    // never reaches running
	rate     map[string]float64 // test download bytes/s per server
	bg       float64            // app traffic bytes/s
	bytes    float64
}

func newWorld() *world {
	t0 := time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC)
	return &world{now: t0, start: t0, dead: map[string]bool{}, rate: map[string]float64{}}
}

func (w *world) Switch(ctx context.Context, h string) error {
	w.cur = h
	w.switches = append(w.switches, h)
	return nil
}
func (w *world) WaitRunning(ctx context.Context, d time.Duration) error {
	if w.dead[w.cur] {
		w.now = w.now.Add(d)
		return errors.New("timeout")
	}
	return nil
}
func (w *world) Bytes() (uint64, error) {
	return uint64(w.bytes + w.bg*w.now.Sub(w.start).Seconds()), nil
}
func (w *world) Run(ctx context.Context, d time.Duration) error {
	w.bytes += w.rate[w.cur] * d.Seconds()
	w.now = w.now.Add(d)
	return nil
}
func (w *world) cfg(dir string) Config {
	return Config{
		StateDir: dir, TestDuration: 8 * time.Second, Interval: 15 * time.Minute,
		Cooldown: 6 * time.Hour, DegradeThreshold: 0.4, DegradeChecks: 3,
		SwitchWhileRunning: true, RunTimeout: 30 * time.Second,
		Now: func() time.Time { return w.now },
		Sleep: func(ctx context.Context, d time.Duration) error {
			w.now = w.now.Add(d)
			return ctx.Err()
		},
	}
}

func sv(names ...string) []servers.Server {
	var out []servers.Server
	for _, n := range names {
		out = append(out, servers.Server{Hostname: n})
	}
	return out
}

func TestRoundPicksHighestThroughput(t *testing.T) {
	w := newWorld()
	w.rate = map[string]float64{"a": 5e6, "b": 20e6, "c": 10e6}
	best, res, err := Round(context.Background(), w, sv("a", "b", "c"), w, w, w.cfg(t.TempDir()))
	if err != nil {
		t.Fatal(err)
	}
	if best.Hostname != "b" || w.cur != "b" {
		t.Fatalf("best=%s cur=%s", best.Hostname, w.cur)
	}
	if len(res) != 3 || res[1].BytesPerSec != 20e6 {
		t.Fatalf("results: %+v", res)
	}
}

func TestRoundSkipsServerThatNeverRuns(t *testing.T) {
	w := newWorld()
	w.rate = map[string]float64{"a": 5e6, "b": 99e6, "c": 10e6}
	w.dead["b"] = true
	best, res, err := Round(context.Background(), w, sv("a", "b", "c"), w, w, w.cfg(t.TempDir()))
	if err != nil {
		t.Fatal(err)
	}
	if best.Hostname != "c" || w.cur != "c" {
		t.Fatalf("best=%s cur=%s", best.Hostname, w.cur)
	}
	if res[1].Err == "" {
		t.Fatal("dead server should record an error")
	}
}

func TestRoundAllFailReturnsToFirst(t *testing.T) {
	w := newWorld()
	w.dead["a"], w.dead["b"] = true, true
	_, _, err := Round(context.Background(), w, sv("a", "b"), w, w, w.cfg(t.TempDir()))
	if err == nil {
		t.Fatal("expected error when every candidate fails")
	}
	if w.cur != "a" {
		t.Fatalf("expected switch back to first, cur=%s", w.cur)
	}
}

// monitor fixture: a is current best at 10 MB/s.
func monitorFor(w *world, dir string, mutate func(*Config)) *Monitor {
	cfg := w.cfg(dir)
	if mutate != nil {
		mutate(&cfg)
	}
	w.cur = "a"
	return &Monitor{
		Sw: w, M: w, T: w, Cfg: cfg, Short: sv("a", "b"), St: &Status{},
		Baseline: 10e6, LastRound: w.now,
	}
}

func TestMonitorNeedsConsecutiveDegradedChecks(t *testing.T) {
	w := newWorld()
	w.rate = map[string]float64{"a": 2e6, "b": 10e6}
	m := monitorFor(w, t.TempDir(), func(c *Config) { c.Cooldown = time.Minute })
	w.now = w.now.Add(time.Hour)
	m.Step(context.Background())
	m.Step(context.Background())
	if len(w.switches) != 0 {
		t.Fatalf("switched early: %v", w.switches)
	}
	// a healthy check resets the streak
	w.rate["a"] = 9e6
	m.Step(context.Background())
	w.rate["a"] = 2e6
	m.Step(context.Background())
	m.Step(context.Background())
	if len(w.switches) != 0 {
		t.Fatalf("streak was not reset: %v", w.switches)
	}
	m.Step(context.Background())
	if len(w.switches) == 0 || w.cur != "b" {
		t.Fatalf("expected round after 3 degraded checks, switches=%v cur=%s", w.switches, w.cur)
	}
}

func TestMonitorHonoursCooldown(t *testing.T) {
	w := newWorld()
	w.rate = map[string]float64{"a": 2e6, "b": 10e6}
	m := monitorFor(w, t.TempDir(), nil) // cooldown 6h, last round = now
	for i := 0; i < 5; i++ {
		m.Step(context.Background())
	}
	if len(w.switches) != 0 {
		t.Fatalf("switched inside cooldown: %v", w.switches)
	}
	w.now = w.now.Add(7 * time.Hour)
	m.Step(context.Background())
	if len(w.switches) == 0 {
		t.Fatal("expected round after cooldown")
	}
}

func TestMonitorWithoutSwitchWhileRunningWaitsForIdle(t *testing.T) {
	w := newWorld()
	w.rate = map[string]float64{"a": 0.2e6, "b": 10e6}
	w.bg = 1.5e6 // app is busy
	m := monitorFor(w, t.TempDir(), func(c *Config) {
		c.SwitchWhileRunning = false
		c.Cooldown = time.Minute
	})
	w.now = w.now.Add(time.Hour)
	for i := 0; i < 4; i++ {
		m.Step(context.Background())
	}
	if len(w.switches) != 0 {
		t.Fatalf("switched while busy: %v", w.switches)
	}
	w.bg = 0 // idle
	m.Step(context.Background())
	if len(w.switches) == 0 {
		t.Fatal("expected round once the tunnel is idle")
	}
}

func writeShortlist(t *testing.T, dir string, s []servers.Server) {
	t.Helper()
	if err := WriteShortlist(dir, s); err != nil {
		t.Fatal(err)
	}
}

func TestRoundSkippedWhenRecentResultExists(t *testing.T) {
	dir := t.TempDir()
	w := newWorld()
	w.rate = map[string]float64{"a": 5e6, "b": 20e6}
	writeShortlist(t, dir, sv("a", "b"))
	if err := WriteResult(dir, ResultFile{Best: "b", Mbps: 160, At: w.now.Add(-time.Hour)}); err != nil {
		t.Fatal(err)
	}
	m, err := Startup(context.Background(), w, w, w, w.cfg(dir))
	if err != nil {
		t.Fatal(err)
	}
	if len(w.switches) != 0 {
		t.Fatalf("round ran despite recent result: %v", w.switches)
	}
	if m == nil || m.Baseline != 160e6/8 {
		t.Fatalf("monitor should resume from result.json: %+v", m)
	}
	// old result: round runs
	if err := WriteResult(dir, ResultFile{Best: "b", Mbps: 160, At: w.now.Add(-7 * time.Hour)}); err != nil {
		t.Fatal(err)
	}
	if _, err := Startup(context.Background(), w, w, w, w.cfg(dir)); err != nil {
		t.Fatal(err)
	}
	if len(w.switches) == 0 {
		t.Fatal("round should run when result is older than cooldown")
	}
	if _, err := os.Stat(filepath.Join(dir, "result.json")); err != nil {
		t.Fatal(err)
	}
}

func TestEmptyShortlistIsNoop(t *testing.T) {
	w := newWorld()
	for name, setup := range map[string]func(string){
		"missing": func(string) {},
		"empty":   func(d string) { writeShortlist(t, d, nil) },
	} {
		dir := t.TempDir()
		setup(dir)
		m, err := Startup(context.Background(), w, w, w, w.cfg(dir))
		if err != nil || m != nil {
			t.Fatalf("%s: m=%v err=%v", name, m, err)
		}
	}
	if len(w.switches) != 0 {
		t.Fatalf("switched with no shortlist: %v", w.switches)
	}
}

func TestHTTPTesterFallsBackToNextURL(t *testing.T) {
	bad := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Error(w, "x", 500)
	}))
	defer bad.Close()
	hits := 0
	good := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		hits++
		w.Write(make([]byte, 1024))
	}))
	defer good.Close()
	tr := HTTPTester{URLs: []string{bad.URL, good.URL}, MaxBytes: 2048}
	if err := tr.Run(context.Background(), 5*time.Second); err != nil {
		t.Fatal(err)
	}
	if hits != 2 {
		t.Fatalf("good hits=%d", hits)
	}
	if err := (HTTPTester{URLs: []string{bad.URL}}).Run(context.Background(), time.Second); err == nil {
		t.Fatal("expected error when all URLs fail")
	}
}

func TestHTTPTesterRepeatsUntilDeadline(t *testing.T) {
	hits := 0
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		hits++
		w.Write(make([]byte, 1024))
	}))
	defer srv.Close()
	start := time.Now()
	if err := (HTTPTester{URLs: []string{srv.URL}}).Run(context.Background(), 150*time.Millisecond); err != nil {
		t.Fatal(err)
	}
	if el := time.Since(start); el < 140*time.Millisecond || el > 2*time.Second {
		t.Fatalf("elapsed %s", el)
	}
	if hits < 2 {
		t.Fatalf("expected repeated requests, got %d", hits)
	}
}

func TestHTTPTesterStopsAtByteCap(t *testing.T) {
	hits := 0
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		hits++
		w.Write(make([]byte, 1024))
	}))
	defer srv.Close()
	start := time.Now()
	if err := (HTTPTester{URLs: []string{srv.URL}, MaxBytes: 4096}).Run(context.Background(), 10*time.Second); err != nil {
		t.Fatal(err)
	}
	if time.Since(start) > 3*time.Second || hits != 4 {
		t.Fatalf("hits=%d elapsed=%s", hits, time.Since(start))
	}
}

func TestRoundDoesNotReconnectWhenLastCandidateIsBest(t *testing.T) {
	w := newWorld()
	w.rate = map[string]float64{"a": 5e6, "b": 20e6}
	best, _, err := Round(context.Background(), w, sv("a", "b"), w, w, w.cfg(t.TempDir()))
	if err != nil || best.Hostname != "b" {
		t.Fatalf("best=%s err=%v", best.Hostname, err)
	}
	if len(w.switches) != 2 {
		t.Fatalf("expected 2 switches, got %v", w.switches)
	}
}

func TestFailedRoundIsRetriedAfterCooldownAndNeverReadsAsDegraded(t *testing.T) {
	dir := t.TempDir()
	w := newWorld()
	w.dead["a"], w.dead["b"] = true, true
	writeShortlist(t, dir, sv("a", "b"))
	m, err := Startup(context.Background(), w, w, w, w.cfg(dir))
	if err != nil || m == nil {
		t.Fatalf("m=%v err=%v", m, err)
	}
	if m.Baseline != 0 {
		t.Fatal("expected no baseline after failed round")
	}
	n := len(w.switches)
	for i := 0; i < 5; i++ {
		m.Step(context.Background())
	}
	if len(w.switches) != n || m.degraded != 0 {
		t.Fatalf("checks ran without a baseline: switches=%d degraded=%d", len(w.switches), m.degraded)
	}
	w.dead = map[string]bool{}
	w.rate = map[string]float64{"a": 5e6, "b": 20e6}
	w.now = w.now.Add(7 * time.Hour)
	m.Step(context.Background())
	if m.Baseline != 20e6 || w.cur != "b" {
		t.Fatalf("round not retried: baseline=%v cur=%s", m.Baseline, w.cur)
	}
}

func TestTunMeterReadsCounter(t *testing.T) {
	p := filepath.Join(t.TempDir(), "rx_bytes")
	os.WriteFile(p, []byte("12345\n"), 0o600)
	n, err := TunMeter{Path: p}.Bytes()
	if err != nil || n != 12345 {
		t.Fatalf("n=%d err=%v", n, err)
	}
}

func TestHTTPTesterEmptyBodyIsFailure(t *testing.T) {
	hits := 0
	empty := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		hits++
		w.WriteHeader(200)
	}))
	defer empty.Close()
	start := time.Now()
	if err := (HTTPTester{URLs: []string{empty.URL}}).Run(context.Background(), 5*time.Second); err == nil {
		t.Fatal("expected error for zero-byte body")
	}
	if time.Since(start) > 2*time.Second || hits != 1 {
		t.Fatalf("hits=%d elapsed=%s", hits, time.Since(start))
	}
	good := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write(make([]byte, 1024))
	}))
	defer good.Close()
	if err := (HTTPTester{URLs: []string{empty.URL, good.URL}, MaxBytes: 1024}).Run(context.Background(), 5*time.Second); err != nil {
		t.Fatalf("should fall through to next URL: %v", err)
	}
}
