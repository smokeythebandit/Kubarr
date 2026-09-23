#!/usr/bin/env bash
# shellcheck disable=SC1091,SC2034,SC2317
set -Eeuo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
bash -n "$ROOT/tests/acceptance/vpn/lab.sh"
bash -n "$ROOT/tests/acceptance/vpn/server.sh"
# shellcheck source=lab.sh
source "$ROOT/tests/acceptance/vpn/lab.sh"

fail() { return 1; }
assert_status() {
  local expected=$1
  shift
  set +e
  ("$@")
  local actual=$?
  set -e
  [[ $actual == "$expected" ]]
}

present='{"items":[{"spec":{"containers":[{"name":"sonarr"},{"name":"gluetun"}]}}]}'
absent='{"items":[{"spec":{"containers":[{"name":"sonarr"}]}}]}'
malformed='{"items":[]}'
vpn_lab_sidecar_present_json "$present"
assert_status 1 vpn_lab_sidecar_present_json "$absent"
assert_status 1 vpn_lab_sidecar_present_json "$malformed"
vpn_lab_sidecar_absent_json "$absent"
assert_status 1 vpn_lab_sidecar_absent_json "$present"
assert_status 1 vpn_lab_sidecar_absent_json "$malformed"

running='{"items":[{"metadata":{"uid":"uid-a"},"status":{"phase":"Running","containerStatuses":[{"name":"sonarr","state":{"running":{"startedAt":"now"}}}]}}]}'
not_running='{"items":[{"metadata":{"uid":"uid-a"},"status":{"phase":"Running","containerStatuses":[{"name":"sonarr","state":{"terminated":{"exitCode":1}}}]}}]}'
vpn_lab_runtime_matches_json "$running" uid-a
assert_status 1 vpn_lab_runtime_matches_json "$running" uid-b
assert_status 1 vpn_lab_runtime_matches_json "$not_running" uid-a
assert_status 1 vpn_lab_runtime_matches_json '{"items":[]}' uid-a

vpn_lab_counters_grew 10 20 11 21
assert_status 1 vpn_lab_counters_grew 10 20 10 21
assert_status 1 vpn_lab_counters_grew 10 20 11 2
assert_status 1 vpn_lab_counters_grew invalid 20 21 22
vpn_lab_epoch_at_or_after 101 100
vpn_lab_epoch_at_or_after 100 100
assert_status 1 vpn_lab_epoch_at_or_after 99 100
assert_status 1 vpn_lab_epoch_at_or_after invalid 100

vpn_lab_exec_bounded() { printf 'CURL_STATUS=28\n'; }
vpn_lab_sonarr_probe_fails 203.0.113.3 timeout-token
vpn_lab_exec_bounded() { printf 'CURL_STATUS=7\n'; }
vpn_lab_sonarr_probe_fails 203.0.113.3 refused-token
vpn_lab_exec_bounded() { printf 'CURL_STATUS=22\n'; }
assert_status 1 vpn_lab_sonarr_probe_fails 203.0.113.3 http-error-token
vpn_lab_exec_bounded() { return 1; }
assert_status 1 vpn_lab_sonarr_probe_fails 203.0.113.3 transport-error-token
vpn_lab_exec_bounded() { printf 'unexpected output\n'; }
assert_status 1 vpn_lab_sonarr_probe_fails 203.0.113.3 malformed-token

vpn_lab_route_uses_fallback '203.0.113.3 via 10.244.0.1 dev eth0 src 10.244.0.2'
assert_status 1 vpn_lab_route_uses_fallback '203.0.113.3 dev tun0 src 10.66.0.2'
vpn_lab_gluetun_env_matches $'on\n10.244.0.0/16,10.96.0.0/16'
assert_status 1 vpn_lab_gluetun_env_matches $'off\n10.244.0.0/16,10.96.0.0/16'
assert_status 1 vpn_lab_gluetun_env_matches $'on\n10.0.0.0/8'

