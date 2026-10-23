#!/usr/bin/env bash
# Cross-OS conformance: the same fixtures and the same expected results
# against whichever daemon this OS runs (VISION.md "Platform support
# criteria" item 3). Fixtures live in tests/conformance/ and are served
# over local HTTP. No network access needed.
# Usage: scripts/test-conformance.sh [release-bin-dir]
# On macOS, HWATUD=<path to hwatud-mac> selects the daemon binary. The
# daemon runs on an isolated socket and the user's daemon is not touched.
set -u
root="$(cd "$(dirname "$0")/.." && pwd)"
b="${1:-$root/target/release}"
w=$(mktemp -d "${TMPDIR:-/tmp}/hw-conf.XXXX")
mkdir -p "$w/run" "$w/state" "$w/bin"
export XDG_RUNTIME_DIR="$w/run" XDG_STATE_HOME="$w/state" XDG_DATA_HOME="$w/data" XDG_CONFIG_HOME="$w/config"
if [[ "$(uname)" == Darwin ]]; then
  export HWATU_SOCKET="$w/run/hwatu.sock"
  ln -sf "${HWATUD:-$b/hwatud}" "$w/bin/hwatud"
  export PATH="$w/bin:$PATH"
else
  export WEBKIT_DISABLE_DMABUF_RENDERER=1 LIBGL_ALWAYS_SOFTWARE=1 WLR_RENDERER=pixman
  unset WAYLAND_DISPLAY DISPLAY
fi

port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
python3 -m http.server "$port" --bind 127.0.0.1 --directory "$root/tests/conformance" >/dev/null 2>&1 &
srv=$!
# A second origin for cross-origin frame checks: localhost may resolve to
# ::1, so serve the same directory there too.
python3 -m http.server "$port" --bind ::1 --directory "$root/tests/conformance" >/dev/null 2>&1 &
srv6=$!
trap 'kill $srv $srv6 2>/dev/null; "$b/hwatu" quit >/dev/null 2>&1; rm -rf "$w"' EXIT
base="http://127.0.0.1:$port"
for _ in $(seq 50); do curl -fs "$base/plain.html" >/dev/null && break; sleep 0.1; done

hw() { "$b/hwatu" "$@" 2>&1 | grep -v "daemon build .* != client build"; }
pass=0; fail=0
ck() { if eval "$2"; then pass=$((pass+1)); echo "ok   - $1"; else fail=$((fail+1)); echo "FAIL - $1"; echo "       got: ${o:0:300}"; fi; }
open_id() { hw "$1" | grep -o 'window [0-9]*' | head -1 | cut -d' ' -f2; }
title() { hw eval --id "$1" 'return document.title'; }

# C1 ping reports the contract fields every engine must expose
o=$(hw ping)
ck "C1 ping reports platform, engine, structured_stops" \
  'grep -q "\"platform\"" <<<"$o" && grep -q "\"engine\"" <<<"$o" && grep -q "structured_stops" <<<"$o"'

# C2 open is headless
id=$(open_id "$base/plain.html"); hw wait-load --id "$id" --until settled >/dev/null
o=$(hw list --json); ck "C2 open is headless" 'grep -q "\"mode\":\"headless\"" <<<"$o"'
o=$(hw content --id "$id" --format text); ck "C3 content returns page text" 'grep -q Hello <<<"$o"'
o=$(hw eval --id "$id" 'return 2 + 40'); ck "C4 eval returns a value" 'grep -q 42 <<<"$o"'
hw shot --id "$id" "$w/plain.png" >/dev/null
o=$(head -c 8 "$w/plain.png" | od -An -c | tr -d ' \n'); ck "C5 shot writes a PNG" '[[ "$o" == *PNG* ]]'
hw close "$id" >/dev/null

# Irreversible stop
id=$(open_id "$base/stops.html"); hw wait-load --id "$id" >/dev/null
o=$(hw click --id "$id" '#add'); ck "C6 ordinary click is not stopped" '! grep -q "\"stop\"" <<<"$o"'
o=$(hw click --id "$id" '#pay'); ck "C7 Place Order returns irreversible_action stop" 'grep -q "\"reason\":\"irreversible_action\"" <<<"$o"'
o=$(hw click --id "$id" --trusted '#pay'); ck "C7b --trusted does not bypass the irreversible stop" 'grep -q "\"reason\":\"irreversible_action\"" <<<"$o"'
o=$(title "$id"); ck "C8 stopped click did not run the handler" 'grep -q added <<<"$o"'
hw click --id "$id" --confirm '#pay' >/dev/null
o=$(title "$id"); ck "C9 --confirm performs the click" 'grep -q paid <<<"$o"'
o=$(hw auth-context --id "$id"); ck "C10 password form reports login_required" 'grep -q "\"reason\":\"login_required\"" <<<"$o"'
hw close "$id" >/dev/null

