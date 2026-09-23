#!/bin/sh
set -eu

IFS=' ' read -r method target _version
while IFS= read -r header && [ "$header" != "$(printf '\r')" ]; do :; done
[ "$method" = GET ] || exit 1

if [ "$target" = /events ]; then
  if [ -f /tmp/kubarr-vpn-events/events ]; then
    body=$(awk 'BEGIN { printf "{\"events\":[" } { if (NR > 1) printf ","; printf "%s", $0 } END { print "]}" }' /tmp/kubarr-vpn-events/events)
  else
    body='{"events":[]}'
  fi
else
  token=${target#*token=}
  [ "$token" != "$target" ] || token=""
  token=${token%%&*}
  peer=${SOCAT_PEERADDR:-unknown}
  event=$(printf '{"token":"%s","peer":"%s"}' "$token" "$peer")
  printf '%s\n' "$event" >>/tmp/kubarr-vpn-events/events
  body=$(printf '{"run":"%s","endpoint":"%s","token":"%s","peer":"%s"}' \
    "$RUN_TOKEN" "$ENDPOINT_NAME" "$token" "$peer")
fi

length=$(printf '%s' "$body" | wc -c)
printf 'HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: %s\r\nConnection: close\r\n\r\n%s' "$length" "$body"
