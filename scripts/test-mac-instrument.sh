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
# Coverage (improver/backlog/macos-e2e-lane.md slices 1+2): headless open
# of a data: URL without focus steal, wait_load, eval roundtrip, nonempty
# PNG screenshot, resize, get_content containing a marker, click on an
# eval-injected button mutating the DOM, navigate between data: pages,
# snapshot interactables, fill_form on a text input + checkbox, type with
# append, press Enter delivering a keydown, scroll moving window.scrollY,
# console log + error capture, list showing exactly the opened windows,
# close, plus one capability-honesty assertion that an unsupported command
# answers a structured error naming the command.
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

# data: URL builder (python quoting keeps pages readable and deterministic).
mkurl() { python3 -c 'import sys,urllib.parse; print("data:text/html," + urllib.parse.quote(sys.argv[1]))' "$1"; }

marker="mac-marker-7Q3Z"
data_url="data:text/html,%3C!doctype%20html%3E%3Ctitle%3Emac-e2e%3C/title%3E%3Ch1%20id=%22marker%22%3E${marker}%3C/h1%3E"

# ---- ping the isolated daemon (the CLI spawns hwatud from PATH) --------
# covers: ping
"$hwatu" ping >/dev/null
out="$("$hwatu" ping)"
check "isolated daemon answers ping as macos instrument" \
    json_assert 'd.get("platform") == "macos"' <<<"$out"

# ---- headless open of a data: URL ---------------------------------------
# covers: open
id="$("$hwatu" --headless "$data_url" | awk '{print $2}')"
out="$("$hwatu" list --json)"
check "headless open registers exactly one window" \
    json_assert 'len(d) == 1' <<<"$out"
check "headless open does not steal focus" \
    json_assert 'd[0].get("focused") in (False, None)' <<<"$out"

# ---- wait_load -----------------------------------------------------------
# covers: wait_load
"$hwatu" wait-load --id "$id" --until dom >/dev/null
ok "wait_load reaches dom"

# ---- eval roundtrip ------------------------------------------------------
# covers: eval
out="$("$hwatu" eval --id "$id" 'return (40 + 2)')"
check "eval roundtrips a value" json_assert 'd == 42' <<<"$out"
out="$("$hwatu" eval --id "$id" '40 + 2')"
check "eval answers expression form without explicit return" json_assert 'd == 42' <<<"$out"
out="$("$hwatu" eval --id "$id" 'return document.querySelector("#marker").textContent')"
check "eval reads the marker DOM" json_assert 'd == "'"$marker"'"' <<<"$out"

# ---- screenshot ----------------------------------------------------------
# covers: screenshot
shot="$work/shot.png"
"$hwatu" shot --id "$id" "$shot" >/dev/null
check "screenshot writes a nonempty PNG" bash -c '[[ -s "$1" && $(head -c 8 "$1" | od -An -tx1 | tr -d " \n") == 89504e470d0a1a0a ]]' _ "$shot"

# ---- resize --------------------------------------------------------------
# covers: resize
"$hwatu" resize --id "$id" 640x480 >/dev/null
out="$("$hwatu" eval --id "$id" 'return ({width: innerWidth, height: innerHeight})')"
check "resize updates DOM viewport to 640x480" json_assert 'd == {"width":640,"height":480}' <<<"$out"

# ---- get_content ---------------------------------------------------------
# covers: get_content
out="$("$hwatu" content --id "$id" --format text)"
check "get_content contains the marker" \
    bash -c 'MARKER="$0" python3 -c "import json,os,sys; d=json.load(sys.stdin); assert os.environ[\"MARKER\"] in json.dumps(d), d"' "$marker" <<<"$out"

# ---- click on an eval-injected button mutating the DOM -------------------
# covers: click
out="$("$hwatu" eval --id "$id" 'var b=document.createElement("button"); b.id="probe"; b.textContent="probe"; b.onclick=function(){document.body.dataset.probed="done"}; document.body.appendChild(b); return "probe-ready"')"
check "eval injects the probe button" json_assert 'd == "probe-ready"' <<<"$out"
"$hwatu" click --id "$id" '#probe' >/dev/null
out="$("$hwatu" eval --id "$id" 'return document.body.dataset.probed')"
check "click dispatches on the injected button and mutates the DOM" json_assert 'd == "done"' <<<"$out"

# ---- navigate between data: pages ----------------------------------------
# covers: navigate
page_b="$(mkurl '<!doctype html><title>B</title><h1 id="mk">PAGE-B-9K2V</h1>')"
"$hwatu" goto --id "$id" "$page_b" >/dev/null
ok "navigate loads page B"
out="$("$hwatu" content --id "$id" --format text)"
check "navigate lands on page B (content shows B marker)" \
    bash -c 'python3 -c "import json,sys; d=json.load(sys.stdin); assert \"PAGE-B-9K2V\" in json.dumps(d), d"' <<<"$out"
out="$("$hwatu" eval --id "$id" 'return document.title')"
check "navigate updates the document title" json_assert 'd == "B"' <<<"$out"

