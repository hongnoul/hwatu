#!/usr/bin/env bash
# Mirror the stripe.com landing page into a stable local fixture.
# The mirror is gitignored (it's Stripe's content); rerun to refresh.
#
# NOTE: an existing reference/ checkout is *frozen ground truth* for
# the perfect-clone gate — this script refuses to clobber it unless
# FORCE=1 is set. Delete or FORCE=1 only when you intend to re-pin.
#
# Completeness: `wget --page-requisites` only fetches assets the HTML
# references directly. Stripe's Next.js build lazy-loads more JS
# chunks and CSS whose names live inside the webpack runtime and
# build manifest, and index.html's assetPrefix points those loads at
# live b.stripecdn.com. This script therefore:
#   1. wget-mirrors the page as before,
#   2. crawls every mirrored HTML/JS file for `static/chunks/*.js` and
#      `static/css/*.css` references and fetches any missing ones from
#      b.stripecdn.com (to a fixpoint: new chunks can name more chunks),
#   3. rewrites absolute https://b.stripecdn.com/mkt-ssr-statics/assets
#      URLs to host-relative /mkt-ssr-statics/assets in index.html, so
#      runtime chunk/CSS loads resolve against the local server.
#
# What is NOT mirrored (and cannot be): analytics beacons to
# q.stripe.com / r.stripe.com are POST endpoints, not assets. They do
# not affect rendering. Run the gate with network blocked or those
# hosts unreachable (e.g. `unshare -n`, a deny-by-default firewall, or
# /etc/hosts null entries) so the reference is provably offline and
# beacon timeouts cannot perturb timing.
set -euo pipefail
cd "$(dirname "$0")"

CDN="https://b.stripecdn.com/mkt-ssr-statics/assets"
UA="Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15"

if [ -d reference ] && [ "${FORCE:-0}" != 1 ]; then
  echo "reference/ exists (frozen ground truth); set FORCE=1 to re-fetch" >&2
  exit 1
fi
rm -rf reference
mkdir -p reference
wget --quiet --page-requisites --convert-links --span-hosts \
  --no-parent --adjust-extension --restrict-file-names=windows \
  --directory-prefix=reference --no-host-directories \
  --user-agent="$UA" \
  --timeout=20 --tries=2 \
  https://stripe.com/ || true
if [ ! -f reference/index.html ]; then
  echo "mirror failed: no reference/index.html" >&2
  exit 1
fi

# ---- lazy chunk/CSS fixpoint crawl ---------------------------------
# Referenced paths look like static/chunks/<name>.js (incl. pages/…
# and workers) or static/css/<hash>.css, relative to …/assets/_next/.
NEXT_DIR="reference/mkt-ssr-statics/assets/_next"
mkdir -p "$NEXT_DIR"
fetched_total=0
for round in 1 2 3 4 5; do
  refs=$(grep -rohE '(static/chunks/[A-Za-z0-9._/-]+\.js|static/css/[A-Za-z0-9._-]+\.css)' \
           reference/index.html "$NEXT_DIR" 2>/dev/null | sort -u)
  fetched=0
  while IFS= read -r p; do
    [ -n "$p" ] || continue
    [ -f "$NEXT_DIR/$p" ] && continue
    mkdir -p "$NEXT_DIR/$(dirname "$p")"
    if wget --quiet --user-agent="$UA" --timeout=20 --tries=2 \
         -O "$NEXT_DIR/$p" "$CDN/_next/$p"; then
      fetched=$((fetched + 1))
    else
      rm -f "$NEXT_DIR/$p"
      echo "warn: could not fetch $p" >&2
    fi
  done <<<"$refs"
  fetched_total=$((fetched_total + fetched))
  echo "round $round: fetched $fetched chunk/css assets" >&2
  [ "$fetched" = 0 ] && break
done

# ---- make runtime loads resolve locally ----------------------------
# index.html carries the CDN prefix twice: in <link>/<script> tags and
# in __NEXT_DATA__.assetPrefix (used for lazy page/chunk loads). The
# webpack publicPath inside the runtime chunk is already host-relative
# (/mkt-ssr-statics/assets/_next/), so rewriting index.html suffices.
sed -i "s|https://b\.stripecdn\.com/mkt-ssr-statics/assets|/mkt-ssr-statics/assets|g" \
  reference/index.html

remaining=$(grep -roh 'https://b\.stripecdn\.com/mkt-ssr-statics' reference --include='*.html' | wc -l)
[ "$remaining" = 0 ] || echo "warn: $remaining live-CDN asset refs remain in HTML" >&2

echo "mirrored to $(pwd)/reference ($(du -sh reference | cut -f1), +$fetched_total lazy assets)"
echo "serve with: python3 -m http.server 8321 --directory reference"
echo "run offline: block b.stripecdn.com, q.stripe.com, r.stripe.com (or all egress)"
