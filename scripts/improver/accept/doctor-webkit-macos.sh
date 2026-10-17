#!/usr/bin/env bash
# Acceptance check for improver/backlog/doctor-webkit-macos.md.
# On a Darwin host with a reachable daemon, `hwatu doctor` must exit 0 and
# the engine line must report the platform's actual engine (WKWebView),
# not demand WebKitGTK.
set -uo pipefail
[[ "$(uname -s)" == "Darwin" ]] || { echo "skip: Darwin-only check"; exit 0; }

bin="${HWATU_BIN:-hwatu}"
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
