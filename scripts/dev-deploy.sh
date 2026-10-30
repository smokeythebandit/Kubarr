#!/usr/bin/env bash
#
# Build the Kubarr images from this working tree and put them on the cluster,
# without going through CI or ghcr.
#
# Why this exists: a full release run takes around twenty minutes, and nothing
# unreleased may reach ghcr -- the published image has to be the exact artifact
# that passed acceptance. So development builds go to a registry on this
# workstation instead, which the node is configured to pull from. ghcr keeps
# meaning "this passed CI".
#
# Setup this depends on, done once:
#   * a registry container on this machine, port 5000
#   * a firewall rule allowing only the node to reach it
#   * machine.registries.mirrors on the node pointing at http://HOST:5000
#
# Usage:
#   scripts/dev-deploy.sh                 # all three components
#   scripts/dev-deploy.sh backend worker  # just these
set -euo pipefail

REGISTRY="${KUBARR_DEV_REGISTRY:-192.168.1.108:5000}"
NAMESPACE="${KUBARR_RELEASE_NAMESPACE:-default}"
PROFILE="${KUBARR_BUILD_PROFILE:-dev-release}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHARTS_ROOT="${KUBARR_CHARTS_ROOT:-$REPO_ROOT/../kubarr-charts}"

# Every build gets its own tag. Reusing one would leave the node free to serve
# a cached layer for a tag whose contents changed -- which has already cost an
# afternoon once, when a truncated layer kept being reused because the digest
# had not moved.
TAG="dev-$(git -C "$REPO_ROOT" rev-parse --short HEAD)-$(date +%H%M%S)"

declare -A DOCKERFILES=(
  [backend]="docker/Dockerfile.api"
  [worker]="docker/Dockerfile.worker"
  [frontend]="docker/Dockerfile.frontend"
)

components=("$@")
if [ ${#components[@]} -eq 0 ]; then
  components=(backend worker frontend)
fi

say() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }

# The Dockerfiles name their base images unqualified (`alpine:3.24`,
# `alpine/helm:4.3.0`). Podman asks which registry to resolve those from, and
# with no TTY to ask it fails outright. Docker Hub is what CI resolves them to,
# so say that once here rather than rewriting every FROM line.
REGISTRIES_CONF="$(mktemp)"
cat > "$REGISTRIES_CONF" <<'CONF'
unqualified-search-registries = ["docker.io"]
short-name-mode = "disabled"
CONF
export CONTAINERS_REGISTRIES_CONF="$REGISTRIES_CONF"
trap 'rm -f "$REGISTRIES_CONF"' EXIT

say "Checking the registry is up and the node can reach it"
curl -fsS --max-time 5 "http://${REGISTRY}/v2/_catalog" >/dev/null || {
  echo "Registry ${REGISTRY} is not answering. Start it with:"
  echo "  podman start kubarr-dev-registry"
  exit 1
}

for component in "${components[@]}"; do
  dockerfile="${DOCKERFILES[$component]:-}"
  if [ -z "$dockerfile" ]; then
    echo "Unknown component: $component" >&2
    exit 1
  fi
  image="${REGISTRY}/kubarr-${component}:${TAG}"

  say "Building kubarr-${component} (profile: ${PROFILE})"
  podman build \
    --build-arg "PROFILE=${PROFILE}" \
    --build-arg "COMMIT_HASH=$(git -C "$REPO_ROOT" rev-parse --short HEAD)" \
    --build-arg "CHANNEL=dev" \
    -f "${REPO_ROOT}/${dockerfile}" \
    -t "$image" \
    "$REPO_ROOT"

  say "Pushing ${image}"
  podman push --tls-verify=false "$image"
done

for component in "${components[@]}"; do
  release="kubarr-${component}"
  image_repo="${REGISTRY}/kubarr-${component}"

  say "Upgrading ${release} to ${TAG}"
  values="$(mktemp)"
  # Never --reuse-values: the image tag is a user-supplied value on these
  # releases, so a chart change alone leaves the old image running and the
  # upgrade reports success having changed nothing.
  helm get values "$release" -n "$NAMESPACE" -o yaml > "$values"
  python3 - "$values" "$image_repo" "$TAG" <<'PY'
import sys, yaml
path, repo, tag = sys.argv[1], sys.argv[2], sys.argv[3]
values = yaml.safe_load(open(path)) or {}
image = values.setdefault("image", {})
image["repository"] = repo
image["tag"] = tag
# The node has no credentials for this registry and needs none; it is plain
# HTTP on the local network.
image["pullPolicy"] = "IfNotPresent"
yaml.safe_dump(values, open(path, "w"), default_flow_style=False)
PY

  chart="${CHARTS_ROOT}/system/${release}"
  helm upgrade "$release" "$chart" -n "$NAMESPACE" -f "$values" --timeout 5m
  rm -f "$values"
done

say "Waiting for rollouts"
for component in "${components[@]}"; do
  kubectl rollout status "deploy/kubarr-${component}" -n "kubarr-${component}" --timeout=300s
done

say "Running on the cluster"
for component in "${components[@]}"; do
  printf '  %-10s ' "$component"
  kubectl get pods -n "kubarr-${component}" \
    -o jsonpath='{range .items[*]}{.status.containerStatuses[*].image}{" "}{.status.phase}{"\n"}{end}' \
    | grep -v '^\s*$' | head -1
done

echo
echo "Tag: ${TAG}"
echo "Nothing was pushed to ghcr. Release images still come from CI."
