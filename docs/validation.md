# Validation on public repositories

**Verdict: on public git history, ingrain does not earn its keep.** Repeated, rule-shaped human
corrections of agent code are rare. The rules the loop learns from history either don't generalize
(0.46% of later corrections prevented, 0.6% precision) or, with an evidence-based gate, aren't
learned at all (0 rules in 17 repos). Both pre-registered bars were missed by a wide margin. Per the
stop rules set before the build, this is a **stop** for "learn rules from git history". The
enforcement side works: hooks, pre-commit, CI, about 40 ms per edit.

Date: 2026-11-15. Binary: this branch. Reproduce with `scripts/validate.sh` and `scripts/summarize.py`.

## Setup

- **17 public repos** with heavy coding-agent use (Claude Code, Codex, Cursor, Copilot agent, Devin),
  picked by counting agent trailers and bot authors in their last 500 commits:
  claude-agent-sdk-python, datasette, PyAirbyte, airbyte-python-cdk, sentry-javascript, logfire,
  livekit/agents, fastmcp, MCP typescript-sdk, stagehand, vscode-python-environments, spec-kit, E2B,
  TanStack/router, posthog-js, typescript-go, langfuse.
- **Mining:** commits since 2025-03-01, newest 6,000 across `HEAD` and `refs/pull/*/head` (so
  fix-ups inside PRs survive squash merges). Up to 1,000 review comments per repo, newest first.
  Formatter runs and refactors (>20 files or >400 lines) skipped.
- **Data:** 7,443 corrections (a human commit rewrote lines an agent commit added, within 14 days, by
  `git blame`) and 370,278 agent hunks. 15,909 review comments yielded only **57** ```` ```suggestion ````
  blocks on agent-written lines.

## Stage 1: are corrections patterns at all?

Bar: ≥20% of corrections are patterns, and ≥30% repeat 3+ times. **Missed.**

An independent reviewer (a subagent that didn't see the clustering) hand-labeled 180 random
corrections from 3 repos. Labels are in [data/stage1-labels.jsonl](data/stage1-labels.jsonl).

| label | share | what it means |
|---|---:|---|
| evolution | 49% (89) | not a mistake: feature or requirement change, rename the agent couldn't know, doc and test text |
| bugfix | 24% (44) | logic fix specific to the situation |
| **pattern** | **10.6% (19)** | a static rule on the agent's code would flag it |
| taste | 8% (15) | simplify, rename, readability |
| structural | 7% (13) | needs repo-wide knowledge (duplicate helper, wrong layer) |

About half of the 19 patterns are already caught by stock tools: unused import, import order,
`Optional[X]`, stray `noqa`, mypy generics. The repo-specific ones are about 5%: a renamed API
(`ClaudeCodeOptions` → `ClaudeAgentOptions`), MCP `Field(default=…)`, a test-marker convention, and
stale model ids.

Token clustering says 25% of corrections sit in a cluster of 3+ fixes. Those clusters are generic
tokens (`if`, `self`, `error`, `str`), not mistakes. The hand labels are the real measure.

## Stage 2: would learned rules have prevented later corrections?

Bar: median per repo ≥10–15% of post-cutoff corrections prevented, ≥90% precision, <50 ms per file.
**Missed on both quality bars.**

Method (`ingrain rules eval`): sort corrections by time and learn only from the oldest 60%. Freeze the
rules. Then on the newest 40%:
- **prevented:** the rule flags the agent's lines and not the human fix.
- **precision:** hits on later agent code. True if a human later corrected that line *and* the rule
  also flags that correction; false if humans kept the code.

**Permissive gate** (precision ≥50% point estimate; rules with no hits accepted):

| | value |
|---|---|
| rules learned | 14 across 17 repos (9 repos: none) |
| later corrections prevented | 14 / 3,056 (**0.46%**); median per repo **0%**; best repo 3.1% |
| precision on later agent code | 26 true / 3,998 false (**0.6%**) |

Per-repo table: [data/eval-permissive-gate.md](data/eval-permissive-gate.md). The rules that did get
learned ban ordinary identifiers that a rename happened to touch three times: `error` (1,183 false
hits), `logfire` (630), `status` (499), `.calls` (41). They passed training because they had almost no
hits to judge in the training window.

**Strict gate, the shipped default** (lower bound of the 90% Wilson interval on precision ≥50%,
judged on agent code written after the rule became learnable if there are ≥5 hits there, otherwise
on all earlier agent code; a rule that never fired is not proposed): **0 rules in all 17 repos.** See
[data/eval-strict-gate.md](data/eval-strict-gate.md). Proposing nothing beats proposing noise.

**LLM path:** on sentry-javascript, sending the 10 clusters that templates couldn't express to
`claude-opus-5-5` added one rule. It prevented 0 more corrections. Model-written rules overfit to
literal types and names in the examples.

**Latency** (the one bar met): scanning a hunk takes 0.6 ms at p95 in the worst repo. The
post-edit hook takes about 40 ms on a 3,000-line file, measured with unrelated processes loading
the machine. Most of that is 2 git spawns of about 18 ms each.

## What moved the numbers during the build (honest list)

Each change came from reading real data, and none changed the verdict:

1. Skip formatter-only commits and cosmetic hunks (`black` reflows showed up as corrections).
2. Mine `refs/pull/*/head` (claude-agent-sdk-python went from 50 to 201 corrections).
3. Precision window starts when a rule becomes learnable (the 3rd fix), so code written before a
   rename isn't counted against the rename rule.
4. True positives must be about what the human fixed, not any edit on the same line (from code
   review).
5. Wilson lower bound plus required evidence. This turned 14 bad rules into 0 rules.

## Why, and what this does *not* rule out

- **Git history is mostly evolution.** 73% of "corrections" are requirement changes or one-off bug
  fixes. A human rewriting agent code is usually changing *what* the code should do, not catching a
  repeated *how* mistake.
- **Repeated mistakes get fixed once, upstream.** When a mistake repeats (a renamed API), the team
  fixes the docs, the types or `CLAUDE.md`, and the repetition stops. That leaves 2–3 events, not a
  stream.
- **Stock linters already cover the syntactic half.** Running the repo's own ruff, eslint or mypy at
  edit time catches it, and that needs no learning.

Not tested, because public data can't measure them:

- **In-session corrections** ("no, use X") in Claude Code transcripts. Capture is implemented and
  unit-tested (`capture/transcript.rs`), but the only transcripts available are private. This is the
  one signal where repetition could plausibly be higher, since nobody commits the agent's first
  attempt.
- **Private repos with strict reviewers** who leave many suggestion blocks. 57 suggestions in 15,909
  public review comments is too thin to judge.
- **Plain-language review comments** ("use the client here"). These would need an LLM to turn into
  rules. Stage 1 suggests the ceiling is low anyway.

## Recommendation

1. **Stop** building "learn rules from history" as a product. Don't productize on these numbers.
2. Keep the enforcement layer (`ingrain check`, hooks, pre-commit, Action, hand-written or ast-grep
   rules). It's small, fast and tested, and useful with hand-written rules or stock linters.
3. If the idea continues, run Stage 1 on **your own Claude Code transcripts**
   (`ingrain rules mine` reads `~/.claude/projects` locally). It's an hour and the one open signal.
4. The data points at Idea 2 (the decision ledger). Most corrections are about intent ("we don't
   want this feature", "this changed"), which is what a decision record captures and a lint rule
   can't.
