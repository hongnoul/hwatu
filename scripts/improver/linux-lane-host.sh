#!/usr/bin/env bash
# Host-side driver for the Linux lane: runs linux-lane.sh inside a
# builder container in the colima VM. Prefers the sandbox-capable
# container (hwatu-builder2: userns allowed, WebKit sandbox exercised)
# and falls back to the plain builder. Long-lived ssh sessions to the
# VM die intermittently, so the lane is started detached inside the
# container and polled with short ssh calls until it finishes.
# Usage: scripts/improver/linux-lane-host.sh [cmd]
set -uo pipefail
cmd="${1:-all}"
sshcfg="$HOME/.colima/_lima/colima/ssh.config"
if [[ ! -f "$sshcfg" ]]; then
    echo "linux-lane-host: colima VM not found ($sshcfg missing); start with 'colima start'" >&2
    exit 2
fi

vm() { # short, retried ssh call
    local out rc attempt
    for attempt in 1 2 3 4 5; do
        out="$(ssh -F "$sshcfg" -o ConnectTimeout=10 lima-colima -- "$*" 2>/dev/null)"
        rc=$?
        if [[ $rc -ne 255 ]]; then
            printf '%s\n' "$out"
            return $rc
        fi
        sleep 2
    done
    echo "linux-lane-host: transport failed 5 times for: $*" >&2
    return 255
}

# Prefer the sandbox-capable container when it is running.
container="hwatu-builder"
if vm "docker inspect -f '{{.State.Running}}' hwatu-builder2 2>/dev/null" | grep -q true; then
    container="hwatu-builder2"
fi
echo "linux-lane-host: container=$container" >&2

run_id="lane-$$-$(date +%s)"
log="/tmp/hwatu-$run_id.log"
rcf="/tmp/hwatu-$run_id.rc"
vm "docker exec -d $container bash -c 'bash /src/scripts/improver/linux-lane.sh $cmd > $log 2>&1; echo \$? > $rcf'" \
    || exit 255

echo "linux-lane-host: started ($cmd), polling..." >&2
shown=0
while :; do
    sleep 10
    # Stream new log lines for progress.
    total="$(vm "docker exec $container bash -c 'wc -l < $log 2>/dev/null || echo 0'")" || continue
    total="${total//[^0-9]/}"
    if [[ -n "$total" && "$total" -gt "$shown" ]]; then
        vm "docker exec $container bash -c 'sed -n $((shown + 1)),${total}p $log'"
        shown=$total
    fi
    rc="$(vm "docker exec $container bash -c 'cat $rcf 2>/dev/null'")" || continue
    if [[ -n "$rc" ]]; then
        vm "docker exec $container bash -c 'rm -f $log $rcf'" >/dev/null || true
        exit "${rc//[^0-9]/}"
    fi
done
