#!/usr/bin/env bash
# Regenerate platform/macos/Sources/PageScripts.swift from page-scripts/.
# The Rust daemon embeds page-scripts/ with include_str!; the Swift daemon
# is built with plain swiftc, so it gets a generated copy. A Rust test in
# crates/hwatud/src/stops.rs fails when the copy is stale.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/platform/macos/Sources/PageScripts.swift"
{
  echo '// GENERATED from page-scripts/irreversible.js. Do not edit by hand.'
  echo '// Regenerate: scripts/gen-page-scripts.sh'
  echo 'let irreversibleJS = #"""'
  cat "$root/page-scripts/irreversible.js"
  echo '"""#'
} >"$out"
echo "wrote $out"
