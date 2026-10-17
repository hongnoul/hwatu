#!/usr/bin/env bash
# Deterministic E2E suite for the macOS verification instrument (hwatud-mac).
# Follows scripts/test-agent-loop.sh conventions (set -euo pipefail,
# pass/fail counters, cleanup trap, assert helpers).
#
# Isolation: the stock `hwatu` CLI resolves its socket from $HWATU_SOCKET or
# $XDG_RUNTIME_DIR/hwatu.sock, the Swift daemon mirrors that logic
# (platform/macos/Sources/main.swift: socketPath), and the CLI spawns
# `hwatud` from PATH when the socket is dead. Exporting a private
# XDG_RUNTIME_DIR therefore spawns an isolated daemon that cannot attach to
# or alter the user's running session. Cleanup sends `quit` to the isolated
# daemon only. (There is no $HWATU_DAEMON_BIN override; PATH resolution is
# what the CLI implements, so this suite relies on it.)
#
# Coverage (first slice of improver/backlog/macos-e2e-lane.md): headless open
# of a data: URL without focus steal, wait_load, eval roundtrip, nonempty
# PNG screenshot, resize, get_content containing a marker, click on an
# eval-injected button mutating the DOM, close, plus one capability-honesty
# assertion that an unsupported command answers a structured error naming
# the command.
#
# Usage: scripts/test-mac-instrument.sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/test-harness.sh
source "$root/scripts/lib/test-harness.sh"
hwatu="$(command -v hwatu)"

hw_private_env "hwatu-mac-instrument-test"
hw_set_quit_cmd "$hwatu"
hw_cleanup_install
hw_counters_init

marker="mac-marker-7Q3Z"
data_url="data:text/html,%3C!doctype%20html%3E%3Ctitle%3Emac-e2e%3C/title%3E%3Ch1%20id=%22marker%22%3E${marker}%3C/h1%3E"

# ---- ping the isolated daemon (the CLI spawns hwatud from PATH) --------
"$hwatu" ping >/dev/null
out="$("$hwatu" ping)"
check "isolated daemon answers ping as macos instrument" \
    json_assert 'd.get("platform") == "macos"' <<<"$out"

# ---- headless open of a data: URL ---------------------------------------
id="$("$hwatu" --headless "$data_url" | awk '{print $2}')"
out="$("$hwatu" list --json)"
check "headless open registers exactly one window" \
    json_assert 'len(d) == 1' <<<"$out"
check "headless open does not steal focus" \
    json_assert 'd[0].get("focused") in (False, None)' <<<"$out"

# ---- wait_load -----------------------------------------------------------
"$hwatu" wait-load --id "$id" --until dom >/dev/null
ok "wait_load reaches dom"

# ---- eval roundtrip ------------------------------------------------------
out="$("$hwatu" eval --id "$id" 'return (40 + 2)')"
check "eval roundtrips a value" json_assert 'd == 42' <<<"$out"
out="$("$hwatu" eval --id "$id" '40 + 2')"
check "eval answers expression form without explicit return" json_assert 'd == 42' <<<"$out"
out="$("$hwatu" eval --id "$id" 'return document.querySelector("#marker").textContent')"
check "eval reads the marker DOM" json_assert 'd == "'"$marker"'"' <<<"$out"

# ---- screenshot ----------------------------------------------------------
shot="$work/shot.png"
"$hwatu" shot --id "$id" "$shot" >/dev/null
check "screenshot writes a nonempty PNG" bash -c '[[ -s "$1" && $(head -c 8 "$1" | od -An -tx1 | tr -d " \n") == 89504e470d0a1a0a ]]' _ "$shot"

# ---- resize --------------------------------------------------------------
"$hwatu" resize --id "$id" 640x480 >/dev/null
out="$("$hwatu" eval --id "$id" 'return ({width: innerWidth, height: innerHeight})')"
check "resize updates DOM viewport to 640x480" json_assert 'd == {"width":640,"height":480}' <<<"$out"

# ---- get_content ---------------------------------------------------------
out="$("$hwatu" content --id "$id" --format text)"
check "get_content contains the marker" \
    bash -c 'MARKER="$0" python3 -c "import json,os,sys; d=json.load(sys.stdin); assert os.environ[\"MARKER\"] in json.dumps(d), d"' "$marker" <<<"$out"

# ---- click on an eval-injected button mutating the DOM -------------------
out="$("$hwatu" eval --id "$id" 'var b=document.createElement("button"); b.id="probe"; b.textContent="probe"; b.onclick=function(){document.body.dataset.probed="done"}; document.body.appendChild(b); return "probe-ready"')"
check "eval injects the probe button" json_assert 'd == "probe-ready"' <<<"$out"
"$hwatu" click --id "$id" '#probe' >/dev/null
out="$("$hwatu" eval --id "$id" 'return document.body.dataset.probed')"
check "click dispatches on the injected button and mutates the DOM" json_assert 'd == "done"' <<<"$out"

# ---- close ---------------------------------------------------------------
"$hwatu" close "$id" >/dev/null
check "close removes the window" bash -c '[[ $("$0" list --json) == "[]" ]]' "$hwatu"

# ---- capability honesty: unsupported command names itself ---------------
if "$hwatu" clock status >/dev/null 2>"$work/clock.err"; then
    bad "unsupported clock command unexpectedly succeeds"
elif grep -q clock "$work/clock.err"; then
    ok "unsupported clock command errors naming the command"
else
    bad "unsupported clock command error does not name the command"
fi

hw_summary "test-mac-instrument"
