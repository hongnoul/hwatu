#!/usr/bin/env bash
# Regenerate platform/macos/Sources/PageScripts.swift from page-scripts/.
# The Rust daemon embeds page-scripts/ with include_str!; the Swift daemon
# is built with plain swiftc, so it gets a generated copy. A Rust test in
# crates/hwatud/src/stops.rs fails when the copy is stale.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
out="$root/platform/macos/Sources/PageScripts.swift"
emit() { # emit <swift-name> <file>
  echo "let $1 = #\"\"\""
  cat "$root/page-scripts/$2"
  echo '"""#'
}
{
  echo '// GENERATED from page-scripts/. Do not edit by hand.'
  echo '// Regenerate: scripts/gen-page-scripts.sh'
  emit irreversibleJS irreversible.js
  emit loginWallJS login-wall.js
  emit challengeJS challenge.js
  emit motionJS motion.js
  emit targetJS target.js
  emit snapshotJS snapshot.js
  emit contentJS content.js
  emit clickJS click.js
  emit typeJS type.js
  emit nativeTimeJS native-time.js
  emit pressJS press.js
  emit visibilityJS visibility.js
  emit expectJS expect.js
  emit scrollJS scroll.js
  emit fillJS fill.js
  emit authContextJS auth-context.js
} >"$out"
echo "wrote $out"
