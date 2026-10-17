#!/usr/bin/env bash
# Recursive-improver measurement harness (Sense + Verify gates).
# See docs/recursive-improver.md. Emits one JSON scoreboard per run to
# improver/state/scoreboard-<ts>.json and exits non-zero if a hard gate fails.
#
# Capability honesty: gates that cannot run on this host (e.g. the WebKitGTK
# daemon on macOS without Docker) are reported as "skipped", never silently
# passed. Usage: scripts/improver/gates.sh [--fast]
set -uo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
fast=0
[[ "${1:-}" == "--fast" ]] && fast=1

ts="$(date -u +%Y%m%dT%H%M%SZ)"
state="$root/improver/state"
mkdir -p "$state"
out="$state/scoreboard-$ts.json"

declare -a names statuses details
gate() { # name status detail
    names+=("$1"); statuses+=("$2"); details+=("$3")
    printf '%-28s %-8s %s\n' "$1" "$2" "$3" >&2
}

host="$(uname -s)"
run() { # name cmd...
    local name="$1"; shift
    local log; log="$(mktemp)"
    if "$@" >"$log" 2>&1; then
        gate "$name" pass "$(tail -1 "$log" | head -c 160)"
    else
        gate "$name" FAIL "$(tail -3 "$log" | tr '\n' ' ' | head -c 240)"
    fi
    rm -f "$log"
}

# ---- Gate 1: build + tests + clippy (portable crates always; daemon when linux)
if [[ "$host" == "Linux" ]]; then
    run "cargo.test" cargo test --workspace --quiet
    run "cargo.clippy" cargo clippy --workspace --quiet -- -D warnings
else
    run "cargo.test.portable" cargo test -p hwatu-ipc -p hwatu --quiet
    run "cargo.clippy.portable" cargo clippy -p hwatu-ipc -p hwatu --quiet -- -D warnings
    if docker info >/dev/null 2>&1; then
        run "cargo.test.linux(docker)" docker build -q -f Dockerfile .
    else
        gate "cargo.test.linux" skipped "no Linux host or docker; GTK daemon unverified on this host"
    fi
fi

# ---- Gate 2: golden IPC fixtures / protocol compatibility
if cargo test -p hwatu-ipc --quiet >/dev/null 2>&1; then
    gate "ipc.compat" pass "hwatu-ipc unit tests green"
else
    gate "ipc.compat" FAIL "hwatu-ipc tests failed"
fi

# ---- Gate 5: constitution lint (portable crates free of native UI deps)
viol=""
for crate in crates/ipc crates/hwatu; do
    for dep in gtk4 webkit6 gdk4 cocoa objc2 windows-sys; do
        grep -q "^$dep" "$crate/Cargo.toml" 2>/dev/null && viol="$viol $crate:$dep"
    done
done
if [[ -z "$viol" ]]; then
    gate "constitution.portable" pass "no native UI deps in portable crates"
else
    gate "constitution.portable" FAIL "$viol"
fi

# ---- Gate 7: structure triage (mechanical pass)
if scripts/improver/triage.sh > "$state/triage-$ts.txt" 2>&1; then
    gate "triage.structure" pass "$(tail -1 "$state/triage-$ts.txt")"
else
    gate "triage.structure" FAIL "$(tail -1 "$state/triage-$ts.txt"); see triage-$ts.txt"
fi

# ---- Scoreboard: E2E + benchmarks (Linux daemon or mac daemon where supported)
e2e_pass=0; e2e_fail=0; e2e_skip=0
if [[ $fast -eq 0 && "$host" == "Linux" ]]; then
    for t in scripts/test-*.sh; do
        if bash "$t" >/dev/null 2>&1; then e2e_pass=$((e2e_pass+1)); else e2e_fail=$((e2e_fail+1)); fi
    done
    gate "e2e.suite" "$([[ $e2e_fail -eq 0 ]] && echo pass || echo FAIL)" "pass=$e2e_pass fail=$e2e_fail"
else
    e2e_skip=1
    gate "e2e.suite" skipped "full E2E requires Linux daemon (host=$host, fast=$fast)"
fi

# mac verification-instrument suite (Darwin): isolated daemon E2E with counts
if [[ "$host" == "Darwin" ]]; then
    run "e2e.mac" bash "$root/scripts/test-mac-instrument.sh"
else
    gate "e2e.mac" skipped "Darwin-only instrument suite (host=$host)"
fi

# ---- Acceptance regression suite: every closed iteration's acceptance
# check keeps running forever (meta-loop review 1). Checks self-skip on
# unsupported hosts, which counts as pass here; real failures gate.
for a in "$root"/scripts/improver/accept/*.sh; do
    [[ -x "$a" ]] || continue
    run "accept.$(basename "$a" .sh)" bash "$a"
done

# ---- Emit JSON scoreboard
{
    echo "{"
    echo "  \"timestamp\": \"$ts\","
    echo "  \"host\": \"$host\","
    echo "  \"git\": \"$(git rev-parse --short HEAD)\","
    echo "  \"e2e\": {\"pass\": $e2e_pass, \"fail\": $e2e_fail, \"skipped\": $e2e_skip},"
    echo "  \"gates\": ["
    for i in "${!names[@]}"; do
        comma=","; [[ $i -eq $((${#names[@]}-1)) ]] && comma=""
        d=$(printf '%s' "${details[$i]}" | python3 -c 'import json,sys;print(json.dumps(sys.stdin.read()))')
        echo "    {\"name\": \"${names[$i]}\", \"status\": \"${statuses[$i]}\", \"detail\": $d}$comma"
    done
    echo "  ]"
    echo "}"
} > "$out"
echo "scoreboard: $out" >&2

# Exit non-zero on any hard FAIL
for s in "${statuses[@]}"; do [[ "$s" == "FAIL" ]] && exit 1; done
exit 0
