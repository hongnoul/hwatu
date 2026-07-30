#!/usr/bin/env bash
# Local repro of a GPU-less CI runner: run a command inside a bubblewrap
# sandbox where /dev/dri is masked with an empty tmpfs, so no DRM render
# node is visible. Everything else binds through read-write.
# Usage: scripts/dev/no-gpu.sh <command...>
set -euo pipefail
exec bwrap \
    --dev-bind / / \
    --tmpfs /dev/dri \
    --die-with-parent \
    -- "$@"
