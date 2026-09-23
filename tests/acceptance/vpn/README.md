# Controlled WireGuard acceptance lab

**Validated live on 2026-09-23: PASS, exit status 0, including cleanup.**
See [the evidence record](validated-run-2026-09-23.md).

This opt-in lane extends `../app-lifecycle.sh` with run-owned Docker networks,
two diagnostic HTTP endpoints, and a real kernel WireGuard gateway. It uses the
production Sonarr chart and Gluetun `v3.41.3`, not a diagnostic application
chart. Browser mutations create a custom provider, assign it to Sonarr, remove
it, and delete it. Shell assertions independently verify handshakes, transfer
counters, routed source identity, outage blocking, endpoint event absence, and
recovery.

The lane never uses the default kubeconfig. Its cluster name always starts with
`kubarr-acceptance-vpn-`; the harness rejects any other prefix and explicitly
rejects `kubarr-local`. Cleanup deletes Kubernetes/NFS clients and the owned Kind
node before removing the gateway and run-labeled Docker networks.

## Topology

- Public internal Docker network: `203.0.113.0/28`; gateway `.2`, HTTP endpoint
  `.3`, control client `.4`, owned Kind node `.10`.
- Private internal Docker network: `10.77.0.0/24`; WireGuard gateway `.1`, HTTP
  endpoint `.2`, Docker bridge `.254`.
- WireGuard: `10.66.0.1/24` server and `10.66.0.2/32` Gluetun client.
- Gluetun v3.41.3 names its custom WireGuard kernel interface `tun0`; the image
  has `ip` but no `wg` CLI, so the lane verifies one `tun0` WireGuard link with
  `ip` and obtains handshake/counter evidence from the server.
- Gluetun firewall exemptions: only `10.244.0.0/16,10.96.0.0/16` after those
  exact Kind CIDRs are verified.

The server forwarding policy defaults to drop and permits the peer only to the
two HTTP targets. Gluetun's large health check is TCP to the private endpoint;
its mandatory small ICMP check uses the documented `0.0.0.0` sentinel for the
WireGuard server. Neither Docker network has external connectivity.

After three gateway-outage rounds, the lane stops only Gluetun PID 1, deletes
the owned pod's `tun0`, and requires the public destination to fall back through
`eth0`. With the normal route present, the independent control still succeeds
while Sonarr is blocked and the endpoint records no protected token. Cleanup
explicitly resumes PID 1 if this phase fails.

## Run

The pinned charts revision is `01e45468f233b03b4a7d70a320aec9075f77c213`, containing
Sonarr's `vpn.extraEnv` support. Build the normal acceptance images and CLI, install the
frontend dependencies and Chromium, then run:

```sh
KUBARR_ACCEPTANCE_DISPOSABLE=1 \
KUBARR_VPN_ACCEPTANCE_DISPOSABLE=1 \
KUBARR_ACCEPTANCE_VPN_LAB=1 \
KUBARR_ACCEPTANCE_CHARTS_REV=01e45468f233b03b4a7d70a320aec9075f77c213 \
tests/acceptance/app-lifecycle.sh
```

`vpn-configure` and `vpn-remove` are separate Playwright projects orchestrated
by the shell lane. Credentials and operation results stay in mode-0600 files in
the mode-0700 work directory. Browser traces, screenshots, and video are off.
Browser diagnostic output is also streamed through a fail-closed redactor for
the generated client private key and admin password.

Host prerequisites include loaded `wireguard`, `nfs`, and `nfsd` kernel modules,
`/dev/net/tun`, and sufficient inotify instances for a second cluster. If Kind's
node exits with systemd's `Failed to create control group inotify object: Too many
open files`, increase the host inotify quota before retrying. The local harness
does not change host sysctls automatically. The dedicated GitHub Actions workflow
loads modules and raises the quota on its disposable runner only.

`.github/workflows/vpn-acceptance.yml` supports manual runs and reuses built image
artifacts when called by release CI. Tagged publication requires the VPN job to
succeed alongside the API and frontend acceptance jobs.

## Application guarantees covered

- Assignment/removal changes and redeployment queue entries commit atomically.
- VPN lookup, disabled assigned providers, and Secret creation failures cannot
  silently invoke Helm without protection.
- Managed VPN values cannot be overridden through custom Helm key/value injection.
- Removal explicitly disables reused VPN chart values; the retired managed
  Secret is deleted only after successful deployment and an assignment recheck.
- UI labels say "configured"/"queued", not "connected" based solely on a DB row.

Some of these failure guarantees are covered by regression tests, not injected
into this live happy-path/outage scenario. Provider edits still apply on the next
redeploy; they do not automatically rotate running app credentials.

This lane proves controlled IPv4 WireGuard routing and kill-switch behavior. It
does not claim IPv6 or DNS leak protection, hostile-container isolation, or
arbitrary internet-provider behavior.
