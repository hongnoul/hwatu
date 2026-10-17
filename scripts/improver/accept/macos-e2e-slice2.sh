#!/usr/bin/env bash
# Acceptance check for improver/backlog/closed-macos-e2e-lane-slice1.md
# follow-up (slice 2): the mac instrument suite must also cover the
# remaining supported verbs of hwatud-mac (platform/macos/README.md):
# navigate, snapshot, fill_form, type, press, scroll, console, list.
# Coverage is detected by marker comments `# covers: <verb>` in the suite,
# and the suite must pass with the enlarged assertion count.
set -uo pipefail
[[ "$(uname -s)" == "Darwin" ]] || { echo "skip: Darwin-only check"; exit 0; }
root="$(cd "$(dirname "$0")/../../.." && pwd)"
suite="$root/scripts/test-mac-instrument.sh"

fail=0
for verb in navigate snapshot fill_form type press scroll console list; do
    if ! grep -q "# covers: .*\b$verb\b" "$suite"; then
        echo "ACCEPT-FAIL: suite does not cover verb: $verb"
        fail=1
    fi
done

if [[ $fail -eq 0 ]]; then
    if ! "$suite" >/tmp/hwatu-slice2-accept.log 2>&1; then
        echo "ACCEPT-FAIL: suite fails:"
        tail -5 /tmp/hwatu-slice2-accept.log
        fail=1
    else
        summary="$(tail -1 /tmp/hwatu-slice2-accept.log)"
        echo "$summary"
        n="$(printf '%s' "$summary" | sed -n 's/.*: \([0-9][0-9]*\) passed.*/\1/p')"
        if [[ -z "$n" || "$n" -lt 20 ]]; then
            echo "ACCEPT-FAIL: expected >=20 assertions after slice 2, got: ${n:-unparsed}"
            fail=1
        fi
    fi
fi

[[ $fail -eq 0 ]] && echo "ACCEPT-PASS"
exit $fail
