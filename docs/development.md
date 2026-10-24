# Development Guide

This guide covers the development workflow for contributing to Kubarr.

## Prerequisites

- **API and worker**: Rust 1.94+, Cargo
- **Frontend**: Node.js 26+, pnpm 10.34.3+ (the package pins 10.34.3)
- **Tools**: Docker, kubectl, Helm 4.3.0, Kind (for local testing)
- **Git**: For version control

## Initial Setup

### 1. Clone the Repository

```bash
git clone https://github.com/smokeythebandit/Kubarr.git
cd Kubarr
```

### 2. Set Up Development Environment

```bash
# Backend setup
cd code/api
cargo build --locked
cargo test --locked --lib

# Frontend setup
cd ../frontend
pnpm install --frozen-lockfile
pnpm dev
```

The API defaults to `0.0.0.0:8000` and a local PostgreSQL database at
`postgres://kubarr:kubarr@localhost:5432/kubarr`. Set `KUBARR_DATABASE_URL` for
your development database; migrations run on startup. From `code/api`, start
the API with `cargo run --locked --bin kubarr-api`.

Vite serves the frontend on port 5173 and proxies `/api` and `/auth` to
`http://127.0.0.1:8000`. To use a different API endpoint, start Vite with
`KUBARR_DEV_API_URL=http://127.0.0.1:8080 pnpm dev`.

App lifecycle operations require the separate worker. From `code/worker`, run
`cargo run --locked` with the same `KUBARR_DATABASE_URL` as the API, a kubeconfig
for your development cluster, and Helm on PATH. Full app proxying requires the
gateway deployment; Vite's proxy covers API and authentication requests.

For a complete installed environment, use the [CLI bootstrap wizard](https://github.com/smokeythebandit/kubarr-cli/blob/main/docs/bootstrap.md).

## Development Workflow

### Feature Development

1. **Create a feature branch:**
   ```bash
   git checkout main
   git pull
   git checkout -b feature/your-feature-name
   ```

2. **Make changes and commit:**
   ```bash
   # Make your changes...
   git add .
   git commit -m "feat: add your feature"

    # CI will run formatting, linting, type checks, and tests.
   ```

3. **Push and create PR:**
   ```bash
   git push -u origin feature/your-feature-name
   gh pr create --title "Add your feature" --body "Description"
   ```

4. **Wait for CI and merge:**
   ```bash
   # After CI passes:
   gh pr merge <PR#> --squash --delete-branch
   ```

## Code Style

### Backend (Rust)

- Follow standard Rust formatting (`cargo fmt`)
- Address all clippy warnings
- Write tests for new functionality
- Document public APIs with doc comments

Example:
```rust
/// Connects to the database with retry logic
///
/// # Arguments
/// * `url` - Database connection string
///
/// # Returns
/// Returns `Ok(Connection)` on success or `Err` after 10 failed attempts
pub async fn connect_with_url(url: &str) -> Result<Connection> {
    // Implementation...
}
```

### Frontend (TypeScript/React)

- Use TypeScript for type safety
- Follow React best practices
- Avoid `any` types (use proper types instead)
- Use functional components with hooks
- Keep components focused and small

Example:
```typescript
interface UserFormProps {
  user: User | null;
  onSubmit: (user: User) => Promise<void>;
  onCancel: () => void;
}

export function UserForm({ user, onSubmit, onCancel }: UserFormProps) {
  // Component implementation...
}
```

## Testing

### Backend Tests

```bash
cd code/api
KUBECONFIG=/dev/null cargo test --locked --lib
cargo test --locked --test app_operations_tests
```

### Frontend Tests

```bash
cd code/frontend
pnpm test                # Run unit tests
pnpm test:watch          # Watch mode
pnpm test:coverage       # With coverage
```

### E2E Tests (Playwright)

```bash
cd code/frontend
pnpm test:browser         # Browser tests
pnpm test:real            # Requires a configured live acceptance environment
```

See [Testing](testing.md) for environment requirements and live-cluster suites.

## Linting

### Run Manually

**Backend:**
```bash
cd code/api
cargo fmt --check   # Check formatting
cargo fmt           # Fix formatting
cargo clippy        # Run linter
```

**Frontend:**
```bash
cd code/frontend
pnpm lint           # Run ESLint
pnpm exec tsc --noEmit # Check TypeScript
```

### CI/CD

All checks run automatically in GitHub Actions:
- Lint checks (backend, frontend, Dockerfiles)
- Unit tests (backend, frontend)
- Build tests (Docker images)
- Documentation build

## Versioning

See [versioning.md](versioning.md) for details on version management and releases.

Update the API, worker, and frontend package versions and their lockfiles before
tagging a release. Run live release checks using the CI/CD workflow's
`release-checks` input; a tag alone does not update package metadata.

## Troubleshooting

### Clippy Warnings

Fix all warnings before committing:
```bash
cd code/api
cargo clippy --fix --allow-dirty
```

### ESLint Errors

```bash
cd code/frontend
pnpm exec eslint . --fix  # Auto-fix what's possible
```

### TypeScript Errors

```bash
cd code/frontend
pnpm exec tsc --noEmit  # Show all errors
```

## Branch Protection

Branch and tag protection are configured in GitHub repository settings. Check
the active rules there before merging or releasing; workflow files alone do not
enforce branch protection.

## Getting Help

- **Issues**: https://github.com/smokeythebandit/Kubarr/issues
- **Discussions**: https://github.com/smokeythebandit/Kubarr/discussions
- **Documentation**: [Full documentation](index.md)

## Contributing

Contributions are welcome! Please:
- Fork the repository
- Create a feature branch
- Follow the code style guidelines above
- Ensure all tests pass
- Submit a pull request

---

**Happy coding! 🚀**
