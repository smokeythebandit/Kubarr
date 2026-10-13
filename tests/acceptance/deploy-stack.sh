#!/usr/bin/env bash
# Fixed Helm fixture for the run-owned Kind clusters used by application tests.
set -Eeuo pipefail

: "${TEST_CONTEXT:?Set the run-owned Kind context}"
: "${TEST_CHARTS_DIR:?Set the pinned, categorized chart checkout}"
: "${TEST_IMAGE_TAG:?Set the locally loaded application image tag}"
: "${TEST_USERNAME:?Set the disposable test administrator}"
: "${TEST_PASSWORD:?Set the disposable test password}"
export TEST_STORAGE_SIZE=${TEST_STORAGE_SIZE:-2Gi}
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

case "$TEST_CONTEXT" in
  kind-kubarr-e2e-*|kind-kubarr-acceptance-*) ;;
  *) printf 'Not an application test context: %s\n' "$TEST_CONTEXT" >&2; exit 1 ;;
esac
[[ $(kubectl config current-context) == "$TEST_CONTEXT" ]] || {
  printf 'The current context must match the run-owned test cluster\n' >&2
  exit 1
}
[[ $TEST_STORAGE_SIZE =~ ^[1-9][0-9]*(Mi|Gi|Ti)$ ]] || {
  printf 'Invalid test storage size\n' >&2
  exit 1
}

charts=(system/managed-nfs system/postgresql monitoring/fluent-bit
        monitoring/victorialogs monitoring/victoriametrics
        system/kubarr-backend system/kubarr-frontend system/openresty system/kubarr-worker)
for chart in "${charts[@]}"; do
  [[ -f "$TEST_CHARTS_DIR/$chart/Chart.yaml" ]] || {
    printf 'Missing test chart: %s\n' "$chart" >&2
    exit 1
  }
done
for chart in "${charts[@]}"; do
  helm dependency build --skip-refresh "$TEST_CHARTS_DIR/$chart" >/dev/null
done

kube=(kubectl --context "$TEST_CONTEXT" --request-timeout=10s)
release() {
  local name=$1 chart=$2
  shift 2
  helm upgrade --install --server-side=false "$name" "$TEST_CHARTS_DIR/$chart" \
    --kube-context "$TEST_CONTEXT" -n default --wait=legacy --timeout=5m "$@" >/dev/null
}
storage=(--set storage.media.existingClaim=media-data --set-string "storage.media.nfs.size=$TEST_STORAGE_SIZE")

release managed-nfs system/managed-nfs \
  --set-string "persistence.size=$TEST_STORAGE_SIZE" --set persistence.storageClassName=standard
release postgresql system/postgresql "${storage[@]}"
release fluent-bit monitoring/fluent-bit
release victorialogs monitoring/victorialogs "${storage[@]}"
release victoriametrics monitoring/victoriametrics "${storage[@]}"

for component in backend frontend worker; do
  args=(--set-string "image.repository=kubarr-$component" --set-string "image.tag=$TEST_IMAGE_TAG" \
        --set image.pullPolicy=Never)
  if [[ $component != frontend ]]; then
    args+=(--set database.secretName=kubarr-db-app "${storage[@]}")
  fi
  if [[ $component == backend ]]; then
    args+=(--set service.type=ClusterIP --set service.nodePort=null --set service.port=8000)
  fi
  release "kubarr-$component" "system/kubarr-$component" "${args[@]}"
done
release openresty system/openresty --set service.type=ClusterIP --set service.port=8080 \
  --set backend.service.namespace=kubarr-backend --set frontend.service.namespace=kubarr-frontend

# Readiness normally follows migrations; bound the explicit database check as well.
deadline=$((SECONDS + 120))
while true; do
  if [[ $(timeout 10s "${kube[@]}" exec -n kubarr-database statefulset/kubarr-db -- \
      psql -X -U kubarr -d kubarr -tAc \
      "SELECT to_regclass('public.storage_config') IS NOT NULL AND to_regclass('public.roles') IS NOT NULL;" \
      2>/dev/null) == t ]]; then
    break
  fi
  (( SECONDS < deadline )) || { printf 'Application schema did not become ready\n' >&2; exit 1; }
  sleep 2
done
python3 "$ROOT/stack-fixture.py" storage-secret | "${kube[@]}" apply -f - >/dev/null
# The disposable password and generated SQL travel on stdin, never command arguments.
python3 "$ROOT/stack-fixture.py" seed-sql | timeout 30s "${kube[@]}" exec -i \
  -n kubarr-database statefulset/kubarr-db -- psql -X -U kubarr -d kubarr -v ON_ERROR_STOP=1 >/dev/null
printf 'Application test stack is ready in %s\n' "$TEST_CONTEXT"
