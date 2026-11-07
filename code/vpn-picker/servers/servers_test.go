package servers

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

const fixture = `{
 "version": 1,
 "protonvpn": {"version": 4, "timestamp": 1, "servers": [
  {"vpn":"wireguard","country":"Netherlands","city":"Amsterdam","server_name":"NL#1","hostname":"nl-1.proton","port_forward":true,"ips":["1.1.1.1","2606:4700::1"]},
  {"vpn":"wireguard","country":"Netherlands","city":"Amsterdam","server_name":"NL#2","hostname":"nl-2.proton","ips":["1.1.1.2"]},
  {"vpn":"openvpn","country":"Netherlands","city":"Amsterdam","hostname":"nl-3.proton","port_forward":true,"ips":["1.1.1.3"]},
  {"vpn":"wireguard","country":"Germany","city":"Berlin","hostname":"de-1.proton","port_forward":true,"ips":["2.2.2.2"]},
  {"vpn":"wireguard","country":"netherlands","city":"Amsterdam","hostname":"nl-sc.proton","port_forward":true,"secure_core":true,"ips":["1.1.1.4"]},
  {"vpn":"wireguard","country":"Netherlands","hostname":"nl-tor.proton","port_forward":true,"tor":true,"ips":["1.1.1.5"]},
  {"vpn":"wireguard","country":"Netherlands","hostname":"nl-free.proton","port_forward":true,"free":true,"ips":["1.1.1.6"]}
 ]},
 "private internet access": {"version": 1, "timestamp": 1, "servers": [
  {"vpn":"wireguard","country":"Netherlands","hostname":"pia-nl","port_forward":true,"ips":["3.3.3.3"]}
 ]}
}`

func TestCandidatesFiltersProviderTypeCountryP2P(t *testing.T) {
	got, err := Candidates([]byte(fixture), Filter{Provider: "ProtonVPN", VPNType: "wireguard", Countries: []string{"netherlands"}, P2P: true})
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 1 || got[0].Hostname != "nl-1.proton" {
		t.Fatalf("got %+v", got)
	}
	if len(got[0].IPs) != 2 {
		t.Fatalf("ips: %v", got[0].IPs)
	}
	all, _ := Candidates([]byte(fixture), Filter{Provider: "protonvpn", VPNType: "wireguard"})
	if len(all) != 3 {
		t.Fatalf("want 3 wireguard non-special, got %d", len(all))
	}
	spaced, _ := Candidates([]byte(fixture), Filter{Provider: "Private Internet Access", Cities: nil, Hostnames: []string{"PIA-NL"}})
	if len(spaced) != 1 {
		t.Fatalf("spaced provider: %+v", spaced)
	}
	city, _ := Candidates([]byte(fixture), Filter{Provider: "protonvpn", Cities: []string{"berlin"}})
	if len(city) != 1 || city[0].Hostname != "de-1.proton" {
		t.Fatalf("city: %+v", city)
	}
}

func TestCandidatesExcludesSecureCoreTorFree(t *testing.T) {
	got, _ := Candidates([]byte(fixture), Filter{Provider: "protonvpn", Countries: []string{"Netherlands"}})
	for _, s := range got {
		if s.SecureCore || s.Tor || s.Free {
			t.Fatalf("special server leaked: %+v", s)
		}
	}
	if len(got) != 3 {
		t.Fatalf("got %d", len(got))
	}
}

func TestCandidatesUnknownProviderReturnsEmptyNoError(t *testing.T) {
	got, err := Candidates([]byte(fixture), Filter{Provider: "nope"})
	if err != nil || len(got) != 0 {
		t.Fatalf("got %v %v", got, err)
	}
}

func TestLoadFallsBackOnHTTPErrorAndOnTruncatedJSON(t *testing.T) {
	status := 500
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(status)
		if status == 200 {
			w.Write([]byte(`{"protonvpn":`))
		}
	}))
	defer srv.Close()
	for _, st := range []int{500, 200} {
		status = st
		b, err := Load(context.Background(), srv.URL, srv.Client())
		if err != nil {
			t.Fatal(err)
		}
		var m map[string]json.RawMessage
		if err := json.Unmarshal(b, &m); err != nil || m["protonvpn"] == nil {
			t.Fatalf("status %d: fallback not valid: %v", st, err)
		}
	}
}

func TestLoadReturnsDownloadedWhenValid(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte(fixture))
	}))
	defer srv.Close()
	b, err := Load(context.Background(), srv.URL, srv.Client())
	if err != nil || !bytes.Equal(b, []byte(fixture)) {
		t.Fatalf("err=%v", err)
	}
}

func TestSample(t *testing.T) {
	s := make([]Server, 10)
	for i := range s {
		s[i].Hostname = string(rune('a' + i))
	}
	if got := Sample(s, 3, newRand(1)); len(got) != 3 {
		t.Fatal(len(got))
	}
	if got := Sample(s, 20, newRand(1)); len(got) != 10 {
		t.Fatal(len(got))
	}
	if s[0].Hostname != "a" {
		t.Fatal("input mutated")
	}
}
