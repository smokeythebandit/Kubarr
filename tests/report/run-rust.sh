#!/usr/bin/env bash
# Capture the cargo exit code independently of tee, including pre-test failures.
set -uo pipefail
log=$1
shift
"$@" 2>&1 | tee "$log"
outcomes=("${PIPESTATUS[@]}")
status=${outcomes[0]}
if (( status == 0 && outcomes[1] != 0 )); then status=${outcomes[1]}; fi
printf '%s\n' "$status" > "$log.exit"
exit "$status"
