#!/usr/bin/env bash
# Acceptance check for improver/backlog/mac-eval-semantics-parity.md.
# docs/agents.md "Eval semantics": js may be an expression OR a function
# body; the daemon picks the form. The mac daemon must match Linux:
#   expression        -> its value        ('document.title', '1+1')
#   explicit return   -> returned value   ('return 6*7')
#   statements+return -> returned value   ('const n = 6*7; return n')
#   bad syntax        -> structured error mentioning SyntaxError, not a hang
# Runs against an isolated daemon in private dirs (same pattern as
# scripts/test-mac-instrument.sh).
set -uo pipefail
[[ "$(uname -s)" == "Darwin" ]] || { echo "skip: Darwin-only check"; exit 0; }

root="$(cd "$(dirname "$0")/../../.." && pwd)"
# shellcheck source=../../../lib/test-harness.sh
source "$root/scripts/lib/test-harness.sh"
bin="${HWATU_BIN:-hwatu}"

hw_private_env "hwatu-eval-parity"
hw_set_quit_cmd "$bin"
hw_cleanup_install

"$bin" --headless 'data:text/html,<title>parity</title>' >/dev/null 2>&1 || {
    echo "ACCEPT-FAIL: could not open test window"; exit 1; }

# The data: URL load races first eval; wait for the title to land.
for _ in $(seq 1 50); do
    [[ "$("$bin" eval 'document.title' 2>&1)" == *parity* ]] && break
    sleep 0.2
done

fail=0
expect() { # desc js want
    local desc="$1" js="$2" want="$3" got
    got="$("$bin" eval "$js" 2>&1)"
    if [[ "$got" == *"$want"* ]]; then
        echo "ok    $desc -> $got"
    else
        echo "ACCEPT-FAIL: $desc: want *$want*, got: $got"
        fail=1
    fi
}

expect "expression document.title" 'document.title' 'parity'
expect "expression 1+1" '1+1' '2'
expect "function body return" 'return 6*7' '42'
expect "statements with return" 'const n = 6*7; return n' '42'
got="$("$bin" eval 'this is not js' 2>&1)"; rc=$?
if [[ $rc -ne 0 && "$got" == *SyntaxError* ]]; then
    echo "ok    bad syntax -> structured SyntaxError"
else
    echo "ACCEPT-FAIL: bad syntax: want nonzero exit + SyntaxError, got rc=$rc: $got"
    fail=1
fi

[[ $fail -eq 0 ]] && echo "ACCEPT-PASS"
exit $fail
