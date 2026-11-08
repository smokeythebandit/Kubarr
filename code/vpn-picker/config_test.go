package main

import (
	"reflect"
	"testing"
	"time"
)

func env(m map[string]string) func(string) string { return func(k string) string { return m[k] } }

func TestDefaults(t *testing.T) {
	c, err := loadConfig(env(nil))
	if err != nil {
		t.Fatal(err)
	}
	if c.Shortlist != 5 || c.Sample != 40 || c.InitBudget != 60*time.Second ||
		c.TestDuration != 8*time.Second || c.Interval != 15*time.Minute ||
		c.DegradeThreshold != 0.4 || c.DegradeChecks != 3 || c.Cooldown != 6*time.Hour ||
		!c.SwitchWhileRunning || c.P2P {
		t.Fatalf("defaults: %+v", c)
	}
	if c.ServersURL != "https://raw.githubusercontent.com/qdm12/gluetun/v3.41.3/internal/storage/servers.json" ||
		c.GluetunURL != "http://127.0.0.1:8001" || c.StateDir != "/picker" || c.StatusAddr != "127.0.0.1:8002" {
		t.Fatalf("defaults: %+v", c)
	}
	want := []string{"https://proof.ovh.net/files/100Mb.dat", "http://speedtest.tele2.net/100MB.zip"}
	if !reflect.DeepEqual(c.TestEndpoints, want) {
		t.Fatalf("endpoints: %v", c.TestEndpoints)
	}
}

func TestOverridesAndVPNKeys(t *testing.T) {
	c, err := loadConfig(env(map[string]string{
		"PICKER_SHORTLIST": "3", "PICKER_INIT_BUDGET": "30s", "PICKER_TEST_SECONDS": "5",
		"PICKER_TEST_ENDPOINTS": "http://a, http://b", "PICKER_COOLDOWN": "1h",
		"PICKER_SWITCH_WHILE_RUNNING": "false", "PICKER_DEGRADE_THRESHOLD": "0.5",
		"VPN_SERVICE_PROVIDER": "protonvpn", "VPN_TYPE": "wireguard",
		"SERVER_COUNTRIES": "Germany, Sweden", "VPN_PORT_FORWARDING": "on",
	}))
	if err != nil {
		t.Fatal(err)
	}
	if c.Shortlist != 3 || c.InitBudget != 30*time.Second || c.TestDuration != 5*time.Second ||
		c.Cooldown != time.Hour || c.SwitchWhileRunning || c.DegradeThreshold != 0.5 {
		t.Fatalf("%+v", c)
	}
	if !reflect.DeepEqual(c.TestEndpoints, []string{"http://a", "http://b"}) ||
		!reflect.DeepEqual(c.Countries, []string{"Germany", "Sweden"}) {
		t.Fatalf("%+v", c)
	}
	if !c.P2PFilter() {
		t.Fatal("port forwarding on should enable the P2P filter")
	}
	if c.Provider != "protonvpn" || c.VPNType != "wireguard" {
		t.Fatalf("%+v", c)
	}
}

func TestP2PFilterFromPickerP2P(t *testing.T) {
	c, _ := loadConfig(env(map[string]string{"PICKER_P2P": "true"}))
	if !c.P2PFilter() {
		t.Fatal("PICKER_P2P=true should enable the P2P filter")
	}
}

func TestInvalidValuesError(t *testing.T) {
	for _, kv := range [][2]string{
		{"PICKER_INIT_BUDGET", "soon"}, {"PICKER_INTERVAL", "x"}, {"PICKER_COOLDOWN", "-1h"},
		{"PICKER_TEST_SECONDS", "abc"}, {"PICKER_SHORTLIST", "many"}, {"PICKER_DEGRADE_CHECKS", "0"},
		{"PICKER_DEGRADE_THRESHOLD", "lots"}, {"PICKER_SWITCH_WHILE_RUNNING", "maybe"},
	} {
		if _, err := loadConfig(env(map[string]string{kv[0]: kv[1]})); err == nil {
			t.Errorf("%s=%s should error", kv[0], kv[1])
		}
	}
}
