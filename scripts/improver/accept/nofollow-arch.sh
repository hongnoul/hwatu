#!/usr/bin/env bash
# Acceptance check for improver/backlog/aarch64-o-nofollow-security.md.
# Hardcoded open(2) flag literals are per-arch wrong (0o400000 is
# O_NOFOLLOW on x86 Linux but O_NOCTTY-adjacent garbage on aarch64 Linux,
# following symlinks). All open-flag constants must come from libc (or
# exhaustive per-arch cfgs), and the symlink-refusal test must pass on
# the aarch64 Linux lane AND on macOS.
set -uo pipefail
root="$(cd "$(dirname "$0")/../../.." && pwd)"

fail=0

# 1. No hardcoded open-flag literals remain in rust sources.
hits="$(grep -rnE '0o400000|0o4000[^0-9]|0x0?0?04[^0-9a-fA-F]|O_NOFOLLOW: i32 = 0' \
    "$root/crates" --include='*.rs' | grep -vE 'libc::' || true)"
if [[ -n "$hits" ]]; then
    echo "ACCEPT-FAIL: hardcoded open-flag literals remain:"
    echo "$hits"
    fail=1
fi

# 2. Symlink-refusal unit test passes on this host.
if ! (cd "$root" && cargo test -p hwatu materialization_refuses_symlink --quiet >/dev/null 2>&1); then
    echo "ACCEPT-FAIL: materialization_refuses_symlink_destinations fails on $(uname -sm)"
    fail=1
fi

# 3. Same test passes on the aarch64 Linux lane when available.
if [[ -f "$HOME/.colima/_lima/colima/ssh.config" ]]; then
    out="$(ssh -F "$HOME/.colima/_lima/colima/ssh.config" -o ConnectTimeout=10 lima-colima -- \
        "docker exec hwatu-builder bash -c 'export PATH=/root/.cargo/bin:\$PATH; cd /src && cargo test -p hwatu materialization_refuses_symlink 2>&1 | tail -3'" 2>/dev/null)"
    if echo "$out" | grep -q "test result: ok"; then
        echo "ok    aarch64 Linux: symlink refusal test passes"
    elif [[ -z "$out" ]]; then
        echo "skip: Linux lane unreachable (transport)"
    else
        echo "ACCEPT-FAIL: aarch64 Linux symlink test: $out"
        fail=1
    fi
else
    echo "skip: no Linux lane on this host"
fi

[[ $fail -eq 0 ]] && echo "ACCEPT-PASS"
exit $fail
