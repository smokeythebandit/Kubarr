// Command vpn-picker picks the fastest VPN server for a gluetun pod.
//
//	vpn-picker init   init container: latency-rank candidates, write the shortlist
//	vpn-picker run    sidecar: speed-test the shortlist through gluetun, then monitor
package main

import (
	"context"
	"fmt"
	"log"
	"math/rand"
	"net/http"
	"os"
	"os/signal"
	"sync/atomic"
	"syscall"
	"time"

	"github.com/smokeythebandit/kubarr-vpn-picker/gluetun"
	"github.com/smokeythebandit/kubarr-vpn-picker/picker"
	"github.com/smokeythebandit/kubarr-vpn-picker/probe"
	"github.com/smokeythebandit/kubarr-vpn-picker/servers"
)

func main() {
	log.SetFlags(log.LstdFlags | log.LUTC)
	mode := ""
	if len(os.Args) > 1 {
		mode = os.Args[1]
	}
	cfg, err := loadConfig(os.Getenv)
	switch mode {
	case "init":
		// The init container must never block or fail the pod.
		defer func() {
			if r := recover(); r != nil {
				log.Printf("init: panic: %v", r)
			}
		}()
		if err != nil {
			log.Printf("init: %v", err)
			return
		}
		runInit(cfg)
	case "run":
		if err != nil {
			log.Printf("run: %v", err)
			waitForSignal() // keep the pod healthy; gluetun keeps its server
			return
		}
		runSidecar(cfg)
	default:
		fmt.Fprintln(os.Stderr, "usage: vpn-picker init|run")
		os.Exit(2)
	}
}

func signalContext() (context.Context, context.CancelFunc) {
	return signal.NotifyContext(context.Background(), syscall.SIGTERM, syscall.SIGINT)
}

func waitForSignal() {
	ctx, stop := signalContext()
	defer stop()
	<-ctx.Done()
}

func runInit(cfg config) {
	ctx, cancel := context.WithTimeout(context.Background(), cfg.InitBudget)
	defer cancel()

	list, err := servers.Load(ctx, cfg.ServersURL, nil)
	if err != nil {
		log.Printf("init: load server list: %v", err)
		return
	}
	cands, err := servers.Candidates(list, servers.Filter{
		Provider: cfg.Provider, VPNType: cfg.VPNType,
		Countries: cfg.Countries, Cities: cfg.Cities, Hostnames: cfg.Hostnames,
		P2P: cfg.P2PFilter(),
	})
	if err != nil {
		log.Printf("init: filter: %v", err)
		return
	}
	if len(cands) == 0 {
		log.Printf("init: no candidates for provider=%q type=%q; leaving gluetun's selection alone", cfg.Provider, cfg.VPNType)
		return
	}
	sample := servers.Sample(cands, cfg.Sample, rand.New(rand.NewSource(time.Now().UnixNano())))
	short := probe.Rank(ctx, sample, probe.ICMPPinger{}, cfg.Shortlist)
	if len(short) == 0 {
		log.Printf("init: ICMP ranked nothing, falling back to TCP/443")
		short = probe.Rank(ctx, sample, probe.TCPPinger{Port: 443}, cfg.Shortlist)
	}
	if len(short) == 0 {
		log.Printf("init: no reachable candidates")
		return
	}
	if err := picker.WriteShortlist(cfg.StateDir, short); err != nil {
		log.Printf("init: write shortlist: %v", err)
		return
	}
	log.Printf("init: shortlist of %d from %d candidates, first %s", len(short), len(cands), short[0].Hostname)
}

func runSidecar(cfg config) {
	ctx, stop := signalContext()
	defer stop()

	var status atomic.Pointer[picker.Status]
	status.Store(&picker.Status{})
	mux := http.NewServeMux()
	mux.HandleFunc("/status", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.Write(status.Load().JSON())
	})
	go func() {
		if err := http.ListenAndServe(cfg.StatusAddr, mux); err != nil {
			log.Printf("status server: %v", err)
		}
	}()

	pc := picker.Config{
		StateDir: cfg.StateDir, TestDuration: cfg.TestDuration, Interval: cfg.Interval,
		Cooldown: cfg.Cooldown, DegradeThreshold: cfg.DegradeThreshold,
		DegradeChecks: cfg.DegradeChecks, SwitchWhileRunning: cfg.SwitchWhileRunning,
	}
	gc := &gluetun.Client{Base: cfg.GluetunURL}
	mon, err := picker.Startup(ctx, gc,
		picker.TunMeter{Path: "/sys/class/net/tun0/statistics/rx_bytes"},
		picker.HTTPTester{URLs: cfg.TestEndpoints}, pc)
	if err != nil {
		log.Printf("run: %v", err)
	}
	if mon == nil {
		<-ctx.Done()
		return
	}
	status.Store(mon.St)
	mon.Run(ctx)
}
