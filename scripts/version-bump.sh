#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

usage() {
    echo "Usage: $0 <major|minor|patch> [OPTIONS]"
    echo ""
    echo "Bump version across backend and frontend."
    echo ""
    echo "Arguments:"
    echo "  major       Bump major version (e.g., 1.2.3 -> 2.0.0)"
    echo "  minor       Bump minor version (e.g., 1.2.3 -> 1.3.0)"
    echo "  patch       Bump patch version (e.g., 1.2.3 -> 1.2.4)"
    echo ""
    echo "Options:"
    echo "  --channel <dev|release|stable>  Release channel (default: stable)"
    echo "  --tag                            Create git tag after bumping"
    echo "  --dry-run                        Show what would change without modifying files"
    echo "  --help                           Show this help message"
    echo ""
    echo "Examples:"
    echo "  $0 patch --tag                    # 0.1.0 -> 0.1.1, create v0.1.1 tag"
    echo "  $0 minor --channel release --tag  # 0.1.0 -> 0.2.0-rc.1, create tag"
    echo "  $0 major --dry-run                # Show what would happen for major bump"
}

# Parse arguments
BUMP_TYPE=""
CHANNEL="stable"
CREATE_TAG=false
DRY_RUN=false

if [ $# -eq 0 ]; then
    usage
    exit 1
fi

while [[ $# -gt 0 ]]; do
    case "$1" in
        major|minor|patch)
            if [ -n "$BUMP_TYPE" ]; then
                echo "Error: Bump type already specified"
                exit 1
            fi
            BUMP_TYPE="$1"
            shift
            ;;
        --channel)
            CHANNEL="$2"
            if [[ ! "$CHANNEL" =~ ^(dev|release|stable)$ ]]; then
                echo "Error: Channel must be dev, release, or stable"
                exit 1
            fi
            shift 2
            ;;
        --tag)
            CREATE_TAG=true
            shift
            ;;
        --dry-run)
            DRY_RUN=true
            shift
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            echo "Error: Unknown argument '$1'"
            usage
            exit 1
            ;;
    esac
done

if [ -z "$BUMP_TYPE" ]; then
    echo "Error: Bump type (major, minor, or patch) is required"
    usage
    exit 1
fi

cd "$PROJECT_ROOT"

# Get current version from package.json
CURRENT_VERSION=$(grep '"version"' code/frontend/package.json | head -1 | sed 's/.*"version": "\(.*\)".*/\1/')
if [ -z "$CURRENT_VERSION" ]; then
    echo "Error: Could not read current version from package.json"
    exit 1
fi

echo "Current version: $CURRENT_VERSION"

# Parse version components
IFS='.' read -r MAJOR MINOR PATCH <<< "$CURRENT_VERSION"

# Strip any pre-release suffix from patch (e.g., "3-rc.1" -> "3")
PATCH=$(echo "$PATCH" | sed 's/-.*//')

# Calculate new version
case "$BUMP_TYPE" in
    major)
        NEW_MAJOR=$((MAJOR + 1))
        NEW_MINOR=0
        NEW_PATCH=0
        ;;
    minor)
        NEW_MAJOR=$MAJOR
        NEW_MINOR=$((MINOR + 1))
        NEW_PATCH=0
        ;;
    patch)
        NEW_MAJOR=$MAJOR
        NEW_MINOR=$MINOR
        NEW_PATCH=$((PATCH + 1))
        ;;
esac

NEW_VERSION="$NEW_MAJOR.$NEW_MINOR.$NEW_PATCH"

# Add channel suffix for release channel
TAG_VERSION="$NEW_VERSION"
if [ "$CHANNEL" = "release" ]; then
    TAG_VERSION="$NEW_VERSION-rc.1"
fi

echo "New version: $NEW_VERSION"
echo "Channel: $CHANNEL"
if [ "$CREATE_TAG" = true ]; then
    echo "Git tag: v$TAG_VERSION"
fi

if [ "$DRY_RUN" = true ]; then
    echo ""
    echo "=== DRY RUN - No files will be modified ==="
    echo ""
    echo "Would update:"
    echo "  - code/frontend/package.json: $CURRENT_VERSION -> $NEW_VERSION"
    echo "  - code/backend/Cargo.toml: $CURRENT_VERSION -> $NEW_VERSION"
    echo "  - code/backend/Cargo.lock (via cargo check)"
    if [ "$CREATE_TAG" = true ]; then
        echo "  - Create git tag: v$TAG_VERSION"
    fi
    exit 0
fi

# Update package.json
echo ""
echo "Updating code/frontend/package.json..."
sed -i "s/\"version\": \"$CURRENT_VERSION\"/\"version\": \"$NEW_VERSION\"/" code/frontend/package.json

# Update Cargo.toml
echo "Updating code/backend/Cargo.toml..."
sed -i "s/^version = \"$CURRENT_VERSION\"/version = \"$NEW_VERSION\"/" code/backend/Cargo.toml

# Update Cargo.lock
echo "Updating code/backend/Cargo.lock..."
cd code/backend
cargo check --quiet 2>/dev/null || cargo check
cd "$PROJECT_ROOT"

echo ""
echo "=== Version Bump Complete ==="
echo ""
echo "Updated files:"
echo "  - code/frontend/package.json"
echo "  - code/backend/Cargo.toml"
echo "  - code/backend/Cargo.lock"
echo ""

if [ "$CREATE_TAG" = true ]; then
    echo "Creating git tag v$TAG_VERSION..."
    git tag -a "v$TAG_VERSION" -m "Release $TAG_VERSION"
    echo ""
    echo "Git tag created: v$TAG_VERSION"
    echo ""
    echo "To push changes and tag:"
    echo "  git add code/frontend/package.json code/backend/Cargo.toml code/backend/Cargo.lock"
    echo "  git commit -m \"Bump version to $NEW_VERSION\""
    echo "  git push"
    echo "  git push origin v$TAG_VERSION"
else
    echo "To commit these changes:"
    echo "  git add code/frontend/package.json code/backend/Cargo.toml code/backend/Cargo.lock"
    echo "  git commit -m \"Bump version to $NEW_VERSION\""
    echo ""
    echo "To create a tag later:"
    echo "  git tag -a v$TAG_VERSION -m \"Release $TAG_VERSION\""
    echo "  git push origin v$TAG_VERSION"
fi
