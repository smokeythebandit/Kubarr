package gluetun

import (
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"
)

func TestSwitchSendsHostnameBody(t *testing.T) {
	var method, path, body string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		b, _ := io.ReadAll(r.Body)
		method, path, body = r.Method, r.URL.Path, string(b)
		w.Write([]byte(`{"outcome":"settings updated"}`))
	}))
	defer srv.Close()
	c := &Client{Base: srv.URL, HTTP: srv.Client()}
	if err := c.Switch(context.Background(), "node-1.example"); err != nil {
		t.Fatal(err)
	}
	if method != http.MethodPut || path != "/v1/vpn/settings" {
		t.Fatalf("got %s %s", method, path)
	}
	want := `{"provider":{"server_selection":{"hostnames":["node-1.example"]}}}`
	if body != want {
		t.Fatalf("body = %s, want %s", body, want)
	}
}

func TestSwitchErrorsOnNon2xx(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Error(w, "nope", http.StatusUnauthorized)
	}))
	defer srv.Close()
	c := &Client{Base: srv.URL, HTTP: srv.Client()}
	if err := c.Switch(context.Background(), "x"); err == nil {
		t.Fatal("expected error")
	}
}

func TestWaitRunningReturnsWhenRunning(t *testing.T) {
	n := 0
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		n++
		if n < 3 {
			w.Write([]byte(`{"status":"stopped"}`))
			return
		}
		w.Write([]byte(`{"status":"running"}`))
	}))
	defer srv.Close()
	c := &Client{Base: srv.URL, HTTP: srv.Client(), Poll: 5 * time.Millisecond}
	if err := c.WaitRunning(context.Background(), 2*time.Second); err != nil {
		t.Fatal(err)
	}
	if n != 3 {
		t.Fatalf("polled %d times", n)
	}
}

func TestWaitRunningTimesOut(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Write([]byte(`{"status":"stopped"}`))
	}))
	defer srv.Close()
	c := &Client{Base: srv.URL, HTTP: srv.Client(), Poll: 5 * time.Millisecond}
	start := time.Now()
	if err := c.WaitRunning(context.Background(), 100*time.Millisecond); err == nil {
		t.Fatal("expected timeout")
	}
	if time.Since(start) > 2*time.Second {
		t.Fatal("took too long")
	}
}
