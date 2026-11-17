# Learned rules

| Source | Signal |
|---|---|
| Git fix-ups (`ingrain rules mine`) | A human commit rewrites lines an agent commit added, within `window_days` |
| PR refs (`--pr-refs`) | Same, on `refs/pull/*/head`, so fix-ups inside PRs survive squash merges |
| Review suggestions (`--github`) | A ```` ```suggestion ```` comment on agent-written lines |
| Claude Code transcripts | The agent edits, you reply, the agent rewrites what it just wrote |

`ingrain rules propose` clusters corrections by what they removed, writes rules (templates first, then a
model), and keeps a rule only if it catches the past mistakes, spares the fixed versions, and
isn't contradicted by agent code humans kept (the fix a rule came from never counts as its own
evidence). One correction is enough to propose a rule, even with no other agent code to test on, so a first
run has something to review. Each rule says whether it's from a single fix or repeated fixes.
Single-fix rules that look like one-offs are held back and listed with the reason: the code is
still used 5+ times today, the name is generic (`r3`, `data`, `.append(`), or every fix was in
test code; `--all` keeps them. Pass
`--min-corrections 3` (or set `min_support` in `.corrections/config.yml`) to wait for a mistake to
repeat. Accepted rules start in shadow mode; `ingrain rules promote` moves them to warn and block
from their hit log. Shadow hits are invisible to the agent, so leaving shadow needs hits a human
later corrected (at least 3, and half of all hits, per the mined corrections); `ingrain rules mode
<id> warn` moves one by hand. `ingrain rules eval` measures the whole loop on your history.

Accepted rules go to `.corrections/rules/`. Add `--local` or `--global` (`ingrain --global rules
propose`) to keep them in a [personal store](personal-stores.md) instead; checks and hooks run the
rules from every store.
