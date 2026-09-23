#!/usr/bin/env bash
# Sourced by app-lifecycle.sh only when KUBARR_ACCEPTANCE_VPN_LAB=1.

VPN_PUBLIC_CIDR=203.0.113.0/28
VPN_PRIVATE_CIDR=10.77.0.0/24
VPN_PUBLIC_ENDPOINT=203.0.113.3
VPN_PRIVATE_ENDPOINT=10.77.0.2
VPN_SERVER_PUBLIC=203.0.113.2
VPN_SERVER_PRIVATE=10.77.0.1
VPN_GLUETUN_IMAGE='qmcgaw/gluetun:v3.41.3'
VPN_GLUETUN_DIGEST='qmcgaw/gluetun@sha256:fa19cc76b2af13d57a8d3dc3066f2ada061b1c761b8aecf989b3877c0486e027'
VPN_LAB_CREATED=0
VPN_GLUETUN_PAUSED=0
VPN_GLUETUN_PAUSED_UID=""

vpn_lab_names() {
  VPN_PUBLIC_NETWORK="kubarr-vpn-public-${RUN_TOKEN,,}"
  VPN_PRIVATE_NETWORK="kubarr-vpn-private-${RUN_TOKEN,,}"
  VPN_SERVER="kubarr-vpn-server-${RUN_TOKEN,,}"
  VPN_PUBLIC_HTTP="kubarr-vpn-public-http-${RUN_TOKEN,,}"
  VPN_PRIVATE_HTTP="kubarr-vpn-private-http-${RUN_TOKEN,,}"
  VPN_CONTROL="kubarr-vpn-control-${RUN_TOKEN,,}"
  VPN_KEYS_VOLUME="kubarr-vpn-keys-${RUN_TOKEN,,}"
  VPN_FIXTURE_IMAGE="kubarr-vpn-fixture:${RUN_TOKEN,,}"
  VPN_CREDENTIALS_FILE="$WORK_DIR/vpn-credentials.json"
  VPN_RESULT_FILE="$WORK_DIR/vpn-results.json"
}

vpn_lab_require_owned_container() {
  local name=$1
  [[ $(docker inspect --format '{{index .Config.Labels "kubarr.acceptance.run"}}' "$name" 2>/dev/null || true) == "$RUN_TOKEN" ]] ||
    fail "refusing unowned VPN lab container: $name"
}

vpn_lab_require_owned_network() {
  local name=$1
  [[ $(docker network inspect --format '{{index .Labels "kubarr.acceptance.run"}}' "$name" 2>/dev/null || true) == "$RUN_TOKEN" ]] ||
    fail "refusing unowned VPN lab network: $name"
}

vpn_lab_require_owned_volume() {
  local name=$1
  [[ $(docker volume inspect --format '{{index .Labels "kubarr.acceptance.run"}}' "$name" 2>/dev/null || true) == "$RUN_TOKEN" ]] ||
    fail "refusing unowned VPN lab volume: $name"
}

vpn_lab_subnets_clear_json() {
  local network_json=$1
  NETWORK_JSON=$network_json python3 - "$VPN_PUBLIC_CIDR" "$VPN_PRIVATE_CIDR" <<'PY'
import ipaddress
import json
import os
import sys

targets = [ipaddress.ip_network(value) for value in sys.argv[1:]]
for network in json.loads(os.environ["NETWORK_JSON"]):
    for config in network.get("IPAM", {}).get("Config", []) or []:
        subnet = config.get("Subnet")
        if not subnet:
            continue
        existing = ipaddress.ip_network(subnet, strict=False)
        if any(existing.overlaps(target) for target in targets):
            raise SystemExit(1)
PY
}

