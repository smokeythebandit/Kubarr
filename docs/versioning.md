# Versioning System

Kubarr uses **Semantic Versioning** (SemVer) with a release channel system to distinguish between development, release candidate, and production builds.

## Version Format

Versions follow the **MAJOR.MINOR.PATCH** format (e.g., `1.2.3`):

- **MAJOR**: Incremented for incompatible API changes or major features
- **MINOR**: Incremented for backwards-compatible new features
- **PATCH**: Incremented for backwards-compatible bug fixes

## Release Channels

Kubarr has three release channels:

| Channel | Description | Tag Pattern | Badge Color | Use Case |
|---------|-------------|-------------|-------------|----------|
| **dev** | Development builds | No tag or unmatched tag | Blue | Active development, latest features |
| **release** | Release candidates | `v*.*.*-rc.*`, `v*.*.*-beta.*` | Yellow | Pre-release testing, beta features |
| **stable** | Production releases | `v*.*.*` | Green | Production deployments |

### Channel Detection

The build system automatically detects the channel based on git tags:

```bash
# Stable release (channel: stable)
git tag v1.2.3

# Release candidate (channel: release)
git tag v1.2.3-rc.1
git tag v1.2.3-beta.1

# Development build (channel: dev)
# No tag or any commit not matching above patterns
```

## Version Bumping Workflow

### Using the Script

The `scripts/version-bump.sh` script automates version updates across the entire codebase:

```bash
# Bump patch version (0.1.0 -> 0.1.1)
./scripts/version-bump.sh patch --tag

# Bump minor version (0.1.0 -> 0.2.0)
./scripts/version-bump.sh minor --tag

# Bump major version (0.1.0 -> 1.0.0)
./scripts/version-bump.sh major --tag

# Create a release candidate
./scripts/version-bump.sh minor --channel release --tag

# Dry run (preview changes without modifying files)
./scripts/version-bump.sh patch --dry-run
```

#### Script Options

- `<major|minor|patch>`: Version component to bump (required)
- `--channel <dev|release|stable>`: Release channel (default: stable)
- `--tag`: Automatically create a git tag after bumping
- `--dry-run`: Show what would change without modifying files

### What Gets Updated

The version-bump script updates:

1. **code/frontend/package.json**: `"version"` field
2. **code/backend/Cargo.toml**: `version` field
3. **code/backend/Cargo.lock**: via `cargo check`

### Manual Version Bump

If you prefer to bump versions manually:

1. Update `code/frontend/package.json` version
2. Update `code/backend/Cargo.toml` version
3. Run `cd code/backend && cargo check` to update Cargo.lock
4. Commit the changes
5. Create a git tag: `git tag -a v1.2.3 -m "Release 1.2.3"`

## Release Workflow

### Development Releases (dev channel)

Development builds are created automatically from any untagged commit:

```bash
# Make changes
git add .
git commit -m "Add new feature"
git push

# Deploy (channel will be "dev")
./scripts/deploy.sh
```

### Release Candidates (release channel)

Create release candidates for testing before stable releases:

```bash
# Bump version and create RC tag
./scripts/version-bump.sh minor --channel release --tag

# Push changes and tag
git push
git push origin v0.2.0-rc.1

# Deploy (channel will be "release")
./scripts/deploy.sh
```

### Stable Releases (stable channel)

Create stable releases for production deployments:

```bash
# Bump version and create stable tag
./scripts/version-bump.sh patch --tag

# Push changes and tag
git push
git push origin v0.1.1

# Deploy (channel will be "stable")
./scripts/deploy.sh
```

### Converting RC to Stable

If you have a release candidate that's ready for production:

```bash
# Create a stable tag at the same commit as the RC
git tag -a v0.2.0 -m "Release 0.2.0" v0.2.0-rc.1
git push origin v0.2.0
```

## Branch Protection Rules

To enforce the versioning workflow and prevent accidental releases, configure the following branch protection rules:

### Main Branch Protection

Configure these rules for the `main` branch in your GitHub repository:

1. **Settings → Branches → Add rule**
2. **Branch name pattern**: `main`
3. **Protect matching branches**:
   - ✅ Require a pull request before merging
     - ✅ Require approvals (at least 1)
     - ✅ Dismiss stale pull request approvals when new commits are pushed
   - ✅ Require status checks to pass before merging
     - ✅ Require branches to be up to date before merging
     - Required checks:
       - `docker / docker (kubarr-backend-test)` - Backend tests
       - `docker / docker (kubarr-frontend-test)` - Frontend tests
       - `docs` - Documentation build
   - ✅ Require conversation resolution before merging
   - ✅ Do not allow bypassing the above settings (even for admins)

### Tag Protection Rules

Protect version tags to prevent accidental modifications or deletions:

1. **Settings → Tags → Add rule**
2. **Tag name pattern**: `v*`
3. **Protection rules**:
   - ✅ Protect matching tags
   - Only allow repository admins to delete protected tags

### Recommended Workflow with Protection

With branch protection enabled:

```bash
# 1. Create a feature branch
git checkout -b feature/my-feature

# 2. Make changes and commit
git add .
git commit -m "Add my feature"

# 3. Push branch
git push origin feature/my-feature

# 4. Create Pull Request on GitHub
# - Tests will run automatically
# - Request review from team member
# - Wait for approval

# 5. Merge PR (via GitHub UI)
# - Squash and merge or merge commit
# - Delete feature branch after merge

# 6. Create release (on main branch)
git checkout main
git pull
./scripts/version-bump.sh patch --tag
git push
git push origin v0.1.1
```

