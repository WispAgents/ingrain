# ingrain

Ingrain learns lint rules from the corrections humans make to coding-agent output,
then enforces them where the agent edits, before commit, and in CI.

Agents repeat the same mistakes across sessions. Writing the fix into `CLAUDE.md` is prose the
agent can ignore. Ingrain turns the corrections you already made into **deterministic
[ast-grep](https://ast-grep.github.io) rules**, tests them on your history, and gives them to the
agent at edit time, together with the fix a human applied last time.

```
corrections ──► clusters ──► candidate rules ──► backtest ──► you review ──► shadow → warn → block
(git fix-ups,    (what was     (templates,         (history,      (ingrain       (hook, pre-commit, CI)
 PR suggestions,  removed)      LLM fallback)       agent code)    review)
 transcripts)
```

> **Status: v0.1, experimental. Validation failed on public history.** Over 17 public repos
> (7,443 corrections), rules learned from older corrections prevented 0.46% of later ones at 0.6%
> precision. With the default evidence gate, no rules are proposed at all. Hand-labeling found only
> 10.6% of corrections are rule-shaped. Read [docs/validation.md](docs/validation.md) first. The
> enforcement side (hooks, pre-commit, CI, about 40 ms per edit) works and is tested, and runs
> hand-written ast-grep rules just as well.

## Install

```sh
cargo install --locked --git https://github.com/WispAgents/ingrain ingrain
```

Needs `git`. `gh` (logged in) for PR review suggestions. Optional: Claude Code (`claude`) or
`ANTHROPIC_API_KEY` for rules that templates can't express.

## Quick start

```sh
cd your-repo
ingrain init                 # .corrections/, Claude Code hooks, git pre-commit hook
ingrain mine --pr-refs --github   # capture corrections
ingrain propose --explain    # cluster, write and backtest rules → .corrections/pending/
ingrain review               # accept / edit / reject, one card per rule
git commit .corrections/rules -m "ingrain rules"
```

Accepted rules start in **shadow** mode (logged only). Promote them when they prove themselves:

```sh
ingrain rules                # modes and hit counts
ingrain promote              # suggestions from the hit log; --apply to write them
ingrain mode avoid-fetch-call block
```

## Where corrections come from

| Source | Signal | Command |
|---|---|---|
| Git fix-ups | A human commit rewrites lines an agent commit added (found with `git blame`) within `window_days` | `ingrain mine` |
| PR refs | Same, on `refs/pull/*/head`, so fix-ups inside PRs survive squash merges | `ingrain mine --pr-refs` |
| Review suggestions | A ```` ```suggestion ```` comment on agent-written lines: an exact before/after | `ingrain mine --github [owner/name]` |
| Claude Code transcripts | The agent edits, you reply, the agent rewrites what it just wrote | read from `~/.claude/projects` by default |

Agent commits are recognized by trailers and authors (`Co-Authored-By: Claude`, `Generated with
Claude Code`, Codex, Cursor, Copilot agent, Devin, Jules, OpenHands…). Add your own markers in
`.corrections/config.yml`. Formatter runs and large refactors are skipped.

## How rules are learned

1. **Cluster.** Each correction is reduced to the salient tokens it *removed* (identifiers,
   keywords, `==`). Corrections sharing a removed token form a cluster. Support counts distinct
   fix events (commits, review comments), not hunks.
2. **Write.** Templates first: banned call (`fetch($$$ARGS)`, `console.log($$$ARGS)`,
   `$OBJ.x($$$ARGS)`) or banned identifier by node kind. Clusters templates can't express go to an
   LLM (`claude -p` or the Anthropic API, `claude-opus-5-5`), with backtest feedback, up to 3 tries.
3. **Backtest.** A rule is proposed only if it:
   - flags the agent's lines in at least `min_support` distinct fixes,
   - flags at most 10% of the corrected versions,
   - has precision ≥ `min_precision` (lower bound of the 90% Wilson interval) on agent code
     written after the rule became learnable, or on all earlier agent code if it fired fewer than
     5 times there.
     A hit on lines a human later corrected, for the reason the rule targets, is a true positive.
     A hit on agent lines humans kept is a false alarm. A rule with no evidence is not proposed.
4. **Review.** You decide. Rejected rules are remembered with their reason and never re-proposed,
   under any id.

`ingrain propose --explain` prints every cluster and why each candidate was accepted or rejected.

## Where checks run

All paths run the same rules and only look at **added lines**, so existing code never blocks.

| Path | Command | What happens on a hit |
|---|---|---|
| After each agent edit | `ingrain check --hook post-edit` (Claude Code `PostToolUse`) | warn: note to the agent; block: the agent must fix it, with the past human fix shown |
| End of the agent's turn | `ingrain check --hook stop` (Claude Code `Stop`) | block: the turn is sent back, up to `stop_max_attempts` times |
| Commit | `ingrain check --staged` (`.git/hooks/pre-commit`) | block: the commit fails |
| Pull request | `ingrain check --diff base...HEAD --format github` ([action](action/action.yml)) | annotations; block fails the check |

Hooks fail open: if ingrain itself errors, the agent's session continues. The pre-commit hook is a
no-op for teammates who don't have ingrain installed.

To keep code a rule flags, add a comment with a reason on the line or the line above:

```ts
// ingrain-ignore avoid-fetch-call: this endpoint streams the response body
const res = await fetch(url);
```

An ignore comment without a reason doesn't count. Overrides are logged; a rule overridden too often
is suggested for demotion.

## Rule files

Plain ast-grep YAML, so they also run with `ast-grep scan`:

```yaml
id: avoid-fetch-call
language: TypeScript
severity: warning
rule:
  pattern: fetch($$$ARGS)
message: "Avoid `fetch(...)` here: in 4 past corrections humans replaced it with `apiClient` / `get`."
metadata:
  ingrain:
    mode: shadow
    support: 4
    origin: template
    evidence: ["commit 3f2c1a9e10 src/feature0.ts: use apiClient in load0", "..."]
    backtest: { before_hits: 4, befores: 4, after_hits: 0, afters: 4, kept_hits: 0, kept: 12, head_hits: 0 }
    example: { before: "const res = await fetch(`/api/items/${id}`);", after: "const res = await apiClient.get(`/items/${id}`);" }
```

## Layout

```
.corrections/            committed
  config.yml
  rules/<id>.yml         accepted rules
  pending/<id>.yml       proposals (gitignored)
  rejected.yml           rejected proposals and reasons
.git/ingrain/            local only
  corrections.jsonl  agent_hunks.jsonl  hits.jsonl
```

## Evaluate on your own history

```sh
ingrain mine --pr-refs --github
ingrain eval --split 0.6 --json report.json
```

`eval` learns rules only from the oldest 60% of corrections, freezes them, and reports how many of
the later corrections they would have prevented and their precision on later agent code.
[`scripts/validate.sh`](scripts/validate.sh) runs this over a list of public repositories.

## Development

```sh
cargo test --all
cargo clippy --all-targets -- -D warnings
```

Crates: `core` (data model, rule files, git), `engine` (ast-grep wrapper), `learn` (capture,
cluster, synth, backtest, eval), `cli` (the `ingrain` binary). [PLAN.md](PLAN.md) is the working
plan.

License: Apache-2.0.
