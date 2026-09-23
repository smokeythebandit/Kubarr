#!/usr/bin/env bash
set -Eeuo pipefail

CHARTS_REV=${KUBARR_ACCEPTANCE_CHARTS_REV:-01e45468f233b03b4a7d70a320aec9075f77c213}
APP_ROOT=${KUBARR_ACCEPTANCE_APP_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}
CHARTS_REPO=${KUBARR_ACCEPTANCE_CHARTS_DIR:-"$APP_ROOT/../kubarr-charts"}
CHARTS_DIR=""
CLI=${KUBARR_ACCEPTANCE_CLI:-"$APP_ROOT/code/cli/target/debug/kubarr"}
IMAGE_TAG=${KUBARR_ACCEPTANCE_IMAGE_TAG:-acceptance-local}
INSTALL_TIMEOUT=${KUBARR_ACCEPTANCE_INSTALL_TIMEOUT:-5m}
APP_TIMEOUT=${KUBARR_ACCEPTANCE_APP_TIMEOUT:-10m}
FRONTEND_ACCEPTANCE=${KUBARR_ACCEPTANCE_FRONTEND:-0}
VPN_LAB_ACCEPTANCE=${KUBARR_ACCEPTANCE_VPN_LAB:-0}
GATEWAY_PORT=${KUBARR_ACCEPTANCE_GATEWAY_PORT:-$((18000 + $$ % 10000))}
RUN_TOKEN=${KUBARR_ACCEPTANCE_RUN_ID:-"local-$$-$(date +%s)"}
RUN_TOKEN=$(printf '%s' "$RUN_TOKEN" | tr -c 'a-zA-Z0-9-' '-' | cut -c1-30)
if [[ $VPN_LAB_ACCEPTANCE == 1 ]]; then
  CLUSTER="kubarr-acceptance-vpn-${RUN_TOKEN,,}"
else
  CLUSTER="kubarr-acceptance-${RUN_TOKEN,,}"
fi
REGISTRY="kubarr-acceptance-registry-${RUN_TOKEN,,}"
WORK_DIR=${KUBARR_ACCEPTANCE_WORK_DIR:-"${TMPDIR:-/tmp}/$CLUSTER"}
export KUBECONFIG="$WORK_DIR/kubeconfig"
NATIVE_PLATFORM=""

CREATED_CLUSTER=0
CREATED_REGISTRY=0
CREATED_WORK_DIR=0
LIFECYCLE_PASSED=0
PORT_FORWARD_PID=""
COOKIE_JAR=""
COMPONENT_POD_SNAPSHOTS='{}'
DRAIN_LOG_FOLLOWER_PID=""
DRAIN_LOG_FILE=""
ACCEPTANCE_RESULT_FILE=""

if [[ $VPN_LAB_ACCEPTANCE == 1 ]]; then
  # shellcheck source=vpn/lab.sh
  # The path is resolved from the explicitly selected application checkout.
  # shellcheck disable=SC1091
  source "$APP_ROOT/tests/acceptance/vpn/lab.sh"
fi

say() { printf '[acceptance] %s\n' "$*"; }
fail() { printf '[acceptance] ERROR: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || fail "required tool not found: $1"; }
on_err() {
  local status=$1 line=$2
  printf '[acceptance] ERROR: command failed (status %s, line %s)\n' "$status" "$line" >&2
  return "$status"
}

native_docker_platform() {
  local architecture
  architecture=$(docker info --format '{{.Architecture}}') || fail "could not determine Docker host architecture"
  case "$architecture" in
    amd64|x86_64) printf 'linux/amd64\n' ;;
    arm64|aarch64) printf 'linux/arm64\n' ;;
    *) fail "unsupported Docker host architecture: $architecture" ;;
  esac
}

load_image_into_kind() {
  local image=$1 archive
  archive=$(mktemp "$WORK_DIR/kind-image.XXXXXX.tar") || return
  docker image save --platform="$NATIVE_PLATFORM" --output "$archive" "$image" || return
  kind load image-archive --name "$CLUSTER" "$archive" || return
  rm -f "$archive"
}

owned_context() {
  [[ -f "$KUBECONFIG" ]] || return 1
  [[ $(kubectl config current-context 2>/dev/null || true) == "kind-$CLUSTER" ]]
}

require_owned_context() {
  owned_context || fail "refusing cluster mutation outside owned context kind-$CLUSTER"
}

diagnostics() {
  (( CREATED_CLUSTER == 1 )) || return 0
  owned_context || return 0
  say "bounded failure diagnostics"
  timeout 15s kubectl get pods -A -o wide 2>/dev/null || true
  timeout 15s kubectl get events -A --sort-by=.lastTimestamp --field-selector type=Warning 2>/dev/null | tail -80 || true
}

owned_kind_node() {
  local node="$CLUSTER-control-plane"
  [[ $(docker inspect --format '{{index .Config.Labels "io.x-k8s.kind.cluster"}}' "$node" 2>/dev/null || true) == "$CLUSTER" ]] &&
    [[ $(docker inspect --format '{{index .Config.Labels "io.x-k8s.kind.role"}}' "$node" 2>/dev/null || true) == control-plane ]]
}

