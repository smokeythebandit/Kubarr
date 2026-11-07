// Package servers loads gluetun's server list and filters it into candidates.
package servers

import (
	"bytes"
	"compress/gzip"
	"context"
	_ "embed"
	"encoding/json"
	"fmt"
	"io"
	"math/rand"
	"net/http"
	"net/netip"
	"strings"
	"time"
)

//go:embed fallback.json.gz
var fallbackGz []byte

const loadTimeout = 20 * time.Second

// Server is one VPN server entry from gluetun's list.
type Server struct {
	Hostname    string
	ServerName  string
	Country     string
	City        string
	IPs         []netip.Addr
	PortForward bool
	SecureCore  bool
	Tor         bool
	Free        bool
	VPN         string
}

// Filter selects candidate servers. Matching is case-insensitive; empty
// slices match everything. P2P requires PortForward.
type Filter struct {
	Provider, VPNType            string
	Countries, Cities, Hostnames []string
	P2P                          bool
}

func fallback() ([]byte, error) {
	zr, err := gzip.NewReader(bytes.NewReader(fallbackGz))
	if err != nil {
		return nil, fmt.Errorf("embedded server list: %w", err)
	}
	defer zr.Close()
	return io.ReadAll(zr)
}

// Load downloads the server list from url (20 s timeout). On any error or
// invalid JSON it returns the embedded fallback list instead.
func Load(ctx context.Context, url string, client *http.Client) ([]byte, error) {
	if b, err := download(ctx, url, client); err == nil {
		return b, nil
	}
	return fallback()
}

func download(ctx context.Context, url string, client *http.Client) ([]byte, error) {
	if client == nil {
		client = http.DefaultClient
	}
	ctx, cancel := context.WithTimeout(ctx, loadTimeout)
	defer cancel()
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return nil, err
	}
	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("status %d", resp.StatusCode)
	}
	b, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}
	if !json.Valid(b) {
		return nil, fmt.Errorf("invalid JSON")
	}
	return b, nil
}

type rawServer struct {
	VPN         string   `json:"vpn"`
	Country     string   `json:"country"`
	City        string   `json:"city"`
	ServerName  string   `json:"server_name"`
	Hostname    string   `json:"hostname"`
	IPs         []string `json:"ips"`
	PortForward bool     `json:"port_forward"`
	SecureCore  bool     `json:"secure_core"`
	Tor         bool     `json:"tor"`
	Free        bool     `json:"free"`
}

func matches(list []string, v string) bool {
	if len(list) == 0 {
		return true
	}
	for _, x := range list {
		if strings.EqualFold(strings.TrimSpace(x), v) {
			return true
		}
	}
	return false
}

// Candidates returns the servers of f.Provider that satisfy f, excluding
// secure-core, Tor and free servers and any hostname shared with one. An unknown provider yields no servers.
func Candidates(list []byte, f Filter) ([]Server, error) {
	var top map[string]json.RawMessage
	if err := json.Unmarshal(list, &top); err != nil {
		return nil, fmt.Errorf("parse server list: %w", err)
	}
	// The Kubarr backend writes ids like "private_internet_access"; gluetun's
	// list keys use spaces.
	key := strings.ReplaceAll(strings.ToLower(strings.TrimSpace(f.Provider)), "_", " ")
	raw, ok := top[key]
	if !ok {
		return nil, nil
	}
	var prov struct {
		Servers []rawServer `json:"servers"`
	}
	if err := json.Unmarshal(raw, &prov); err != nil {
		return nil, fmt.Errorf("parse provider %q: %w", f.Provider, err)
	}
	// gluetun's filters never exclude secure-core/Tor/free entries, and some
	// hostnames are shared with such a sibling that has different IPs. Pinning
	// such a hostname could connect to the sibling, so drop it entirely.
	type hostKey struct{ vpn, host string }
	tainted := map[hostKey]bool{}
	for _, r := range prov.Servers {
		if r.SecureCore || r.Tor || r.Free {
			tainted[hostKey{strings.ToLower(r.VPN), strings.ToLower(r.Hostname)}] = true
		}
	}
	var out []Server
	for _, r := range prov.Servers {
		if r.SecureCore || r.Tor || r.Free || tainted[hostKey{strings.ToLower(r.VPN), strings.ToLower(r.Hostname)}] {
			continue
		}
		if f.VPNType != "" && !strings.EqualFold(f.VPNType, r.VPN) {
			continue
		}
		if f.P2P && !r.PortForward {
			continue
		}
		if !matches(f.Countries, r.Country) || !matches(f.Cities, r.City) || !matches(f.Hostnames, r.Hostname) {
			continue
		}
		s := Server{
			Hostname: r.Hostname, ServerName: r.ServerName, Country: r.Country, City: r.City,
			PortForward: r.PortForward, SecureCore: r.SecureCore, Tor: r.Tor, Free: r.Free, VPN: r.VPN,
		}
		for _, ip := range r.IPs {
			if a, err := netip.ParseAddr(ip); err == nil {
				s.IPs = append(s.IPs, a)
			}
		}
		out = append(out, s)
	}
	return out, nil
}

// Sample returns up to n servers chosen at random without modifying s.
func Sample(s []Server, n int, rnd *rand.Rand) []Server {
	c := append([]Server(nil), s...)
	rnd.Shuffle(len(c), func(i, j int) { c[i], c[j] = c[j], c[i] })
	if n < len(c) {
		c = c[:n]
	}
	return c
}
