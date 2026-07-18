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
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) say "note: $INSTALL_DIR is not on your PATH" ;;
esac
say "try: hwatu example.com"