VPN_PUBLIC_CIDR=203.0.113.0/28
VPN_PRIVATE_CIDR=10.77.0.0/24
vpn_lab_subnets_clear_json '[{"IPAM":{"Config":[{"Subnet":"172.18.0.0/16"}]}}]'
assert_status 1 vpn_lab_subnets_clear_json '[{"IPAM":{"Config":[{"Subnet":"203.0.113.0/24"}]}}]'
assert_status 1 vpn_lab_subnets_clear_json '[{"IPAM":{"Config":[{"Subnet":"10.77.0.0/25"}]}}]'

secret_mode=absent
kubectl() {
  if [[ $1 == get && $2 == pod ]]; then
    printf '%s\n' "$absent"
  elif [[ $1 == get && $2 == secret ]]; then
    [[ $secret_mode == absent ]] && return 0
    return 1
  else
    return 2
  fi
}
vpn_lab_sidecar_removed
secret_mode=transport-error
assert_status 1 vpn_lab_sidecar_removed

owned_context() { return 0; }
resume_pods='{"items":[]}'
resume_exec=0
kubectl() {
  if [[ $1 == get && $2 == pod ]]; then
    printf '%s\n' "$resume_pods"
  elif [[ $1 == exec ]]; then
    [[ $4 == pod/sonarr-owned && $5 == -c && $6 == gluetun && $8 == kill && $9 == -CONT && ${10} == 1 ]] || return 2
    resume_exec=$((resume_exec + 1))
  else
    return 2
  fi
}
VPN_GLUETUN_PAUSED=1
VPN_GLUETUN_PAUSED_UID=uid-owned
vpn_lab_resume_gluetun
[[ $VPN_GLUETUN_PAUSED == 0 && -z $VPN_GLUETUN_PAUSED_UID && $resume_exec == 0 ]]
resume_pods='{"items":[{"metadata":{"name":"sonarr-replacement","uid":"uid-other"}}]}'
VPN_GLUETUN_PAUSED=1
VPN_GLUETUN_PAUSED_UID=uid-owned
vpn_lab_resume_gluetun
[[ $VPN_GLUETUN_PAUSED == 0 && -z $VPN_GLUETUN_PAUSED_UID && $resume_exec == 0 ]]
resume_pods='{"items":[{"metadata":{"name":"sonarr-owned","uid":"uid-owned"}}]}'
VPN_GLUETUN_PAUSED=1
VPN_GLUETUN_PAUSED_UID=uid-owned
vpn_lab_resume_gluetun
[[ $VPN_GLUETUN_PAUSED == 0 && -z $VPN_GLUETUN_PAUSED_UID && $resume_exec == 1 ]]

grep -q 'HEALTH_TARGET_ADDRESSES' "$ROOT/tests/acceptance/vpn/lab.sh"
grep -q '{"name": "HEALTH_ICMP_TARGET_IPS", "value": "0.0.0.0"}' "$ROOT/tests/acceptance/vpn/lab.sh"
grep -q 'PUBLICIP_ENABLED' "$ROOT/tests/acceptance/vpn/lab.sh"
grep -q 'DNS_SERVER' "$ROOT/tests/acceptance/vpn/lab.sh"
grep -q 'iptables -P FORWARD DROP' "$ROOT/tests/acceptance/vpn/server.sh"
if grep -q '10.0.0.0/8' "$ROOT/tests/acceptance/vpn/lab.sh"; then
  exit 1
fi
grep -q 'kubarr-acceptance-vpn-' "$ROOT/tests/acceptance/app-lifecycle.sh"
grep -q 'KUBARR_VPN_ACCEPTANCE_DISPOSABLE' "$ROOT/tests/acceptance/vpn/lab.sh"
grep -q 'kubarr-local' "$ROOT/tests/acceptance/vpn/lab.sh"

printf 'VPN acceptance helper checks passed\n'