# ---- snapshot ------------------------------------------------------------
# covers: snapshot
page_snap="$(mkurl '<!doctype html><title>snap</title><button id="btn">ClickMeSnap</button><input id="nm" name="nm" placeholder="YourNameSnap">')"
"$hwatu" goto --id "$id" "$page_snap" >/dev/null
"$hwatu" wait-load --id "$id" --until dom >/dev/null
out="$("$hwatu" snapshot --id "$id")"
check "snapshot lists the button interactable" \
    json_assert 'any(e.get("tag") == "button" and "ClickMeSnap" in e.get("text", "") for e in d.get("interactables", []))' <<<"$out"
check "snapshot lists the named input interactable" \
    json_assert 'any(e.get("tag") == "input" and e.get("name") == "nm" for e in d.get("interactables", []))' <<<"$out"

# ---- fill_form -----------------------------------------------------------
# covers: fill_form
page_form="$(mkurl '<!doctype html><title>form</title><form id="f"><input id="nm" name="nm"><input id="cb" type="checkbox"></form>')"
"$hwatu" goto --id "$id" "$page_form" >/dev/null
"$hwatu" wait-load --id "$id" --until dom >/dev/null
out="$("$hwatu" fill --id "$id" --field '#nm=hello42' --field '#cb:checked=true')"
check "fill_form fills both fields" json_assert 'd.get("filled") == 2' <<<"$out"
out="$("$hwatu" eval --id "$id" 'return document.querySelector("#nm").value + "|" + document.querySelector("#cb").checked')"
check "fill_form values read back via eval" json_assert 'd == "hello42|true"' <<<"$out"

# ---- type ----------------------------------------------------------------
# covers: type
out="$("$hwatu" type --id "$id" '#nm' 'XYZ' --no-clear)"
check "type appends without clearing" json_assert 'd.get("value") == "hello42XYZ"' <<<"$out"
out="$("$hwatu" eval --id "$id" 'return document.querySelector("#nm").value')"
check "typed text reads back via eval" json_assert 'd == "hello42XYZ"' <<<"$out"

# ---- press ---------------------------------------------------------------
# covers: press
page_press="$(mkurl '<!doctype html><title>press</title><input id="in">')"
"$hwatu" goto --id "$id" "$page_press" >/dev/null
"$hwatu" wait-load --id "$id" --until dom >/dev/null
out="$("$hwatu" eval --id "$id" 'var el=document.querySelector("#in"); el.addEventListener("keydown", function(e){window.__pk="hit:"+e.key}); el.focus(); return document.activeElement.id')"
check "press target input is focused" json_assert 'd == "in"' <<<"$out"
out="$("$hwatu" press --id "$id" Enter)"
check "press Enter answers the key" json_assert 'd.get("key") == "Enter"' <<<"$out"
out="$("$hwatu" eval --id "$id" 'return window.__pk || "unset"')"
check "press Enter delivers a keydown to the focused input" json_assert 'd == "hit:Enter"' <<<"$out"

# ---- scroll --------------------------------------------------------------
# covers: scroll
page_tall="$(mkurl '<!doctype html><title>tall</title><div style="height:4000px">TALLMARKER</div>')"
"$hwatu" goto --id "$id" "$page_tall" >/dev/null
"$hwatu" wait-load --id "$id" --until dom >/dev/null
"$hwatu" resize --id "$id" 640x480 >/dev/null
out="$("$hwatu" scroll --id "$id" --to-y 1500)"
check "scroll to-y answers the new offset" json_assert 'd.get("y") == 1500' <<<"$out"
out="$("$hwatu" eval --id "$id" 'return window.scrollY')"
check "scroll moves window.scrollY" json_assert 'd == 1500' <<<"$out"

# ---- console -------------------------------------------------------------
# covers: console
"$hwatu" console --id "$id" --clear >/dev/null
"$hwatu" eval --id "$id" 'console.log("con-log-ABC"); return "logged"' >/dev/null
"$hwatu" eval --id "$id" 'setTimeout(function(){ throw new Error("con-err-XYZ") }, 100); return "armed"' >/dev/null
sleep 2
out="$("$hwatu" console --id "$id")"
check "console captures page log text" \
    json_assert 'any("con-log-ABC" in e.get("text", "") for e in d)' <<<"$out"
check "console captures a thrown error entry" \
    json_assert 'any(e.get("level") == "error" for e in d)' <<<"$out"

# ---- list ----------------------------------------------------------------
# covers: list
page_b2="$(mkurl '<!doctype html><title>B2</title><h1>second-window</h1>')"
id2="$("$hwatu" --headless "$page_b2" | awk '{print $2}')"
out="$("$hwatu" list --json)"
check "list shows exactly the two windows this suite opened" \
    json_assert 'len(d) == 2' <<<"$out"
check "list reports the correct window ids" \
    json_assert 'sorted(w["id"] for w in d) == sorted(['"$id, $id2"'])' <<<"$out"
"$hwatu" close "$id2" >/dev/null
out="$("$hwatu" list --json)"
check "list shows one window after closing the second" \
    json_assert 'len(d) == 1 and d[0]["id"] == '"$id" <<<"$out"

# ---- close ---------------------------------------------------------------
# covers: close
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
