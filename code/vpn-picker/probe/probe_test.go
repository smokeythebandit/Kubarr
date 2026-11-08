package probe

import (
	"context"
	"errors"
	"net"
	"net/netip"
	"sync"
	"testing"
	"time"

	"golang.org/x/net/icmp"
	"golang.org/x/net/ipv4"

	"github.com/smokeythebandit/kubarr-vpn-picker/servers"
)

type fake struct {
	mu    sync.Mutex
	rtts  map[netip.Addr][]time.Duration
	calls map[netip.Addr]int
	delay time.Duration
}

func (f *fake) RTT(ctx context.Context, ip netip.Addr) (time.Duration, error) {
	if f.delay > 0 {
		select {
		case <-time.After(f.delay):
		case <-ctx.Done():
			return 0, ctx.Err()
		}
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.calls == nil {
		f.calls = map[netip.Addr]int{}
	}
	r, ok := f.rtts[ip]
	if !ok || len(r) == 0 {
		return 0, errors.New("unreachable")
	}
	i := f.calls[ip]
	f.calls[ip]++
	return r[i%len(r)], nil
}

func srv(name, ip string) servers.Server {
	return servers.Server{Hostname: name, IPs: []netip.Addr{netip.MustParseAddr(ip)}}
}

func ms(v ...int) []time.Duration {
	var o []time.Duration
	for _, x := range v {
		o = append(o, time.Duration(x)*time.Millisecond)
	}
	return o
}

func TestRankSortsByMedianAndDropsUnreachable(t *testing.T) {
	f := &fake{rtts: map[netip.Addr][]time.Duration{
		netip.MustParseAddr("1.0.0.1"): ms(50, 50, 50),
		netip.MustParseAddr("1.0.0.2"): ms(10, 200, 20), // median 20
		netip.MustParseAddr("1.0.0.3"): ms(30, 30, 30),
	}}
	in := []servers.Server{srv("a", "1.0.0.1"), srv("b", "1.0.0.2"), srv("c", "1.0.0.3"), srv("dead", "1.0.0.9")}
	got := Rank(context.Background(), in, f, 10)
	var names []string
	for _, s := range got {
		names = append(names, s.Hostname)
	}
	want := []string{"b", "c", "a"}
	if len(names) != 3 || names[0] != want[0] || names[1] != want[1] || names[2] != want[2] {
		t.Fatalf("got %v", names)
	}
}

func TestRankKeepsN(t *testing.T) {
	f := &fake{rtts: map[netip.Addr][]time.Duration{
		netip.MustParseAddr("1.0.0.1"): ms(1),
		netip.MustParseAddr("1.0.0.2"): ms(2),
		netip.MustParseAddr("1.0.0.3"): ms(3),
	}}
	got := Rank(context.Background(), []servers.Server{srv("c", "1.0.0.3"), srv("a", "1.0.0.1"), srv("b", "1.0.0.2")}, f, 2)
	if len(got) != 2 || got[0].Hostname != "a" || got[1].Hostname != "b" {
		t.Fatalf("got %+v", got)
	}
}

func TestRankRespectsContextDeadline(t *testing.T) {
	f := &fake{delay: 5 * time.Second, rtts: map[netip.Addr][]time.Duration{
		netip.MustParseAddr("1.0.0.1"): ms(1),
	}}
	ctx, cancel := context.WithTimeout(context.Background(), 50*time.Millisecond)
	defer cancel()
	start := time.Now()
	got := Rank(ctx, []servers.Server{srv("a", "1.0.0.1")}, f, 5)
	if time.Since(start) > time.Second {
		t.Fatal("hung")
	}
	if len(got) != 0 {
		t.Fatalf("got %+v", got)
	}
}

func echoMsg(t icmp.Type, id, seq int) *icmp.Message {
	return &icmp.Message{Type: t, Body: &icmp.Echo{ID: id, Seq: seq}}
}

func TestMatchesEcho(t *testing.T) {
	target := netip.MustParseAddr("9.9.9.9")
	peer := &net.IPAddr{IP: net.ParseIP("9.9.9.9")}
	reply := echoMsg(ipv4.ICMPTypeEchoReply, 7, 3)
	if !matchesEcho(peer, reply, target, 7, 3) {
		t.Fatal("right source/id/seq should match")
	}
	if matchesEcho(&net.IPAddr{IP: net.ParseIP("8.8.8.8")}, reply, target, 7, 3) {
		t.Fatal("wrong source matched")
	}
	if matchesEcho(peer, reply, target, 7, 4) {
		t.Fatal("wrong seq matched")
	}
	if matchesEcho(peer, reply, target, 8, 3) {
		t.Fatal("wrong id matched")
	}
	if matchesEcho(peer, echoMsg(ipv4.ICMPTypeEcho, 7, 3), target, 7, 3) {
		t.Fatal("request matched as reply")
	}
}

func TestTCPPingerRefusedCountsAsSuccess(t *testing.T) {
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	port := l.Addr().(*net.TCPAddr).Port
	l.Close()
	rtt, err := TCPPinger{Port: port}.RTT(context.Background(), netip.MustParseAddr("127.0.0.1"))
	if err != nil || rtt <= 0 {
		t.Fatalf("rtt=%v err=%v", rtt, err)
	}
}

func TestTCPPingerTimeoutIsError(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()
	if _, err := (TCPPinger{Port: 443}).RTT(ctx, netip.MustParseAddr("10.255.255.1")); err == nil {
		t.Fatal("expected error")
	}
}

func TestRankReturnsOnlyFinishedBeforeDeadline(t *testing.T) {
	f := &slowFake{slow: netip.MustParseAddr("1.0.0.2")}
	ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
	defer cancel()
	got := Rank(ctx, []servers.Server{srv("fast", "1.0.0.1"), srv("slow", "1.0.0.2")}, f, 5)
	if len(got) != 1 || got[0].Hostname != "fast" {
		t.Fatalf("got %+v", got)
	}
}

type slowFake struct{ slow netip.Addr }

func (f *slowFake) RTT(ctx context.Context, ip netip.Addr) (time.Duration, error) {
	if ip == f.slow {
		<-ctx.Done()
		return 0, ctx.Err()
	}
	return time.Millisecond, nil
}
