#!/usr/bin/env bash
# Acceptance check for improver/backlog/doctor-webkit-macos.md.
# On a Darwin host, `hwatu doctor` must exit 0 and the engine line must
# report the platform's actual engine (WKWebView), not demand WebKitGTK.
# Hermetic: a private XDG_RUNTIME_DIR makes the CLI spawn its own daemon
# instead of whatever daemon (or proxy) owns the user session's socket.
set -uo pipefail
[[ "$(uname -s)" == "Darwin" ]] || { echo "skip: Darwin-only check"; exit 0; }

work="$(mktemp -d "${TMPDIR:-/tmp}/hwatu-doctor-accept.XXXXXX")"
export XDG_RUNTIME_DIR="$work/run"; mkdir -p "$XDG_RUNTIME_DIR"
bin="${HWATU_BIN:-hwatu}"
cleanup() { "$bin" quit >/dev/null 2>&1 || true; rm -rf "$work"; }
trap cleanup EXIT INT TERM

out="$("$bin" doctor 2>&1)"; rc=$?
echo "$out"

fail=0
if [[ $rc -ne 0 ]]; then
    echo "ACCEPT-FAIL: doctor exited $rc on macOS"
    fail=1
fi
if grep -q "WebKitGTK (libwebkitgtk-6.0) not found" <<<"$out"; then
    echo "ACCEPT-FAIL: doctor demands WebKitGTK on macOS"
    fail=1
fi
if ! grep -qiE "ok +(webkit|engine).*(wkwebview|webkit\.framework)" <<<"$out"; then
    echo "ACCEPT-FAIL: engine line does not report the macOS engine"
    fail=1
fi
[[ $fail -eq 0 ]] && echo "ACCEPT-PASS"
exit $fail
