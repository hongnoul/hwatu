#!/usr/bin/env bash
# Mirror the stripe.com landing page into a stable local fixture.
# The mirror is gitignored (it's Stripe's content); rerun to refresh.
set -euo pipefail
cd "$(dirname "$0")"
rm -rf reference
mkdir -p reference
wget --quiet --page-requisites --convert-links --span-hosts \
  --no-parent --adjust-extension --restrict-file-names=windows \
  --directory-prefix=reference --no-host-directories \
  --user-agent="Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15" \
  --timeout=20 --tries=2 \
  https://stripe.com/ || true
if [ ! -f reference/index.html ]; then
  echo "mirror failed: no reference/index.html" >&2
  exit 1
fi
echo "mirrored to $(pwd)/reference ($(du -sh reference | cut -f1))"
