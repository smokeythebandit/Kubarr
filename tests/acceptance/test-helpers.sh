#!/usr/bin/env bash
# shellcheck disable=SC1091,SC2034,SC2317
set -Eeuo pipefail

DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SCRIPT="$DIR/app-lifecycle.sh"

[[ -x "$SCRIPT" ]] || { printf 'not executable: %s\n' "$SCRIPT" >&2; exit 1; }
bash -n "$SCRIPT"
# shellcheck source=app-lifecycle.sh
source "$SCRIPT"

assert_status() {
  local expected=$1
  shift
  set +e
  ("$@")
  local actual=$?
  set -e
  [[ $actual == "$expected" ]] || {
    printf 'expected status %s, got %s: %s\n' "$expected" "$actual" "$*" >&2
    exit 1
  }
}

test_opt_in_guard() {
  local output
  output=$(env -i PATH="$PATH" HOME="${HOME:-/tmp}" bash "$SCRIPT" 2>&1 || true)
  [[ $output == *KUBARR_ACCEPTANCE_DISPOSABLE=1* ]]
}

test_operation_succeeded_polling() {
  local counter
  counter=$(mktemp)
  printf '0\n' >"$counter"
  api() {
    local count
    count=$(<"$counter")
    if (( count == 0 )); then
      printf '1\n' >"$counter"
      printf '{"status":"queued"}\n'
    else
      printf '{"status":"succeeded"}\n'
    fi
  }
  sleep() { :; }
  wait_for 5 "mock operation" operation_succeeded mock-id
  rm -f "$counter"
}

frontend_operation_id_fixture() {
  local result_file=$1 field=$2
  ACCEPTANCE_RESULT_FILE=$result_file
  frontend_operation_id "$field"
}

test_frontend_operation_result_contract() {
  local test_dir result output
  test_dir=$(mktemp -d)
  result=$test_dir/results.json
  printf '{"install_id":"install-123","update_id":"update-456"}\n' >"$result"
  chmod 600 "$result"

  output=$(frontend_operation_id_fixture "$result" install_id)
  [[ $output == install-123 ]]
  output=$(frontend_operation_id_fixture "$result" update_id)
  [[ $output == update-456 ]]
  assert_status 1 frontend_operation_id_fixture "$result" restart_id >/dev/null 2>&1

  chmod 644 "$result"
  assert_status 1 frontend_operation_id_fixture "$result" install_id >/dev/null 2>&1
  rm -rf "$test_dir"
}

operation_running_with_status() {
  local mocked_status=$1
  api() { printf '{"id":"mock-id","app_name":"sonarr","operation":"update","status":"%s","message":"mock","error":"mock"}\n' "$mocked_status"; }
  operation_running mock-id
}

operation_running_api_error() {
  api() { return 22; }
  operation_running mock-id
}

test_operation_running_statuses() {
  local output status
  assert_status 1 operation_running_with_status queued
  operation_running_with_status running

  set +e
  output=$(operation_running_with_status succeeded 2>&1)
  status=$?
  set -e
  [[ $status == 1 && $output == *"succeeded before running was observed"* && $output == *"cannot claim the worker drain was exercised"* ]]

  set +e
  output=$(operation_running_with_status failed 2>&1)
  status=$?
  set -e
  [[ $status == 1 && $output == *"failed before the worker drain was exercised"* ]]

  assert_status 1 operation_running_api_error
}

verify_drain_log_fixture() {
  local log_path=$1 operation_id=$2
  verify_drain_log "$log_path" "$operation_id"
}

test_verify_drain_log_event_order() {
  local test_dir log
  test_dir=$(mktemp -d)
  log=$test_dir/worker.log

  printf '%s\n' \
    '2026-09-20T10:00:00Z INFO Kubarr worker shutdown signal received' \
    '2026-09-20T10:00:01Z INFO App operation finished operation_id=update-correct app=sonarr operation=update' >"$log"
  chmod 600 "$log"
  verify_drain_log_fixture "$log" update-correct

  printf '%s\n' \
    '2026-09-20T10:00:00Z INFO App operation finished operation_id=update-correct app=sonarr operation=update' \
    '2026-09-20T10:00:01Z INFO Kubarr worker shutdown signal received' >"$log"
  assert_status 1 verify_drain_log_fixture "$log" update-correct

  printf '%s\n' \
    '2026-09-20T10:00:00Z INFO Kubarr worker shutdown signal received' \
    '2026-09-20T10:00:01Z INFO App operation finished operation_id=update-other app=sonarr operation=update' >"$log"
  assert_status 1 verify_drain_log_fixture "$log" update-correct

  printf '%s\n' \
    '2026-09-20T10:00:01Z INFO App operation finished operation_id=update-correct app=sonarr operation=update' >"$log"
  assert_status 1 verify_drain_log_fixture "$log" update-correct

  rm -rf "$test_dir"
}

