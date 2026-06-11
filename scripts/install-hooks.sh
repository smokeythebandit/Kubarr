#!/bin/bash
# Install Git hooks for Kubarr development
# Run this after cloning the repository: ./scripts/install-hooks.sh

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
HOOKS_DIR="$PROJECT_ROOT/.git/hooks"

echo "📋 Installing Git hooks for Kubarr..."
echo ""

# Create hooks directory if it doesn't exist
mkdir -p "$HOOKS_DIR"

# Install pre-commit hook
cat > "$HOOKS_DIR/pre-commit" << 'EOF'
#!/bin/bash
# Pre-commit hook for Kubarr
# Runs linting checks before allowing commit

set -e

echo "🔍 Running pre-commit checks..."
echo ""

# Get list of staged files
STAGED_FILES=$(git diff --cached --name-only --diff-filter=ACM)

# Check if backend files changed
BACKEND_CHANGED=$(echo "$STAGED_FILES" | grep "^code/backend/" || true)
# Check if frontend files changed
FRONTEND_CHANGED=$(echo "$STAGED_FILES" | grep "^code/frontend/" || true)

EXIT_CODE=0

# Backend checks
if [ -n "$BACKEND_CHANGED" ]; then
    echo "📦 Backend files changed, running checks..."

    # Rust format check
    echo "  ⏳ Checking Rust formatting..."
    if ! (cd code/backend && cargo fmt --check 2>&1); then
        echo "  ❌ Rust formatting check failed!"
        echo "  💡 Run: cd code/backend && cargo fmt"
        EXIT_CODE=1
    else
        echo "  ✅ Rust formatting OK"
    fi

    # Clippy
    echo "  ⏳ Running clippy..."
    if ! (cd code/backend && cargo clippy --no-deps -- -D warnings 2>&1 | grep -E "^(error|warning)" || true); then
        CLIPPY_OUTPUT=$(cd code/backend && cargo clippy --no-deps -- -D warnings 2>&1)
        if echo "$CLIPPY_OUTPUT" | grep -q "error\|warning"; then
            echo "  ❌ Clippy found issues!"
            echo "$CLIPPY_OUTPUT" | grep -E "^(error|warning)" | head -10
            echo "  💡 Fix the issues above before committing"
            EXIT_CODE=1
        else
            echo "  ✅ Clippy passed"
        fi
    else
        echo "  ✅ Clippy passed"
    fi

    echo ""
fi

# Frontend checks
if [ -n "$FRONTEND_CHANGED" ]; then
    echo "🎨 Frontend files changed, running checks..."

    # TypeScript check
    echo "  ⏳ Checking TypeScript..."
    if ! (cd code/frontend && npx tsc --noEmit 2>&1 | head -20); then
        echo "  ❌ TypeScript check failed!"
        echo "  💡 Fix TypeScript errors above"
        EXIT_CODE=1
    else
        echo "  ✅ TypeScript OK"
    fi

    # ESLint (only on staged files)
    echo "  ⏳ Running ESLint..."
    FRONTEND_STAGED=$(echo "$STAGED_FILES" | grep "^code/frontend/.*\.\(ts\|tsx\)$" || true)
    if [ -n "$FRONTEND_STAGED" ]; then
        if ! (cd code/frontend && npx eslint $(echo "$FRONTEND_STAGED" | sed 's|^code/frontend/||') --max-warnings 0 2>&1); then
            echo "  ❌ ESLint found issues!"
            echo "  💡 Fix linting errors above"
            EXIT_CODE=1
        else
            echo "  ✅ ESLint passed"
        fi
    fi

    echo ""
fi

if [ $EXIT_CODE -eq 0 ]; then
    echo "✅ All pre-commit checks passed!"
else
    echo ""
    echo "❌ Pre-commit checks failed!"
    echo ""
    echo "To skip these checks (not recommended), use:"
    echo "  git commit --no-verify"
    echo ""
fi

exit $EXIT_CODE
EOF

# Make hook executable
chmod +x "$HOOKS_DIR/pre-commit"

echo "✅ Pre-commit hook installed successfully!"
echo ""
echo "The hook will automatically run before each commit to:"
echo "  📦 Check Rust formatting (cargo fmt)"
echo "  📦 Run Clippy lint checks"
echo "  🎨 Check TypeScript compilation"
echo "  🎨 Run ESLint on staged files"
echo ""
echo "To skip the hook (not recommended):"
echo "  git commit --no-verify"
echo ""
echo "📚 See docs/development.md for more information"
