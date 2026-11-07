// Package probe ranks servers by measured round-trip time.
package probe

import (
	"context"
	"fmt"
	"net"
	"net/netip"
	"os"
	"sort"
	"sync"
	"time"

	"golang.org/x/net/icmp"
	"golang.org/x/net/ipv4"
	"golang.org/x/net/ipv6"

	"github.com/smokeythebandit/kubarr-vpn-picker/servers"
)

const (
	probesPerServer = 3
	parallelism     = 8
	probeTimeout    = 2 * time.Second
)

// Pinger measures one round trip to ip.
type Pinger interface {
	RTT(ctx context.Context, ip netip.Addr) (time.Duration, error)
}

// TCPPinger times a TCP handshake to Port (no privileges needed).
type TCPPinger struct{ Port int }

func (p TCPPinger) RTT(ctx context.Context, ip netip.Addr) (time.Duration, error) {
	ctx, cancel := context.WithTimeout(ctx, probeTimeout)
	defer cancel()
	var d net.Dialer
	start := time.Now()
	c, err := d.DialContext(ctx, "tcp", netip.AddrPortFrom(ip, uint16(p.Port)).String())
	if err != nil {
		return 0, err
	}
	rtt := time.Since(start)
	c.Close()
	return rtt, nil
}

// ICMPPinger sends ICMP echo using a raw (privileged) socket.
type ICMPPinger struct{}

func (ICMPPinger) RTT(ctx context.Context, ip netip.Addr) (time.Duration, error) {
	network, listen := "ip4:icmp", "0.0.0.0"
	var reqType icmp.Type = ipv4.ICMPTypeEcho
	var replyType icmp.Type = ipv4.ICMPTypeEchoReply
	proto := 1
	if ip.Is6() {
		network, listen, proto = "ip6:ipv6-icmp", "::", 58
		reqType, replyType = ipv6.ICMPTypeEchoRequest, ipv6.ICMPTypeEchoReply
	}
	conn, err := icmp.ListenPacket(network, listen)
	if err != nil {
		return 0, err
	}
	defer conn.Close()
	deadline := time.Now().Add(probeTimeout)
	if d, ok := ctx.Deadline(); ok && d.Before(deadline) {
		deadline = d
	}
	conn.SetDeadline(deadline)
	id := os.Getpid() & 0xffff
	msg := icmp.Message{Type: reqType, Body: &icmp.Echo{ID: id, Seq: 1, Data: []byte("kubarr")}}
	wb, err := msg.Marshal(nil)
	if err != nil {
		return 0, err
	}
	start := time.Now()
	if _, err := conn.WriteTo(wb, &net.IPAddr{IP: ip.AsSlice()}); err != nil {
		return 0, err
	}
	buf := make([]byte, 1500)
	for {
		n, _, err := conn.ReadFrom(buf)
		if err != nil {
			return 0, err
		}
		rm, err := icmp.ParseMessage(proto, buf[:n])
		if err != nil {
			continue
		}
		if echo, ok := rm.Body.(*icmp.Echo); ok && rm.Type == replyType && echo.ID == id && echo.Seq == 1 {
			return time.Since(start), nil
		}
		if ctx.Err() != nil {
			return 0, fmt.Errorf("canceled: %w", ctx.Err())
		}
	}
}

func median(d []time.Duration) time.Duration {
	sort.Slice(d, func(i, j int) bool { return d[i] < d[j] })
	return d[len(d)/2]
}

// probeServer returns the median RTT over successful probes to the server's
// first IP; ok is false when every probe failed or ctx ended.
func probeServer(ctx context.Context, s servers.Server, p Pinger) (time.Duration, bool) {
	if len(s.IPs) == 0 {
		return 0, false
	}
	var rtts []time.Duration
	for i := 0; i < probesPerServer && ctx.Err() == nil; i++ {
		if d, err := p.RTT(ctx, s.IPs[0]); err == nil {
			rtts = append(rtts, d)
		}
	}
	if len(rtts) == 0 || ctx.Err() != nil {
		return 0, false
	}
	return median(rtts), true
}

// Rank probes each server 3 times (parallelism 8), drops unreachable ones,
// and returns up to keep servers sorted by ascending median RTT. When ctx
// ends, servers that finished before then are still returned.
func Rank(ctx context.Context, in []servers.Server, p Pinger, keep int) []servers.Server {
	type result struct {
		s   servers.Server
		rtt time.Duration
	}
	var (
		mu  sync.Mutex
		res []result
		wg  sync.WaitGroup
	)
	sem := make(chan struct{}, parallelism)
loop:
	for _, s := range in {
		select {
		case sem <- struct{}{}:
		case <-ctx.Done():
			break loop
		}
		wg.Add(1)
		go func(s servers.Server) {
			defer wg.Done()
			defer func() { <-sem }()
			if rtt, ok := probeServer(ctx, s, p); ok {
				mu.Lock()
				res = append(res, result{s, rtt})
				mu.Unlock()
			}
		}(s)
	}
	wg.Wait()
	sort.SliceStable(res, func(i, j int) bool { return res[i].rtt < res[j].rtt })
	if keep >= 0 && keep < len(res) {
		res = res[:keep]
	}
	out := make([]servers.Server, len(res))
	for i, r := range res {
		out[i] = r.s
	}
	return out
}
