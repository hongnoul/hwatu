#!/usr/bin/env bash
# hwatu installer: downloads the latest release binaries to ~/.local/bin.
#   curl -fsSL https://raw.githubusercontent.com/hongnoul/hwatu/main/scripts/install.sh | bash
set -euo pipefail

REPO="hongnoul/hwatu"
INSTALL_DIR="${HWATU_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf '\033[1;35mhwatu:\033[0m %s\n' "$*"; }
die() { printf '\033[1;31mhwatu:\033[0m %s\n' "$*" >&2; exit 1; }

# --- platform ---------------------------------------------------------------
os=$(uname -s)
[ "$os" = "Linux" ] || die "prebuilt binaries are Linux-only for now (got $os). Build from source: cargo build --release"

case "$(uname -m)" in
  x86_64)          arch=x86_64 ;;
  aarch64 | arm64) arch=aarch64 ;;
  *) die "unsupported architecture $(uname -m). Build from source: cargo build --release" ;;
esac
artifact="hwatu-linux-${arch}"

# --- runtime dependency check ------------------------------------------------
has_webkit() {
  # ldconfig may live in /usr/sbin, which is not always on PATH in
  # non-login shells (curl | bash). Note: grep -q would SIGPIPE ldconfig
  # under `set -o pipefail`, so read the full output instead.
  for lc in ldconfig /sbin/ldconfig /usr/sbin/ldconfig; do
    if command -v "$lc" >/dev/null 2>&1; then
      "$lc" -p 2>/dev/null | grep libwebkitgtk-6.0 >/dev/null && return 0
      break
    fi
  done
  compgen -G "/usr/lib/libwebkitgtk-6.0.so*" >/dev/null 2>&1 \
    || compgen -G "/usr/lib/*/libwebkitgtk-6.0.so*" >/dev/null 2>&1
}

if ! has_webkit; then
  say "runtime dependency libwebkitgtk-6.0 not found."
  if command -v pacman >/dev/null 2>&1; then
    say "install it with: sudo pacman -S webkitgtk-6.0"
  elif command -v apt-get >/dev/null 2>&1; then
    say "install it with: sudo apt install libwebkitgtk-6.0-4"
  elif command -v dnf >/dev/null 2>&1; then
    say "install it with: sudo dnf install webkitgtk6.0"
  else
    say "install webkitgtk-6.0 with your package manager."
  fi
  say "continuing install; hwatud will need it to run."
fi

# --- download ----------------------------------------------------------------
# Use the /releases/latest/download/ redirect instead of api.github.com:
# the API is rate-limited per IP (60/hr unauthenticated) and fails on
# shared networks, while the redirect endpoint is not rate-limited.
url="https://github.com/${REPO}/releases/latest/download/${artifact}.tar.gz"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

say "downloading ${artifact} (latest release)..."
final_url=$(curl -fsSL -o "$tmp/pkg.tar.gz" -w '%{url_effective}' "$url") \
  || die "download failed: $url"

# Recover the tag from the resolved asset URL for the install message.
# The redirect may land on release-assets.githubusercontent.com, so fall
# back gracefully when no tag segment is present.
tag=$(printf '%s\n' "$final_url" | grep -o '/releases/download/[^/]*/' | cut -d/ -f4 || true)
[ -n "$tag" ] || tag="latest"

if curl -fsSL "https://github.com/${REPO}/releases/latest/download/${artifact}.tar.gz.sha256" \
    -o "$tmp/pkg.sha256" 2>/dev/null; then
  (cd "$tmp" && sed "s|${artifact}.tar.gz|pkg.tar.gz|" pkg.sha256 | sha256sum -c --quiet -) \
    || die "checksum verification failed"
fi

tar xzf "$tmp/pkg.tar.gz" -C "$tmp"
mkdir -p "$INSTALL_DIR"
install -m755 "$tmp/${artifact}/hwatu" "$tmp/${artifact}/hwatud" "$INSTALL_DIR/"

say "installed hwatu + hwatud ${tag} to ${INSTALL_DIR}"

# --- search engine ------------------------------------------------------------
# Input in the URL bar / CLI that isn't a URL becomes a web search.
# Pick the engine here (like `npm init` prompts); it lands in
# ~/.config/hwatu/search.conf and is editable any time. Keep the list
# in sync with ENGINES in crates/hwatud/src/search.rs.
config_dir="${XDG_CONFIG_HOME:-$HOME/.config}/hwatu"
search_conf="$config_dir/search.conf"

engine_names=(duckduckgo google bing brave startpage kagi ecosia)

