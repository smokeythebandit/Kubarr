#!/bin/bash
set -e

# Kubarr installation script
# Usage: curl -sfL https://raw.githubusercontent.com/smokeythebandit/Kubarr/main/install.sh | sh -

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
KUBARR_NAMESPACE="kubarr"
KUBARR_VERSION="${KUBARR_VERSION:-latest}"
MIN_RAM_MB=2048
SKIP_K3S_INSTALL=false

# Print functions
print_info() {
    echo -e "${BLUE}ℹ${NC} $1"
}

print_success() {
    echo -e "${GREEN}✓${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}⚠${NC} $1"
}

print_error() {
    echo -e "${RED}✗${NC} $1"
}

print_header() {
    echo ""
    echo "╔════════════════════════════════════════════╗"
    echo "║          Kubarr Installation Script        ║"
    echo "║   Kubernetes Dashboard for Media Servers   ║"
    echo "╚════════════════════════════════════════════╝"
    echo ""
}

# Check if running as root
check_root() {
    if [ "$EUID" -eq 0 ]; then
        print_warning "Running as root. k3s installation will proceed with root privileges."
    else
        print_info "Running as non-root user. You may be prompted for sudo password."
    fi
}

# Detect OS and architecture
detect_system() {
    print_info "Detecting system information..."

    OS=$(uname -s | tr '[:upper:]' '[:lower:]')
    ARCH=$(uname -m)

    # macOS is not supported (k3s is Linux-only)
    if [ "$OS" = "darwin" ]; then
        echo ""
        print_error "macOS detected. k3s is not supported on macOS."
        echo ""
        echo "Options:"
        echo "  1. Set KUBECONFIG to an existing cluster and re-run"
        echo "  2. Install Rancher Desktop or Docker Desktop with Kubernetes enabled"
        echo "  3. Use a Linux VM or VPS"
        echo ""
        exit 1
    fi

    case "$ARCH" in
        x86_64)
            ARCH="amd64"
            ;;
        aarch64|arm64)
            ARCH="arm64"
            ;;
        armv7l)
            ARCH="arm"
            ;;
        *)
            print_error "Unsupported architecture: $ARCH"
            exit 1
            ;;
    esac

    print_success "Detected: $OS/$ARCH"

    # Check RAM
    if [ "$OS" = "linux" ]; then
        TOTAL_RAM=$(free -m | awk '/^Mem:/{print $2}')
        if [ "$TOTAL_RAM" -lt "$MIN_RAM_MB" ]; then
            print_warning "System has ${TOTAL_RAM}MB RAM. Recommended: ${MIN_RAM_MB}MB or more."
            print_warning "Kubarr may run slowly with limited resources."
            read -p "Continue anyway? (y/N) " -n 1 -r
            echo
            if [[ ! $REPLY =~ ^[Yy]$ ]]; then
                exit 1
            fi
        fi
    fi
}

# Check prerequisites
check_prerequisites() {
    print_info "Checking prerequisites..."

    # Check if curl is available
    if ! command -v curl &> /dev/null; then
        print_error "curl is required but not installed."
        exit 1
    fi

    print_success "Prerequisites satisfied"
}

# Check for an existing accessible Kubernetes cluster
check_existing_cluster() {
    if [ -n "$KUBECONFIG" ] && kubectl get nodes &>/dev/null; then
        print_success "Using existing Kubernetes cluster"
        SKIP_K3S_INSTALL=true
        return
    fi

    SKIP_K3S_INSTALL=false
}

# Install k3s
install_k3s() {
    if [ "$SKIP_K3S_INSTALL" = "true" ]; then
        return
    fi

    if command -v k3s &> /dev/null; then
        print_success "k3s is already installed"
        K3S_VERSION=$(k3s --version | head -n1 | cut -d' ' -f3)
        print_info "Version: $K3S_VERSION"
        return
    fi

    print_info "Installing k3s (lightweight Kubernetes)..."
    print_info "This may take a few minutes..."

    # Install k3s with minimal components
    curl -sfL https://get.k3s.io | sh -s - \
        --write-kubeconfig-mode 644 \
        --disable traefik \
        --disable servicelb

    print_success "k3s installed successfully"
}

# Wait for k3s to be ready
wait_for_k3s() {
    if [ "$SKIP_K3S_INSTALL" = "true" ]; then
        return
    fi

    print_info "Waiting for Kubernetes to be ready..."

    # Set KUBECONFIG for k3s
    export KUBECONFIG=/etc/rancher/k3s/k3s.yaml

    # Wait for node to be ready (max 60 seconds)
    for i in {1..60}; do
        if kubectl get nodes 2>/dev/null | grep -q "Ready"; then
            print_success "Kubernetes is ready"
            return
        fi
        sleep 1
    done

    print_error "Kubernetes did not become ready in time"
    exit 1
}

