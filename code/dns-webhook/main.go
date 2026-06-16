package main

import (
	"bytes"
	"context"
	"crypto"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha512"
	"crypto/x509"
	"encoding/base64"
	"encoding/pem"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"strconv"
	"strings"
	"time"

	cmmeta "github.com/cert-manager/cert-manager/pkg/apis/meta/v1"
	whapi "github.com/cert-manager/cert-manager/pkg/acme/webhook/apis/acme/v1alpha1"
	"github.com/cert-manager/cert-manager/pkg/acme/webhook/cmd"
	corev1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/kubernetes"
	"k8s.io/client-go/rest"
)

const solverName = "transip"

type transipSolver struct {
	client kubernetes.Interface
	http   *http.Client
}

type solverConfig struct {
	Zone            string                 `json:"zone"`
	SecretName      string                 `json:"secretName"`
	SecretNamespace string                 `json:"secretNamespace"`
	TokenKey        string                 `json:"tokenKey"`
	SecretRef       *cmmeta.SecretKeySelector `json:"secretRef,omitempty"`
}

type transipDnsResponse struct {
	DNSEntries []transipDnsEntry `json:"dnsEntries"`
}

type transipDnsEntry struct {
	Name    string `json:"name"`
	Expire  int    `json:"expire"`
	Type    string `json:"type"`
	Content string `json:"content"`
}

func main() {
	groupName := os.Getenv("GROUP_NAME")
	if groupName == "" {
		groupName = "dns.kubarr.local"
	}

	cmd.RunWebhookServer(groupName, &transipSolver{})
}

func (s *transipSolver) Name() string { return solverName }

func (s *transipSolver) Initialize(config *rest.Config, _ <-chan struct{}) error {
	client, err := kubernetes.NewForConfig(config)
	if err != nil {
		return err
	}
	s.client = client
	s.http = &http.Client{Timeout: 30 * time.Second}
	return nil
}

func (s *transipSolver) Present(ch *whapi.ChallengeRequest) error {
	config, err := parseConfig(ch)
	if err != nil {
		return err
	}
	token, err := s.accessToken(ch, config)
	if err != nil {
		return err
	}
	recordName, err := transipRecordName(ch.ResolvedFQDN, config.Zone)
	if err != nil {
		return err
	}

	entries, err := s.readEntries(config.Zone, token)
	if err != nil {
		return err
	}
	for _, entry := range entries {
		if entry.Name == recordName && entry.Type == "TXT" && entry.Content == ch.Key {
			return nil
		}
	}
	entries = append(entries, transipDnsEntry{Name: recordName, Expire: 300, Type: "TXT", Content: ch.Key})
	return s.writeEntries(config.Zone, token, entries)
}

func (s *transipSolver) CleanUp(ch *whapi.ChallengeRequest) error {
	config, err := parseConfig(ch)
	if err != nil {
		return err
	}
	token, err := s.accessToken(ch, config)
	if err != nil {
		return err
	}
	recordName, err := transipRecordName(ch.ResolvedFQDN, config.Zone)
	if err != nil {
		return err
	}

	entries, err := s.readEntries(config.Zone, token)
	if err != nil {
		return err
	}
	filtered := entries[:0]
	for _, entry := range entries {
		if entry.Name == recordName && entry.Type == "TXT" && entry.Content == ch.Key {
			continue
		}
		filtered = append(filtered, entry)
	}
	if len(filtered) == len(entries) {
		return nil
	}
	return s.writeEntries(config.Zone, token, filtered)
}

func parseConfig(ch *whapi.ChallengeRequest) (*solverConfig, error) {
	if ch.Config == nil {
		return nil, errors.New("missing TransIP webhook config")
	}
	var config solverConfig
	if err := json.Unmarshal(ch.Config.Raw, &config); err != nil {
		return nil, err
	}
	config.Zone = strings.Trim(strings.TrimPrefix(strings.TrimSpace(config.Zone), "*."), ".")
	if config.Zone == "" {
		return nil, errors.New("missing TransIP zone")
	}
	if config.TokenKey == "" {
		config.TokenKey = "token"
	}
	return &config, nil
}

func (s *transipSolver) accessToken(ch *whapi.ChallengeRequest, config *solverConfig) (string, error) {
	name := config.SecretName
	namespace := config.SecretNamespace
	key := config.TokenKey
	if config.SecretRef != nil {
		name = config.SecretRef.Name
		key = config.SecretRef.Key
	}
	if namespace == "" {
		namespace = ch.ResourceNamespace
	}
	if name == "" || key == "" {
		return "", errors.New("missing TransIP token secret reference")
	}

	secret, err := s.client.CoreV1().Secrets(namespace).Get(context.Background(), name, metav1.GetOptions{})
	if err != nil {
		return "", err
	}
	authType := secretString(secret, "authType")
	if authType == "" {
		authType = secretString(secret, "auth_type")
	}
	if authType == "" {
		authType = "token"
	}
	if authType == "private_key" {
		return s.accessTokenFromPrivateKey(secret)
	}

	token := strings.TrimSpace(string(secret.Data[key]))
	if token == "" {
		return "", fmt.Errorf("secret %s/%s key %s is empty", namespace, name, key)
	}
	_ = corev1.SecretTypeOpaque
	return token, nil
}

