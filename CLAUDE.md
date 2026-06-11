# Claude Code Instructions for Kubarr

## Development Environment

### Kubernetes Setup
- Using Kind cluster named `kubarr`
- Backend runs in namespace `kubarr`
- Supports both **local** and **remote** build/deploy workflows

### Port Forwarding - ALWAYS DO THIS AFTER DEPLOY
**CRITICAL:** After EVERY backend deployment/restart, IMMEDIATELY run:

**Local deployment:**
```bash
kubectl port-forward -n kubarr svc/kubarr-backend 8080:8000 &
```

**Remote deployment:**
```bash
kubectl --context kind-kubarr port-forward -n kubarr svc/kubarr-backend 8080:8000 &
```

Always verify it's working after redeployment:
```bash
curl -s http://localhost:8080/api/health
```

---

### Build and Deploy Backend (Local)
```bash
# Build Docker image
cd /home/bmartens/Projects/Kubarr
docker build -f docker/Dockerfile.backend -t kubarr-backend:latest --build-arg PROFILE=dev-release .

# Load into Kind cluster
kind load docker-image kubarr-backend:latest --name kubarr

# Restart deployment
kubectl rollout restart deployment/kubarr-backend -n kubarr
kubectl rollout status deployment/kubarr-backend -n kubarr --timeout=60s

# !!! CRITICAL - MUST restart port forward after EVERY deployment !!!
kubectl port-forward -n kubarr svc/kubarr-backend 8080:8000 >/dev/null 2>&1 &
sleep 2
curl -s http://localhost:8080/api/health  # Verify it works
```

**NEVER FORGET:** The port-forward ALWAYS breaks after deployment. Run it immediately.

---

### Build and Deploy Backend (Remote)

Use the remote workflow to offload builds to a more powerful server on the local network.

#### One-Time Remote Setup
```bash
# Configure remote server, Docker context, and Kind cluster
./scripts/remote-server-setup.sh --host <REMOTE_IP> --user <REMOTE_USER>

# Optionally specify an SSH key
./scripts/remote-server-setup.sh --host <REMOTE_IP> --user <REMOTE_USER> --key ~/.ssh/id_ed25519
```

This script will:
1. Verify SSH key-based authentication to the remote server
2. Check Docker is installed and running on the remote server
3. Create a Docker context named `kubarr-remote` targeting the remote Docker daemon over SSH
4. Create a Kind cluster on the remote server (via the Docker context)
5. Retrieve and merge the remote kubeconfig into `~/.kube/config`

#### Remote Build and Deploy
```bash
# Switch to remote Docker context
docker context use kubarr-remote

# Build Docker image (runs on remote server)
cd /home/bmartens/Projects/Kubarr
docker build -f docker/Dockerfile.backend -t kubarr-backend:latest --build-arg PROFILE=dev-release .

# Load into remote Kind cluster
kind load docker-image kubarr-backend:latest --name kubarr

# Restart deployment (use remote kubectl context)
kubectl --context kind-kubarr rollout restart deployment/kubarr-backend -n kubarr
kubectl --context kind-kubarr rollout status deployment/kubarr-backend -n kubarr --timeout=60s

# !!! CRITICAL - MUST restart port forward after EVERY deployment !!!
kubectl --context kind-kubarr port-forward -n kubarr svc/kubarr-backend 8080:8000 >/dev/null 2>&1 &
sleep 2
curl -s http://localhost:8080/api/health  # Verify it works
```

#### Using the Deploy Script (Remote)
```bash
# Automated deploy with --remote flag
./scripts/deploy.sh --remote
```

#### Switching Between Local and Remote
```bash
# Switch to remote
docker context use kubarr-remote

# Switch back to local
docker context use default
```

**NEVER FORGET:** The port-forward ALWAYS breaks after deployment. Run it immediately.

#### Remote Troubleshooting
- **`DOCKER_HOST` env var set:** Must be **unset** when using Docker contexts (`unset DOCKER_HOST`)
- **Build context is large:** Docker context transfers files over SSH. Ensure `.dockerignore` is optimized to minimize transfer size
- **x509 certificate errors:** The Kind cluster must use the remote server's actual IP as `apiServerAddress`, not `0.0.0.0` or `127.0.0.1`
- **Port 6443 unreachable:** Ensure the remote server's firewall allows connections on port 6443 (Kind API server)
- **SSH connection failures:** Verify SSH key-based auth works: `ssh <USER>@<HOST> 'echo OK'`
- **Kind not found on remote:** Kind runs **locally** and targets the remote Docker daemon via the Docker context — it does NOT need to be installed on the remote server

---

### SSH MCP Server for Remote Execution (Optional)

Configure an SSH MCP server so Claude Code can execute commands directly on the remote server. This is a **user-level** configuration stored in `~/.claude/mcp.json`, not a project-level setting.

#### Setup
```bash
# Add the SSH MCP server to Claude Code (user-level scope)
claude mcp add --transport stdio ssh-mcp --scope user -- npx -y ssh-mcp -- \
  --host=<REMOTE_IP> \
  --user=<REMOTE_USER> \
  --privateKeyPath=~/.ssh/id_ed25519
```

Replace `<REMOTE_IP>`, `<REMOTE_USER>`, and the key path with your actual values.

#### What This Enables
- Claude Code can run commands on the remote server via the `ssh-mcp` MCP tool
- Useful for inspecting remote Docker state, checking Kind cluster health, or running diagnostics
- Commands execute over SSH using key-based authentication (no password prompts)

#### Verification
After adding the MCP server, restart Claude Code and verify the tool is available:
```bash
# Check MCP server is registered
claude mcp list
```

#### Notes
- The SSH MCP server requires `npx` (Node.js) to be available locally
- The private key must have no passphrase (or use an SSH agent)
- This configuration is **optional** — all build/deploy commands work without it via Docker contexts and kubectl
- To remove the MCP server: `claude mcp remove ssh-mcp`
