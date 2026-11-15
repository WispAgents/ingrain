# Ingrain: working plan

This is the single source of truth for the build. Re-read it before each stage and tick items as they land.
Nothing here lives in agent memory.

## Goal

Ingrain learns lint rules from the corrections humans make to coding-agent output and enforces them
where the agent edits, before commit, and in CI. It must be **validated on real public repos** with a
time-split replay before we call it valuable.

## Decisions (fixed unless this file changes)

- **Language:** Rust workspace, one binary `ingrain`.
- **Matching engine:** `ast-grep-core` + `ast-grep-config` + `ast-grep-language` (0.51) used as libraries.
  Rule files are valid ast-grep YAML with an `ingrain` block under `metadata`.
- **Git:** shell out to `git`. **GitHub:** shell out to `gh api` (uses the user's login; we never read tokens).
- **LLM:** only for rule writing. Backends: `claude` CLI headless (default, uses the user's Claude Code login),
  the Anthropic Messages API over raw HTTP when `ANTHROPIC_API_KEY` is set (`claude-opus-5-5`), or `none`.
  `ingrain check` never calls a model.
- **Deterministic first:** template rules (banned call / banned identifier) are tried before the LLM.
- **Storage:** proposals in `.corrections/pending/` (gitignored), accepted rules in `.corrections/rules/`,
  rejected in `.corrections/rejected.yml`, local data (corrections, hit log) in `.git/ingrain/`.
- **Modes:** `shadow` → `warn` → `block`. New rules always start in `shadow`.
- **Checks only flag added lines** (git diff), never existing code.
- Review UI is a line-based interactive prompt plus non-interactive subcommands (no TUI framework).

## Architecture

```
crates/
  core/    ingrain-core    model types, rule file format, store paths, git + diff helpers, language detection
  engine/  ingrain-engine  ast-grep wrapper: parse, compile rules, scan text, salient tokens
  learn/   ingrain-learn   capture (git fix-ups, GitHub review suggestions, transcripts), cluster, synth (template + LLM),
                           backtest, time-split evaluation
  cli/     ingrain         commands: init, mine, propose, review, check, stats, eval; hook adapters
action/    GitHub Action (composite) running `ingrain check --diff`
```

### Correction mining from git (works on any clone)
1. Classify commits: agent (trailers such as `Co-Authored-By: Claude`, `Generated with Claude Code`, Codex, bot authors
   like `copilot-swe-agent[bot]`) vs human (not agent, not dependency bots).
2. For each human commit H (skip big refactors: >20 files or >400 changed lines), diff `H^..H` with `-U0`.
   For each hunk that removes lines, `git blame --porcelain` those lines at `H^`.
   Lines that came from an agent commit A within the window (default 14 days) → a correction:
   before = agent lines (with context), after = the human replacement (with context).
3. Agent hunks (added lines of agent commits) are kept as the population for precision: a hunk is "corrected"
   if a correction points at its lines, otherwise "kept".
4. Optional `--pr-refs` fetches `refs/pull/*/head` so in-PR fix-ups survive squash merges.

### Clustering
Salient tokens = identifier-like leaves from the tree-sitter parse. removed = in before not after, added = after not before.
Group key = language family + sorted removed set. Pure additions (no removed tokens) are counted as "not a pattern".

### Rule synthesis and backtest
- Template: banned call `X($$$ARGS)` / `$O.X($$$ARGS)`, banned identifier by kind + regex.
- LLM fallback: prompt with the cluster's before/after examples, returns ast-grep YAML, retried with backtest feedback (≤3).
- Accept a candidate only if: matches ≥ min_support befores (hits on the changed lines), matches 0 afters,
  and precision on known agent hunks (corrected vs kept) ≥ threshold.

### Time-split evaluation (`ingrain eval`)
Corrections and agent hunks sorted by time; cutoff T at a fraction of the timeline.
Learn only from data before T; freeze rules; score after T:
- recall = share of post-T corrections whose before is flagged on the changed lines (and after is not)
- precision (lower bound) = flagged corrected hunks / all flagged agent hunks after T
- also: share of corrections that are patterns (Stage 1), check latency per file.

## Validation bar (from the research thread)
- Stage 1: ≥20% of corrections are patterns; ≥30% repeat 3+ times in the same repo.
- Stage 2: median per repo ≥10–15% of post-T corrections prevented, precision ≥90% on a hand-judged sample, <50 ms per file.
- Report honestly if a bar is missed. Stop/narrow rules are in the research thread.

## Stages (tick as done)

- [x] 0. Plan file, workspace, CI-quality tooling (fmt, clippy, tests)
- [x] 1. core: model, rule format, store, git/diff helpers, language detection
- [x] 2. engine: ast-grep wrapper, rule compile + scan, salient tokens, latency test
- [x] 3. learn/capture: git fix-up miner (+ PR refs), GitHub review suggestions, Claude Code transcripts
- [x] 4. learn/cluster + synth (template, LLM) + backtest
- [x] 5. cli: init, mine, propose, review (interactive + accept/reject/edit), check (hook post-edit/stop, staged, diff, github format), stats/promote
- [x] 6. Hook install for Claude Code, git pre-commit, GitHub Action
- [x] 7. Tests: unit + fixture repo integration tests (scripted agent commit → human fix-up)
- [x] 8. eval on 10–20 public repos; write `docs/validation.md` with real numbers
- [x] 9. Independent code review by a subagent; fix findings
- [x] 10. README, push to WispAgents/ingrain

## Public repo candidates
From the discovery run (last 500 commits per repo; counts are approximate):
prefect (Devin+Claude, Py), pydantic/logfire (Claude, Py), livekit/agents (Claude, Py), langfuse (Cursor+Claude, TS),
PrefectHQ/fastmcp (Claude, Py), microsoft/agent-framework (Copilot, Py), airbytehq/airbyte-python-cdk (Devin, Py),
airbytehq/PyAirbyte (Devin, Py), microsoft/vscode-python-environments (Copilot, TS), microsoft/pyright (Copilot, TS),
microsoft/typescript-go (Copilot, Go), crewAI (Claude, Py), agno (Claude, Py), github/spec-kit (Copilot, Py),
getsentry/sentry-javascript (Claude+Codex, TS), firecrawl (Claude, TS), e2b-dev/E2B (Claude+Devin), modelcontextprotocol/typescript-sdk (Claude, TS),
browserbase/stagehand (Claude, TS), TanStack/router (Claude+Copilot, TS), PostHog/posthog-js (Claude, TS),
electric (Claude, TS), anthropics/claude-agent-sdk-python (Claude, Py), simonw/datasette (Claude, Py).

## Log
- 2026-11-15: core/engine/learn crates built, 36 unit tests green. LLM backend default is the `claude` CLI (no API key in env).
- Precision population = agent hunks; TP = hit on lines a human later corrected, FP = hit on kept agent lines.
- Cluster support counts distinct fix events (commit / review comment / session), not hunks.
- 2026-11-15: First real runs (datasette, claude-agent-sdk-python, PyAirbyte): git "corrections" are mostly evolution, not mistakes.
  Added: skip formatter commits/cosmetic hunks, `propose --explain`, precision window starting when the rule becomes learnable
  (3rd distinct fix), so pre-rename code isn't counted as a false alarm.
- Stage 1 hand-label (180 random corrections, 3 repos, independent subagent): evolution 89, bugfix 44, pattern 19 (10.6%),
  taste 15, structural 13. Bar was >=20% -> MISSED. Of the 19, ~10 are already caught by stock linters (ruff/mypy);
  ~9 (5%) are repo-specific (renamed API ClaudeCodeOptions, MCP Field defaults, anyio marker, model ids).
- Hook latency was ~270 ms under load: 5 git spawns. Cut to 2 (one rev-parse, diff first, ls-files only for new files).
- Big repos (langfuse: 41k commits with PR refs) are slow to mine; validate.sh now caps at the newest 6000 commits and resumes.
- Code review (Claude subagent; Codex was out of quota): 15 findings, all correctness/number-inflation ones fixed.
- Final batch, 17 repos: 7,443 corrections, 370k agent hunks, 57 suggestion blocks in 15,909 review comments.
  Permissive gate: 14 rules, 14/3,056 later corrections prevented (0.46%), precision 0.6%.
  Strict gate (Wilson lower bound + required evidence, now the default): 0 rules. Stage 2 MISSED.
- Hook latency ~40 ms/edit on a 3k-line file (2 git spawns). Scan p95 < 1 ms per hunk. Latency bar met.
- Verdict: stop "learn rules from git history". Open signal: in-session transcript corrections (private data).
  Details: docs/validation.md.