func (s *transipSolver) accessTokenFromPrivateKey(secret *corev1.Secret) (string, error) {
	login := secretString(secret, "login")
	privateKeyPEM := secretString(secret, "private_key")
	if login == "" {
		return "", errors.New("TransIP login is missing")
	}
	if privateKeyPEM == "" {
		return "", errors.New("TransIP private key is missing")
	}
	globalKey := true
	if value := secretString(secret, "global_key"); value != "" {
		parsed, err := strconv.ParseBool(value)
		if err != nil {
			return "", err
		}
		globalKey = parsed
	}

	body := map[string]any{
		"login":           login,
		"nonce":           nonce(),
		"read_only":       false,
		"expiration_time": "30 minutes",
		"label":           "kubarr",
		"global_key":      globalKey,
	}
	bodyBytes, err := json.Marshal(body)
	if err != nil {
		return "", err
	}
	key, err := parsePrivateKey(privateKeyPEM)
	if err != nil {
		return "", err
	}
	digest := sha512.Sum512(bodyBytes)
	signature, err := rsa.SignPKCS1v15(rand.Reader, key, crypto.SHA512, digest[:])
	if err != nil {
		return "", err
	}

	request, err := http.NewRequest(http.MethodPost, "https://api.transip.nl/v6/auth", bytes.NewReader(bodyBytes))
	if err != nil {
		return "", err
	}
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Signature", base64.StdEncoding.EncodeToString(signature))
	response, err := s.http.Do(request)
	if err != nil {
		return "", err
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		responseBody, _ := io.ReadAll(io.LimitReader(response.Body, 512))
		return "", fmt.Errorf("TransIP auth failed: %s %s", response.Status, strings.TrimSpace(string(responseBody)))
	}
	var authResponse struct {
		Token string `json:"token"`
	}
	if err := json.NewDecoder(response.Body).Decode(&authResponse); err != nil {
		return "", err
	}
	if strings.TrimSpace(authResponse.Token) == "" {
		return "", errors.New("TransIP auth response did not include a token")
	}
	return strings.TrimSpace(authResponse.Token), nil
}

func secretString(secret *corev1.Secret, key string) string {
	return strings.TrimSpace(string(secret.Data[key]))
}

func parsePrivateKey(value string) (*rsa.PrivateKey, error) {
	block, _ := pem.Decode([]byte(value))
	if block == nil {
		return nil, errors.New("invalid PEM private key")
	}
	if key, err := x509.ParsePKCS1PrivateKey(block.Bytes); err == nil {
		return key, nil
	}
	key, err := x509.ParsePKCS8PrivateKey(block.Bytes)
	if err != nil {
		return nil, err
	}
	rsaKey, ok := key.(*rsa.PrivateKey)
	if !ok {
		return nil, errors.New("private key is not RSA")
	}
	return rsaKey, nil
}

func nonce() string {
	buffer := make([]byte, 16)
	if _, err := rand.Read(buffer); err != nil {
		return fmt.Sprintf("%d", time.Now().UnixNano())
	}
	return fmt.Sprintf("%x", buffer)
}

func (s *transipSolver) readEntries(zone, token string) ([]transipDnsEntry, error) {
	request, err := http.NewRequest(http.MethodGet, fmt.Sprintf("https://api.transip.nl/v6/domains/%s/dns", zone), nil)
	if err != nil {
		return nil, err
	}
	request.Header.Set("Authorization", "Bearer "+token)
	response, err := s.http.Do(request)
	if err != nil {
		return nil, err
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		body, _ := io.ReadAll(io.LimitReader(response.Body, 512))
		return nil, fmt.Errorf("TransIP DNS read failed: %s %s", response.Status, strings.TrimSpace(string(body)))
	}
	var body transipDnsResponse
	if err := json.NewDecoder(response.Body).Decode(&body); err != nil {
		return nil, err
	}
	return body.DNSEntries, nil
}

func (s *transipSolver) writeEntries(zone, token string, entries []transipDnsEntry) error {
	body, err := json.Marshal(transipDnsResponse{DNSEntries: entries})
	if err != nil {
		return err
	}
	request, err := http.NewRequest(http.MethodPut, fmt.Sprintf("https://api.transip.nl/v6/domains/%s/dns", zone), bytes.NewReader(body))
	if err != nil {
		return err
	}
	request.Header.Set("Authorization", "Bearer "+token)
	request.Header.Set("Content-Type", "application/json")
	response, err := s.http.Do(request)
	if err != nil {
		return err
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		responseBody, _ := io.ReadAll(io.LimitReader(response.Body, 512))
		return fmt.Errorf("TransIP DNS update failed: %s %s", response.Status, strings.TrimSpace(string(responseBody)))
	}
	return nil
}

func transipRecordName(fqdn, zone string) (string, error) {
	fqdn = strings.Trim(strings.ToLower(strings.TrimSpace(fqdn)), ".")
	zone = strings.Trim(strings.ToLower(strings.TrimSpace(zone)), ".")
	if fqdn == zone {
		return "@", nil
	}
	suffix := "." + zone
	if strings.HasSuffix(fqdn, suffix) {
		return strings.TrimSuffix(fqdn, suffix), nil
	}
	return "", fmt.Errorf("FQDN %s is not inside TransIP zone %s", fqdn, zone)
}