id=$(open_id "$base/wizard.html"); hw wait-load --id "$id" >/dev/null
hw click --id "$id" '#f' >/dev/null; o=$(title "$id")
ck "C11 Finish on a wizard page is not stopped" 'grep -q done <<<"$o"'
hw close "$id" >/dev/null
id=$(open_id "$base/review.html"); hw wait-load --id "$id" >/dev/null
o=$(hw click --id "$id" '#f'); ck "C12 Finish on a checkout review with a total is stopped" 'grep -q "\"reason\":\"irreversible_action\"" <<<"$o"'
o=$(title "$id"); ck "C13 order not placed" '! grep -q placed <<<"$o"'
hw close "$id" >/dev/null

# Login wall rendered by JS after a mutating loading indicator. settled
# returns at the load event, before the form; quiescent waits for it.
id=$(open_id "$base/late-login.html"); hw wait-load --id "$id" --until settled >/dev/null
o=$(hw auth-context --id "$id"); ck "C14a control: settled returns before the JS login form" 'grep -q "\"login_required\":null" <<<"$o"'
hw close "$id" >/dev/null
id=$(open_id "$base/late-login.html"); hw wait-load --id "$id" --until quiescent >/dev/null
o=$(hw auth-context --id "$id"); ck "C14 JS-rendered login form reports login_required after quiescent" 'grep -q "\"reason\":\"login_required\"" <<<"$o"'
o=$(hw expect --id "$id" '#go' --visible --timeout-ms 3000); ck "C15 expect finds the rendered control" '! grep -qi "error\|timed out" <<<"$o"'
hw close "$id" >/dev/null

# Challenge
id=$(open_id "$base/challenge.html"); hw wait-load --id "$id" >/dev/null
o=$(hw challenge --id "$id"); ck "C16 challenge detected with type" 'grep -q "\"status\":\"challenge\"" <<<"$o" && grep -q "\"challenge_type\":\"turnstile\"" <<<"$o"'
o=$(hw challenge --id "$id" --wait --timeout-ms 1500); ck "C17 challenge --wait timeout returns challenge stop with show retry" \
  'grep -q "\"reason\":\"challenge\"" <<<"$o" && grep -q "\"command\":\"show\"" <<<"$o"'
hw close "$id" >/dev/null
id=$(open_id "$base/challenge-clears.html"); hw wait-load --id "$id" >/dev/null
o=$(hw challenge --id "$id" --wait --timeout-ms 8000); ck "C18 challenge that clears reports cleared" 'grep -q "\"status\":\"cleared\"" <<<"$o"'
hw close "$id" >/dev/null
id=$(open_id "$base/plain.html"); hw wait-load --id "$id" >/dev/null
o=$(hw challenge --id "$id"); ck "C19 plain page reports clear" 'grep -q "\"status\":\"clear\"" <<<"$o"'

# batch stops at the first stop and reports it
o=$(hw goto --id "$id" "$base/stops.html")
o=$(hw batch "[{\"cmd\":\"click\",\"id\":$id,\"selector\":\"#add\"},{\"cmd\":\"click\",\"id\":$id,\"selector\":\"#pay\"},{\"cmd\":\"eval\",\"id\":$id,\"js\":\"return 1\"}]")
ck "C20 batch halts at the irreversible stop" 'grep -q irreversible_action <<<"$o" && ! grep -q "\"complete\":true" <<<"$o"'
o=$(title "$id"); ck "C21 batch did not run the stopped handler" '! grep -q paid <<<"$o"'

# Trusted input. Where the engine supports it headless, the page sees
# isTrusted events. Elsewhere the daemon must refuse with an error naming
# show instead of silently sending untrusted events.
o=$(hw goto --id "$id" "$base/trusted.html")
caps=$(hw ping)
if grep -q '"trusted_input_headless":true' <<<"$caps"; then
  hw click --id "$id" --trusted '#b' >/dev/null
  hw type --id "$id" --trusted '#i' ab >/dev/null
  o=$(hw eval --id "$id" 'return JSON.stringify({log, v: document.getElementById("i").value})')
  ck "C22 trusted click and type arrive with isTrusted" 'grep -q "click:true" <<<"$o" && grep -q "key:true" <<<"$o" && ! grep -q ":false" <<<"$o"'
  ck "C23 trusted type sets the value" 'grep -q "ab" <<<"$o"'
else
  o=$(hw click --id "$id" --trusted '#b'); ck "C22 headless trusted click refused with show hint" 'grep -q "show" <<<"$o"'
  o=$(hw eval --id "$id" 'return log.length'); ck "C23 refused trusted click sent no events" 'grep -q "0" <<<"$o"'
