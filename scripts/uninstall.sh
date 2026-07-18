#!/usr/bin/env bash
# Remove hwatu binaries installed by install.sh.
set -euo pipefail
INSTALL_DIR="${HWATU_INSTALL_DIR:-$HOME/.local/bin}"
"$INSTALL_DIR/hwatu" quit 2>/dev/null || true
rm -f "$INSTALL_DIR/hwatu" "$INSTALL_DIR/hwatud"
echo "hwatu: removed from $INSTALL_DIR"