delete_owned_cluster() {
  local namespace_json
  local -a namespaces
  if ! docker inspect "$CLUSTER-control-plane" >/dev/null 2>&1; then
    return 0
  fi
  owned_kind_node || {
    printf '[acceptance] ERROR: refusing to delete unowned Kind node %s-control-plane\n' "$CLUSTER" >&2
    return 1
  }
  owned_context || {
    printf '[acceptance] ERROR: cannot safely drain clients without owned context kind-%s\n' "$CLUSTER" >&2
    return 1
  }

  namespace_json=$(kubectl get namespaces -o json) || return
  mapfile -t namespaces < <(jq -r \
    '.items[].metadata.name | select(. != "default" and . != "kube-node-lease" and
      . != "kube-public" and . != "kube-system" and . != "kubarr-storage")' <<<"$namespace_json")
  if (( ${#namespaces[@]} > 0 )); then
    say "deleting NFS client namespaces before managed-nfs"
    # Namespace completion keeps the NFS server available through pod volume teardown.
    timeout 120s kubectl delete namespace "${namespaces[@]}" --wait=true --timeout=110s || return
  fi

  say "deleting managed-nfs after client teardown"
  timeout 90s kubectl delete namespace kubarr-storage --ignore-not-found --wait=true --timeout=80s || return
  timeout 120s kind delete cluster --name "$CLUSTER" || return
  ! docker inspect "$CLUSTER-control-plane" >/dev/null 2>&1
}

cleanup() {
  local status=$? cleanup_status=0
  trap - EXIT INT TERM
  set +e
  if [[ -n "$DRAIN_LOG_FOLLOWER_PID" ]]; then
    kill "$DRAIN_LOG_FOLLOWER_PID" 2>/dev/null || true
    wait "$DRAIN_LOG_FOLLOWER_PID" 2>/dev/null || true
    DRAIN_LOG_FOLLOWER_PID=""
  fi
  if [[ -n "$DRAIN_LOG_FILE" ]]; then
    rm -f "$DRAIN_LOG_FILE"
    DRAIN_LOG_FILE=""
  fi
  if (( status != 0 )); then diagnostics; fi
  if [[ -n "$PORT_FORWARD_PID" ]]; then kill "$PORT_FORWARD_PID" 2>/dev/null || true; fi
  if (( CREATED_CLUSTER == 1 )) && [[ $VPN_LAB_ACCEPTANCE == 1 ]]; then
    vpn_lab_resume_gluetun || cleanup_status=$?
  fi
  if (( CREATED_CLUSTER == 1 )); then
    delete_owned_cluster || cleanup_status=$?
  fi
  if (( cleanup_status == 0 )) && [[ $VPN_LAB_ACCEPTANCE == 1 ]]; then
    vpn_lab_cleanup || cleanup_status=$?
  fi
  if (( cleanup_status == 0 && CREATED_REGISTRY == 1 )); then
    local id
    id=$(docker inspect --format '{{.Id}}' "$REGISTRY" 2>/dev/null || true)
    if [[ -n "$id" && $(docker inspect --format '{{index .Config.Labels "kubarr.acceptance.run"}}' "$id" 2>/dev/null || true) == "$RUN_TOKEN" ]]; then
      docker rm -f "$id" >/dev/null 2>&1 || cleanup_status=$?
    fi
  fi
  if (( cleanup_status == 0 && CREATED_WORK_DIR == 1 )); then
    rm -rf "$WORK_DIR" || cleanup_status=$?
  fi
  if (( cleanup_status != 0 )); then
    printf '[acceptance] ERROR: cleanup failed (status %s); preserving resources and kubeconfig at %s\n' \
      "$cleanup_status" "$WORK_DIR" >&2
    (( status != 0 )) || status=$cleanup_status
  fi
  if (( status == 0 && LIFECYCLE_PASSED == 1 )); then
    if [[ $VPN_LAB_ACCEPTANCE == 1 ]]; then
      say "PASS: real WireGuard handshake, routed identity, kill switch, outage recovery, UI removal, Sonarr uninstall, and scoped cleanup"
    elif frontend_acceptance_enabled; then
      say "PASS: real frontend settings/VPN and Sonarr lifecycle, gateway API validation, NFS persistence, and scoped cleanup"
    else
      say "PASS: install, gateway API, NFS persistence, chart upgrade, worker drain, restart, uninstall, and scoped cleanup"
    fi
  fi
  exit "$status"
}

on_int() { exit 130; }
on_term() { exit 143; }

wait_for() {
  local timeout=$1 description=$2
  shift 2
  local deadline=$((SECONDS + timeout))
  until "$@"; do
    (( SECONDS < deadline )) || fail "timed out waiting for $description"
    sleep 3
  done
}

frontend_acceptance_enabled() {
  [[ $FRONTEND_ACCEPTANCE == 1 ]]
}

run_frontend_projects() {
  local project phase args=()
  for project in "$@"; do
    args+=(--project "$project")
  done
  phase=$(IFS=-; printf '%s' "$*")
  say "running real frontend acceptance project(s): $*"
  (
    cd "$APP_ROOT/code/frontend"
    KUBARR_REAL_ACCEPTANCE=1 \
      BASE_URL="http://127.0.0.1:$GATEWAY_PORT" \
      TEST_USERNAME="$ADMIN_USER" \
      TEST_PASSWORD="$ADMIN_PASSWORD" \
      ACCEPTANCE_RUN_ID="$RUN_TOKEN" \
      ACCEPTANCE_CHART_VERSION_A="$VERSION_A" \
      ACCEPTANCE_CHART_VERSION_B="$VERSION_B" \
      ACCEPTANCE_RESULT_FILE="$ACCEPTANCE_RESULT_FILE" \
      ACCEPTANCE_PHASE="$phase" \
      node node_modules/@playwright/test/cli.js test \
        --config playwright.real.config.ts "${args[@]}"
  )
}

frontend_operation_id() {
  local field=$1 mode operation_id
  [[ -f $ACCEPTANCE_RESULT_FILE && ! -L $ACCEPTANCE_RESULT_FILE && -O $ACCEPTANCE_RESULT_FILE ]] ||
    fail "frontend result file is missing or not run-owned: $ACCEPTANCE_RESULT_FILE"
  mode=$(stat -c '%a' "$ACCEPTANCE_RESULT_FILE") || return
  [[ $mode == 600 ]] || fail "frontend result file must have mode 600, got $mode"
  operation_id=$(jq -er --arg field "$field" '.[$field] | strings | select(length > 0)' \
    "$ACCEPTANCE_RESULT_FILE") || fail "frontend result lacks a non-empty $field"
  printf '%s\n' "$operation_id"
}

api() {
  local method=$1 path=$2
  shift 2
  curl --fail --silent --show-error --max-time 30 -X "$method" \
    -b "$COOKIE_JAR" -c "$COOKIE_JAR" "$@" "http://127.0.0.1:$GATEWAY_PORT$path"
}

sonarr_api() {
  local method=$1 path=$2 response status curl_status validation
  shift 2
  response=$(mktemp "$WORK_DIR/sonarr-api-response.XXXXXX") || return
  chmod 600 "$response"
  if status=$(curl --silent --show-error --max-time 30 --output "$response" --write-out '%{http_code}' \
    -X "$method" -b "$COOKIE_JAR" -c "$COOKIE_JAR" "$@" "http://127.0.0.1:$GATEWAY_PORT$path"); then
    curl_status=0
  else
    curl_status=$?
  fi
  if (( curl_status != 0 )); then
    printf '[acceptance] ERROR: Sonarr API %s %s transport failed (curl status %s)\n' \
      "$method" "$path" "$curl_status" >&2
    rm -f "$response"
    return "$curl_status"
  fi
  if [[ ! "$status" =~ ^2[0-9][0-9]$ ]]; then
    printf '[acceptance] ERROR: Sonarr API %s %s returned HTTP %s\n' "$method" "$path" "$status" >&2
    validation=$(jq -c '
      if type == "array" then
        map({propertyName, errorMessage, severity, errorCode} | with_entries(select(.value != null)))
      elif type == "object" and (.propertyName != null or .errorMessage != null or .errorCode != null) then
        [{propertyName, errorMessage, severity, errorCode} | with_entries(select(.value != null))]
      else [] end
    ' "$response" 2>/dev/null || printf '[]')
    if [[ "$validation" != "[]" ]]; then
      printf '[acceptance] Sonarr validation errors: %s\n' "$validation" >&2
    fi
    rm -f "$response"
    return 1
  fi
  cat "$response"
  rm -f "$response"
}

operation_succeeded() {
  local id=$1 response status
  response=$(api GET "/api/apps/operations/$id") || return 1
  status=$(jq -r '.status' <<<"$response")
  if [[ "$status" == failed ]]; then
    printf '%s\n' "$response" | jq '{id,app_name,operation,status,message,error}' >&2
    fail "operation $id failed"
  fi
  [[ "$status" == succeeded ]]
}

operation_running() {
  local id=$1 response status
  response=$(api GET "/api/apps/operations/$id") || return 1
  status=$(jq -er '.status | strings' <<<"$response") || return 1
  case "$status" in
    queued) return 1 ;;
    running) return 0 ;;
    failed)
      printf '%s\n' "$response" | jq '{id,app_name,operation,status,message,error}' >&2
      fail "operation $id failed before the worker drain was exercised"
      ;;
    succeeded)
      fail "operation $id succeeded before running was observed; cannot claim the worker drain was exercised"
      ;;
    *) fail "operation $id returned unexpected status: $status" ;;
  esac
}

