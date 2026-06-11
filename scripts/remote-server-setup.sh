#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LOCAL_BIN="$PROJECT_ROOT/bin"

REMOTE_HOST=""
REMOTE_USER=""
SSH_KEY=""
readonly DOCKER_CONTEXT_NAME="kubarr-remote"
readonly KIND_CLUSTER_NAME="kubarr"

# Temp files to clean up on exit
TEMP_FILES=()

cleanup() {
    for f in "${TEMP_FILES[@]}"; do
        rm -f "$f"
    done
}
trap cleanup EXIT

usage() {
    echo "Usage: $0 --host <REMOTE_IP> --user <REMOTE_USER> [--key <SSH_KEY_PATH>]"
    echo ""
    echo "Set up a remote server for Kubarr build compute."
    echo ""
    echo "Options:"
    echo "  --host    Remote server IP address (required)"
    echo "  --user    Remote SSH user (required)"
    echo "  --key     Path to SSH private key (default: ~/.ssh/id_rsa or ~/.ssh/id_ed25519)"
    echo "  --help    Show this help message"
    echo ""
    echo "Example:"
    echo "  $0 --host 192.168.1.100 --user bmartens"
    exit 1
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --host)
            if [[ $# -lt 2 ]]; then
                echo "Error: --host requires a value"
                usage
            fi
            REMOTE_HOST="$2"
            shift 2
            ;;
        --user)
            if [[ $# -lt 2 ]]; then
                echo "Error: --user requires a value"
                usage
            fi
            REMOTE_USER="$2"
            shift 2
            ;;
        --key)
            if [[ $# -lt 2 ]]; then
                echo "Error: --key requires a value"
                usage
            fi
            SSH_KEY="$2"
            shift 2
            ;;
        --help)
            usage
            ;;
        *)
            echo "Error: Unknown option: $1"
            usage
            ;;
    esac
done

# Validate required arguments
if [[ -z "$REMOTE_HOST" ]]; then
    echo "Error: --host is required"
    usage
fi

if [[ -z "$REMOTE_USER" ]]; then
    echo "Error: --user is required"
    usage
fi

echo "=== Kubarr Remote Server Setup ==="
echo ""
echo "Remote host: $REMOTE_USER@$REMOTE_HOST"
echo ""

# Ensure local bin is on PATH
mkdir -p "$LOCAL_BIN"
export PATH="$LOCAL_BIN:$PATH"

# --- Step 1: Check local prerequisites ---

echo "--- Checking local prerequisites ---"

if ! command -v docker &> /dev/null; then
    echo "Error: Docker is required locally but not installed."
    exit 1
fi
echo "  Docker CLI: OK"

if ! command -v kind &> /dev/null; then
    echo "  Installing kind to $LOCAL_BIN..."
    curl -Lo "$LOCAL_BIN/kind" https://kind.sigs.k8s.io/dl/v0.24.0/kind-linux-amd64
    chmod +x "$LOCAL_BIN/kind"
fi
echo "  kind: OK"

if ! command -v kubectl &> /dev/null; then
    echo "  Installing kubectl to $LOCAL_BIN..."
    curl -LO "https://dl.k8s.io/release/$(curl -L -s https://dl.k8s.io/release/stable.txt)/bin/linux/amd64/kubectl"
    chmod +x kubectl
    mv kubectl "$LOCAL_BIN/kubectl"
fi
echo "  kubectl: OK"

# Check for DOCKER_HOST override
if [[ -n "${DOCKER_HOST:-}" ]]; then
    echo ""
    echo "Warning: DOCKER_HOST is set to '$DOCKER_HOST'"
    echo "This will override Docker context settings."
    echo "Please unset it before using docker contexts:"
    echo "  unset DOCKER_HOST"
    echo ""
fi

# --- Step 2: Verify SSH connectivity ---

echo ""
echo "--- Verifying SSH connectivity ---"

# Determine SSH key to use
SSH_OPTS=(-o BatchMode=yes -o ConnectTimeout=10 -o StrictHostKeyChecking=accept-new)

if [[ -n "$SSH_KEY" ]]; then
    if [[ ! -f "$SSH_KEY" ]]; then
        echo "Error: SSH key not found: $SSH_KEY"
        exit 1
    fi
    SSH_OPTS+=(-i "$SSH_KEY")
    echo "  Using SSH key: $SSH_KEY"
fi

if ! ssh "${SSH_OPTS[@]}" "$REMOTE_USER@$REMOTE_HOST" 'echo OK' &> /dev/null; then
    echo "Error: SSH key-based authentication failed for $REMOTE_USER@$REMOTE_HOST"
    echo ""
    echo "Please set up SSH key-based authentication:"
    echo "  1. Generate a key (if you don't have one):"
    echo "     ssh-keygen -t ed25519"
    echo "  2. Copy the key to the remote server:"
    echo "     ssh-copy-id $REMOTE_USER@$REMOTE_HOST"
    echo "  3. Verify it works:"
    echo "     ssh $REMOTE_USER@$REMOTE_HOST 'echo OK'"
    echo "  4. Re-run this script"
    exit 1
fi
echo "  SSH connectivity: OK"

# --- Step 3: Check Docker on remote server ---

echo ""
echo "--- Checking Docker on remote server ---"

if ! ssh "${SSH_OPTS[@]}" "$REMOTE_USER@$REMOTE_HOST" 'command -v docker' &> /dev/null; then
    echo "Error: Docker is not installed on the remote server."
    echo ""
    echo "Install Docker on the remote server:"
    echo "  # Ubuntu/Debian:"
    echo "  curl -fsSL https://get.docker.com | sh"
    echo ""
    echo "  # Then add your user to the docker group:"
    echo "  sudo usermod -aG docker $REMOTE_USER"
    echo "  # Log out and back in for group changes to take effect"
    exit 1
fi
echo "  Docker installed: OK"

# Check if remote user can run docker without sudo
if ! ssh "${SSH_OPTS[@]}" "$REMOTE_USER@$REMOTE_HOST" 'docker ps' &> /dev/null; then
    echo "Error: User '$REMOTE_USER' cannot run Docker commands on the remote server."
    echo ""
    echo "Add the user to the docker group:"
    echo "  ssh $REMOTE_USER@$REMOTE_HOST 'sudo usermod -aG docker $REMOTE_USER'"
    echo "  # Log out and back in for group changes to take effect"
    exit 1
fi
echo "  Docker permissions: OK"

# SC2155: Declare and assign separately
REMOTE_DOCKER_VERSION=""
REMOTE_DOCKER_VERSION=$(ssh "${SSH_OPTS[@]}" "$REMOTE_USER@$REMOTE_HOST" 'docker --version')
echo "  Remote Docker: $REMOTE_DOCKER_VERSION"

# --- Step 4: Create Docker context ---

echo ""
echo "--- Setting up Docker context ---"

DOCKER_HOST_URL="ssh://$REMOTE_USER@$REMOTE_HOST"

if docker context inspect "$DOCKER_CONTEXT_NAME" &> /dev/null; then
    echo "  Docker context '$DOCKER_CONTEXT_NAME' already exists"

    # Verify the existing context points to the correct host
    # SC2155: Declare and assign separately
    EXISTING_HOST=""
    EXISTING_HOST=$(docker context inspect "$DOCKER_CONTEXT_NAME" --format '{{.Endpoints.docker.Host}}')
    if [[ "$EXISTING_HOST" != "$DOCKER_HOST_URL" ]]; then
        echo "  Warning: Existing context points to '$EXISTING_HOST', expected '$DOCKER_HOST_URL'"
        echo "  Removing and recreating context..."
        docker context rm "$DOCKER_CONTEXT_NAME" -f
        docker context create "$DOCKER_CONTEXT_NAME" --docker "host=$DOCKER_HOST_URL"
        echo "  Docker context recreated"
    else
        echo "  Docker context host matches: OK"
    fi
else
    docker context create "$DOCKER_CONTEXT_NAME" --docker "host=$DOCKER_HOST_URL"
    echo "  Docker context '$DOCKER_CONTEXT_NAME' created"
fi

# Verify Docker context works
if ! docker --context "$DOCKER_CONTEXT_NAME" ps &> /dev/null; then
    echo "Error: Docker context '$DOCKER_CONTEXT_NAME' failed to connect."
    echo "Verify SSH connectivity and Docker daemon status on remote server."
    exit 1
fi
echo "  Docker context verification: OK"

# --- Step 5: Create Kind cluster on remote Docker ---

echo ""
echo "--- Setting up Kind cluster on remote Docker ---"

# Check if Kind cluster already exists (via remote docker context)
if docker --context "$DOCKER_CONTEXT_NAME" ps --filter "name=kubarr-control-plane" --format '{{.Names}}' 2>/dev/null | grep -q "kubarr-control-plane"; then
    echo "  Kind cluster '$KIND_CLUSTER_NAME' already exists on remote server"
    echo "  To recreate, first delete it:"
    echo "    docker context use $DOCKER_CONTEXT_NAME"
    echo "    kind delete cluster --name $KIND_CLUSTER_NAME"
    echo "    docker context use default"
else
    echo "  Creating Kind cluster '$KIND_CLUSTER_NAME' on remote server..."

    KIND_CONFIG_FILE="$PROJECT_ROOT/kind-remote-config.yaml"
    if [[ ! -f "$KIND_CONFIG_FILE" ]]; then
        echo "Error: Kind remote config not found: $KIND_CONFIG_FILE"
        exit 1
    fi

    # Substitute the remote IP into the Kind config
    TEMP_KIND_CONFIG=$(mktemp)
    TEMP_FILES+=("$TEMP_KIND_CONFIG")
    sed "s/__API_SERVER_ADDRESS__/$REMOTE_HOST/" "$KIND_CONFIG_FILE" > "$TEMP_KIND_CONFIG"

    # Switch to remote docker context for Kind cluster creation
    docker context use "$DOCKER_CONTEXT_NAME"

    kind create cluster --name "$KIND_CLUSTER_NAME" --wait 60s --config="$TEMP_KIND_CONFIG"

    # Switch back to default context
    docker context use default

    echo "  Kind cluster created on remote server"
fi

# --- Step 6: Retrieve and configure kubeconfig ---

echo ""
echo "--- Configuring kubeconfig ---"

# Get kubeconfig from the Kind cluster (needs remote docker context active)
docker context use "$DOCKER_CONTEXT_NAME"

REMOTE_KUBECONFIG=$(mktemp)
TEMP_FILES+=("$REMOTE_KUBECONFIG")
kind get kubeconfig --name "$KIND_CLUSTER_NAME" > "$REMOTE_KUBECONFIG"

# Switch back to default context
docker context use default

# Verify and fix the server address in kubeconfig
# Kind may set it to 127.0.0.1 even with apiServerAddress configured
if grep -q "127\.0\.0\.1" "$REMOTE_KUBECONFIG"; then
    echo "  Fixing kubeconfig server address (127.0.0.1 -> $REMOTE_HOST)..."
    sed -i "s/127\.0\.0\.1/$REMOTE_HOST/g" "$REMOTE_KUBECONFIG"
fi

# Merge with existing kubeconfig
KUBE_DIR="$HOME/.kube"
mkdir -p "$KUBE_DIR"

if [[ -f "$KUBE_DIR/config" ]]; then
    echo "  Merging remote kubeconfig with existing config..."
    MERGED_CONFIG=$(mktemp)
    TEMP_FILES+=("$MERGED_CONFIG")
    KUBECONFIG="$KUBE_DIR/config:$REMOTE_KUBECONFIG" kubectl config view --merge --flatten > "$MERGED_CONFIG"
    cp "$KUBE_DIR/config" "$KUBE_DIR/config.backup"
    mv "$MERGED_CONFIG" "$KUBE_DIR/config"
    chmod 600 "$KUBE_DIR/config"
    echo "  Existing config backed up to $KUBE_DIR/config.backup"
else
    cp "$REMOTE_KUBECONFIG" "$KUBE_DIR/config"
    chmod 600 "$KUBE_DIR/config"
fi

echo "  Kubeconfig configured: OK"

# --- Step 7: Verify cluster access ---

echo ""
echo "--- Verifying cluster access ---"

# Test port 6443 connectivity
if ! timeout 5 bash -c "echo > /dev/tcp/$REMOTE_HOST/6443" 2>/dev/null; then
    echo "Warning: Port 6443 on $REMOTE_HOST may be blocked by a firewall."
    echo "Ensure the port is open for Kubernetes API access."
    echo ""
fi

if kubectl --context "kind-$KIND_CLUSTER_NAME" get nodes &> /dev/null; then
    NODE_STATUS=$(kubectl --context "kind-$KIND_CLUSTER_NAME" get nodes --no-headers 2>/dev/null)
    echo "  Cluster nodes:"
    echo "  $NODE_STATUS"
    echo "  Cluster access: OK"
else
    echo "Warning: Could not verify cluster access."
    echo "The cluster may still be starting up. Try again in a moment:"
    echo "  kubectl --context kind-$KIND_CLUSTER_NAME get nodes"
fi

# --- Done ---

echo ""
echo "=== Remote Server Setup Complete ==="
echo ""
echo "Docker context: $DOCKER_CONTEXT_NAME"
echo "Kind cluster:   $KIND_CLUSTER_NAME (on $REMOTE_HOST)"
echo "Kubectl context: kind-$KIND_CLUSTER_NAME"
echo ""
echo "Usage:"
echo "  # Build on remote server"
echo "  docker --context $DOCKER_CONTEXT_NAME build -f docker/Dockerfile.backend -t kubarr-backend:latest --build-arg PROFILE=dev-release ."
echo ""
echo "  # Load image into remote Kind cluster"
echo "  docker context use $DOCKER_CONTEXT_NAME"
echo "  kind load docker-image kubarr-backend:latest --name $KIND_CLUSTER_NAME"
echo "  docker context use default"
echo ""
echo "  # Deploy and access"
echo "  kubectl --context kind-$KIND_CLUSTER_NAME apply -f k8s/"
echo "  kubectl --context kind-$KIND_CLUSTER_NAME port-forward -n kubarr svc/kubarr-backend 8080:8000 &"
echo ""
echo "  # Or use deploy script with --remote flag"
echo "  ./scripts/deploy.sh --remote"
echo ""
echo "To tear down:"
echo "  docker context use $DOCKER_CONTEXT_NAME"
echo "  kind delete cluster --name $KIND_CLUSTER_NAME"
echo "  docker context use default"
echo "  docker context rm $DOCKER_CONTEXT_NAME"
