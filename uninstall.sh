#!/bin/bash
set -e

# Kubarr uninstallation script
# Usage: curl -sfL https://raw.githubusercontent.com/smokeythebandit/Kubarr/main/uninstall.sh | sh -

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
KUBARR_NAMESPACE="kubarr"

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
    echo "║        Kubarr Uninstallation Script        ║"
    echo "╚════════════════════════════════════════════╝"
    echo ""
}

# Confirm uninstallation
confirm_uninstall() {
    print_warning "This will remove Kubarr from your system."
    print_warning "All Kubarr data and deployed applications will be deleted."
    echo ""
    read -p "Are you sure you want to continue? (y/N) " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        print_info "Uninstallation cancelled."
        exit 0
    fi
}

# Check if k3s is installed
check_k3s() {
    if ! command -v k3s &> /dev/null; then
        print_error "k3s is not installed or not in PATH"
        exit 1
    fi

    if ! command -v kubectl &> /dev/null; then
        print_error "kubectl is not installed or not in PATH"
        exit 1
    fi

    export KUBECONFIG=/etc/rancher/k3s/k3s.yaml
}

# Remove Kubarr
remove_kubarr() {
    print_info "Removing Kubarr..."

    if ! kubectl get namespace "$KUBARR_NAMESPACE" &>/dev/null; then
        print_warning "Kubarr namespace not found, may already be uninstalled"
        return
    fi

    # Uninstall Helm release
    if command -v helm &> /dev/null; then
        if helm list -n "$KUBARR_NAMESPACE" | grep -q "kubarr"; then
            print_info "Uninstalling Kubarr Helm release..."
            helm uninstall kubarr -n "$KUBARR_NAMESPACE" --wait || true
            print_success "Kubarr Helm release removed"
        fi
    fi

    # Delete namespace (this will clean up any remaining resources)
    print_info "Deleting Kubarr namespace..."
    kubectl delete namespace "$KUBARR_NAMESPACE" --wait --timeout=60s || true
    print_success "Kubarr namespace removed"
}

# Optionally remove k3s
remove_k3s() {
    echo ""
    print_warning "Do you also want to remove k3s (Kubernetes)?"
    print_info "Only do this if you're not using k3s for anything else."
    read -p "Remove k3s? (y/N) " -n 1 -r
    echo
    if [[ $REPLY =~ ^[Yy]$ ]]; then
        print_info "Removing k3s..."

        if [ -f /usr/local/bin/k3s-uninstall.sh ]; then
            /usr/local/bin/k3s-uninstall.sh
            print_success "k3s removed successfully"
        else
            print_warning "k3s uninstall script not found"
        fi
    else
        print_info "k3s will be kept on your system"
    fi
}

# Print completion message
print_completion() {
    echo ""
    echo "╔════════════════════════════════════════════╗"
    echo "║    Kubarr Uninstallation Complete! 👋     ║"
    echo "╚════════════════════════════════════════════╝"
    echo ""
    print_success "Kubarr has been removed from your system."
    echo ""
    print_info "Thank you for trying Kubarr!"
    print_info "Feedback and issues: https://github.com/smokeythebandit/Kubarr/issues"
    echo ""
}

# Main uninstallation flow
main() {
    print_header
    confirm_uninstall
    check_k3s
    remove_kubarr
    remove_k3s
    print_completion
}

# Run main function
main
