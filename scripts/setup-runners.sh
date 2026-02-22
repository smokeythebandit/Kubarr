#!/bin/bash
# Sets up Docker-based GitHub Actions self-hosted runners on the server.
#
# Run this script ON the server (with sudo available):
#   bash scripts/setup-runners.sh
#
# After running, create /mnt/storage/personal/runners/.env with your PAT:
#   cp infra/runners/.env.example /mnt/storage/personal/runners/.env
#   $EDITOR /mnt/storage/personal/runners/.env
#
# Then start the runners:
#   systemctl start github-runners.service

set -euo pipefail

RUNNERS_DIR="/mnt/storage/personal/runners"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
INFRA_DIR="$SCRIPT_DIR/runners"

# ── Step 0: Remove orphaned old runner directory ──────────────────────────────
if [ -d /mnt/storage/personal/github-runners ]; then
    echo "Removing orphaned /mnt/storage/personal/github-runners ..."
    sudo rm -rf /mnt/storage/personal/github-runners
fi

# ── Step 1: Create runner directory structure ─────────────────────────────────
echo "Creating $RUNNERS_DIR ..."
sudo mkdir -p "$RUNNERS_DIR/work"
sudo chown -R "$USER:$USER" "$RUNNERS_DIR"

# ── Step 2: Copy docker-compose and cleanup script ───────────────────────────
echo "Installing docker-compose.yml and cleanup.sh ..."
cp "$INFRA_DIR/docker-compose.yml" "$RUNNERS_DIR/docker-compose.yml"
cp "$INFRA_DIR/cleanup.sh"         "$RUNNERS_DIR/cleanup.sh"
chmod +x "$RUNNERS_DIR/cleanup.sh"

# Create .env from example if it doesn't exist yet
if [ ! -f "$RUNNERS_DIR/.env" ]; then
    cp "$INFRA_DIR/.env.example" "$RUNNERS_DIR/.env"
    echo ""
    echo "  !! ACTION REQUIRED: edit $RUNNERS_DIR/.env and set your GITHUB_TOKEN !!"
    echo ""
fi

# ── Step 3: Install systemd units ────────────────────────────────────────────
echo "Installing systemd units ..."
sudo cp "$INFRA_DIR/systemd/github-runners.service" /etc/systemd/system/github-runners.service
sudo cp "$INFRA_DIR/systemd/ci-cleanup.service"     /etc/systemd/system/ci-cleanup.service
sudo cp "$INFRA_DIR/systemd/ci-cleanup.timer"       /etc/systemd/system/ci-cleanup.timer

sudo systemctl daemon-reload

# ── Step 4: Enable services ───────────────────────────────────────────────────
echo "Enabling github-runners.service (start after setting .env) ..."
sudo systemctl enable github-runners.service

echo "Enabling and starting ci-cleanup.timer ..."
sudo systemctl enable --now ci-cleanup.timer

# ── Step 5: Pull runner image ─────────────────────────────────────────────────
echo "Pulling myoung34/github-runner:latest ..."
docker pull myoung34/github-runner:latest

# ── Done ──────────────────────────────────────────────────────────────────────
echo ""
echo "Setup complete. Next steps:"
echo "  1. Edit $RUNNERS_DIR/.env  (set GITHUB_TOKEN)"
echo "  2. systemctl start github-runners.service"
echo "  3. Check GitHub: repo Settings → Actions → Runners"
echo "  4. Verify: systemctl status github-runners.service"