fi

# Upload: the file chooser receives the armed file without a panel; the
# page can read the bytes; a non-file input is refused.
printf 'hello upload\n' > "$w/up.txt"
o=$(hw goto --id "$id" "$base/upload.html")
o=$(hw upload --id "$id" '#f' "$w/up.txt"); ck "C24 upload reports the file" 'grep -q "\"uploaded\":\"up.txt\"" <<<"$o"'
o=$(hw eval --id "$id" 'for (let i = 0; i < 20 && !got; i++) await new Promise(r => setTimeout(r, 50)); return JSON.stringify(got)')
ck "C25 page reads the uploaded bytes" 'grep -q "hello upload" <<<"$o"'
o=$(hw upload --id "$id" '#t' "$w/up.txt"); ck "C26 upload to a non-file input is refused" 'grep -q "not an <input type=file>" <<<"$o"'

# Network log: method, status and type per completed request; --clear drains.
o=$(hw goto --id "$id" "$base/net.html")
hw eval --id "$id" 'return await posted' >/dev/null
o=$(hw net --id "$id")
netq() { python3 -c 'import json,sys; e=json.loads(sys.argv[1]); sys.exit(0 if any('"$1"' for x in e) else 1)' "$o"; }
ck "C27 net logs the document with status 200" 'netq "x[\"url\"].endswith(\"/net.html\") and x.get(\"status\")==200 and x[\"type\"]==\"document\""'
ck "C28 net logs the stylesheet type" 'netq "x[\"url\"].endswith(\"/net.css\") and x[\"type\"]==\"stylesheet\""'
ck "C29 net logs a 404 subresource with its status" 'netq "x[\"url\"].endswith(\"/missing.png\") and x.get(\"status\")==404"'
ck "C30 net logs the fetch POST method" 'netq "x[\"method\"]==\"POST\" and \"post\" in x[\"url\"]"'
hw net --id "$id" --clear >/dev/null
o=$(hw net --id "$id"); ck "C31 net --clear drains the buffer" '[[ "$o" == "[]" ]]'

# Pixel diff: identical windows match; a moved block is a significant
# region; a baseline PNG from shot compares equal. Engines without a
# native diff are diffed by the client from shot output.
wa=$(open_id "$base/diff-a.html"); wb=$(open_id "$base/diff-a.html")
hw wait-load --id "$wa" >/dev/null; hw wait-load --id "$wb" >/dev/null
o=$(hw diff --id "$wa" --other "$wb"); ck "C32 identical windows: 0 significant regions" 'grep -q "\"significant_regions\":0" <<<"$o" && grep -q "\"match_percent\":100" <<<"$o"'
hw eval --id "$wb" 'document.getElementById("box").style.left = "400px"; return 1' >/dev/null; sleep 0.3
o=$(hw diff --id "$wa" --other "$wb" --heatmap "$w/hm.png"); ck "C33 moved block: significant region and heatmap" \
  '! grep -q "\"significant_regions\":0" <<<"$o" && grep -q worst_region <<<"$o" && [[ -s "$w/hm.png" ]]'
hw shot --id "$wa" "$w/base.png" >/dev/null
o=$(hw diff --id "$wa" --baseline "$w/base.png"); ck "C34 diff against own shot baseline matches" 'grep -q "\"significant_regions\":0" <<<"$o"'
hw close "$wa" >/dev/null; hw close "$wb" >/dev/null

# Motion inventory (declared CSS/WAAPI) and JSON-safe eval values.
o=$(hw goto --id "$id" "$base/motion.html")
o=$(hw motion --id "$id")
ck "C35 motion reports the running keyframe animation" 'grep -q "\"name\":\"spin\"" <<<"$o" && grep -q "\"duration_ms\":2000" <<<"$o"'
ck "C36 motion reports the idle declared transition" 'grep -q "\"property\":\"opacity\"" <<<"$o"'
o=$(hw eval --id "$id" 'return [Infinity, NaN, 1]'); ck "C37 non-finite eval values serialize as null" 'grep -q "\[null,null,1\]" <<<"$o"'
o=$(hw ping); ck "C38 daemon alive after non-finite values" 'grep -q "\"engine\"" <<<"$o"'