removed_state_matches() {
  local operation_id=$1 response
  response=$(api GET /api/apps/sonarr/state) || return 1
  jq -e --arg operation "$operation_id" \
    '.desired_state == "removed" and .observed_state == "not_installed" and
     .healthy == false and .last_operation_id == $operation' <<<"$response" >/dev/null
}

state_matches() {
  local installed=$1 available=$2 update=$3 operation_id=$4 response summary
  response=$(api GET /api/apps/sonarr/state) || return 1
  summary=$(jq -c '{desired_state,observed_state,healthy,installed_chart_version,available_chart_version,update_available,last_operation_id}' <<<"$response") || return 1
  if [[ "$summary" != "${LAST_STATE_SUMMARY:-}" ]]; then
    printf '[acceptance] observed Sonarr state: %s\n' "$summary"
    LAST_STATE_SUMMARY=$summary
  fi
  jq -e --arg installed "$installed" --arg available "$available" --arg operation "$operation_id" --argjson update "$update" \
    '.desired_state == "installed" and .observed_state == "installed" and .healthy == true and
     .installed_chart_version == $installed and .available_chart_version == $available and
     .update_available == $update and .last_operation_id == $operation' <<<"$response" >/dev/null
}

namespace_absent() {
  local output
  output=$(kubectl get namespace sonarr --ignore-not-found -o json) || return 1
  [[ -z "$output" ]]
}

helm_release_absent() {
  local output
  # Helm 4 lists all statuses by default; Helm 3's --all flag was removed.
  output=$(helm list -A --filter '^sonarr$' -o json) || return 1
  jq -e 'map(select(.name == "sonarr")) | length == 0' <<<"$output" >/dev/null
}

sentinel_matches() {
  local namespace=$1 resource=$2 container=$3 path=$4 actual
  actual=$(kubectl exec -n "$namespace" "$resource" -c "$container" -- cat "$path") || return 1
  [[ "$actual" == "$RUN_TOKEN" ]]
}

helm_metadata() {
  local field=$1
  helm get metadata sonarr -n sonarr -o json | jq -er --arg field "$field" '.[$field]'
}

pod_uid_changed() {
  local old_uid=$1 current
  current=$(kubectl get pod -n sonarr -l app.kubernetes.io/name=sonarr \
    -o jsonpath='{.items[0].metadata.uid}' 2>/dev/null || true)
  [[ -n "$current" && "$current" != "$old_uid" ]] || return 1
  kubectl wait -n sonarr --for=condition=Ready pod -l app.kubernetes.io/name=sonarr \
    --timeout="$APP_TIMEOUT" >/dev/null
}

