#!/usr/bin/env bash
# publish.sh — upload demo assets to GitHub and verify they render.
#
#   1. Uploads <base>.release.mp4 + <base>.readme.webp to the
#      `readme-assets` release (created if missing).
#   2. Rewrites the README hero image to the jcode pattern:
#      webp autoplaying inline, click-through to the mp4.
#   3. VERIFIES the result on github.com: asset URLs return 200 and
#      the rendered README actually references them. If hwatu is
#      running, also renders the GitHub page and screenshots it.
#
# Usage: scripts/demo/record/publish.sh /tmp/out/demo-raw
#        (pass the base path; .release.mp4/.readme.webp appended)
set -euo pipefail

BASE="${1:?usage: publish.sh <base path, no extension>}"
MP4="$BASE.release.mp4"
WEBP="$BASE.readme.webp"
REPO="hongnoul/hwatu"
TAG="readme-assets"
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"

[ -f "$MP4" ] && [ -f "$WEBP" ] || { echo "missing $MP4 / $WEBP"; exit 1; }

# ---- 1. upload ---------------------------------------------------
if ! gh release view "$TAG" -R "$REPO" >/dev/null 2>&1; then
  gh release create "$TAG" -R "$REPO" --title "README assets" \
    --notes "Binary assets referenced by README.md. Not a software release." \
    --prerelease
fi
gh release upload "$TAG" -R "$REPO" --clobber \
  "$MP4#hwatu-demo.mp4" "$WEBP#hwatu-demo.webp" 2>/dev/null || \
  gh release upload "$TAG" -R "$REPO" --clobber "$MP4" "$WEBP"

MP4_URL="https://github.com/$REPO/releases/download/$TAG/$(basename "$MP4")"
WEBP_URL="https://github.com/$REPO/releases/download/$TAG/$(basename "$WEBP")"

# ---- 2. README hero ----------------------------------------------
HERO="<a href=\"$MP4_URL\"><img src=\"$WEBP_URL\" alt=\"hwatu demo: an agent converging a page to pixel-parity, score on screen\" width=\"800\"></a>"
cd "$REPO_DIR"
if grep -q "spawn-demo.svg" README.md; then
  # Replace the old hero line wholesale.
  sed -i "s|!\[hwatu spawning windows.*spawn-demo.svg)|$HERO|" README.md
elif grep -q "hwatu-demo.webp" README.md; then
  echo "README already references the demo webp (URLs are stable); no edit needed."
else
  echo "WARNING: no hero anchor found in README.md; insert manually:"; echo "$HERO"
fi
if ! git diff --quiet README.md; then
  git add README.md
  git commit -m "readme: replace spawn svg with convergence demo video"
  git push origin main
fi

# ---- 3. verify on github.com -------------------------------------
echo; echo "=== verification ==="
fail=0
for url in "$MP4_URL" "$WEBP_URL"; do
  code=$(curl -sL -o /dev/null -w "%{http_code}" "$url")
  echo "  $code  $url"
  [ "$code" = 200 ] || fail=1
done

# Check both GitHub's raw branch and its rendered README API. The
# repository homepage HTML is edge-cached and can briefly serve the
# preceding commit immediately after a push.
raw=$(curl -fsSL -H 'Cache-Control: no-cache' \
  "https://raw.githubusercontent.com/$REPO/main/README.md?cb=$(date +%s%N)")
rendered=$(gh api -H 'Accept: application/vnd.github.html+json' \
  "repos/$REPO/readme" 2>/dev/null || true)
echo "$raw" | grep -q "$(basename "$WEBP")" \
  && echo "  ok   raw README references demo webp" \
  || { echo "  FAIL raw README missing demo webp"; fail=1; }
echo "$rendered" | grep -q "$(basename "$WEBP")" \
  && echo "  ok   rendered README references demo webp" \
  || { echo "  FAIL rendered README missing demo webp"; fail=1; }

# Bonus: dogfood. If a hwatu daemon is reachable, render the repo page
# and screenshot it, so a human (or agent) can eyeball the hero.
if hwatu ping >/dev/null 2>&1; then
  id=$(hwatu --headless --json "https://github.com/$REPO" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  hwatu wait-load --id "$id" --timeout-ms 20000 >/dev/null
  probe=$(hwatu eval --id "$id" "const i=[...document.images].find(i=>i.src.includes('$(basename "$WEBP")')); if(!i)return {found:false}; i.scrollIntoView({block:'center'}); return {found:true,complete:i.complete,width:i.naturalWidth,height:i.naturalHeight,link:i.closest('a')?.href}" 2>/dev/null || true)
  echo "$probe" | grep -q '"found":true' \
    && echo "  ok   live GitHub DOM loaded demo image" \
    || { echo "  FAIL live GitHub DOM missing demo image"; fail=1; }
  hwatu shot --id "$id" /tmp/hwatu-readme-live.png >/dev/null
  hwatu close "$id" >/dev/null
  echo "  shot /tmp/hwatu-readme-live.png (live github render)"
fi

[ "$fail" = 0 ] && echo "PUBLISH VERIFIED" || { echo "PUBLISH FAILED VERIFICATION"; exit 1; }
