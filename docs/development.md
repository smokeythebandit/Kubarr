# Development Guide

This guide covers the development workflow for contributing to Kubarr.

## Prerequisites

- **Backend**: Rust 1.83+, Cargo
- **Frontend**: Node.js 20+, npm
- **Tools**: Docker, kubectl, Kind (for local testing)
- **Git**: For version control

## Initial Setup

### 1. Clone the Repository

```bash
git clone https://github.com/smokeythebandit/Kubarr.git
cd Kubarr
```

### 2. Install Git Hooks

**IMPORTANT**: Run this after cloning to set up pre-commit checks:

```bash
./scripts/install-hooks.sh
```

This installs a pre-commit hook that automatically runs:
- Rust formatting checks (`cargo fmt`)
- Clippy lint checks
- TypeScript compilation
- ESLint on changed files

### 3. Set Up Development Environment

```bash
# Backend setup
cd code/backend
cargo build
cargo test

# Frontend setup
cd ../frontend
npm install
npm run dev
```

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

   # Pre-commit hooks will run automatically:
   # ✅ Checks Rust formatting
   # ✅ Runs clippy
   # ✅ Checks TypeScript
   # ✅ Runs ESLint
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

## Pre-Commit Hooks

### What Gets Checked

The pre-commit hook runs different checks based on which files changed:

**Backend files** (`code/backend/**`):
- `cargo fmt --check` - Ensures code is formatted
- `cargo clippy -- -D warnings` - Catches common mistakes and style issues

**Frontend files** (`code/frontend/**`):
- `tsc --noEmit` - Checks TypeScript compilation
- `eslint` - Lints changed files (with `--max-warnings 0`)

### If Checks Fail

The commit will be blocked with helpful error messages:

```bash
❌ Pre-commit checks failed!

📦 Backend files changed, running checks...
  ❌ Rust formatting check failed!
  💡 Run: cd code/backend && cargo fmt

To skip these checks (not recommended), use:
  git commit --no-verify
```

**Fix the issues** shown in the output, then try committing again.

### Skipping Hooks (Not Recommended)

Only skip hooks if absolutely necessary (e.g., WIP commits):

```bash
git commit --no-verify -m "WIP: work in progress"
```

**Note**: CI will still run these checks, so you'll need to fix them eventually.

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
cd code/backend
cargo test
cargo test --test integration_tests
```

### Frontend Tests

```bash
cd code/frontend
npm test                # Run unit tests
npm run test:watch      # Watch mode
npm run test:coverage   # With coverage
```

### E2E Tests (Playwright)

```bash
cd code/frontend
npx playwright test
npx playwright test --ui  # Interactive mode
```

## Linting

### Run Manually

**Backend:**
```bash
cd code/backend
cargo fmt --check   # Check formatting
cargo fmt           # Fix formatting
cargo clippy        # Run linter
```

**Frontend:**
```bash
cd code/frontend
npm run lint        # Run ESLint
npx tsc --noEmit    # Check TypeScript
```

### CI/CD

All checks run automatically in GitHub Actions:
- Lint checks (backend, frontend, Dockerfiles)
- Unit tests (backend, frontend)
- Build tests (Docker images)
- Documentation build

## Versioning

See [versioning.md](versioning.md) for details on version management and releases.

Quick version bump:
```bash
./scripts/version-bump.sh patch --tag
git push && git push origin v0.1.1
```

## Troubleshooting

### Pre-commit Hook Not Running

```bash
# Reinstall hooks
./scripts/install-hooks.sh

# Verify hook is executable
ls -la .git/hooks/pre-commit
```

### Clippy Warnings

Fix all warnings before committing:
```bash
cd code/backend
cargo clippy --fix --allow-dirty
```

### ESLint Errors

```bash
cd code/frontend
npm run lint -- --fix  # Auto-fix what's possible
```

### TypeScript Errors

```bash
cd code/frontend
npx tsc --noEmit  # Show all errors
```

## Branch Protection

The `main` branch is protected:
- ✅ Requires pull requests
- ✅ Requires CI checks to pass
- ✅ No force pushes allowed
- ✅ Version tags (`v*`) are protected

See [versioning.md#branch-protection-rules](versioning.md#branch-protection-rules) for details.

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
