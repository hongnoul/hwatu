#!/usr/bin/env bash
# aur-publish.sh: publish/update the hwatu AUR package from packaging/PKGBUILD.
# Idempotent: run it after every release. Bumps require packaging/PKGBUILD to
# already have the new pkgver + sha256 (see scripts/release checklist).
#
# One-time prerequisite (cannot be automated, human account creation):
#   1. Create an account at https://aur.archlinux.org/register
#   2. Paste ~/.ssh/id_ed25519.pub into "My Account" -> SSH Public Key
# After that, this script does everything.
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd)
pkgname=hwatu
workdir=$(mktemp -d)
trap 'rm -rf "$workdir"' EXIT

# Sanity: PKGBUILD builds and sources verify before we push anything.
cp "$repo_root/packaging/PKGBUILD" "$workdir/"
( cd "$workdir" && makepkg --verifysource )

# Clone (or create) the AUR repo.
if ! git clone "ssh://aur@aur.archlinux.org/$pkgname.git" "$workdir/aur" 2>/dev/null; then
  echo "error: cannot reach AUR over SSH." >&2
  echo "Did you add your SSH key at https://aur.archlinux.org -> My Account?" >&2
  exit 1
fi

cp "$repo_root/packaging/PKGBUILD" "$workdir/aur/PKGBUILD"
( cd "$workdir/aur" && makepkg --printsrcinfo > .SRCINFO )

# The staged packaging/aur/.SRCINFO must match what makepkg regenerates
# from the canonical PKGBUILD. Fail loudly here (before any push) rather
# than publishing a PKGBUILD/.SRCINFO pair that trips the sync guard in CI.
if ! diff -u "$repo_root/packaging/aur/.SRCINFO" "$workdir/aur/.SRCINFO" >&2; then
  echo "error: regenerated .SRCINFO differs from packaging/aur/.SRCINFO." >&2
  echo "Run: cp packaging/PKGBUILD packaging/aur/PKGBUILD && (cd packaging/aur && makepkg --printsrcinfo > .SRCINFO)" >&2
  exit 1
fi

# Pre-flight: refuse to no-op or downgrade against the live AUR version.
live_ver=$(curl -s "https://aur.archlinux.org/rpc/v5/info?arg[]=$pkgname" \
  | python3 -c 'import json,sys; r=json.load(sys.stdin)["results"]; print(r[0]["Version"] if r else "none")' 2>/dev/null || echo unknown)
echo "live AUR version: $live_ver"
pkgver=$(sed -n 's/^pkgver=\(.*\)$/\1/p' "$workdir/aur/PKGBUILD" | head -1)
pkgrel=$(sed -n 's/^pkgrel=\(.*\)$/\1/p' "$workdir/aur/PKGBUILD" | head -1)
if [ "$live_ver" = "$pkgver-$pkgrel" ]; then
  echo "AUR already at $live_ver; nothing to push."
  exit 0
fi

cd "$workdir/aur"
git add PKGBUILD .SRCINFO
if git diff --cached --quiet; then
  echo "AUR already up to date."
  exit 0
fi
git commit -m "update to $pkgver"
git push origin HEAD:master
echo "published $pkgname $pkgver to AUR: https://aur.archlinux.org/packages/$pkgname"