worker_old_pods_gone_with_no_current_pods() {
  COMPONENT_POD_SNAPSHOTS='{"worker":{"worker-old":"uid-old"}}'
  kubectl() { printf '{"items":[]}\n'; }
  worker_old_pods_gone
}

worker_old_pod_still_present() {
  COMPONENT_POD_SNAPSHOTS='{"worker":{"worker-old":"uid-old"}}'
  kubectl() { printf '{"items":[{"metadata":{"name":"worker-old","uid":"uid-old"}}]}\n'; }
  worker_old_pods_gone
}

worker_pod_snapshot_connection_error() {
  kubectl() { return 1; }
  worker_pod_snapshot
}

worker_old_pods_gone_connection_error() {
  kubectl() { return 1; }
  worker_old_pods_gone
}

test_worker_old_pods_gone_with_replacement_uid() {
  COMPONENT_POD_SNAPSHOTS='{"worker":{"worker-old":"uid-old"}}'
  kubectl() {
    printf '%s\n' '{"items":[{"metadata":{"name":"worker-old","uid":"uid-new"}}]}'
  }
  worker_old_pods_gone
}

test_component_snapshots_are_independent() {
  COMPONENT_POD_SNAPSHOTS='{}'
  kubectl() {
    case "$*" in
      *kubarr-backend*) printf '%s\n' '{"items":[{"metadata":{"name":"backend-old","uid":"backend-uid"}}]}' ;;
      *kubarr-worker*) printf '%s\n' '{"items":[{"metadata":{"name":"worker-old","uid":"worker-uid"}}]}' ;;
      *) return 2 ;;
    esac
  }
  component_pod_snapshot backend kubarr-backend app.kubernetes.io/name=kubarr-backend
  worker_pod_snapshot
  jq -e '.backend == {"backend-old":"backend-uid"} and
    .worker == {"worker-old":"worker-uid"}' <<<"$COMPONENT_POD_SNAPSHOTS" >/dev/null
}

test_component_old_pods_gone_checks_only_snapshotted_uids() {
  COMPONENT_POD_SNAPSHOTS='{"backend":{"backend-old":"backend-uid"}}'
  kubectl() {
    printf '%s\n' '{"items":[{"metadata":{"name":"unrelated","uid":"other-uid"}}]}'
  }
  component_old_pods_gone backend kubarr-backend app.kubernetes.io/name=kubarr-backend
}

test_chart_b_has_bounded_acceptance_delay() {
  local test_dir helm_bin rendered_a rendered_b
  test_dir=$(mktemp -d)
  helm_bin=$(command -v helm || true)
  if [[ -z $helm_bin && -x $APP_ROOT/.acceptance-runs/tools/bin/helm ]]; then
    helm_bin=$APP_ROOT/.acceptance-runs/tools/bin/helm
  fi
  [[ -n $helm_bin ]] || { printf 'Helm is required for chart fixture checks\n' >&2; return 1; }

  WORK_DIR=$test_dir
  CHARTS_DIR=$CHARTS_REPO
  mkdir "$WORK_DIR/packages"
  helm() { "$helm_bin" "$@"; }
  prepare_chart_variant a 4.1.7-test.1
  prepare_chart_variant b 4.1.7-test.2
  rendered_a=$WORK_DIR/rendered-a.yaml
  rendered_b=$WORK_DIR/rendered-b.yaml
  helm template sonarr "$WORK_DIR/chart-a" --namespace sonarr --set exporter.enabled=false >"$rendered_a"
  helm template sonarr "$WORK_DIR/chart-b" --namespace sonarr --set exporter.enabled=false >"$rendered_b"
  python3 - "$rendered_a" "$rendered_b" <<'PY'
import sys
import yaml

def deployment(path):
    with open(path, encoding="utf-8") as stream:
        return next(doc for doc in yaml.safe_load_all(stream)
                    if doc and doc.get("kind") == "Deployment")

a = deployment(sys.argv[1])
b = deployment(sys.argv[2])
a_init = {item["name"]: item for item in a["spec"]["template"]["spec"]["initContainers"]}
b_init = {item["name"]: item for item in b["spec"]["template"]["spec"]["initContainers"]}
assert "acceptance-drain-delay" not in a_init
delay = b_init["acceptance-drain-delay"]
assert delay["image"] == "busybox:1.37.0"
assert delay["command"][:2] == ["sh", "-c"]
seconds = int(delay["command"][2].split()[1])
assert 0 < seconds <= 20
assert delay["resources"]["requests"] == {"cpu": "1m", "memory": "1Mi"}
assert delay["resources"]["limits"] == {"cpu": "10m", "memory": "8Mi"}
PY
  rm -rf "$test_dir"
}

