#!/usr/bin/env bash
# Generate release notes for a tag: grouped commit subjects since the
# previous tag, ending with a compare link.
set -euo pipefail

tag="${1:?usage: generate_release_notes.sh <tag>}"
repo_url="https://github.com/hongnoul/hwatu"

prev=$(git describe --tags --abbrev=0 "${tag}^" 2>/dev/null || true)

echo "## ${tag}"
echo
if [ -n "$prev" ]; then
  range="${prev}..${tag}"
else
  range="${tag}"
fi

git log --no-merges --pretty='- %s' "$range" 2>/dev/null || git log --no-merges --pretty='- %s'
echo
if [ -n "$prev" ]; then
  echo "**Full changelog**: ${repo_url}/compare/${prev}...${tag}"
else
  echo "**Full changelog**: ${repo_url}/commits/${tag}"
fi
