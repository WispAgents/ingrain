#!/bin/sh
# Time-split validation of ingrain on public repositories.
#
#   scripts/validate.sh <workdir> owner/name [owner/name ...]
#
# For each repo: clone (if missing) into <workdir>, mine git history since $SINCE
# (with PR refs and review suggestions), then run `ingrain eval` and save the JSON
# report to <workdir>/reports/<owner>_<name>.json. Needs `gh` logged in.
set -u
WORK=$1; shift
SINCE=${SINCE:-2025-03-01}
LLM=${LLM:-none}
BIN=${INGRAIN:-ingrain}
mkdir -p "$WORK/reports"
for repo in "$@"; do
  dir="$WORK/$(echo "$repo" | tr / _)"
  [ -d "$dir" ] || git clone -q "https://github.com/$repo.git" "$dir" || continue
  echo "== $repo"
  start=$(date +%s)
  "$BIN" --repo "$dir" mine --pr-refs --github "$repo" --review-pages 10 --since "$SINCE" --no-transcripts \
    > "$WORK/reports/$(basename "$dir").mine.txt" 2>&1 || echo "mine failed for $repo"
  "$BIN" --repo "$dir" eval --llm "$LLM" --json "$WORK/reports/$(basename "$dir").json" \
    > "$WORK/reports/$(basename "$dir").eval.txt" 2>&1 || echo "eval failed for $repo"
  echo "   $(( $(date +%s) - start ))s"
done
echo EXIT=0
