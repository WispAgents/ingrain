#!/bin/sh
# Time-split validation of ingrain on public repositories.
#
#   scripts/validate.sh <workdir> owner/name [owner/name ...]
#
# For each repo: clone (if missing) into <workdir>, mine git history since $SINCE
# (with PR refs and review suggestions), then run `ingrain rules eval` and save the JSON
# report to <workdir>/reports/<owner>_<name>.json. Needs `gh` logged in.
set -u
WORK=$1; shift
SINCE=${SINCE:-2025-03-01}
MAX_COMMITS=${MAX_COMMITS:-6000}   # newest commits across HEAD and PR refs
LLM=${LLM:-none}
BIN=${INGRAIN:-ingrain}
mkdir -p "$WORK/reports"
for repo in "$@"; do
  dir="$WORK/$(echo "$repo" | tr / _)"
  report="$WORK/reports/$(basename "$dir").json"
  if [ -f "$report" ] && [ -z "${FORCE:-}" ]; then echo "== $repo (done, skipping)"; continue; fi
  [ -d "$dir" ] || git clone -q "https://github.com/$repo.git" "$dir" || continue
  echo "== $repo"
  start=$(date +%s)
  "$BIN" --repo "$dir" rules mine --pr-refs --github "$repo" --review-pages 10 --since "$SINCE" --max-commits "$MAX_COMMITS" --no-transcripts \
    > "$WORK/reports/$(basename "$dir").mine.txt" 2>&1 || echo "mine failed for $repo"
  "$BIN" --repo "$dir" rules eval --llm "$LLM" --json "$report" \
    > "$WORK/reports/$(basename "$dir").eval.txt" 2>&1 || echo "eval failed for $repo"
  echo "   $(( $(date +%s) - start ))s"
done
echo EXIT=0