worker_pod_snapshot_without_singleton() {
  kubectl() { printf '{"items":[]}\n'; }
  worker_pod_snapshot
}

failed_operation() {
  api() { printf '{"id":"mock-id","app_name":"sonarr","operation":"install","status":"failed","message":"failed","error":"mock"}\n'; }
  operation_succeeded mock-id
}

timed_out_poll() {
  wait_for 0 "mock timeout" false
}

test_cleanup_preserves_unowned_directory() {
  local caller_dir
  caller_dir=$(mktemp -d)
  printf 'caller-owned\n' >"$caller_dir/sentinel"
  set +e
  (set +e; WORK_DIR=$caller_dir; CREATED_WORK_DIR=0; CREATED_CLUSTER=0; CREATED_REGISTRY=0; false; cleanup) >/dev/null 2>&1
  local status=$?
  set -e
  [[ $status == 1 && -f "$caller_dir/sentinel" ]]
  rm -rf "$caller_dir"
}

test_cluster_cleanup_orders_nfs_server_last() {
  local test_dir log inspect_count=0
  test_dir=$(mktemp -d)
  log=$test_dir/calls
  CLUSTER=test-cluster
  KUBECONFIG=$test_dir/kubeconfig
  : >"$KUBECONFIG"
  owned_kind_node() { return 0; }
  owned_context() { return 0; }
  kubectl() {
    if [[ $1 == get ]]; then
      printf '{"items":[{"metadata":{"name":"kube-system"}},{"metadata":{"name":"kubarr-storage"}},{"metadata":{"name":"kubarr-database"}},{"metadata":{"name":"sonarr"}}]}\n'
    else
      printf 'kubectl <%s>\n' "$*" >>"$log"
    fi
  }
  timeout() { shift; "$@"; }
  kind() { printf 'kind <%s>\n' "$*" >>"$log"; }
  docker() {
    (( inspect_count += 1 ))
    (( inspect_count == 1 )) && return 0
    return 1
  }

  delete_owned_cluster
  [[ $(sed -n '1p' "$log") == 'kubectl <delete namespace kubarr-database sonarr --wait=true --timeout=110s>' ]]
  [[ $(sed -n '2p' "$log") == 'kubectl <delete namespace kubarr-storage --ignore-not-found --wait=true --timeout=80s>' ]]
  [[ $(sed -n '3p' "$log") == 'kind <delete cluster --name test-cluster>' ]]
  rm -rf "$test_dir"
}

unowned_cluster_delete_is_rejected() {
  docker() { return 0; }
  owned_kind_node() { return 1; }
  delete_owned_cluster
}

failed_cluster_cleanup() {
  local test_dir=$1
  WORK_DIR=$test_dir
  CREATED_WORK_DIR=1 CREATED_CLUSTER=1 CREATED_REGISTRY=0
  delete_owned_cluster() { return 23; }
  cleanup
}

test_cleanup_failure_is_reported_and_preserved() {
  local test_dir status
  test_dir=$(mktemp -d)
  set +e
  (failed_cluster_cleanup "$test_dir") >/dev/null 2>&1
  status=$?
  set -e
  [[ $status == 23 && -d $test_dir ]]
  rm -rf "$test_dir"
}

connection_error_is_not_absent() {
  kubectl() { return 1; }
  namespace_absent
}

helm_connection_error_is_not_absent() {
  helm() { return 1; }
  helm_release_absent
}

test_helm4_release_absence_contract() {
  local releases='[]'
  helm() {
    [[ "$*" == 'list -A --filter ^sonarr$ -o json' ]] || return 2
    printf '%s\n' "$releases"
  }
  helm_release_absent
  for status in deployed failed pending-upgrade uninstalling; do
    releases="[{\"name\":\"sonarr\",\"status\":\"$status\"}]"
    if helm_release_absent; then
      printf 'Remaining %s release was incorrectly considered absent\n' "$status" >&2
      return 1
    fi
  done
}

test_signal_cleanup() {
  local handler expected signal_dir status
  for handler in on_int on_term; do
    [[ $handler == on_int ]] && expected=130 || expected=143
    signal_dir=$(mktemp -d)
    set +e
    (
      WORK_DIR=$signal_dir
      CREATED_WORK_DIR=1
      CREATED_CLUSTER=0
      CREATED_REGISTRY=0
      trap cleanup EXIT
      "$handler"
    ) >/dev/null 2>&1
    status=$?
    set -e
    [[ $status == "$expected" && ! -e $signal_dir ]]
  done
}