# Install Helm if not present
install_helm() {
    if command -v helm &> /dev/null; then
        print_success "Helm is already installed"
        return
    fi

    print_info "Installing Helm..."
    curl -sfL https://raw.githubusercontent.com/helm/helm/main/scripts/get-helm-3 | bash
    print_success "Helm installed successfully"
}

# Install or upgrade Kubarr (idempotent)
install_or_upgrade_kubarr() {
    print_info "Deploying Kubarr to Kubernetes..."

    # Use k3s KUBECONFIG if we installed k3s (not already set)
    if [ "$SKIP_K3S_INSTALL" != "true" ]; then
        export KUBECONFIG=/etc/rancher/k3s/k3s.yaml
    fi

    # Create namespace
    kubectl create namespace "$KUBARR_NAMESPACE" 2>/dev/null || true

    # Build common Helm arguments as an array for safe quoting
    HELM_ARGS=(
        kubarr
        oci://ghcr.io/smokeythebandit/kubarr/charts/kubarr
        -n "$KUBARR_NAMESPACE"
        --set frontend.service.type=NodePort
        --set frontend.service.nodePort=30080
        --set backend.service.type=NodePort
        --set backend.service.nodePort=30081
        --wait
        --timeout 5m
    )

    if [ "$KUBARR_VERSION" != "latest" ]; then
        HELM_ARGS+=(--version "$KUBARR_VERSION")
    fi

    # Check if Kubarr is already installed
    if helm list -n "$KUBARR_NAMESPACE" | grep -q kubarr; then
        CURRENT_VERSION=$(helm list -n "$KUBARR_NAMESPACE" -o json 2>/dev/null | \
            grep -o '"chart":"kubarr-[^"]*"' | head -n1 | sed 's/"chart":"kubarr-//;s/"//')
        print_info "Kubarr already installed (chart version: ${CURRENT_VERSION:-unknown})"
        if [ "$KUBARR_VERSION" = "latest" ]; then
            print_info "Upgrading to latest version..."
        else
            print_info "Upgrading to version: $KUBARR_VERSION"
        fi
        helm upgrade "${HELM_ARGS[@]}"
        print_success "Kubarr upgraded successfully"
    else
        print_info "Installing Kubarr Helm chart..."
        helm install "${HELM_ARGS[@]}"
        print_success "Kubarr deployed successfully"
    fi
}

# Detect the node's accessible IP address
detect_node_ip() {
    NODE_IP=$(kubectl get nodes \
        -o jsonpath='{.items[0].status.addresses[?(@.type=="InternalIP")].address}' \
        2>/dev/null || true)

    if [ -z "$NODE_IP" ]; then
        NODE_IP="127.0.0.1"
    fi
}

# Get credentials
get_credentials() {
    print_info "Retrieving access credentials..."

    if [ "$SKIP_K3S_INSTALL" != "true" ]; then
        export KUBECONFIG=/etc/rancher/k3s/k3s.yaml
    fi

    # Wait for backend pod to be ready
    kubectl wait --for=condition=ready pod -l app=kubarr-backend -n "$KUBARR_NAMESPACE" --timeout=60s &>/dev/null || true

    print_info "Credentials are configured by the bootstrap component"
    print_warning "Make sure to change the default password after first login!"
}

# Print completion message
print_completion() {
    detect_node_ip

    echo ""
    echo "╔════════════════════════════════════════════╗"
    echo "║     Kubarr Installation Complete! 🎉      ║"
    echo "╚════════════════════════════════════════════╝"
    echo ""
    print_success "Installation successful!"
    echo ""
    print_success "Kubarr is ready at: http://${NODE_IP}:30080"
    echo ""
    echo "Useful commands:"
    echo "  • Check status:    ${BLUE}kubectl get pods -n $KUBARR_NAMESPACE${NC}"
    echo "  • View logs:       ${BLUE}kubectl logs -n $KUBARR_NAMESPACE -l app=kubarr-backend${NC}"
    echo "  • Uninstall:       ${BLUE}helm uninstall kubarr -n $KUBARR_NAMESPACE${NC}"
    echo ""
    echo "Documentation: https://github.com/smokeythebandit/Kubarr/tree/main/docs"
    echo ""
}

# Main installation flow
main() {
    print_header
    check_root
    detect_system
    check_prerequisites
    check_existing_cluster
    install_k3s
    wait_for_k3s
    install_helm
    install_or_upgrade_kubarr
    get_credentials
    print_completion
}

# Run main function
main
