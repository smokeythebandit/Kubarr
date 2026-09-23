#!/usr/bin/env bash
# shellcheck disable=SC1091,SC2034
set -Eeuo pipefail

DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=app-lifecycle.sh
source "$DIR/app-lifecycle.sh"

WORK_DIR=$(mktemp -d)
COOKIE_JAR=/dev/null
GATEWAY_PORT=18080
trap 'rm -rf "$WORK_DIR"' EXIT

MOCK_STATUS=200
MOCK_BODY=''
MOCK_CURL_STATUS=0

curl() {
  local output=""
  while (( $# > 0 )); do
    case "$1" in
      --output)
        output=$2
        shift 2
        ;;
      --write-out|-X|-b|-c|-H|--max-time|--data-binary)
        shift 2
        ;;
      *)
        shift
        ;;
    esac
  done
  [[ -n "$output" ]] || return 64
  printf '%s' "$MOCK_BODY" >"$output"
  printf '%s' "$MOCK_STATUS"
  return "$MOCK_CURL_STATUS"
}

MOCK_STATUS=202
MOCK_BODY='{"accepted":true}'
result=$(sonarr_api PUT /sonarr/api/v3/config/host -H 'X-Api-Key: generated-secret')
[[ "$result" == '{"accepted":true}' ]]

MOCK_STATUS=400
MOCK_BODY='[{"propertyName":"InstanceName","errorMessage":"Must start or end with Sonarr","severity":"error","errorCode":"RegularExpressionValidator","attemptedValue":"generated-secret"}]'
if sonarr_api PUT /sonarr/api/v3/config/host -H 'X-Api-Key: generated-secret' \
  >"$WORK_DIR/rejected.out" 2>"$WORK_DIR/rejected.err"; then
  printf 'expected Sonarr API rejection\n' >&2
  exit 1
fi
diagnostic=$(<"$WORK_DIR/rejected.err")
[[ "$diagnostic" == *'Sonarr API PUT /sonarr/api/v3/config/host returned HTTP 400'* ]]
[[ "$diagnostic" == *'"propertyName":"InstanceName"'* ]]
[[ "$diagnostic" == *'"errorCode":"RegularExpressionValidator"'* ]]
[[ "$diagnostic" != *generated-secret* ]]
[[ ! -s "$WORK_DIR/rejected.out" ]]

MOCK_STATUS=503
MOCK_BODY='{"apiKey":"generated-secret","message":"unstructured failure"}'
if sonarr_api GET /sonarr/api/v3/config/host -H 'X-Api-Key: generated-secret' \
  >/dev/null 2>"$WORK_DIR/unstructured.err"; then
  printf 'expected Sonarr API rejection\n' >&2
  exit 1
fi
diagnostic=$(<"$WORK_DIR/unstructured.err")
[[ "$diagnostic" == *'Sonarr API GET /sonarr/api/v3/config/host returned HTTP 503'* ]]
[[ "$diagnostic" != *generated-secret* ]]
[[ "$diagnostic" != *'unstructured failure'* ]]

if compgen -G "$WORK_DIR/sonarr-api-response.*" >/dev/null; then
  printf 'Sonarr API response temporary file was not removed\n' >&2
  exit 1
fi

printf 'Sonarr API helper checks passed\n'