vpn_lab_assert_subnets_unused() {
  local network_json
  local -a network_ids
  mapfile -t network_ids < <(docker network ls -q)
  ((${#network_ids[@]} > 0)) || return
  network_json=$(docker network inspect "${network_ids[@]}") || return
  vpn_lab_subnets_clear_json "$network_json" ||
    fail "VPN lab subnet overlaps an existing Docker network; refusing to alter existing networks"
}

vpn_lab_setup() {
  require_owned_context
  [[ $CLUSTER == kubarr-acceptance-vpn-* && $CLUSTER != kubarr-local ]] ||
    fail "VPN lab requires an owned kubarr-acceptance-vpn-* cluster"
  [[ $KUBECONFIG == "$WORK_DIR/kubeconfig" ]] || fail "VPN lab kubeconfig must be inside its private work directory"
  [[ ${KUBARR_VPN_ACCEPTANCE_DISPOSABLE:-} == 1 ]] ||
    fail "set KUBARR_VPN_ACCEPTANCE_DISPOSABLE=1 to authorize the WireGuard lab"
  [[ -e /dev/net/tun ]] || fail "/dev/net/tun is required"
  [[ -d /sys/module/wireguard ]] || grep -qw wireguard /proc/modules ||
    fail "the host WireGuard kernel module must already be loaded"

  local cluster_networking
  cluster_networking=$(kubectl get configmap kubeadm-config -n kube-system -o jsonpath='{.data.ClusterConfiguration}' |
    python3 -c 'import sys,yaml; n=yaml.safe_load(sys.stdin)["networking"]; print(n["podSubnet"] + "," + n["serviceSubnet"])') ||
    fail "could not read owned Kind cluster networking"
  [[ $cluster_networking == '10.244.0.0/16,10.96.0.0/16' ]] ||
    fail "VPN lab requires Kind pod/service CIDRs 10.244.0.0/16 and 10.96.0.0/16, got $cluster_networking"

  vpn_lab_names
  vpn_lab_assert_subnets_unused
  mkdir -m 700 "$WORK_DIR/vpn-secrets"
  if docker image inspect "$VPN_FIXTURE_IMAGE" >/dev/null 2>&1; then
    fail "refusing to overwrite existing VPN fixture image: $VPN_FIXTURE_IMAGE"
  fi
  docker build --pull --label "kubarr.acceptance.run=$RUN_TOKEN" --tag "$VPN_FIXTURE_IMAGE" \
    "$APP_ROOT/tests/acceptance/vpn" >/dev/null

  docker run --rm --user "$(id -u):$(id -g)" --label "kubarr.acceptance.run=$RUN_TOKEN" \
    -v "$WORK_DIR/vpn-secrets:/out" "$VPN_FIXTURE_IMAGE" sh -ec '
      umask 077
      wg genkey > /out/server-private
      wg pubkey < /out/server-private > /out/server-public
      wg genkey > /out/client-private
      wg pubkey < /out/client-private > /out/client-public
    '
  [[ $(stat -c '%a' "$WORK_DIR/vpn-secrets/server-private") == 600 &&
     $(stat -c '%a' "$WORK_DIR/vpn-secrets/client-private") == 600 ]] ||
    fail "VPN private keys must be mode 600"

  VPN_LAB_CREATED=1
  docker volume create --label "kubarr.acceptance.run=$RUN_TOKEN" "$VPN_KEYS_VOLUME" >/dev/null
  tar -C "$WORK_DIR/vpn-secrets" -cf - server-private client-public |
    docker run --rm -i --label "kubarr.acceptance.run=$RUN_TOKEN" \
      -v "$VPN_KEYS_VOLUME:/keys" "$VPN_FIXTURE_IMAGE" sh -ec '
        umask 077
        tar -xf - -C /keys
        chown 0:0 /keys/server-private /keys/client-public
        chmod 0600 /keys/server-private /keys/client-public
      '
  docker network create --internal --subnet "$VPN_PUBLIC_CIDR" --gateway 203.0.113.1 \
    --label "kubarr.acceptance.run=$RUN_TOKEN" "$VPN_PUBLIC_NETWORK" >/dev/null
  docker network create --internal --subnet "$VPN_PRIVATE_CIDR" --gateway 10.77.0.254 \
    --label "kubarr.acceptance.run=$RUN_TOKEN" "$VPN_PRIVATE_NETWORK" >/dev/null
  docker run -d --name "$VPN_PUBLIC_HTTP" --network "$VPN_PUBLIC_NETWORK" --ip "$VPN_PUBLIC_ENDPOINT" \
    --label "kubarr.acceptance.run=$RUN_TOKEN" --read-only --tmpfs /tmp:rw,noexec,nosuid,size=1m --cap-drop ALL \
    -e RUN_TOKEN="$RUN_TOKEN" -e ENDPOINT_NAME=public "$VPN_FIXTURE_IMAGE" /usr/local/bin/endpoint.sh >/dev/null
  docker run -d --name "$VPN_PRIVATE_HTTP" --network "$VPN_PRIVATE_NETWORK" --ip "$VPN_PRIVATE_ENDPOINT" \
    --label "kubarr.acceptance.run=$RUN_TOKEN" --read-only --tmpfs /tmp:rw,noexec,nosuid,size=1m --cap-drop ALL \
    -e RUN_TOKEN="$RUN_TOKEN" -e ENDPOINT_NAME=private "$VPN_FIXTURE_IMAGE" /usr/local/bin/endpoint.sh >/dev/null
  docker run -d --name "$VPN_SERVER" --network "$VPN_PUBLIC_NETWORK" --ip "$VPN_SERVER_PUBLIC" \
    --label "kubarr.acceptance.run=$RUN_TOKEN" --cap-drop ALL --cap-add NET_ADMIN \
    --device /dev/net/tun --sysctl net.ipv4.conf.all.src_valid_mark=1 --sysctl net.ipv4.ip_forward=1 \
    -v "$VPN_KEYS_VOLUME:/run/kubarr-vpn:ro" "$VPN_FIXTURE_IMAGE" >/dev/null
  docker network connect --ip "$VPN_SERVER_PRIVATE" "$VPN_PRIVATE_NETWORK" "$VPN_SERVER"
  docker run -d --name "$VPN_CONTROL" --network "$VPN_PUBLIC_NETWORK" --ip 203.0.113.4 \
    --label "kubarr.acceptance.run=$RUN_TOKEN" --read-only --cap-drop ALL "$VPN_FIXTURE_IMAGE" sleep infinity >/dev/null
  docker network connect --ip 203.0.113.10 "$VPN_PUBLIC_NETWORK" "$CLUSTER-control-plane"

  local private_key public_key
  private_key=$(<"$WORK_DIR/vpn-secrets/client-private")
  public_key=$(<"$WORK_DIR/vpn-secrets/server-public")
  jq -n --arg provider_name "acceptance-$RUN_TOKEN-wireguard" --arg private_key "$private_key" \
    --arg public_key "$public_key" --arg endpoint_ip "$VPN_SERVER_PUBLIC" \
    '{provider_name:$provider_name,private_key:$private_key,public_key:$public_key,
      addresses:["10.66.0.2/32"],endpoint_ip:$endpoint_ip,endpoint_port:51820,
      firewall_outbound_subnets:"10.244.0.0/16,10.96.0.0/16"}' >"$VPN_CREDENTIALS_FILE"
  chmod 600 "$VPN_CREDENTIALS_FILE"
  printf '{}\n' >"$VPN_RESULT_FILE"
  chmod 600 "$VPN_RESULT_FILE"

  vpn_lab_control_probe setup
}

vpn_lab_patch_chart() {
  local chart=$1
  grep -q 'extraEnv:' "$chart/values.yaml" ||
    fail "Sonarr chart must provide vpn.extraEnv; update the pinned charts revision"
  grep -q 'Values.vpn.extraEnv' "$chart/templates/deployment.yaml" ||
    fail "Sonarr deployment must render vpn.extraEnv"
  python3 - "$chart/values.yaml" <<'PY'
import sys
import yaml

path = sys.argv[1]
with open(path, encoding="utf-8") as stream:
    values = yaml.safe_load(stream)
values["vpn"]["extraEnv"] = [
    {"name": "HEALTH_TARGET_ADDRESSES", "value": "10.77.0.2:8080"},
    {"name": "HEALTH_ICMP_TARGET_IPS", "value": "0.0.0.0"},
    {"name": "PUBLICIP_ENABLED", "value": "no"},
    {"name": "DNS_SERVER", "value": "off"},
]
with open(path, "w", encoding="utf-8") as stream:
    yaml.safe_dump(values, stream, sort_keys=False)
PY
}

vpn_lab_verify_gluetun_image() {
  docker image inspect "$VPN_GLUETUN_IMAGE" --format '{{json .RepoDigests}}' |
    jq -e --arg digest "$VPN_GLUETUN_DIGEST" 'index($digest) != null' >/dev/null ||
    fail "Gluetun v3.41.3 does not match the acceptance-pinned digest"
}

vpn_lab_run_browser() {
  local project=$1
  (
    set -o pipefail
    cd "$APP_ROOT/code/frontend" || return
    KUBARR_VPN_REAL_ACCEPTANCE=1 BASE_URL="http://127.0.0.1:$GATEWAY_PORT" \
      TEST_USERNAME="$ADMIN_USER" TEST_PASSWORD="$ADMIN_PASSWORD" ACCEPTANCE_RUN_ID="$RUN_TOKEN" \
      VPN_LAB_CREDENTIALS_FILE="$VPN_CREDENTIALS_FILE" ACCEPTANCE_RESULT_FILE="$VPN_RESULT_FILE" \
      node node_modules/@playwright/test/cli.js test --config playwright.vpn.config.ts --project "$project" 2>&1 |
      VPN_LAB_CREDENTIALS_FILE="$VPN_CREDENTIALS_FILE" ADMIN_PASSWORD="$ADMIN_PASSWORD" \
        python3 "$APP_ROOT/tests/acceptance/vpn/redact_output.py"
  )
}

vpn_lab_control_probe() {
  local phase=$1
  local token="control-$phase-$RUN_TOKEN"
  vpn_lab_require_owned_container "$VPN_CONTROL"
  docker exec "$VPN_CONTROL" curl --fail --silent --show-error --max-time 5 --noproxy '*' \
    "http://$VPN_PUBLIC_ENDPOINT:8080/?token=$token" |
    jq -e --arg run "$RUN_TOKEN" '.run == $run and .endpoint == "public" and .peer == "203.0.113.4"' >/dev/null
}

vpn_lab_sonarr_probe() {
  local address=$1 endpoint=$2 peer=$3 token=$4 output
  output=$(kubectl exec -n sonarr deployment/sonarr -c sonarr -- curl --fail --silent --show-error \
    --connect-timeout 3 --max-time 5 --noproxy '*' "http://$address:8080/?token=$token") || return
  jq -e --arg run "$RUN_TOKEN" --arg endpoint "$endpoint" --arg peer "$peer" --arg token "$token" \
    '.run == $run and .endpoint == $endpoint and .peer == $peer and .token == $token' <<<"$output" >/dev/null
}

vpn_lab_exec_bounded() {
  timeout 10s kubectl "$@"
}

vpn_lab_curl_status_is_blocked() {
  [[ $1 == 7 || $1 == 28 ]]
}

vpn_lab_route_uses_fallback() {
  [[ $1 == *' dev eth0 '* || $1 == *' dev eth0' ]]
}

vpn_lab_gluetun_env_matches() {
  [[ $1 == $'on\n10.244.0.0/16,10.96.0.0/16' ]]
}

vpn_lab_sonarr_probe_fails() {
  local output status
  # shellcheck disable=SC2016 # $1 and $2 are intentionally expanded by the remote shell.
  output=$(vpn_lab_exec_bounded exec -n sonarr deployment/sonarr -c sonarr -- sh -c '
    curl --fail --silent --connect-timeout 3 --max-time 5 --noproxy "*" "$1" >/dev/null 2>&1
    status=$?
    printf "CURL_STATUS=%s\n" "$status"
    exit 0
  ' sh "http://$1:8080/?token=$2") || return 1
  [[ $output =~ ^CURL_STATUS=([0-9]+)$ ]] || return 1
  status=${BASH_REMATCH[1]}
  vpn_lab_curl_status_is_blocked "$status"
}

vpn_lab_event_absent() {
  local container=$1 token=$2
  vpn_lab_require_owned_container "$container"
  docker exec "$container" curl --fail --silent --max-time 5 http://127.0.0.1:8080/events |
    jq -e --arg token "$token" 'all(.events[]; .token != $token)' >/dev/null
}

vpn_lab_handshake_after() {
  local epoch=$1 latest
  latest=$(docker exec "$VPN_SERVER" wg show wg0 latest-handshakes | awk 'NR == 1 {print $2}') || return
  vpn_lab_epoch_at_or_after "$latest" "$epoch"
}

vpn_lab_epoch_at_or_after() {
  local actual=$1 epoch=$2
  [[ $actual =~ ^[0-9]+$ && $epoch =~ ^[0-9]+$ ]] && (( actual >= epoch && actual > 0 ))
}

vpn_lab_gluetun_interface_ready() {
  local interfaces
  interfaces=$(kubectl exec -n sonarr deployment/sonarr -c gluetun -- \
    sh -c 'ip -o link show type wireguard | cut -d: -f2 | tr -d " "') || return
  [[ $interfaces == tun0 ]]
}

vpn_lab_transfer_counters() {
  docker exec "$VPN_SERVER" wg show wg0 transfer | awk 'NR == 1 {print $2, $3}'
}

vpn_lab_counters_grew() {
  local before_rx=$1 before_tx=$2 after_rx=$3 after_tx=$4
  [[ $before_rx =~ ^[0-9]+$ && $before_tx =~ ^[0-9]+$ &&
     $after_rx =~ ^[0-9]+$ && $after_tx =~ ^[0-9]+$ ]] &&
    (( after_rx > before_rx && after_tx > before_tx ))
}

vpn_lab_sonarr_runtime_matches() {
  local expected_uid=$1 pods
  pods=$(kubectl get pod -n sonarr -l app.kubernetes.io/name=sonarr -o json) || return
  vpn_lab_runtime_matches_json "$pods" "$expected_uid"
}

vpn_lab_runtime_matches_json() {
  local pods=$1 expected_uid=$2
  jq -e --arg uid "$expected_uid" '
    (.items | length) == 1 and
    .items[0].metadata.uid == $uid and
    .items[0].status.phase == "Running" and
    (.items[0].status.containerStatuses |
      any(.name == "sonarr" and .state.running != null))
  ' <<<"$pods" >/dev/null
}

vpn_lab_sidecar_present_json() {
  jq -e '(.items | length) == 1 and
    (.items[0].spec.containers | any(.name == "gluetun"))' <<<"$1" >/dev/null
}

vpn_lab_sidecar_absent_json() {
  jq -e '(.items | length) == 1 and
    (.items[0].spec.containers | all(.name != "gluetun"))' <<<"$1" >/dev/null
}

vpn_lab_sidecar_ready() {
  local pods
  pods=$(kubectl get pod -n sonarr -l app.kubernetes.io/name=sonarr -o json) || return
  vpn_lab_sidecar_present_json "$pods" && vpn_lab_gluetun_interface_ready &&
    kubectl wait -n sonarr --for=condition=Ready pod -l app.kubernetes.io/name=sonarr --timeout=5s >/dev/null 2>&1
}

vpn_lab_sidecar_removed() {
  local pods secret
  pods=$(kubectl get pod -n sonarr -l app.kubernetes.io/name=sonarr -o json) || return
  vpn_lab_sidecar_absent_json "$pods" || return
  secret=$(kubectl get secret vpn-sonarr -n sonarr --ignore-not-found -o name) || return
  [[ -z $secret ]]
}

vpn_lab_resume_gluetun() {
  (( VPN_GLUETUN_PAUSED == 1 )) || return 0
  owned_context || return 1
  local pod_name pods
  pods=$(kubectl get pod -n sonarr -l app.kubernetes.io/name=sonarr -o json) || return
  pod_name=$(jq -r --arg uid "$VPN_GLUETUN_PAUSED_UID" \
    '[.items[] | select(.metadata.uid == $uid) | .metadata.name] | if length == 1 then .[0] else empty end' \
    <<<"$pods") || return
  if [[ -z $pod_name ]]; then
    VPN_GLUETUN_PAUSED=0
    VPN_GLUETUN_PAUSED_UID=""
    return 0
  fi
  kubectl exec -n sonarr "pod/$pod_name" -c gluetun -- kill -CONT 1 >/dev/null || return
  VPN_GLUETUN_PAUSED=0
  VPN_GLUETUN_PAUSED_UID=""
}

vpn_lab_run_scenario() {
  local before_uid assigned_uid assignment_epoch restart_epoch removed_public
  local before_rx before_tx after_rx after_tx round outage_public outage_private
  before_uid=$(kubectl get pod -n sonarr -l app.kubernetes.io/name=sonarr -o jsonpath='{.items[0].metadata.uid}')
  vpn_lab_sonarr_probe "$VPN_PUBLIC_ENDPOINT" public 203.0.113.10 "baseline-public-$RUN_TOKEN"
  local baseline_private="baseline-private-$RUN_TOKEN"
  vpn_lab_sonarr_probe_fails "$VPN_PRIVATE_ENDPOINT" "$baseline_private" || fail "private endpoint was reachable before VPN assignment"
  vpn_lab_event_absent "$VPN_PRIVATE_HTTP" "$baseline_private" || fail "private endpoint recorded baseline protected traffic"
  say "VPN phase=baseline sonarr_uid=$before_uid public_identity=203.0.113.10 private=unreachable"

  assignment_epoch=$(date +%s)
  vpn_lab_run_browser vpn-configure
  jq -e '.vpn_provider_id | strings | length > 0' "$VPN_RESULT_FILE" >/dev/null
  jq -e '.vpn_assign_id | strings | test("^[0-9a-fA-F-]{36}$")' "$VPN_RESULT_FILE" >/dev/null
  wait_for 180 "Sonarr VPN sidecar readiness" vpn_lab_sidecar_ready
  assigned_uid=$(kubectl get pod -n sonarr -l app.kubernetes.io/name=sonarr -o jsonpath='{.items[0].metadata.uid}')
  [[ $assigned_uid != "$before_uid" ]] || fail "VPN assignment did not replace the Sonarr pod"
  wait_for 60 "assignment WireGuard handshake" vpn_lab_handshake_after "$assignment_epoch"
  read -r before_rx before_tx < <(vpn_lab_transfer_counters)
  vpn_lab_sonarr_probe "$VPN_PUBLIC_ENDPOINT" public "$VPN_SERVER_PUBLIC" "tunnel-public-1-$RUN_TOKEN"
  vpn_lab_sonarr_probe "$VPN_PRIVATE_ENDPOINT" private "$VPN_SERVER_PRIVATE" "tunnel-private-1-$RUN_TOKEN"
  vpn_lab_sonarr_probe "$VPN_PUBLIC_ENDPOINT" public "$VPN_SERVER_PUBLIC" "tunnel-public-2-$RUN_TOKEN"
  vpn_lab_sonarr_probe "$VPN_PRIVATE_ENDPOINT" private "$VPN_SERVER_PRIVATE" "tunnel-private-2-$RUN_TOKEN"
  read -r after_rx after_tx < <(vpn_lab_transfer_counters)
  vpn_lab_counters_grew "$before_rx" "$before_tx" "$after_rx" "$after_tx" ||
    fail "WireGuard receive and transmit counters did not both grow after assignment"
  say "VPN phase=assigned interface=tun0 server_peer=$VPN_SERVER_PUBLIC private_peer=$VPN_SERVER_PRIVATE rx_delta=$((after_rx-before_rx)) tx_delta=$((after_tx-before_tx))"

  vpn_lab_require_owned_container "$VPN_SERVER"
  docker stop --time 5 "$VPN_SERVER" >/dev/null
  for round in 1 2 3; do
    vpn_lab_sonarr_runtime_matches "$assigned_uid" || fail "Sonarr runtime changed during outage round $round"
    vpn_lab_control_probe "outage-$round"
    outage_public="outage-public-$round-$RUN_TOKEN"
    outage_private="outage-private-$round-$RUN_TOKEN"
    vpn_lab_sonarr_probe_fails "$VPN_PUBLIC_ENDPOINT" "$outage_public" || fail "public traffic was not conclusively blocked in outage round $round"
    vpn_lab_sonarr_probe_fails "$VPN_PRIVATE_ENDPOINT" "$outage_private" || fail "private traffic was not conclusively blocked in outage round $round"
    vpn_lab_event_absent "$VPN_PUBLIC_HTTP" "$outage_public" || fail "public endpoint recorded protected outage traffic in round $round"
    vpn_lab_event_absent "$VPN_PRIVATE_HTTP" "$outage_private" || fail "private endpoint recorded protected outage traffic in round $round"
    say "VPN phase=outage round=$round sonarr_uid=$assigned_uid control=reachable protected_public=blocked protected_private=blocked"
  done

  local gluetun_env route_loss_token route_loss_route
  vpn_lab_sonarr_runtime_matches "$assigned_uid" || fail "Sonarr runtime changed before route-loss phase"
  # Expand the VPN environment inside the target container, not this shell.
  # shellcheck disable=SC2016
  gluetun_env=$(kubectl exec -n sonarr deployment/sonarr -c gluetun -- sh -c \
    'printf "%s\n%s" "$FIREWALL" "$FIREWALL_OUTBOUND_SUBNETS"') ||
    fail "could not read Gluetun firewall environment"
  vpn_lab_gluetun_env_matches "$gluetun_env" || fail "Gluetun firewall environment does not match the strict lab contract"
  kubectl exec -n sonarr deployment/sonarr -c gluetun -- kill -STOP 1 >/dev/null ||
    fail "could not stop Gluetun PID 1 for route-loss phase"
  VPN_GLUETUN_PAUSED=1
  VPN_GLUETUN_PAUSED_UID=$assigned_uid
  kubectl exec -n sonarr deployment/sonarr -c gluetun -- ip link delete tun0 >/dev/null ||
    fail "could not remove the owned pod WireGuard interface"
  route_loss_route=$(kubectl exec -n sonarr deployment/sonarr -c gluetun -- \
    ip route get "$VPN_PUBLIC_ENDPOINT") || fail "public endpoint has no fallback route after tun0 deletion"
  vpn_lab_route_uses_fallback "$route_loss_route" ||
    fail "public endpoint did not fall back to eth0 after tun0 deletion: $route_loss_route"
  vpn_lab_sonarr_runtime_matches "$assigned_uid" || fail "Sonarr runtime changed during route-loss phase"
  vpn_lab_control_probe route-loss
  route_loss_token="route-loss-public-$RUN_TOKEN"
  vpn_lab_sonarr_probe_fails "$VPN_PUBLIC_ENDPOINT" "$route_loss_token" ||
    fail "public traffic was not conclusively blocked with the normal route restored"
  vpn_lab_event_absent "$VPN_PUBLIC_HTTP" "$route_loss_token" ||
    fail "public endpoint recorded route-loss protected traffic"
  say "VPN phase=route-loss sonarr_uid=$assigned_uid route=$(tr '\n' ' ' <<<"$route_loss_route") firewall=on protected_public=blocked control=reachable event=absent"
  vpn_lab_resume_gluetun || fail "could not resume Gluetun after route-loss phase"

  restart_epoch=$(date +%s)
  docker start "$VPN_SERVER" >/dev/null
  wait_for 60 "WireGuard recovery handshake" vpn_lab_handshake_after "$restart_epoch"
  read -r before_rx before_tx < <(vpn_lab_transfer_counters)
  vpn_lab_sonarr_probe "$VPN_PUBLIC_ENDPOINT" public "$VPN_SERVER_PUBLIC" "restored-public-1-$RUN_TOKEN"
  vpn_lab_sonarr_probe "$VPN_PRIVATE_ENDPOINT" private "$VPN_SERVER_PRIVATE" "restored-private-1-$RUN_TOKEN"
  vpn_lab_sonarr_probe "$VPN_PUBLIC_ENDPOINT" public "$VPN_SERVER_PUBLIC" "restored-public-2-$RUN_TOKEN"
  vpn_lab_sonarr_probe "$VPN_PRIVATE_ENDPOINT" private "$VPN_SERVER_PRIVATE" "restored-private-2-$RUN_TOKEN"
  read -r after_rx after_tx < <(vpn_lab_transfer_counters)
  vpn_lab_counters_grew "$before_rx" "$before_tx" "$after_rx" "$after_tx" ||
    fail "WireGuard receive and transmit counters did not both grow after restart"
  vpn_lab_control_probe restored
  say "VPN phase=recovered handshake_epoch=$restart_epoch rx_delta=$((after_rx-before_rx)) tx_delta=$((after_tx-before_tx)) identities=$VPN_SERVER_PUBLIC,$VPN_SERVER_PRIVATE"

  vpn_lab_run_browser vpn-remove
  jq -e '.vpn_remove_id | strings | test("^[0-9a-fA-F-]{36}$")' "$VPN_RESULT_FILE" >/dev/null
  wait_for 180 "Sonarr VPN sidecar removal" vpn_lab_sidecar_removed
  removed_public="removed-public-$RUN_TOKEN"
  vpn_lab_sonarr_probe "$VPN_PUBLIC_ENDPOINT" public 203.0.113.10 "$removed_public"
  local removed_private="removed-private-$RUN_TOKEN"
  vpn_lab_sonarr_probe_fails "$VPN_PRIVATE_ENDPOINT" "$removed_private" || fail "private endpoint remained reachable after VPN removal"
  vpn_lab_event_absent "$VPN_PRIVATE_HTTP" "$removed_private" || fail "private endpoint recorded traffic after VPN removal"
  say "VPN phase=removed public_identity=203.0.113.10 private=unreachable sidecar=absent secret=absent"
}

vpn_lab_cleanup() {
  (( VPN_LAB_CREATED == 1 )) || return 0
  [[ ${CLUSTER:-} != kubarr-local && ${CLUSTER:-} == kubarr-acceptance-vpn-* ]] || {
    printf '[acceptance] ERROR: refusing VPN lab cleanup for cluster name %s\n' "${CLUSTER:-unset}" >&2
    return 1
  }
  vpn_lab_names
  local name
  for name in "$VPN_CONTROL" "$VPN_PRIVATE_HTTP" "$VPN_PUBLIC_HTTP" "$VPN_SERVER"; do
    if docker inspect "$name" >/dev/null 2>&1; then
      vpn_lab_require_owned_container "$name" || return
      timeout 20s docker rm -f "$name" >/dev/null || return
    fi
  done
  for name in "$VPN_PRIVATE_NETWORK" "$VPN_PUBLIC_NETWORK"; do
    if docker network inspect "$name" >/dev/null 2>&1; then
      vpn_lab_require_owned_network "$name" || return
      timeout 20s docker network rm "$name" >/dev/null || return
    fi
  done
  if docker volume inspect "$VPN_KEYS_VOLUME" >/dev/null 2>&1; then
    vpn_lab_require_owned_volume "$VPN_KEYS_VOLUME" || return
    timeout 20s docker volume rm "$VPN_KEYS_VOLUME" >/dev/null || return
  fi
  if docker image inspect "$VPN_FIXTURE_IMAGE" >/dev/null 2>&1; then
    [[ $(docker image inspect --format '{{index .Config.Labels "kubarr.acceptance.run"}}' "$VPN_FIXTURE_IMAGE") == "$RUN_TOKEN" ]] ||
      fail "refusing unowned VPN fixture image: $VPN_FIXTURE_IMAGE"
    docker image rm "$VPN_FIXTURE_IMAGE" >/dev/null || return
  fi
}