pick_engine() {
  # Preseeded (HWATU_SEARCH_ENGINE=google curl ... | bash)?
  if [ -n "${HWATU_SEARCH_ENGINE:-}" ]; then
    printf '%s' "$HWATU_SEARCH_ENGINE"
    return
  fi
  # curl | bash: stdin is the script, so prompt on the terminal.
  if [ ! -e /dev/tty ] || ! : </dev/tty 2>/dev/null; then
    printf 'duckduckgo'
    return
  fi
  say "choose a search engine (used when URL-bar input isn't a URL):"
  local i=1
  for name in "${engine_names[@]}"; do
    if [ "$name" = duckduckgo ]; then
      printf '  %d) %s (default)\n' "$i" "$name"
    else
      printf '  %d) %s\n' "$i" "$name"
    fi
    i=$((i + 1))
  done
  printf '\033[1;35mhwatu:\033[0m pick [1-%d] or enter a URL template with %%s: ' "${#engine_names[@]}"
  local choice
  IFS= read -r choice </dev/tty || choice=""
  case "$choice" in
    "") printf 'duckduckgo' ;;
    [1-9] | [1-9][0-9])
      if [ "$choice" -ge 1 ] && [ "$choice" -le "${#engine_names[@]}" ]; then
        printf '%s' "${engine_names[$((choice - 1))]}"
      else
        say "no such option, using duckduckgo" >&2
        printf 'duckduckgo'
      fi
      ;;
    *) printf '%s' "$choice" ;;  # engine name or custom %s template
  esac
}

if [ -s "$search_conf" ]; then
  say "keeping existing search engine: $(grep -m1 -v '^\s*#' "$search_conf" || true) ($search_conf)"
else
  engine=$(pick_engine)
  mkdir -p "$config_dir"
  printf '# search engine: duckduckgo | google | bing | brave | startpage | kagi | ecosia\n# or a URL template containing %%s, e.g. https://example.com/search?q=%%s\n%s\n' \
    "$engine" >"$search_conf"
  say "search engine set to ${engine} (edit ${search_conf} to change)"
fi

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) say "note: $INSTALL_DIR is not on your PATH" ;;
esac

# --- WM rule: background windows must not steal focus --------------------------
# `hwatu --background` (the default when a coding agent drives hwatu)
# maps a window without requesting activation, but most tiling
# compositors focus every new window anyway. One rule matching
# app-id "hwatu-background" makes the no-focus-steal guarantee hard.
# Installed by default; preseed HWATU_WM_RULE=no to skip.
ask_yes() {
  # $1: prompt. Defaults to yes; curl|bash without a tty also means yes.
  # (stderr redirect first: redirections apply left to right, and the
  # /dev/tty open failure itself is the message to silence.)
  if [ ! -e /dev/tty ] || ! : 2>/dev/null </dev/tty; then
    return 0
  fi
  printf '\033[1;35mhwatu:\033[0m %s [Y/n]: ' "$1"
  local answer
  IFS= read -r answer </dev/tty || answer=""
  case "$answer" in
    [nN]*) return 1 ;;
    *) return 0 ;;
  esac
}

install_wm_rule() {
  [ "${HWATU_WM_RULE:-yes}" = "no" ] && return 0

  local niri_conf="${XDG_CONFIG_HOME:-$HOME/.config}/niri/config.kdl"
  local hypr_conf="${XDG_CONFIG_HOME:-$HOME/.config}/hypr/hyprland.conf"
  local sway_conf="${XDG_CONFIG_HOME:-$HOME/.config}/sway/config"

  if [ -f "$niri_conf" ]; then
    grep -q 'app-id="hwatu-background"' "$niri_conf" && return 0
    ask_yes "add a niri rule so hwatu agent/background windows never steal focus?" || return 0
    printf '\n// hwatu --background: agent-verification windows must not steal focus.\nwindow-rule {\n    match app-id="hwatu-background"\n    open-focused false\n}\n' >>"$niri_conf"
    say "added window rule to $niri_conf (niri reloads config automatically)"
  elif [ -f "$hypr_conf" ]; then
    grep -q 'hwatu-background' "$hypr_conf" && return 0
    ask_yes "add a Hyprland rule so hwatu agent/background windows never steal focus?" || return 0
    printf '\n# hwatu --background: agent-verification windows must not steal focus.\nwindowrule = noinitialfocus, class:^(hwatu-background)$\n' >>"$hypr_conf"
    say "added window rule to $hypr_conf (hyprctl reload to apply)"
  elif [ -f "$sway_conf" ]; then
    grep -q 'hwatu-background' "$sway_conf" && return 0
    ask_yes "add a sway rule so hwatu agent/background windows never steal focus?" || return 0
    printf '\n# hwatu --background: agent-verification windows must not steal focus.\nno_focus [app_id="hwatu-background"]\n' >>"$sway_conf"
    say "added window rule to $sway_conf (swaymsg reload to apply)"
  else
    say "note: to keep hwatu --background windows from stealing focus, add a WM rule"
    say "      matching app-id \"hwatu-background\" (see README, e.g. niri open-focused false)"
  fi
}
install_wm_rule

say "next: hwatu setup"
say "      detects agent workflows and previews connections without changing config"
