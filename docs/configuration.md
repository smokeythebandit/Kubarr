# Configuration Reference

This document provides a comprehensive reference for configuring Kubarr, including environment variables, Helm chart values, and common configuration scenarios.

## Table of Contents

- [Environment Variables](#environment-variables)
- [Helm Chart Values](#helm-chart-values)
- [Authentication Configuration](#authentication-configuration)
- [Storage Configuration](#storage-configuration)
- [Network Policy Configuration](#network-policy-configuration)
- [Resource Configuration](#resource-configuration)
- [Security Configuration](#security-configuration)

## Environment Variables

The Kubarr backend supports the following environment variables:

| Variable | Description | Default | Required |
|----------|-------------|---------|----------|
| `KUBARR_IN_CLUSTER` | Enable in-cluster Kubernetes API access | `true` | No |
| `KUBARR_DEFAULT_NAMESPACE` | Default namespace for media applications | `media` | No |
| `KUBARR_LOG_LEVEL` | Logging level (TRACE, DEBUG, INFO, WARN, ERROR) | `INFO` | No |
| `KUBARR_OAUTH2_ISSUER_URL` | OAuth2 issuer URL for token validation | `http://kubarr.kubarr.svc.cluster.local:8000` | No |
| `KUBARR_DATABASE_URL` | PostgreSQL connection string | - | Yes (if using database) |
| `KUBARR_JWT_SECRET` | Secret key for JWT token signing | - | Yes |
| `KUBARR_GLUETUN_IMAGE` | Docker image for the Gluetun VPN sidecar container | `qmcgaw/gluetun:v3.40` | No |

### Setting Environment Variables

#### Via Helm Chart

```yaml
backend:
  env:
    - name: KUBARR_LOG_LEVEL
      value: "DEBUG"
    - name: KUBARR_DEFAULT_NAMESPACE
      value: "media-apps"
    - name: KUBARR_JWT_SECRET
      valueFrom:
        secretKeyRef:
          name: kubarr-secrets
          key: jwt-secret
```

#### Via Kubernetes ConfigMap/Secret

```bash
# Create a secret for sensitive values
kubectl create secret generic kubarr-secrets \
  --from-literal=jwt-secret='your-secure-random-string' \
  -n kubarr

# Reference in deployment
kubectl set env deployment/kubarr-backend \
  KUBARR_JWT_SECRET=secretKeyRef:kubarr-secrets:jwt-secret \
  -n kubarr
```

## Helm Chart Values

### Namespace Configuration

```yaml
namespace:
  create: true          # Create namespace if it doesn't exist
  name: kubarr         # Namespace name
```

### Backend Configuration

```yaml
backend:
  replicaCount: 1      # Number of backend replicas

  image:
    repository: kubarr-backend    # Image repository
    pullPolicy: Never             # Image pull policy (IfNotPresent, Always, Never)
    tag: "latest"                # Image tag

  service:
    type: NodePort     # Service type (ClusterIP, NodePort, LoadBalancer)
    port: 8000        # Service port
    targetPort: 8000  # Container port
    nodePort: 30080   # NodePort (if type is NodePort)

  resources:
    limits:
      cpu: 500m       # Maximum CPU allocation
      memory: 512Mi   # Maximum memory allocation
    requests:
      cpu: 100m       # Minimum CPU allocation
      memory: 256Mi   # Minimum memory allocation

  livenessProbe:
    httpGet:
      path: /api/system/health
      port: 8000
    initialDelaySeconds: 1
    periodSeconds: 10

  readinessProbe:
    httpGet:
      path: /api/system/health
      port: 8000
    initialDelaySeconds: 0
    periodSeconds: 1

  env:
    - name: KUBARR_IN_CLUSTER
      value: "true"
    - name: KUBARR_DEFAULT_NAMESPACE
      value: "media"
    - name: KUBARR_LOG_LEVEL
      value: "INFO"
```

### Frontend Configuration

```yaml
frontend:
  replicaCount: 1      # Number of frontend replicas

  image:
    repository: kubarr-frontend
    pullPolicy: Never
    tag: "latest"

  service:
    type: ClusterIP    # Service type
    port: 80          # Service port

  resources:
    limits:
      cpu: 50m        # Maximum CPU allocation
      memory: 32Mi    # Maximum memory allocation
    requests:
      cpu: 10m        # Minimum CPU allocation
      memory: 16Mi    # Minimum memory allocation
```

### Service Account and RBAC

```yaml
serviceAccount:
  create: true               # Create service account
  name: kubarr              # Service account name
  annotations: {}           # Service account annotations

rbac:
  create: true              # Create RBAC resources (ClusterRole, ClusterRoleBinding)
  rules:                    # RBAC rules for cluster access
    - apiGroups: [""]
      resources: ["namespaces"]
      verbs: ["get", "list", "watch", "create", "delete"]
    - apiGroups: ["apps"]
      resources: ["deployments", "replicasets", "daemonsets", "statefulsets"]
      verbs: ["get", "list", "watch", "create", "update", "delete", "patch"]
    # Additional rules...
```

## Authentication Configuration

Kubarr uses JWT-based authentication with support for OAuth2 and 2FA (TOTP).

### Basic Authentication Setup

```yaml
auth:
  # Admin user (created on first run)
  adminUser:
    username: "admin"
    email: "admin@kubarr.local"
    password: ""  # Auto-generated if empty

  # JWT configuration
  jwt:
    existingSecret: ""              # Use existing Kubernetes secret
    privateKey: ""                  # RSA private key (auto-generated if empty)
    publicKey: ""                   # RSA public key (auto-generated if empty)
    accessTokenExpire: 3600         # Access token expiry (seconds)
    refreshTokenExpire: 604800      # Refresh token expiry (seconds)

  # User registration
  registration:
    enabled: true                   # Allow new user registration
    requireApproval: true           # Require admin approval for new users
```

### Using Existing JWT Secret

```yaml
auth:
  jwt:
    existingSecret: "kubarr-jwt-keys"
```

Create the secret manually:

```bash
# Generate RSA key pair
openssl genrsa -out private.pem 2048
openssl rsa -in private.pem -outform PEM -pubout -out public.pem

# Create Kubernetes secret
kubectl create secret generic kubarr-jwt-keys \
  --from-file=private-key=private.pem \
  --from-file=public-key=public.pem \
  -n kubarr

# Clean up local keys
rm private.pem public.pem
```

### OAuth2 Configuration

```yaml
oauth2:
  enabled: true                     # Enable OAuth2 provider
  provider:
    issuerUrl: "http://localhost:8080"  # OAuth2 issuer URL
```

For production with custom domain:

```yaml
oauth2:
  enabled: true
  provider:
    issuerUrl: "https://kubarr.example.com"
```

### Disabling User Registration

```yaml
auth:
  registration:
    enabled: false     # Disable new user registration
```

### Custom Token Expiry

```yaml
auth:
  jwt:
    accessTokenExpire: 7200      # 2 hours
    refreshTokenExpire: 2592000  # 30 days
```

## Storage Configuration

Kubarr supports hostPath storage for the file browser feature.

### Enable HostPath Storage

```yaml
storage:
  hostPath:
    enabled: true
    rootPath: "/mnt/data/kubarr"  # Host path on Kubernetes nodes
  mountPath: /data/storage         # Mount path inside containers
```

### Storage Examples

#### Local Development (Kind)

```yaml
storage:
  hostPath:
    enabled: true
    rootPath: "/tmp/kubarr-storage"
  mountPath: /data/storage
```

#### Production with NFS-backed Storage

```yaml
storage:
  hostPath:
    enabled: true
    rootPath: "/mnt/nfs/media"  # NFS mount on nodes
  mountPath: /data/storage
```

#### Disable Storage

```yaml
storage:
  hostPath:
    enabled: false
```

## Network Policy Configuration

Network policies control which namespaces and pods can communicate with Kubarr and vice versa.

### Basic Network Policy

```yaml
networkPolicy:
  enabled: true                    # Enable network policies
  allowedNamespaces:              # Namespaces Kubarr can access
    - sonarr
    - radarr
    - qbittorrent
    - transmission
    - deluge
    - rutorrent
    - jackett
    - jellyfin
    - jellyseerr
    - sabnzbd
    - victoriametrics
    - victorialogs
    - grafana
```

### Custom Namespace Access

```yaml
networkPolicy:
  enabled: true
  allowedNamespaces:
    - media
    - monitoring
    - downloads
    - custom-app
```

### Disable Network Policies

```yaml
networkPolicy:
  enabled: false  # Allow all traffic (not recommended for production)
```

## Resource Configuration

### Production Resource Limits

```yaml
backend:
  resources:
    limits:
      cpu: 2000m
      memory: 2Gi
    requests:
      cpu: 500m
      memory: 1Gi

frontend:
  resources:
    limits:
      cpu: 200m
      memory: 128Mi
    requests:
      cpu: 50m
      memory: 64Mi
```

### Development Resource Limits

```yaml
backend:
  resources:
    limits:
      cpu: 500m
      memory: 512Mi
    requests:
      cpu: 100m
      memory: 256Mi

frontend:
  resources:
    limits:
      cpu: 50m
      memory: 32Mi
    requests:
      cpu: 10m
      memory: 16Mi
```

## Security Configuration

### Pod Security Context

```yaml
podSecurityContext:
  runAsNonRoot: true   # Run as non-root user
  runAsUser: 1000      # User ID
  fsGroup: 1000        # Group ID for volume ownership
```

### Container Security Context

```yaml
securityContext:
  allowPrivilegeEscalation: false  # Prevent privilege escalation
  capabilities:
    drop:
      - ALL                        # Drop all capabilities
  readOnlyRootFilesystem: false    # Allow writes to filesystem
```

### Hardened Security (Production)

```yaml
podSecurityContext:
  runAsNonRoot: true
  runAsUser: 1000
  fsGroup: 1000
  seccompProfile:
    type: RuntimeDefault

securityContext:
  allowPrivilegeEscalation: false
  capabilities:
    drop:
      - ALL
  readOnlyRootFilesystem: true
  runAsNonRoot: true
  runAsUser: 1000
```

## Advanced Configuration

### Node Selector

```yaml
nodeSelector:
  kubernetes.io/hostname: specific-node
  disktype: ssd
```

### Tolerations

```yaml
tolerations:
  - key: "dedicated"
    operator: "Equal"
    value: "kubarr"
    effect: "NoSchedule"
```

### Affinity Rules

```yaml
affinity:
  podAntiAffinity:
    preferredDuringSchedulingIgnoredDuringExecution:
      - weight: 100
        podAffinityTerm:
          labelSelector:
            matchExpressions:
              - key: app.kubernetes.io/name
                operator: In
                values:
                  - kubarr-backend
          topologyKey: kubernetes.io/hostname
```

### Pod Annotations

```yaml
podAnnotations:
  prometheus.io/scrape: "true"
  prometheus.io/port: "8000"
  prometheus.io/path: "/metrics"
```

### Image Pull Secrets

```yaml
imagePullSecrets:
  - name: registry-credentials
```

## Complete Production Example

```yaml
namespace:
  create: true
  name: kubarr

backend:
  replicaCount: 3
  image:
    repository: ghcr.io/yourusername/kubarr-backend
    pullPolicy: IfNotPresent
    tag: "v1.0.0"
  service:
    type: ClusterIP
    port: 8000
  resources:
    limits:
      cpu: 2000m
      memory: 2Gi
    requests:
      cpu: 500m
      memory: 1Gi
  env:
    - name: KUBARR_LOG_LEVEL
      value: "INFO"
    - name: KUBARR_DEFAULT_NAMESPACE
      value: "media"

frontend:
  replicaCount: 2
  image:
    repository: ghcr.io/yourusername/kubarr-frontend
    pullPolicy: IfNotPresent
    tag: "v1.0.0"
  resources:
    limits:
      cpu: 200m
      memory: 128Mi
    requests:
      cpu: 50m
      memory: 64Mi

auth:
  jwt:
    existingSecret: "kubarr-jwt-keys"
    accessTokenExpire: 3600
    refreshTokenExpire: 604800
  registration:
    enabled: true
    requireApproval: true

oauth2:
  enabled: true
  provider:
    issuerUrl: "https://kubarr.example.com"

storage:
  hostPath:
    enabled: true
    rootPath: "/mnt/nfs/media"
  mountPath: /data/storage

networkPolicy:
  enabled: true
  allowedNamespaces:
    - sonarr
    - radarr
    - jellyfin
    - monitoring

podSecurityContext:
  runAsNonRoot: true
  runAsUser: 1000
  fsGroup: 1000

securityContext:
  allowPrivilegeEscalation: false
  capabilities:
    drop:
      - ALL
  readOnlyRootFilesystem: false

nodeSelector:
  disktype: ssd

affinity:
  podAntiAffinity:
    preferredDuringSchedulingIgnoredDuringExecution:
      - weight: 100
        podAffinityTerm:
          labelSelector:
            matchExpressions:
              - key: app.kubernetes.io/name
                operator: In
                values:
                  - kubarr-backend
          topologyKey: kubernetes.io/hostname
```

## See Also

- [Quick Start](./quick-start.md) - Get Kubarr running in minutes
