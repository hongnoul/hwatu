#!/usr/bin/env bash
# Shared preamble for hwatu E2E / acceptance scripts: private-XDG isolated
# daemon bootstrap, cleanup trap, and pass/fail counter helpers.
#
# Extracted from scripts/test-agent-loop.sh (Linux reference, lines 1-51) and
# scripts/test-mac-instrument.sh (Darwin flavor), which both inlined the same
# mktemp workdir + XDG env exports + cleanup-trap + ok/bad/check block.
# bash 3.2 compatible (no associative arrays, no `local -n`, no `[[ -v ]]`).
#
# Usage (see scripts/test-mac-instrument.sh for a full example):
#   source "$root/scripts/lib/test-harness.sh"   # path relative to script dir
#   hw_private_env "my-test-tag"                 # mktemp workdir + XDG exports
#   hw_set_quit_cmd "$hwatu_or_bin"              # CLI used to quit the daemon
#   hw_cleanup_install                            # trap hw_cleanup EXIT INT TERM
#   hw_counters_init                              # pass=0 fail=0 (optional)
#   ... assertions via ok / bad / check / json_assert ...
#   hw_summary "my-suite"                        # "my-suite: N passed, M failed"
#
# Migration pattern for the remaining Linux scripts (test-agent-loop.sh,
# test-render.sh, test-watch.sh, test-handoff.sh, ...): delete the inline
# mktemp/XDG/cleanup/counter block and replace it with the five lines above.
# Shape notes from studying those scripts:
#   - agent-loop tracks daemon_pid/server_pid/watch_pid: replace each
#     `<name>_pid=$!` site with `hw_track_pid $!` and drop the custom
#     cleanup; the lib kills tracked pids, sends quit, waits, removes workdir.
#   - render tracks only server_pid and traps EXIT only: hw_track_pid plus
#     `hw_cleanup_install EXIT` covers it.
#   - watch spawns hwatud itself and reaps subscribers via
#     `jobs -p | xargs -r kill -9`: keep that line in a per-script
#     `hw_cleanup_hook()` defined before hw_cleanup_install; the lib calls
#     the hook inside hw_cleanup.
#   - handoff uses a different check() signature (name + exit code): convert
#     its call sites to the standard `check <desc> <command...>` form when
#     migrating, or keep its local helper; do not add a second check to lib.
#   - handoff also exports XDG_DATA_HOME: set that one extra export in the
#     script after hw_private_env (the lib covers run/state/config).
# Counter idiom: ok/bad/check/json_assert match the agent-loop/mac-instrument
# form; bad() reports to stderr. hw_summary prints the exact
# "NAME: N passed, M failed" line gates.sh and triage parse via tail -1.
#
# The lib owns workdir/XDG/cleanup/counters only. Suites keep their own
# fixtures, assertions, and coverage in the sourcing script.

# Double-source guard (safe under `set -u`).
if [[ -n "${HW_TEST_HARNESS_LOADED:-}" ]]; then
    return 0 2>/dev/null || exit 0
fi
HW_TEST_HARNESS_LOADED=1

# Globals owned by the lib. `work` keeps the historical name scripts use.
hw_work=""
work=""
hw_quit_cmd=""
hw_pids=""
pass=0
fail=0

hw_private_env() { # hw_private_env [tag]
    local tag="${1:-hwatu-test}"
    work="$(mktemp -d "${TMPDIR:-/tmp}/${tag}.XXXXXX")"
    hw_work="$work"
    export XDG_RUNTIME_DIR="$work/run"
    export XDG_STATE_HOME="$work/state"
    export XDG_CONFIG_HOME="$work/config"
    mkdir -p "$XDG_RUNTIME_DIR" "$XDG_STATE_HOME" "$XDG_CONFIG_HOME"
}

hw_set_quit_cmd() { # hw_set_quit_cmd <cli-path>
    hw_quit_cmd="$1"
}

hw_track_pid() { # hw_track_pid <pid>
    if [[ -n "${1:-}" ]]; then
        hw_pids="$hw_pids $1"
    fi
}

hw_cleanup() {
    if [[ -n "$hw_quit_cmd" ]]; then
        "$hw_quit_cmd" quit >/dev/null 2>&1 || true
    fi
    local p
    # Intentional word splitting over the tracked pid list.
    # shellcheck disable=SC2086
    for p in $hw_pids; do
        [[ -n "$p" ]] && kill "$p" 2>/dev/null || true
    done
    if declare -F hw_cleanup_hook >/dev/null 2>&1; then
        hw_cleanup_hook || true
    fi
    wait 2>/dev/null || true
    if [[ -n "$work" ]]; then
        rm -rf "$work"
    fi
}

hw_cleanup_install() { # hw_cleanup_install [signals...] (default: EXIT INT TERM)
    if [[ $# -eq 0 ]]; then
        trap hw_cleanup EXIT INT TERM
    else
        # shellcheck disable=SC2064
        trap "hw_cleanup" "$@"
    fi
}

hw_counters_init() {
    pass=0
    fail=0
}

ok()  { pass=$((pass + 1)); echo "ok   - $1"; }
bad() { fail=$((fail + 1)); echo "FAIL - $1" >&2; }
check() { # check <description> <command...>
    local desc="$1"; shift
    if "$@"; then ok "$desc"; else bad "$desc"; fi
}
json_assert() { # json_assert '<python expression using d>' <<<response
    local expression="$1"
    python3 -c "import json,sys; d=json.load(sys.stdin); assert ($expression), d"
}

hw_summary() { # hw_summary <suite-name>
    local name="$1"
    echo
    echo "$name: $pass passed, $fail failed"
    [[ "$fail" -eq 0 ]]
}