# Point clicks (`click --trusted --at x,y`) reach a cross-origin frame as
# trusted input at the right spot, and the irreversible guard still applies
# to the element at the point.
caps=$(hw ping)
o=$(hw goto --id "$id" "$base/frame-host.html"); sleep 0.5
if grep -q '"trusted_input_headless":true' <<<"$caps"; then
  # tile centre: iframe at (100,100) + tile at (150,100) + 30 = (280, 230)
  hw click --id "$id" --trusted --at 280,230 >/dev/null
  o=$(hw eval --id "$id" 'for (let i = 0; i < 20 && !childClicks.length; i++) await new Promise(r => setTimeout(r, 50)); return JSON.stringify(childClicks)')
  ck "C42 point click lands on the tile inside a cross-origin frame, trusted" 'grep -q "trusted.\{0,3\}:true" <<<"$o" && grep -q "onTile.\{0,3\}:true" <<<"$o"'
  o=$(hw click --id "$id" --trusted --at 560,120); ck "C43 point click on Place Order returns irreversible_action" 'grep -q "\"reason\":\"irreversible_action\"" <<<"$o"'
else
  o=$(hw click --id "$id" --trusted --at 280,230); ck "C42 headless point click refused with show hint" 'grep -q "show" <<<"$o"'
  o=$(hw click --id "$id" --trusted --at 560,120); ck "C43 point click on Place Order returns irreversible_action" 'grep -q "\"reason\":\"irreversible_action\"" <<<"$o"'
fi
o=$(hw click --id "$id" --at 280,230); ck "C44 --at without --trusted is rejected" 'grep -q "needs --trusted" <<<"$o"'

# Irreversible labels in many languages, classified in the real engine on
# a checkout page. The guard must stop every `stop` label and pass every
# `go` label. A held-out set (never tuned against) is reported only.
o=$(hw goto --id "$id" "$base/checkout/review.html")
label_js() { # label_js <stop+go|heldout>: the classify-labels.js eval body
  python3 - "$root" "$1" <<'PY'
import json, sys
root, which = sys.argv[1], sys.argv[2]
d = json.load(open(f"{root}/tests/conformance/irreversible-labels.json"))
labels = d["heldout"] if which == "heldout" else {"stop": d["stop"], "go": d["go"]}
classify = open(f"{root}/page-scripts/irreversible.js").read().strip()
body = open(f"{root}/scripts/classify-labels.js").read()
print(body.replace("__HWATU_LABELS__", json.dumps(labels, ensure_ascii=False)).replace("__HWATU_CLASSIFY__", classify))
PY
}
o=$(hw eval --id "$id" "$(label_js gated)")
ck "C40 irreversible guard stops every multilingual stop label on checkout" 'grep -q "\"stop_missed\":\[\]" <<<"$o"'
ck "C41 irreversible guard passes every navigation/section label" 'grep -q "\"go_stopped\":\[\]" <<<"$o"'
echo "   held-out labels (not gated): $(hw eval --id "$id" "$(label_js heldout)")"

# show renders the session as a shot or a diff without making it visible.
o=$(hw show "$id" --as shot "$w/show.png"); ck "C45 show --as shot writes a PNG" '[[ -s "$w/show.png" ]]'
o=$(hw show "$id" --as diff --baseline "$w/show.png"); ck "C46 show --as diff reports a diff" 'grep -q "\"significant_regions\"" <<<"$o"'
o=$(hw list --json); ck "C47 show --as shot/diff leave the window headless" 'grep -q "\"mode\":\"headless\"" <<<"$o"'

# Reply shapes: a fixed command sequence must produce the committed
# golden output byte for byte (timings and ids normalized), so a skill
# reads the same JSON on every engine. Regenerate on purpose with
# scripts/conformance-shapes.sh after an intended contract change.
o=$("$root/scripts/conformance-shapes.sh" "$b/hwatu" "$base" | diff "$root/tests/conformance/shapes.golden" - 2>&1)
ck "C39 every command's reply matches tests/conformance/shapes.golden" '[[ -z "$o" ]]'; [[ -n "$o" ]] && echo "$o" | head -8

# Session persistence (VISION: credentials entered once, the session
# persists across runs): cookies and localStorage survive a daemon
# restart. Runs last because it restarts the daemon.
hw goto --id "$id" "$base/plain.html" >/dev/null
hw eval --id "$id" 'document.cookie = "hwatu_conf=1; max-age=3600; path=/"; localStorage.setItem("hwatu_conf", "1"); return 1' >/dev/null
hw quit >/dev/null; sleep 1
pid=$(hw "$base/plain.html" | grep -o 'window [0-9]*' | head -1 | cut -d' ' -f2); hw wait-load --id "$pid" >/dev/null
o=$(hw eval --id "$pid" 'return JSON.stringify({c: document.cookie.includes("hwatu_conf=1"), ls: localStorage.getItem("hwatu_conf")})')
ck "C48 cookies and localStorage survive a daemon restart" 'grep -q "c.\{0,3\}:true" <<<"$o" && grep -q "ls.\{0,3\}:.\{0,3\}1" <<<"$o"'

echo "pass=$pass fail=$fail ($(uname))"
[[ $fail -eq 0 ]]
