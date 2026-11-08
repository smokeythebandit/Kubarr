// Package gluetun talks to gluetun's HTTP control API.
package gluetun

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"time"
)

// Client controls one gluetun instance. Base is e.g. http://127.0.0.1:8001.
type Client struct {
	Base string
	HTTP *http.Client
	// Poll is the WaitRunning polling interval (default 1 s).
	Poll time.Duration
}

func (c *Client) client() *http.Client {
	if c.HTTP != nil {
		return c.HTTP
	}
	return &http.Client{Timeout: 10 * time.Second}
}

// Switch pins gluetun to a single server hostname; gluetun reconnects.
func (c *Client) Switch(ctx context.Context, hostname string) error {
	body, err := json.Marshal(map[string]any{"provider": map[string]any{
		"server_selection": map[string]any{"hostnames": []string{hostname}},
	}})
	if err != nil {
		return err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPut, strings.TrimRight(c.Base, "/")+"/v1/vpn/settings", bytes.NewReader(body))
	if err != nil {
		return err
	}
	req.Header.Set("Content-Type", "application/json")
	resp, err := c.client().Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode/100 != 2 {
		return fmt.Errorf("switch to %s: gluetun returned %s", hostname, resp.Status)
	}
	return nil
}

func (c *Client) running(ctx context.Context) bool {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, strings.TrimRight(c.Base, "/")+"/v1/vpn/status", nil)
	if err != nil {
		return false
	}
	resp, err := c.client().Do(req)
	if err != nil {
		return false
	}
	defer resp.Body.Close()
	var st struct {
		Status string `json:"status"`
	}
	if resp.StatusCode/100 != 2 || json.NewDecoder(resp.Body).Decode(&st) != nil {
		return false
	}
	return st.Status == "running"
}

// WaitRunning polls the VPN status until it is "running" or timeout elapses.
func (c *Client) WaitRunning(ctx context.Context, timeout time.Duration) error {
	poll := c.Poll
	if poll <= 0 {
		poll = time.Second
	}
	ctx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()
	for {
		if c.running(ctx) {
			return nil
		}
		select {
		case <-ctx.Done():
			return fmt.Errorf("gluetun not running after %s: %w", timeout, ctx.Err())
		case <-time.After(poll):
		}
	}
}