component_pod_snapshot() {
  local component=$1 namespace=$2 selector=$3 expected_count=${4:-1} pods snapshot
  pods=$(kubectl get pods -n "$namespace" -l "$selector" -ojson) || return 1
  snapshot=$(jq -cer \
    'reduce (.items[]? | select(.metadata.name != null and .metadata.uid != null)) as $pod
      ({}; .[$pod.metadata.name] = $pod.metadata.uid)' <<<"$pods") || return 1
  if [[ $(jq 'length' <<<"$snapshot") != "$expected_count" ]]; then
    printf '[acceptance] ERROR: expected exactly %s %s pod(s) when taking rollout snapshot\n' \
      "$expected_count" "$component" >&2
    return 1
  fi
  COMPONENT_POD_SNAPSHOTS=$(jq -cer --arg component "$component" --argjson snapshot "$snapshot" \
    '. + {($component): $snapshot}' <<<"$COMPONENT_POD_SNAPSHOTS") || return 1
}

component_old_pods_gone() {
  local component=$1 namespace=$2 selector=$3 pods current old
  old=$(jq -cer --arg component "$component" '.[$component] | select(type == "object" and length > 0)' \
    <<<"$COMPONENT_POD_SNAPSHOTS") || return 1
  pods=$(kubectl get pods -n "$namespace" -l "$selector" \
    --ignore-not-found -ojson) || return 1
  current=$(jq -cer \
    'reduce (.items[]? | select(.metadata.name != null and .metadata.uid != null)) as $pod
      ({}; .[$pod.metadata.name] = $pod.metadata.uid)' <<<"$pods") || return 1
  jq -ne --argjson old "$old" --argjson current "$current" \
    '([ $current | to_entries[].value ] as $current_uids |
      [ $old | to_entries[].value ] |
      all(.[]; . as $uid | ($current_uids | index($uid)) == null))' >/dev/null || return 1
}

worker_pod_snapshot() {
  component_pod_snapshot worker kubarr-worker app.kubernetes.io/name=kubarr-worker
}

worker_old_pods_gone() {
  component_old_pods_gone worker kubarr-worker app.kubernetes.io/name=kubarr-worker
}

start_worker_drain_log_follower() {
  local old_pod
  [[ -z "$DRAIN_LOG_FOLLOWER_PID" && -z "$DRAIN_LOG_FILE" ]] ||
    fail "worker drain log follower is already active"
  old_pod=$(jq -er '.worker | to_entries | select(length == 1) | .[0].key' \
    <<<"$COMPONENT_POD_SNAPSHOTS") ||
    fail "could not identify the snapshotted worker pod for drain logging"
  DRAIN_LOG_FILE=$(mktemp "$WORK_DIR/worker-drain.XXXXXX.log") || return
  chmod 600 "$DRAIN_LOG_FILE"
  kubectl logs -n kubarr-worker -f "$old_pod" -c worker --since=120s \
    >"$DRAIN_LOG_FILE" 2>&1 &
  DRAIN_LOG_FOLLOWER_PID=$!
}

finish_worker_drain_log_follower() {
  local pid=$DRAIN_LOG_FOLLOWER_PID deadline
  [[ -n "$pid" ]] || fail "worker drain log follower was not started"
  deadline=$((SECONDS + 30))
  while kill -0 "$pid" 2>/dev/null; do
    if (( SECONDS >= deadline )); then
      kill "$pid" 2>/dev/null || true
      break
    fi
    sleep 1
  done
  wait "$pid" 2>/dev/null || true
  DRAIN_LOG_FOLLOWER_PID=""
}

verify_drain_log() {
  local log_path=$1 operation_id=$2
  [[ -r "$log_path" ]] || {
    printf '[acceptance] ERROR: worker drain log is unavailable\n' >&2
    return 1
  }
  awk -v operation_id="$operation_id" '
    index($0, "Kubarr worker shutdown signal received") && !signal_line {
      signal_line = NR
    }
    index($0, "App operation finished") {
      needle = "operation_id=" operation_id
      position = index($0, needle)
      suffix = substr($0, position + length(needle))
      if (position && (suffix == "" || suffix ~ /^[[:space:]]/) && !finished_line) {
        finished_line = NR
      }
    }
    END {
      if (!signal_line) {
        print "[acceptance] ERROR: drain log lacks the worker shutdown signal event" > "/dev/stderr"
        exit 1
      }
      if (!finished_line) {
        print "[acceptance] ERROR: drain log lacks the matching App operation finished event for " operation_id > "/dev/stderr"
        exit 1
      }
      if (signal_line >= finished_line) {
        print "[acceptance] ERROR: matching App operation finished event preceded the worker shutdown signal" > "/dev/stderr"
        exit 1
      }
    }
  ' "$log_path"
}

worker_deployment_matches_drain_contract() {
  local deployment
  deployment=$(kubectl get deployment kubarr-worker -n kubarr-worker -ojson) || return 1
  jq -e '
    .spec.replicas == 1 and
    .spec.strategy.type == "Recreate" and
    .spec.template.spec.terminationGracePeriodSeconds == 660
  ' <<<"$deployment" >/dev/null
}

source_configmap() {
  local variant=$1 namespace
  require_owned_context
  for namespace in kubarr-backend kubarr-worker; do
    kubectl create configmap kubarr-acceptance-chart-source -n "$namespace" \
      --from-file=Chart.yaml="$WORK_DIR/chart-$variant/Chart.yaml" \
      --dry-run=client -o yaml | kubectl apply -f - >/dev/null
  done
}

patch_catalog_source() {
  local namespace=$1 deployment=$2 container=$3
  require_owned_context
  kubectl get deployment "$deployment" -n "$namespace" -o json | jq -e \
    --arg container "$container" --arg registry "oci://$REGISTRY_IP:5000/kubarr-charts" \
    '.spec.template.spec.containers[] | select(.name == $container) |
     (.env | map(select(.name == "KUBARR_CHARTS_SOURCE_DIR" and .value == "/catalog-source")) | length == 1) and
     (.env | map(select(.name == "KUBARR_CHARTS_REGISTRY" and .value == $registry)) | length == 1)' >/dev/null ||
    fail "$namespace/$deployment does not contain expected $container chart-source environment"
  kubectl patch deployment "$deployment" -n "$namespace" --type=strategic -p "{
    \"spec\":{\"template\":{\"spec\":{
      \"containers\":[{\"name\":\"$container\",\"volumeMounts\":[{\"name\":\"catalog-source\",\"mountPath\":\"/catalog-source\",\"readOnly\":true}]}],
      \"volumes\":[{\"name\":\"catalog-source\",\"configMap\":{\"name\":\"kubarr-acceptance-chart-source\"}}]
    }}}}" >/dev/null
}

