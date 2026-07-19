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

cd "$workdir/aur"
git add PKGBUILD .SRCINFO
if git diff --cached --quiet; then
  echo "AUR already up to date."
  exit 0
fi
pkgver=$(grep -oP '^pkgver=\K.*' PKGBUILD)
git commit -m "update to $pkgver"
git push origin HEAD:master
echo "published $pkgname $pkgver to AUR: https://aur.archlinux.org/packages/$pkgname"
