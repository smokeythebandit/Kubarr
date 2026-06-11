#!/bin/bash
# Test script for app proxy accessibility
# Usage: ./test-app-proxy.sh <username> <password>

set -e

BASE_URL="${BASE_URL:-http://localhost:8080}"
USERNAME="${1:-admin}"
PASSWORD="${2:-admin}"

echo "Testing app proxy accessibility..."
echo "Base URL: $BASE_URL"
echo "Username: $USERNAME"
echo ""

# Login and get session cookie
echo "Logging in..."
LOGIN_RESPONSE=$(curl -s -c /tmp/kubarr-cookies.txt -X POST "$BASE_URL/auth/login" \
  -H "Content-Type: application/json" \
  -d "{\"username\":\"$USERNAME\",\"password\":\"$PASSWORD\"}" \
  -w "\n%{http_code}")

HTTP_CODE=$(echo "$LOGIN_RESPONSE" | tail -1)
BODY=$(echo "$LOGIN_RESPONSE" | head -n -1)

if [ "$HTTP_CODE" != "200" ]; then
  echo "Login failed with status $HTTP_CODE"
  echo "Response: $BODY"
  exit 1
fi

echo "Login successful!"
echo ""

# Get list of installed apps
echo "Getting installed apps..."
INSTALLED=$(curl -s -b /tmp/kubarr-cookies.txt "$BASE_URL/api/apps/installed")
echo "Installed apps: $INSTALLED"
echo ""

# Get catalog to know which apps are browseable
CATALOG=$(curl -s -b /tmp/kubarr-cookies.txt "$BASE_URL/api/apps/catalog")

# Test each browseable installed app
APPS="sonarr radarr jellyfin plex jellyseerr deluge transmission rutorrent qbittorrent jackett"
FAILURES=""
SUCCESSES=""

echo "Testing app proxies..."
echo ""

for app in $APPS; do
  # Check if app is installed
  if echo "$INSTALLED" | grep -q "\"$app\""; then
    # Test the proxy
    RESPONSE=$(curl -s -b /tmp/kubarr-cookies.txt -o /tmp/app-response.html -w "%{http_code}" "$BASE_URL/$app/")

    # Check HTTP status
    if [ "$RESPONSE" -ge 200 ] && [ "$RESPONSE" -lt 400 ]; then
      # Check if response contains app-specific content (not frontend HTML)
      if grep -qi "favicon.svg" /tmp/app-response.html && grep -qi "Kubarr Dashboard" /tmp/app-response.html; then
        echo "✗ $app: HTTP $RESPONSE - Returns frontend HTML instead of app content"
        FAILURES="$FAILURES $app"
      else
        echo "✓ $app: HTTP $RESPONSE - OK"
        SUCCESSES="$SUCCESSES $app"
      fi
    else
      echo "✗ $app: HTTP $RESPONSE - Error"
      FAILURES="$FAILURES $app"
    fi
  else
    echo "- $app: Not installed, skipping"
  fi
done

echo ""
echo "Results:"
echo "  Successes:$SUCCESSES"
echo "  Failures:$FAILURES"

# Cleanup
rm -f /tmp/kubarr-cookies.txt /tmp/app-response.html

if [ -n "$FAILURES" ]; then
  exit 1
fi

echo ""
echo "All tests passed!"