prepare_chart_variant() {
  local variant=$1 version=$2 source="$CHARTS_DIR/media-manager/sonarr"
  cp -a "$source" "$WORK_DIR/chart-$variant"
  sed -i "s/^version:.*/version: $version/" "$WORK_DIR/chart-$variant/Chart.yaml"
  grep -q 'checksum/config:' "$WORK_DIR/chart-$variant/templates/deployment.yaml" ||
    fail "Sonarr deployment checksum annotation anchor is missing"
  sed -i '/checksum\/config:/a\        kubarr.io/acceptance-chart-version: "{{ .Chart.Version }}"' \
    "$WORK_DIR/chart-$variant/templates/deployment.yaml"
  sed -i 's/image: busybox:latest/image: busybox:1.37.0/' "$WORK_DIR/chart-$variant/templates/deployment.yaml"
  grep -q 'image: busybox:1.37.0' "$WORK_DIR/chart-$variant/templates/deployment.yaml" ||
    fail "Sonarr init image pin was not applied"
  if [[ $variant == b ]]; then
    # Acceptance-only controlled delay keeps the real chart update running long enough
    # to deliver SIGTERM to its worker without changing Sonarr's runtime behavior.
    sed -i '/^      initContainers:$/a\
        - name: acceptance-drain-delay\
          image: busybox:1.37.0\
          command: ["sh", "-c", "sleep 15"]\
          securityContext:\
            allowPrivilegeEscalation: false\
            capabilities:\
              drop: ["ALL"]\
          resources:\
            requests:\
              cpu: 1m\
              memory: 1Mi\
            limits:\
              cpu: 10m\
              memory: 8Mi' "$WORK_DIR/chart-$variant/templates/deployment.yaml"
  fi
  if [[ $VPN_LAB_ACCEPTANCE == 1 ]]; then
    vpn_lab_patch_chart "$WORK_DIR/chart-$variant"
  fi
  helm package "$WORK_DIR/chart-$variant" --destination "$WORK_DIR/packages" >/dev/null
}

registry_ready() {
  curl --fail --silent --output /dev/null --max-time 5 "http://127.0.0.1:$REGISTRY_HOST_PORT/v2/"
}

