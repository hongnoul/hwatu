#!/usr/bin/env bash
# post-reddit.sh: submit a post to a subreddit via the official Reddit API.
#
# One-time setup (2 minutes, human):
#   1. https://www.reddit.com/prefs/apps -> create app -> type "script"
#   2. Note the client id (under the app name) and secret.
# Then:
#   export REDDIT_CLIENT_ID=... REDDIT_CLIENT_SECRET=...
#   export REDDIT_USER=... REDDIT_PASS=...
#
# Usage:
#   scripts/post-reddit.sh SUBREDDIT "Title" URL            # link post
#   scripts/post-reddit.sh SUBREDDIT "Title" --text "body"  # text post
set -euo pipefail

sub=${1:?usage: post-reddit.sh SUBREDDIT TITLE URL|--text BODY}
title=${2:?missing title}
: "${REDDIT_CLIENT_ID:?}" "${REDDIT_CLIENT_SECRET:?}" "${REDDIT_USER:?}" "${REDDIT_PASS:?}"
ua="hwatu-launch:v1 (by /u/$REDDIT_USER)"

token=$(curl -sS -A "$ua" -u "$REDDIT_CLIENT_ID:$REDDIT_CLIENT_SECRET" \
  --data-urlencode grant_type=password \
  --data-urlencode "username=$REDDIT_USER" --data-urlencode "password=$REDDIT_PASS" \
  https://www.reddit.com/api/v1/access_token | python3 -c 'import json,sys; print(json.load(sys.stdin)["access_token"])')

if [ "${3:-}" = "--text" ]; then
  kind=self; extra=(--data-urlencode "text=${4:?missing body}")
else
  kind=link; extra=(--data-urlencode "url=${3:?missing url}")
fi

resp=$(curl -sS -A "$ua" -H "Authorization: bearer $token" \
  --data-urlencode "sr=$sub" --data-urlencode "title=$title" \
  --data-urlencode "kind=$kind" --data-urlencode api_type=json \
  "${extra[@]}" https://oauth.reddit.com/api/submit)

echo "$resp" | python3 -c '
import json, sys
d = json.load(sys.stdin)["json"]
if d["errors"]:
    print("error:", d["errors"], file=sys.stderr); sys.exit(1)
print("posted:", d["data"]["url"])'
