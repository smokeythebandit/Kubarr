#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LOCAL_BIN="$PROJECT_ROOT/bin"

echo "=== Kubarr Local K8s Setup ==="

# Create local bin directory
mkdir -p "$LOCAL_BIN"
export PATH="$LOCAL_BIN:$PATH"

# Check for Docker
if ! command -v docker &> /dev/null; then
    echo "Error: Docker is required but not installed."
    exit 1
fi

# Install kind if not present
if ! command -v kind &> /dev/null; then
    echo "Installing kind to $LOCAL_BIN..."
    curl -Lo "$LOCAL_BIN/kind" https://kind.sigs.k8s.io/dl/v0.24.0/kind-linux-amd64
    chmod +x "$LOCAL_BIN/kind"
fi

# Install kubectl if not present
if ! command -v kubectl &> /dev/null; then
    echo "Installing kubectl to $LOCAL_BIN..."
    curl -LO "https://dl.k8s.io/release/$(curl -L -s https://dl.k8s.io/release/stable.txt)/bin/linux/amd64/kubectl"
    chmod +x kubectl
    mv kubectl "$LOCAL_BIN/kubectl"
fi

# Create kind cluster if not exists
if ! kind get clusters 2>/dev/null | grep -q "kubarr"; then
    echo "Creating kind cluster 'kubarr' with port mappings..."

    # Create kind config with port mappings
    cat <<EOF | kind create cluster --name kubarr --wait 60s --config=-
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
nodes:
- role: control-plane
  extraPortMappings:
  - containerPort: 30080
    hostPort: 8080
    protocol: TCP
EOF
else
    echo "Kind cluster 'kubarr' already exists"
fi

# Set kubectl context
kubectl cluster-info --context kind-kubarr

echo ""
echo "=== Setup Complete ==="
echo ""
echo "Run ./scripts/deploy.sh to build and deploy Kubarr"
echo ""
echo "Once deployed, access Kubarr at: http://localhost:8080"
echo ""
echo "To delete the cluster:"
echo "  kind delete cluster --name kubarr"
