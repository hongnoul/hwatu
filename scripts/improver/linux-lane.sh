#!/usr/bin/env bash
# Linux-lane runner for the recursive improver, executed INSIDE the
# hwatu-builder container (ubuntu 24.04, webkitgtk 6.0, repo bind-mounted
# at /src, cargo in /root/.cargo/bin). Usage:
#   colima ssh -- docker exec hwatu-builder bash /src/scripts/improver/linux-lane.sh [cmd]
# cmds: build | test | clippy | e2e | all (default)
set -uo pipefail
export PATH=/root/.cargo/bin:/usr/local/cargo/bin:$PATH
cd /src
cmd="${1:-all}"
# The colima VM has 8GiB; full-parallel rustc gets OOM-killed. Bound
# build parallelism (observed: -j4 killed, -j2 completes).
jobs="${CARGO_BUILD_JOBS:-2}"

status=0
section() { echo; echo "=== $1 ==="; }

if [[ "$cmd" == build || "$cmd" == all ]]; then
    section "cargo build --workspace"
    cargo build -j "$jobs" --workspace 2>&1 | tail -3 || status=1
fi
if [[ "$cmd" == test || "$cmd" == all ]]; then
    section "cargo test --workspace"
    cargo test -j "$jobs" --workspace 2>&1 | grep -E "^(test result|error|warning: unused)" | tail -20 || status=1
fi
if [[ "$cmd" == clippy || "$cmd" == all ]]; then
    section "cargo clippy --workspace -- -D warnings"
    if cargo clippy -j "$jobs" --workspace --quiet -- -D warnings 2>&1 | tail -5; then
        echo "clippy: clean"
    else
        status=1
    fi
fi
if [[ "$cmd" == e2e || "$cmd" == all ]]; then
    section "E2E suite (headless, private XDG dirs)"
    # The GTK daemon needs a display; tests use their own daemon in
    # private dirs. Inside the container there is no Wayland/X11, so
    # run under a virtual display if available.
    #
    # WARNING (capability honesty): WebKitGTK's bwrap web-process
    # sandbox needs user namespaces, which plain docker denies, so the
    # sandbox is DISABLED here. This lane therefore does NOT exercise
    # the production sandbox path; a userns-enabled or privileged
    # container is the follow-up (improver/backlog/linux-lane-green.md).
    export WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1
    export WEBKIT_DISABLE_DMABUF_RENDERER=1
    export LIBGL_ALWAYS_SOFTWARE=1
    export GTK_A11Y=none
    if ! command -v Xvfb >/dev/null && ! command -v weston >/dev/null; then
        echo "no Xvfb/weston in container; E2E needs a display server."
        echo "install: apt-get install -y xvfb   (then rerun with cmd=e2e)"
    else
        pass=0; fail=0; skip=0; failed=""
        for t in scripts/test-*.sh; do
            case "$t" in
                *test-mac-instrument*) continue ;;
                # Explicit, reasoned skips (not silent failures):
                *test-clone-suite*)
                    # needs live internet (webkit.org/stripe/scale clones)
                    skip=$((skip+1)); continue ;;
                *test-display-free*)
                    # needs a headless Wayland compositor (cage/labwc)
                    command -v cage >/dev/null || { skip=$((skip+1)); continue; } ;;
                *test-clone-scroll-effect*)
                    # clone sweep exceeds the 60s eval budget under
                    # pure-software rendering in the 4-core VM; runs on
                    # real Linux hardware. Open item:
                    # improver/backlog/clone-scroll-effect-aarch64.md
                    skip=$((skip+1)); continue ;;
            esac
            if command -v xvfb-run >/dev/null; then
                runner=(xvfb-run -a bash "$t")
            else
                runner=(bash "$t")
            fi
            if "${runner[@]}" >/tmp/hwatu-e2e-$(basename "$t").log 2>&1; then
                pass=$((pass+1))
            else
                fail=$((fail+1)); failed="$failed $(basename "$t")"
            fi
        done
        echo "e2e: $pass passed, $fail failed, $skip skipped (no-internet/no-compositor)"
        [[ -n "$failed" ]] && echo "failed:$failed"
        [[ $fail -gt 0 ]] && status=1
    fi
fi

echo
echo "linux-lane: exit $status"
exit $status
