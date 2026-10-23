#!/usr/bin/env bash
# Reference skill (VISION.md "Platform support criteria" item 4): an
# authenticated, multi-step task on a real public site (saucedemo.com) that
# stops before the irreversible step, then completes after confirmation.
# Driven through the real CLI and the real MCP stdio server against an
# isolated display-free daemon. Needs network access.
# Usage: scripts/test-reference-skill.sh [release-bin-dir]
# On macOS, HWATUD=<path to hwatud-mac> selects the daemon binary; the
# daemon runs on an isolated socket and the user's daemon is not touched.
set -u
root="$(cd "$(dirname "$0")/.." && pwd)"
b="${1:-$root/target/release}"
w=$(mktemp -d "${TMPDIR:-/tmp}/hw-acc.XXXX")
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
hw() { "$b/hwatu" "$@" 2>&1 | grep -v "daemon build .* != client build"; }
pass=0; fail=0
ck() { if eval "$2"; then pass=$((pass+1)); echo "ok   - $1"; else fail=$((fail+1)); echo "FAIL - $1"; fi; }

# R: doctor green
o=$(hw doctor); ck "R1 doctor: all checks passed" 'grep -q "all checks passed" <<<"$o"'

# R: open is headless
o=$(hw https://www.saucedemo.com/); echo "   open: $o"; id=$(grep -o 'window [0-9]*' <<<"$o" | head -1 | cut -d' ' -f2)
hw wait-load --id "$id" --until quiescent >/dev/null
o=$(hw list --json); ck "R2 window opened headless" 'grep -q "\"mode\":\"headless\"" <<<"$o"'

# R: login wall reported as structured stop
o=$(hw auth-context --id "$id"); echo "   R3 raw: ${o:0:240}"; ck "R3 login_required on real login page" 'grep -q "\"reason\":\"login_required\"" <<<"$o"'

# Agent enters the credential (the user would supply it)
hw type --id "$id" '#user-name' standard_user >/dev/null
hw type --id "$id" '#password' secret_sauce >/dev/null
o=$(hw click --id "$id" '#login-button'); ck "R4 login click is not stopped" '! grep -q "\"stop\"" <<<"$o"'
o=$(hw expect --id "$id" '.inventory_list' --visible --timeout-ms 10000); ck "R5 inventory visible after login" '[[ $? -eq 0 ]] && ! grep -qi error <<<"$o"'
o=$(hw auth-context --id "$id"); ck "R6 no login_required after login" 'grep -q "\"login_required\":null" <<<"$o"'

# Multi-step: add to cart, cart, checkout, fill info, continue (all reversible)
o=$(hw click --id "$id" '#add-to-cart-sauce-labs-backpack'); ck "R7 add-to-cart not treated as irreversible" '! grep -q "\"stop\"" <<<"$o"'
hw click --id "$id" '.shopping_cart_link' >/dev/null
hw expect --id "$id" '#checkout' --visible --timeout-ms 10000 >/dev/null
o=$(hw click --id "$id" '#checkout'); ck "R8 Checkout button not treated as irreversible" '! grep -q "\"stop\"" <<<"$o"'
hw expect --id "$id" '#first-name' --visible --timeout-ms 10000 >/dev/null
o=$(hw batch "[{\"cmd\":\"type\",\"id\":$id,\"selector\":\"#first-name\",\"text\":\"A\"},{\"cmd\":\"type\",\"id\":$id,\"selector\":\"#last-name\",\"text\":\"B\"},{\"cmd\":\"type\",\"id\":$id,\"selector\":\"#postal-code\",\"text\":\"02139\"},{\"cmd\":\"click\",\"id\":$id,\"selector\":\"#continue\"}]")
ck "R9 batch fills form and continues (complete:true)" 'grep -q "\"complete\":true" <<<"$o"'
hw expect --id "$id" '#finish' --visible --timeout-ms 10000 >/dev/null
o=$(hw content --id "$id" --format text --max-chars 2000); ck "R10 overview shows total" 'grep -q "Total" <<<"$o"'

# Irreversible step: Finish places the order
o=$(hw click --id "$id" '#finish'); echo "   finish (unconfirmed): $(head -2 <<<"$o" | tr '\n' ' ')"
ck "R11 Finish (place order) returns irreversible_action stop" 'grep -q "irreversible_action" <<<"$o"'
o=$(hw eval --id "$id" 'return location.pathname'); ck "R12 still on overview, order not placed" 'grep -q "checkout-step-two" <<<"$o"'

# Same click through MCP stdio (what a stock harness calls)
mcp_out=$(printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"acc","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"click\",\"arguments\":{\"id\":$id,\"selector\":\"#finish\"}}}" \
  "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"shot\",\"arguments\":{\"id\":$id,\"path\":\"$w/overview.png\"}}}" \
  | "$b/hwatu" mcp 2>&1)
ck "R13 MCP tools/list advertises show, shot, type, fill, batch, content, press" 'for t in show hide shot type fill batch content press; do grep -q "\"name\":\"$t\"" <<<"$mcp_out" || exit 1; done'
ck "R14 MCP click on Finish returns irreversible_action" 'grep -q irreversible_action <<<"$mcp_out"'
ck "R15 MCP shot (vision name) writes PNG" '[[ -s "$w/overview.png" ]]'
o=$(hw eval --id "$id" 'return location.pathname'); ck "R16 still not placed after MCP attempt" 'grep -q "checkout-step-two" <<<"$o"'

# show is the only visibility path; display-free reports it cannot show
o=$(hw show "$id")
if [[ "$(uname)" == Darwin ]]; then
  l=$(hw list --json)
  ck "R17 show makes the window visible (mode leaves headless)" '! grep -q "\"mode\":\"headless\"" <<<"$l"'
  hw hide "$id" >/dev/null
  l=$(hw list --json)
  ck "R17b hide returns it to headless" 'grep -q "\"mode\":\"headless\"" <<<"$l"'
else
  ck "R17 show answers (display-free: structured no-display error)" 'grep -q "no display" <<<"$o"'
fi
o=$(hw --focus https://example.com); ck "R18 visible open flags rejected" 'grep -q "hwatu show" <<<"$o"'
o=$(hw history foo); ck "R19 removed command history is rejected" 'grep -q "was removed" <<<"$o"'
o=$(hw how to exit vim); ck "R19b non-URL words are rejected, not searched" 'grep -q "not a URL" <<<"$o"'

# User says yes: confirmed click completes the order
o=$(hw click --id "$id" --confirm '#finish'); ck "R20 confirmed Finish performs the click" '! grep -q "\"stop\"" <<<"$o"'
o=$(hw expect --id "$id" '.complete-header' --visible --timeout-ms 10000); o2=$(hw content --id "$id" --format text --max-chars 500)
ck "R21 order complete page after confirm" 'grep -qi "thank you" <<<"$o2"'

hw quit >/dev/null
echo "pass=$pass fail=$fail"
[[ $fail -eq 0 ]]
