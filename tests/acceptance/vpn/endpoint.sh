#!/bin/sh
set -eu

mkdir -p /tmp/kubarr-vpn-events
exec socat TCP-LISTEN:8080,reuseaddr,fork EXEC:/usr/local/bin/endpoint-request.sh
