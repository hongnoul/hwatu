#!/usr/bin/env bash
# post-hn.sh: submit a Show HN post to Hacker News from the command line.
# HN has no write API: this drives the same web forms a browser would,
# with your credentials. One post, no vote manipulation, fully within ToS.
#
# Usage:
#   HN_USER=you HN_PASS=... scripts/post-hn.sh "Title here" "https://github.com/you/repo"
#
# Prints the new item URL on success. Post your prepared first comment
# there immediately (see .astrophile/drafts/show-hn.md).
set -euo pipefail

title=${1:?usage: post-hn.sh TITLE URL}
url=${2:?usage: post-hn.sh TITLE URL}
: "${HN_USER:?set HN_USER}" "${HN_PASS:?set HN_PASS}"

jar=$(mktemp)
trap 'rm -f "$jar"' EXIT
ua="Mozilla/5.0 (X11; Linux x86_64) hwatu-launch-script"

# 1. Log in.
curl -sS -c "$jar" -A "$ua" -o /dev/null \
  --data-urlencode "acct=$HN_USER" --data-urlencode "pw=$HN_PASS" \
  --data-urlencode "goto=news" https://news.ycombinator.com/login
if ! curl -sS -b "$jar" -A "$ua" https://news.ycombinator.com/submit | grep -q 'name="fnid"'; then
  echo "error: login failed (bad credentials, or HN is rate-limiting this IP)" >&2
  exit 1
fi

# 2. Fetch the submit form to get the one-time fnid token.
fnid=$(curl -sS -b "$jar" -A "$ua" https://news.ycombinator.com/submit \
  | grep -oP 'name="fnid" value="\K[^"]+')

# 3. Submit.
curl -sS -b "$jar" -A "$ua" -o /dev/null -w '%{http_code}' \
  --data-urlencode "fnid=$fnid" --data-urlencode "fnop=submit-page" \
  --data-urlencode "title=$title" --data-urlencode "url=$url" \
  --data-urlencode "text=" https://news.ycombinator.com/r | grep -qE '^(200|302)'

# 4. Find the new item id from your submissions page.
item=$(curl -sS -b "$jar" -A "$ua" "https://news.ycombinator.com/submitted?id=$HN_USER" \
  | grep -oP 'item\?id=\K[0-9]+' | head -1)
echo "submitted: https://news.ycombinator.com/item?id=$item"
echo "NOW: post your first comment there (drafts/show-hn.md) and answer everything for 3h."
