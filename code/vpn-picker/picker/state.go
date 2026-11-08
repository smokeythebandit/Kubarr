package picker

import (
	"encoding/json"
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/smokeythebandit/kubarr-vpn-picker/servers"
)

const (
	shortlistFile = "shortlist.json"
	resultFile    = "result.json"
)

// ResultFile is /picker/result.json.
type ResultFile struct {
	Best string    `json:"best"`
	Mbps float64   `json:"mbps"`
	At   time.Time `json:"at"`
}

func writeJSON(dir, name string, v any) error {
	b, err := json.Marshal(v)
	if err != nil {
		return err
	}
	tmp := filepath.Join(dir, "."+name+".tmp")
	if err := os.WriteFile(tmp, b, 0o644); err != nil {
		return err
	}
	return os.Rename(tmp, filepath.Join(dir, name))
}

// WriteShortlist stores the ranked shortlist for the sidecar.
func WriteShortlist(dir string, s []servers.Server) error {
	return writeJSON(dir, shortlistFile, s)
}

// ReadShortlist returns the shortlist; a missing file is an empty list.
func ReadShortlist(dir string) ([]servers.Server, error) {
	b, err := os.ReadFile(filepath.Join(dir, shortlistFile))
	if errors.Is(err, fs.ErrNotExist) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	var s []servers.Server
	return s, json.Unmarshal(b, &s)
}

func WriteResult(dir string, r ResultFile) error { return writeJSON(dir, resultFile, r) }

// ReadResult returns the last result, or nil if there is none.
func ReadResult(dir string) *ResultFile {
	b, err := os.ReadFile(filepath.Join(dir, resultFile))
	if err != nil {
		return nil
	}
	var r ResultFile
	if json.Unmarshal(b, &r) != nil {
		return nil
	}
	return &r
}

// Status is the state served on /status.
type Status struct {
	mu   sync.Mutex
	data statusData
}

type statusData struct {
	Current   string    `json:"current"`
	Results   []Result  `json:"results"`
	LastRound time.Time `json:"last_round"`
	LastCheck time.Time `json:"last_check"`
	LastMbps  float64   `json:"last_check_mbps"`
	NextCheck time.Time `json:"next_check"`
	Degraded  int       `json:"degraded_checks"`
}

func (s *Status) update(f func(*statusData)) {
	s.mu.Lock()
	defer s.mu.Unlock()
	f(&s.data)
}

// JSON returns the current status document.
func (s *Status) JSON() []byte {
	s.mu.Lock()
	defer s.mu.Unlock()
	b, _ := json.Marshal(s.data)
	return b
}
