#!/usr/bin/env bash
# Acceptance check for improver/backlog/macos-e2e-lane.md (first slice).
# A mac instrument suite must exist, run against a private daemon state,
# and gates.sh must surface its pass/fail counts instead of only a smoke line.
set -uo pipefail
[[ "$(uname -s)" == "Darwin" ]] || { echo "skip: Darwin-only check"; exit 0; }
root="$(cd "$(dirname "$0")/../../.." && pwd)"

fail=0
if [[ ! -x "$root/scripts/test-mac-instrument.sh" ]]; then
    echo "ACCEPT-FAIL: scripts/test-mac-instrument.sh missing or not executable"
    fail=1
else
    if ! "$root/scripts/test-mac-instrument.sh" >/tmp/hwatu-mac-instrument.log 2>&1; then
        echo "ACCEPT-FAIL: mac instrument suite failed:"
        tail -5 /tmp/hwatu-mac-instrument.log
        fail=1
    else
        tail -1 /tmp/hwatu-mac-instrument.log
    fi
fi
if ! grep -q 'test-mac-instrument' "$root/scripts/improver/gates.sh"; then
    echo "ACCEPT-FAIL: gates.sh does not run the mac instrument suite"
    fail=1
fi
[[ $fail -eq 0 ]] && echo "ACCEPT-PASS"
exit $fail