test_native_docker_platform() {
  docker() { printf 'x86_64\n'; }
  [[ $(native_docker_platform) == linux/amd64 ]]
  docker() { printf 'amd64\n'; }
  [[ $(native_docker_platform) == linux/amd64 ]]
  docker() { printf 'aarch64\n'; }
  [[ $(native_docker_platform) == linux/arm64 ]]
  docker() { printf 'arm64\n'; }
  [[ $(native_docker_platform) == linux/arm64 ]]
}

unsupported_docker_platform() {
  docker() { printf 'riscv64\n'; }
  native_docker_platform
}

test_image_archive_load() {
  local test_dir log archive
  test_dir=$(mktemp -d)
  log=$test_dir/calls
  WORK_DIR=$test_dir NATIVE_PLATFORM=linux/amd64 CLUSTER=test-cluster
  docker() {
    {
      printf 'docker'
      printf ' <%s>' "$@"
      printf '\n'
    } >>"$log"
    archive=$5
    : >"$archive"
  }
  kind() {
    {
      printf 'kind'
      printf ' <%s>' "$@"
      printf '\n'
    } >>"$log"
  }

  load_image_into_kind example/image:test
  archive=$(sed -n 's/^docker <image> <save> <--platform=linux\/amd64> <--output> <\([^>]*\)> <example\/image:test>$/\1/p' "$log")
  [[ -n $archive && ! -e $archive ]]
  [[ $(sed -n '2p' "$log") == "kind <load> <image-archive> <--name> <test-cluster> <$archive>" ]]
  rm -rf "$test_dir"
}

failed_image_archive_import() {
  local test_dir=$1 log=$2
  WORK_DIR=$test_dir NATIVE_PLATFORM=linux/arm64 CLUSTER=test-cluster
  docker() {
    {
      printf 'docker'
      printf ' <%s>' "$@"
      printf '\n'
    } >>"$log"
    : >"$5"
  }
  kind() {
    {
      printf 'kind'
      printf ' <%s>' "$@"
      printf '\n'
    } >>"$log"
    return 42
  }
  load_image_into_kind example/failure:test
}

test_image_archive_failure_propagates() {
  local test_dir log archive
  test_dir=$(mktemp -d)
  log=$test_dir/calls
  assert_status 42 failed_image_archive_import "$test_dir" "$log"
  archive=$(sed -n 's/^docker <image> <save> <--platform=linux\/arm64> <--output> <\([^>]*\)> <example\/failure:test>$/\1/p' "$log")
  [[ -n $archive && -f $archive ]]
  [[ $(sed -n '2p' "$log") == "kind <load> <image-archive> <--name> <test-cluster> <$archive>" ]]
  rm -rf "$test_dir"
}

non_owned_context_is_rejected() {
  owned_context() { return 1; }
  require_owned_context
}

test_opt_in_guard
test_operation_succeeded_polling
test_frontend_operation_result_contract
test_operation_running_statuses
test_verify_drain_log_event_order >/dev/null 2>&1
assert_status 0 worker_old_pods_gone_with_no_current_pods
assert_status 1 worker_old_pod_still_present
assert_status 1 worker_pod_snapshot_connection_error
assert_status 1 worker_pod_snapshot_without_singleton >/dev/null 2>&1
assert_status 1 worker_old_pods_gone_connection_error
test_worker_old_pods_gone_with_replacement_uid
test_component_snapshots_are_independent
test_component_old_pods_gone_checks_only_snapshotted_uids
test_chart_b_has_bounded_acceptance_delay
assert_status 1 failed_operation >/dev/null 2>&1
assert_status 1 timed_out_poll >/dev/null 2>&1
test_cleanup_preserves_unowned_directory
test_cluster_cleanup_orders_nfs_server_last
assert_status 1 unowned_cluster_delete_is_rejected >/dev/null 2>&1
test_cleanup_failure_is_reported_and_preserved
assert_status 130 on_int
assert_status 143 on_term
assert_status 1 connection_error_is_not_absent
assert_status 1 helm_connection_error_is_not_absent
assert_status 0 test_helm4_release_absence_contract
assert_status 1 non_owned_context_is_rejected >/dev/null 2>&1
test_signal_cleanup
test_native_docker_platform
assert_status 1 unsupported_docker_platform >/dev/null 2>&1
test_image_archive_load
test_image_archive_failure_propagates

printf 'acceptance helper checks passed\n'