main() {
trap cleanup EXIT
trap on_int INT
trap on_term TERM
trap 'on_err "$?" "$LINENO"' ERR

[[ ${KUBARR_ACCEPTANCE_DISPOSABLE:-} == 1 ]] || fail "set KUBARR_ACCEPTANCE_DISPOSABLE=1 to authorize disposable Docker/Kind resources"
for tool in docker kind kubectl helm curl jq git sed tar cmp; do need "$tool"; done
if frontend_acceptance_enabled; then
  need node
  need stat
  [[ -f "$APP_ROOT/code/frontend/playwright.real.config.ts" ]] ||
    fail "real frontend Playwright config not found"
  [[ -f "$APP_ROOT/code/frontend/node_modules/@playwright/test/cli.js" ]] ||
    fail "frontend dependencies are not installed"
fi
if [[ $VPN_LAB_ACCEPTANCE == 1 ]]; then
  need node
  need python3
  [[ $FRONTEND_ACCEPTANCE == 0 ]] || fail "VPN lab and full frontend acceptance are separate lanes"
  [[ -f "$APP_ROOT/code/frontend/playwright.vpn.config.ts" ]] || fail "VPN Playwright config not found"
  [[ -f "$APP_ROOT/code/frontend/node_modules/@playwright/test/cli.js" ]] || fail "frontend dependencies are not installed"
fi
[[ -x "$CLI" ]] || fail "CLI not executable: $CLI (build with cargo build --locked --manifest-path code/cli/Cargo.toml)"
[[ -f "$CHARTS_REPO/media-manager/sonarr/Chart.yaml" ]] || fail "invalid charts path: $CHARTS_REPO"
[[ $(git -C "$CHARTS_REPO" rev-parse HEAD) == "$CHARTS_REV" ]] || fail "charts checkout must be pinned at $CHARTS_REV"
if [[ $VPN_LAB_ACCEPTANCE == 1 ]]; then
  grep -q 'extraEnv:' "$CHARTS_REPO/media-manager/sonarr/values.yaml" ||
    fail "pinned Sonarr chart lacks vpn.extraEnv"
  grep -q 'Values.vpn.extraEnv' "$CHARTS_REPO/media-manager/sonarr/templates/deployment.yaml" ||
    fail "pinned Sonarr deployment does not render vpn.extraEnv"
fi
NATIVE_PLATFORM=$(native_docker_platform)
[[ ! -e "$WORK_DIR" ]] || fail "work directory already exists: $WORK_DIR"
kind get clusters 2>/dev/null | awk -v name="$CLUSTER" '$0 == name { found=1 } END { exit !found }' && fail "refusing existing cluster $CLUSTER"
docker inspect "$REGISTRY" >/dev/null 2>&1 && fail "refusing existing registry $REGISTRY"
if command -v ss >/dev/null 2>&1 && ss -ltn "sport = :$GATEWAY_PORT" | grep -q LISTEN; then
  fail "gateway port $GATEWAY_PORT is already in use"
fi
if [[ ! -r /proc/filesystems ]] || ! grep -qw nfs /proc/filesystems || ! grep -qw nfsd /proc/filesystems; then
  fail "host NFS support is unavailable; load nfs and nfsd modules explicitly before running"
fi

mkdir -m 700 "$WORK_DIR" "$WORK_DIR/packages" "$WORK_DIR/installer-charts" "$WORK_DIR/chart-source"
CREATED_WORK_DIR=1
git -C "$CHARTS_REPO" archive "$CHARTS_REV" | tar -x -C "$WORK_DIR/chart-source"
CHARTS_DIR="$WORK_DIR/chart-source"
COOKIE_JAR="$WORK_DIR/cookies"
touch "$COOKIE_JAR"
chmod 600 "$COOKIE_JAR"
if frontend_acceptance_enabled; then
  ACCEPTANCE_RESULT_FILE="$WORK_DIR/frontend-results.json"
  printf '{}\n' >"$ACCEPTANCE_RESULT_FILE"
  chmod 600 "$ACCEPTANCE_RESULT_FILE"
fi

say "creating isolated Kind cluster $CLUSTER"
CREATED_CLUSTER=1
kind create cluster --name "$CLUSTER" --kubeconfig "$KUBECONFIG" \
  --image "${KUBARR_ACCEPTANCE_KIND_IMAGE:-kindest/node:v1.35.8@sha256:07b2536e30b803ed61d1677a79df6115f798ce64c80f9e22f6ed45afd09323c0}" \
  --wait 180s
owned_context || fail "isolated kubeconfig does not select owned context kind-$CLUSTER"
if [[ $VPN_LAB_ACCEPTANCE == 1 ]]; then
  vpn_lab_setup
fi

say "starting run-owned OCI registry"
CREATED_REGISTRY=1
docker run -d --name "$REGISTRY" --label "kubarr.acceptance.run=$RUN_TOKEN" \
  -p 127.0.0.1::5000 registry:2 >/dev/null
docker network connect kind "$REGISTRY"
REGISTRY_IP=$(docker inspect --format '{{(index .NetworkSettings.Networks "kind").IPAddress}}' "$REGISTRY")
REGISTRY_HOST_PORT=$(docker port "$REGISTRY" 5000/tcp | sed -n 's/.*://p')
[[ "$REGISTRY_IP" =~ ^[0-9a-fA-F:.]+$ ]] || fail "could not determine registry address"
[[ "$REGISTRY_HOST_PORT" =~ ^[0-9]+$ ]] || fail "could not determine registry host port"
wait_for 60 "OCI registry readiness" registry_ready

say "loading current Kubarr images"
for image in kubarr-backend kubarr-frontend kubarr-worker; do
  docker image inspect "$image:$IMAGE_TAG" >/dev/null 2>&1 || fail "missing local image $image:$IMAGE_TAG"
  load_image_into_kind "$image:$IMAGE_TAG"
done

DEPENDENCY_IMAGES=(
  itsthenetwork/nfs-server-alpine:12
  postgres:16-alpine
  fluent/fluent-bit:5.1.0
  victoriametrics/victoria-metrics:v1.149.0
  victoriametrics/victoria-logs:v1.52.0
  openresty/openresty:1.27.1.2-alpine
  linuxserver/sonarr:4.0.19
  busybox:1.37.0
)
if [[ $VPN_LAB_ACCEPTANCE == 1 ]]; then
  DEPENDENCY_IMAGES+=("$VPN_GLUETUN_IMAGE")
fi
if frontend_acceptance_enabled; then
  # The UI installs chart defaults, while the API scenario explicitly disables the exporter.
  DEPENDENCY_IMAGES+=(ghcr.io/onedr0p/exportarr:v2.3.0)
fi
say "pre-pulling and loading pinned dependency images"
for image in "${DEPENDENCY_IMAGES[@]}"; do
  docker pull --platform="$NATIVE_PLATFORM" "$image" >/dev/null
  load_image_into_kind "$image"
done
if [[ $VPN_LAB_ACCEPTANCE == 1 ]]; then
  vpn_lab_verify_gluetun_image
fi

say "preparing pinned installer and Sonarr charts"
for chart in system/managed-nfs system/postgresql monitoring/fluent-bit monitoring/victorialogs \
  monitoring/victoriametrics system/kubarr-backend system/kubarr-frontend system/openresty system/kubarr-worker; do
  helm dependency build --skip-refresh "$CHARTS_DIR/$chart" >/dev/null
  ln -s "$CHARTS_DIR/$chart" "$WORK_DIR/installer-charts/${chart##*/}"
done
helm dependency build --skip-refresh "$CHARTS_DIR/media-manager/sonarr" >/dev/null
VERSION_A=4.1.7-acceptance.1
VERSION_B=4.1.7-acceptance.2
prepare_chart_variant a "$VERSION_A"
prepare_chart_variant b "$VERSION_B"
helm template sonarr "$WORK_DIR/chart-a" --namespace sonarr --set exporter.enabled=false >"$WORK_DIR/rendered-a.yaml"
helm template sonarr "$WORK_DIR/chart-b" --namespace sonarr --set exporter.enabled=false >"$WORK_DIR/rendered-b.yaml"
grep -q "kubarr.io/acceptance-chart-version: \"$VERSION_A\"" "$WORK_DIR/rendered-a.yaml" || fail "A render lacks rollout annotation"
grep -q "kubarr.io/acceptance-chart-version: \"$VERSION_B\"" "$WORK_DIR/rendered-b.yaml" || fail "B render lacks rollout annotation"
cmp -s "$WORK_DIR/rendered-a.yaml" "$WORK_DIR/rendered-b.yaml" && fail "A and B rendered manifests are identical"
for package in "$WORK_DIR/packages"/*.tgz; do
  helm push "$package" "oci://127.0.0.1:$REGISTRY_HOST_PORT/kubarr-charts" --plain-http >/dev/null
done

say "bootstrapping Kubarr through the real CLI"
export KUBARR_CHARTS_DIR="$WORK_DIR/installer-charts"
export KUBARR_BACKEND_IMAGE=kubarr-backend KUBARR_FRONTEND_IMAGE=kubarr-frontend KUBARR_WORKER_IMAGE=kubarr-worker
export KUBARR_IMAGE_TAG="$IMAGE_TAG" KUBARR_IMAGE_PULL_POLICY=Never
cat >"$WORK_DIR/bootstrap-values.yaml" <<EOF
env:
  - name: KUBARR_IN_CLUSTER
    value: "true"
  - name: KUBARR_DEFAULT_NAMESPACE
    value: "media"
  - name: KUBARR_LOG_LEVEL
    value: "INFO"
  - name: KUBARR_OAUTH2_ISSUER_URL
    value: "http://kubarr-backend.kubarr-backend.svc.cluster.local:8000"
  - name: HOME
    value: /tmp
  - name: XDG_CACHE_HOME
    value: /tmp/.cache
  - name: HELM_CACHE_HOME
    value: /tmp/helm/cache
  - name: HELM_CONFIG_HOME
    value: /tmp/helm/config
  - name: HELM_DATA_HOME
    value: /tmp/helm/data
  - name: KUBARR_CHARTS_SOURCE_DIR
    value: /catalog-source
  - name: KUBARR_CHARTS_REGISTRY
    value: "oci://$REGISTRY_IP:5000/kubarr-charts"
  - name: KUBARR_CHARTS_PLAIN_HTTP
    value: "true"
  - name: KUBARR_CHARTS_SYNC_INTERVAL
    value: "3600"
EOF
ADMIN_USER=acceptance-admin
ADMIN_PASSWORD=$(printf 'acceptance-%s-Aa1!' "$RUN_TOKEN")
require_owned_context
  "$CLI" bootstrap --cluster-mode existing --storage-mode managed-nfs --storage-size 2Gi \
  --storage-class standard --admin-username "$ADMIN_USER" --admin-email acceptance@example.invalid \
  --admin-password "$ADMIN_PASSWORD" --values "$WORK_DIR/bootstrap-values.yaml"

  say "verifying worker singleton drain contract"
  worker_deployment_matches_drain_contract ||
    fail "worker Deployment must use one replica, Recreate, and a 660-second termination grace period"
  source_configmap a
  component_pod_snapshot backend kubarr-backend app.kubernetes.io/name=kubarr-backend
  patch_catalog_source kubarr-backend kubarr-backend backend
  worker_pod_snapshot
  patch_catalog_source kubarr-worker kubarr-worker worker
  kubectl rollout status deployment/kubarr-backend -n kubarr-backend --timeout="$INSTALL_TIMEOUT"
  kubectl rollout status deployment/kubarr-worker -n kubarr-worker --timeout="$INSTALL_TIMEOUT"
  wait_for 120 "old backend pod after initial catalog rollout" \
    component_old_pods_gone backend kubarr-backend app.kubernetes.io/name=kubarr-backend
  wait_for 720 "old worker pods after initial catalog rollout" worker_old_pods_gone

say "starting gateway tunnel and authenticating"
kubectl port-forward --address 127.0.0.1 -n openresty svc/kubarr-gateway "$GATEWAY_PORT:8080" \
  >"$WORK_DIR/port-forward.log" 2>&1 &
PORT_FORWARD_PID=$!
wait_for 120 "gateway readiness" curl --fail --silent --output /dev/null --max-time 5 \
  "http://127.0.0.1:$GATEWAY_PORT/api/system/health"
jq -n --arg username "$ADMIN_USER" --arg password "$ADMIN_PASSWORD" \
  '{username:$username,password:$password}' >"$WORK_DIR/login.json"
curl --fail --silent --show-error --max-time 30 -b "$COOKIE_JAR" -c "$COOKIE_JAR" \
  -H 'Content-Type: application/json' --data-binary @"$WORK_DIR/login.json" \
  "http://127.0.0.1:$GATEWAY_PORT/auth/login" >/dev/null
rm -f "$WORK_DIR/login.json"
if frontend_acceptance_enabled; then
  run_frontend_projects real-settings-vpn real-app-install
  install_id=$(frontend_operation_id install_id)
else
  if [[ $VPN_LAB_ACCEPTANCE != 1 ]]; then
    unset ADMIN_PASSWORD
  fi
  api POST /api/apps/sync >/dev/null
  install_response=$(api POST /api/apps/install -H 'Content-Type: application/json' \
    --data '{"app_name":"sonarr","custom_config":{"exporter.enabled":"false"}}')
  install_id=$(jq -er '.id' <<<"$install_response")
fi
wait_for 660 "Sonarr install operation" operation_succeeded "$install_id"
wait_for 300 "healthy Sonarr A state" state_matches "$VERSION_A" "$VERSION_A" false "$install_id"

require_owned_context
SONARR_API_KEY=$(kubectl exec -n sonarr deployment/sonarr -c sonarr -- sh -c \
  "sed -n 's/.*<ApiKey>\\([^<]*\\)<\\/ApiKey>.*/\\1/p' /config/config.xml" | tr -d '\r\n')
[[ "$SONARR_API_KEY" =~ ^[A-Za-z0-9]+$ ]] || fail "Sonarr generated API key was unavailable"
say "phase: reading Sonarr status API"
sonarr_api GET /sonarr/api/v3/system/status -H "X-Api-Key: $SONARR_API_KEY" | jq -e '.appName == "Sonarr"' >/dev/null
say "phase: reading Sonarr host config"
sonarr_api GET /sonarr/api/v3/config/host -H "X-Api-Key: $SONARR_API_KEY" >"$WORK_DIR/host-config.json"
jq '.instanceName="Kubarr Acceptance Sonarr"' "$WORK_DIR/host-config.json" >"$WORK_DIR/host-config-update.json"
chmod 600 "$WORK_DIR/host-config.json" "$WORK_DIR/host-config-update.json"
say "phase: updating Sonarr host config"
sonarr_api PUT /sonarr/api/v3/config/host -H "X-Api-Key: $SONARR_API_KEY" -H 'Content-Type: application/json' \
  --data-binary @"$WORK_DIR/host-config-update.json" >/dev/null
rm -f "$WORK_DIR/host-config.json" "$WORK_DIR/host-config-update.json"
SENTINEL=".kubarr-acceptance-$RUN_TOKEN"
say "phase: writing Sonarr sentinel"
kubectl exec -n sonarr deployment/sonarr -c sonarr -- sh -c "printf '%s' '$RUN_TOKEN' > '/media/$SENTINEL'"

if [[ $VPN_LAB_ACCEPTANCE == 1 ]]; then
  say "running controlled WireGuard VPN lifecycle"
  vpn_lab_run_scenario
  unset ADMIN_PASSWORD
  delete_response=$(api DELETE /api/apps/sonarr)
  delete_id=$(jq -er '.id' <<<"$delete_response")
  wait_for 180 "Sonarr uninstall operation" operation_succeeded "$delete_id"
  wait_for 120 "removed Sonarr state" removed_state_matches "$delete_id"
  wait_for 180 "Sonarr namespace deletion" namespace_absent
  helm_release_absent || fail "Sonarr Helm release remains after VPN acceptance uninstall"
  require_owned_context
  sentinel_matches kubarr-backend deployment/kubarr-backend backend "/data/media/$SENTINEL" ||
    fail "Backend sentinel content changed after VPN acceptance uninstall"
  LIFECYCLE_PASSED=1
  say "VPN lifecycle checks passed; verifying scoped cleanup"
  return
fi

uid_a=$(kubectl get pod -n sonarr -l app.kubernetes.io/name=sonarr -o jsonpath='{.items[0].metadata.uid}')
image_a=$(kubectl get deployment sonarr -n sonarr -o jsonpath='{.spec.template.spec.containers[?(@.name=="sonarr")].image}')
revision_a=$(helm_metadata revision)
[[ $(helm_metadata version) == "$VERSION_A" ]] || fail "Helm metadata did not record Sonarr A"
image_id_a=$(kubectl get pod -n sonarr -l app.kubernetes.io/name=sonarr \
  -o jsonpath='{.items[0].status.containerStatuses[?(@.name=="sonarr")].imageID}')

say "switching catalog to B and upgrading"
source_configmap b
require_owned_context
worker_pod_snapshot
component_pod_snapshot backend kubarr-backend app.kubernetes.io/name=kubarr-backend
kubectl rollout restart deployment/kubarr-backend -n kubarr-backend >/dev/null
kubectl rollout restart deployment/kubarr-worker -n kubarr-worker >/dev/null
kubectl rollout status deployment/kubarr-backend -n kubarr-backend --timeout="$INSTALL_TIMEOUT"
kubectl rollout status deployment/kubarr-worker -n kubarr-worker --timeout="$INSTALL_TIMEOUT"
wait_for 120 "old backend pod after source B rollout" \
  component_old_pods_gone backend kubarr-backend app.kubernetes.io/name=kubarr-backend
wait_for 720 "old worker pods after source B rollout" worker_old_pods_gone
if frontend_acceptance_enabled; then
  run_frontend_projects real-app-upgrade
  update_id=$(frontend_operation_id update_id)
  wait_for 660 "Sonarr frontend update operation" operation_succeeded "$update_id"
else
  api POST /api/apps/sync >/dev/null
  wait_for 120 "B update availability" state_matches "$VERSION_A" "$VERSION_B" true "$install_id"
  update_response=$(api POST /api/apps/sonarr/update)
  update_id=$(jq -er '.id' <<<"$update_response")
  wait_for 60 "Sonarr update operation to be observed running" operation_running "$update_id"
  say "phase: restart worker with update in flight"
  require_owned_context
  worker_pod_snapshot
  start_worker_drain_log_follower
  kubectl rollout restart deployment/kubarr-worker -n kubarr-worker >/dev/null
  kubectl rollout status deployment/kubarr-worker -n kubarr-worker --timeout=720s
  wait_for 720 "old worker pod after in-flight update drain" worker_old_pods_gone
  finish_worker_drain_log_follower
  say "phase: after drain, verifying in-flight update completion"
  wait_for 660 "Sonarr update operation" operation_succeeded "$update_id"
  verify_drain_log "$DRAIN_LOG_FILE" "$update_id" || fail "worker drain event order was not proven"
  rm -f "$DRAIN_LOG_FILE"
  DRAIN_LOG_FILE=""
  say "drain guarantee: shutdown signal preceded update completion"
fi
wait_for 300 "healthy Sonarr B state" state_matches "$VERSION_B" "$VERSION_B" false "$update_id"
wait_for 300 "Sonarr chart rollout" pod_uid_changed "$uid_a"

image_b=$(kubectl get deployment sonarr -n sonarr -o jsonpath='{.spec.template.spec.containers[?(@.name=="sonarr")].image}')
revision_b=$(helm_metadata revision)
[[ $(helm_metadata version) == "$VERSION_B" ]] || fail "Helm metadata did not record Sonarr B"
image_id_b=$(kubectl get pod -n sonarr -l app.kubernetes.io/name=sonarr \
  -o jsonpath='{.items[0].status.containerStatuses[?(@.name=="sonarr")].imageID}')
[[ "$image_b" == "$image_a" ]] || fail "Sonarr image changed across chart-only upgrade"
[[ -n "$image_id_a" && "$image_id_b" == "$image_id_a" ]] || fail "Sonarr image digest changed across chart-only upgrade"
(( revision_b > revision_a )) || fail "Helm revision did not increase"
sonarr_api GET /sonarr/api/v3/config/host -H "X-Api-Key: $SONARR_API_KEY" | jq -e '.instanceName == "Kubarr Acceptance Sonarr"' >/dev/null
sentinel_matches sonarr deployment/sonarr sonarr "/media/$SENTINEL" ||
  fail "Sonarr sentinel content changed after chart upgrade"

say "restarting and uninstalling Sonarr"
uid_b=$(kubectl get pod -n sonarr -l app.kubernetes.io/name=sonarr -o jsonpath='{.items[0].metadata.uid}')
if frontend_acceptance_enabled; then
  run_frontend_projects real-app-restart
  restart_id=$(frontend_operation_id restart_id)
else
  restart_response=$(api POST /api/apps/sonarr/restart)
  restart_id=$(jq -er '.id' <<<"$restart_response")
fi
wait_for 180 "Sonarr restart operation" operation_succeeded "$restart_id"
wait_for 300 "replacement Sonarr pod" pod_uid_changed "$uid_b"
wait_for 120 "reconciled Sonarr restart state" state_matches "$VERSION_B" "$VERSION_B" false "$restart_id"
sonarr_api GET /sonarr/api/v3/config/host -H "X-Api-Key: $SONARR_API_KEY" | jq -e '.instanceName == "Kubarr Acceptance Sonarr"' >/dev/null
sentinel_matches sonarr deployment/sonarr sonarr "/media/$SENTINEL" ||
  fail "Sonarr sentinel content changed after restart"

if frontend_acceptance_enabled; then
  run_frontend_projects real-app-uninstall
  delete_id=$(frontend_operation_id delete_id)
  unset ADMIN_PASSWORD
else
  delete_response=$(api DELETE /api/apps/sonarr)
  delete_id=$(jq -er '.id' <<<"$delete_response")
fi
wait_for 180 "Sonarr uninstall operation" operation_succeeded "$delete_id"
wait_for 120 "removed Sonarr state" removed_state_matches "$delete_id"
wait_for 180 "Sonarr namespace deletion" namespace_absent
helm_release_absent || fail "Sonarr Helm release remains after uninstall"
require_owned_context
sentinel_matches kubarr-backend deployment/kubarr-backend backend "/data/media/$SENTINEL" ||
  fail "Backend sentinel content changed after Sonarr uninstall"

unset SONARR_API_KEY
LIFECYCLE_PASSED=1
say "lifecycle checks passed; verifying scoped cleanup"
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
  main "$@"
fi