### Enforcing Version Tags

To require that all deployments to production use tagged releases, you can:

1. **GitHub Actions**: Add a condition to deployment workflows
   ```yaml
   if: startsWith(github.ref, 'refs/tags/v')
   ```

2. **Kubernetes**: Use image tags based on git tags instead of `:latest`
   ```yaml
   image: kubarr-backend:v0.1.1
   ```

3. **Monitoring**: Set up alerts for deployments with `channel: dev` in production

## Version Display

Version information is displayed in multiple locations:

### UI Footer

The frontend footer shows:
- Channel badge (color-coded: blue=dev, yellow=release, green=stable)
- Version number (e.g., `v0.1.0`)
- Short commit hash (7 characters)
- Build date

Example: `[dev] v0.1.0 a1b2c3d (2026-02-16) | [dev] v0.1.0 a1b2c3d (2026-02-16)`

### API Endpoint

Version information is available via REST API:

```bash
curl http://localhost:8080/api/system/version | jq
```

Response:
```json
{
  "version": "0.1.0",
  "channel": "dev",
  "commit_hash": "a1b2c3d",
  "build_time": "2026-02-16T10:30:00Z",
  "rust_version": "1.83",
  "backend": "rust"
}
```

## CI/CD Integration

### GitHub Actions

The build workflow automatically extracts version metadata:

```yaml
- name: Extract version metadata
  id: meta
  run: |
    VERSION=$(grep '"version"' code/frontend/package.json | head -1 | sed 's/.*"version": "\(.*\)".*/\1/')
    echo "version=$VERSION" >> $GITHUB_OUTPUT

    if [[ "${{ github.ref }}" =~ ^refs/tags/v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
      CHANNEL="stable"
    elif [[ "${{ github.ref }}" =~ ^refs/tags/v[0-9]+\.[0-9]+\.[0-9]+-(rc|beta) ]]; then
      CHANNEL="release"
    else
      CHANNEL="dev"
    fi
    echo "channel=$CHANNEL" >> $GITHUB_OUTPUT

- name: Build
  run: |
    docker build \
      --build-arg VERSION=${{ steps.meta.outputs.version }} \
      --build-arg CHANNEL=${{ steps.meta.outputs.channel }} \
      ...
```

### Local Builds

The deploy script automatically detects the channel:

```bash
./scripts/deploy.sh
```

The script will:
1. Extract version from `code/frontend/package.json`
2. Check for git tags on current commit
3. Determine channel based on tag pattern
4. Pass version and channel to Docker builds

## Docker Build Arguments

Both Dockerfiles accept version metadata as build arguments:

**Backend (docker/Dockerfile.backend):**
```dockerfile
ARG COMMIT_HASH=unknown
ARG BUILD_TIME=unknown
ARG CHANNEL=dev

ENV COMMIT_HASH=${COMMIT_HASH}
ENV BUILD_TIME=${BUILD_TIME}
ENV CHANNEL=${CHANNEL}
```

**Frontend (docker/Dockerfile.frontend):**
```dockerfile
ARG VERSION=0.0.0
ARG CHANNEL=dev
ARG COMMIT_HASH=unknown
ARG BUILD_TIME=unknown

ENV VITE_VERSION=${VERSION}
ENV VITE_CHANNEL=${CHANNEL}
ENV VITE_COMMIT_HASH=${COMMIT_HASH}
ENV VITE_BUILD_TIME=${BUILD_TIME}
```

## Troubleshooting

### Version not updating in UI

1. Verify build args are passed to Docker:
   ```bash
   docker build --build-arg VERSION=0.1.0 --build-arg CHANNEL=dev ...
   ```

2. Check environment variables in container:
   ```bash
   kubectl exec -n kubarr deployment/kubarr-backend -- env | grep -E 'VERSION|CHANNEL|COMMIT'
   ```

3. Verify API endpoint returns correct version:
   ```bash
   curl http://localhost:8080/api/system/version
   ```

### Channel not detected correctly

1. Check current git tag:
   ```bash
   git describe --exact-match --tags
   ```

2. Verify tag pattern matches expected format:
   - Stable: `v1.2.3` (exactly)
   - Release: `v1.2.3-rc.1` or `v1.2.3-beta.1`

3. Re-run deploy script to pick up tag:
   ```bash
   ./scripts/deploy.sh
   ```

### Version mismatch between frontend and backend

Both should use the same version from `package.json`. If they differ:

1. Run version-bump script to sync:
   ```bash
   ./scripts/version-bump.sh patch
   ```

2. Or manually update both files:
   - `code/frontend/package.json`
   - `code/backend/Cargo.toml`

## Best Practices

1. **Always tag releases**: Use `./scripts/version-bump.sh` with `--tag` to ensure consistent versioning

2. **Use release candidates**: Test with `--channel release` before creating stable tags

3. **Keep versions synchronized**: Frontend and backend versions should always match

4. **Document breaking changes**: When bumping major version, document what changed in the commit message or changelog

5. **Automate deployments**: Use GitHub Actions to deploy tagged releases automatically

6. **Monitor channels**: Set up alerts for unexpected channels in production (e.g., `dev` channel in prod should trigger an alert)

7. **Clean up old tags**: Remove RC/beta tags after promoting to stable:
   ```bash
   git tag -d v1.2.3-rc.1
   git push origin :refs/tags/v1.2.3-rc.1
   ```
