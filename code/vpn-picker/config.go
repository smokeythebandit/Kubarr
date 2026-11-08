package main

import (
	"fmt"
	"strconv"
	"strings"
	"time"
)

const defaultServersURL = "https://raw.githubusercontent.com/qdm12/gluetun/v3.41.3/internal/storage/servers.json"

type config struct {
	Shortlist, Sample  int
	InitBudget         time.Duration
	TestDuration       time.Duration
	TestEndpoints      []string
	Interval           time.Duration
	DegradeThreshold   float64
	DegradeChecks      int
	Cooldown           time.Duration
	SwitchWhileRunning bool
	P2P                bool
	ServersURL         string
	GluetunURL         string
	StateDir           string
	StatusAddr         string

	Provider, VPNType            string
	Countries, Cities, Hostnames []string
	PortForwarding               bool
}

// P2PFilter reports whether init should restrict candidates to port-forward capable servers.
func (c config) P2PFilter() bool { return c.P2P || c.PortForwarding }

func list(s string) []string {
	var out []string
	for _, p := range strings.Split(s, ",") {
		if p = strings.TrimSpace(p); p != "" {
			out = append(out, p)
		}
	}
	return out
}

// loadConfig reads the PICKER_* and VPN Secret variables via getenv.
func loadConfig(getenv func(string) string) (config, error) {
	var errs []string
	str := func(k, def string) string {
		if v := getenv(k); v != "" {
			return v
		}
		return def
	}
	integer := func(k string, def int) int {
		v := getenv(k)
		if v == "" {
			return def
		}
		n, err := strconv.Atoi(v)
		if err != nil || n < 0 {
			errs = append(errs, fmt.Sprintf("%s=%q: not a non-negative integer", k, v))
			return def
		}
		return n
	}
	dur := func(k string, def time.Duration) time.Duration {
		v := getenv(k)
		if v == "" {
			return def
		}
		d, err := time.ParseDuration(v)
		if err != nil || d <= 0 {
			errs = append(errs, fmt.Sprintf("%s=%q: not a positive duration", k, v))
			return def
		}
		return d
	}
	boolean := func(k string, def bool) bool {
		v := getenv(k)
		if v == "" {
			return def
		}
		b, err := strconv.ParseBool(v)
		if err != nil {
			errs = append(errs, fmt.Sprintf("%s=%q: not a boolean", k, v))
			return def
		}
		return b
	}

	c := config{
		Shortlist:          integer("PICKER_SHORTLIST", 5),
		Sample:             integer("PICKER_SAMPLE", 40),
		InitBudget:         dur("PICKER_INIT_BUDGET", 60*time.Second),
		Interval:           dur("PICKER_INTERVAL", 15*time.Minute),
		Cooldown:           dur("PICKER_COOLDOWN", 6*time.Hour),
		DegradeChecks:      integer("PICKER_DEGRADE_CHECKS", 3),
		SwitchWhileRunning: boolean("PICKER_SWITCH_WHILE_RUNNING", true),
		P2P:                boolean("PICKER_P2P", false),
		ServersURL:         str("PICKER_SERVERS_URL", defaultServersURL),
		GluetunURL:         str("PICKER_GLUETUN_URL", "http://127.0.0.1:8001"),
		StateDir:           str("PICKER_STATE_DIR", "/picker"),
		StatusAddr:         str("PICKER_STATUS_ADDR", "127.0.0.1:8002"),
		Provider:           getenv("VPN_SERVICE_PROVIDER"),
		VPNType:            getenv("VPN_TYPE"),
		Countries:          list(getenv("SERVER_COUNTRIES")),
		Cities:             list(getenv("SERVER_CITIES")),
		Hostnames:          list(getenv("SERVER_HOSTNAMES")),
		PortForwarding:     strings.EqualFold(getenv("VPN_PORT_FORWARDING"), "on"),
		DegradeThreshold:   0.4,
		TestEndpoints:      []string{"https://proof.ovh.net/files/100Mb.dat", "http://speedtest.tele2.net/100MB.zip"},
	}
	// testSeconds is a plain number of seconds (a Go duration is accepted too).
	c.TestDuration = 8 * time.Second
	if v := getenv("PICKER_TEST_SECONDS"); v != "" {
		if n, err := strconv.Atoi(v); err == nil && n > 0 {
			c.TestDuration = time.Duration(n) * time.Second
		} else if d, err := time.ParseDuration(v); err == nil && d > 0 {
			c.TestDuration = d
		} else {
			errs = append(errs, fmt.Sprintf("PICKER_TEST_SECONDS=%q: not a positive number of seconds", v))
		}
	}
	if c.DegradeChecks < 1 {
		errs = append(errs, fmt.Sprintf("PICKER_DEGRADE_CHECKS=%q: must be at least 1", getenv("PICKER_DEGRADE_CHECKS")))
	}
	if v := getenv("PICKER_DEGRADE_THRESHOLD"); v != "" {
		f, err := strconv.ParseFloat(v, 64)
		if err != nil || f <= 0 || f > 1 {
			errs = append(errs, fmt.Sprintf("PICKER_DEGRADE_THRESHOLD=%q: not a number in (0,1]", v))
		} else {
			c.DegradeThreshold = f
		}
	}
	if eps := list(getenv("PICKER_TEST_ENDPOINTS")); len(eps) > 0 {
		c.TestEndpoints = eps
	}
	if len(errs) > 0 {
		return c, fmt.Errorf("invalid configuration: %s", strings.Join(errs, "; "))
	}
	return c, nil
}
