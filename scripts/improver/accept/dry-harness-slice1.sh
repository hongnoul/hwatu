#!/usr/bin/env bash
# Acceptance check for improver/backlog/dry-test-harness-preamble.md slice 1.
# A shared harness lib must exist and every Darwin-runnable script that
# previously inlined the private-XDG daemon-bootstrap preamble must source
# it instead. Linux-only scripts are a later slice (need a Linux host to
# re-verify behavior after migration).
set -uo pipefail
root="$(cd "$(dirname "$0")/../../.." && pwd)"
lib="$root/scripts/lib/test-harness.sh"

fail=0
if [[ ! -f "$lib" ]]; then
    echo "ACCEPT-FAIL: scripts/lib/test-harness.sh missing"
    fail=1
fi

# Darwin-runnable scripts that must consume the lib rather than inline it.
darwin_scripts=(
    scripts/test-mac-instrument.sh
    scripts/improver/accept/mac-eval-semantics-parity.sh
)
for s in "${darwin_scripts[@]}"; do
    if ! grep -q 'lib/test-harness.sh' "$root/$s"; then
        echo "ACCEPT-FAIL: $s does not source scripts/lib/test-harness.sh"
        fail=1
    fi
    if grep -q 'XDG_RUNTIME_DIR="\$work/run"' "$root/$s"; then
        echo "ACCEPT-FAIL: $s still inlines the private-XDG preamble"
        fail=1
    fi
done

# Behavior preserved on this host.
if [[ "$(uname -s)" == "Darwin" && $fail -eq 0 ]]; then
    if ! "$root/scripts/test-mac-instrument.sh" >/tmp/hwatu-dry-accept.log 2>&1; then
        echo "ACCEPT-FAIL: migrated test-mac-instrument.sh fails:"
        tail -5 /tmp/hwatu-dry-accept.log
        fail=1
    else
        tail -1 /tmp/hwatu-dry-accept.log
    fi
    if ! "$root/scripts/improver/accept/mac-eval-semantics-parity.sh" >/tmp/hwatu-dry-accept2.log 2>&1; then
        echo "ACCEPT-FAIL: migrated mac-eval-semantics-parity.sh fails:"
        tail -5 /tmp/hwatu-dry-accept2.log
        fail=1
    fi
fi

[[ $fail -eq 0 ]] && echo "ACCEPT-PASS"
exit $fail
