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

### Time-split evaluation (`ingrain rules eval`)
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

---

# v0.2: decisions merged into ingrain

decision-ledger (WispAgents/decision-ledger) is folded into ingrain: one binary, one set of hooks,
one check, one Action. The decision-ledger repo is removed after wisp moves over.

## Decisions
- New crate `crates/ledger` (`ingrain-ledger`): decision records, scope, decision checks, manifests,
  extraction, agent messages. Reuses `ingrain-core` (git, diff) and `ingrain-engine` (ast-grep).
- `ingrain-core::git` takes the ledger's improved version (relative_in_repo, before_rev, -w, end-of-options).
- CLI: `ingrain decisions <add|extract|review|accept|reject|supersede|list|show|for|stale>` and
  `ingrain rules <mine|propose|review|accept|reject|list|mode|promote|eval>` (user request: both groups namespaced).
- One hook entry point: `ingrain hook <session-start|pre-tool-use|post-tool-use|stop>`; `check --hook` kept hidden
  for v0.1 settings, with a deprecation note.
  session-start: repo-wide decisions. pre-tool-use: deny edits breaking decision checks, else scoped
  decisions once per session. post-tool-use: learned rules (old post-edit). stop: both, edited files only.
- `ingrain check` runs learned rules and decision checks together (pre-commit, CI, Action).
- One session state file (shown, edited, stop attempts).
- Study moves to docs/ledger-study/.

## Steps
- [x] 1. core git superset
- [x] 2. ledger crate
- [x] 3. CLI: decisions commands, unified hook, unified check, install
- [x] 4. Tests ported, all green, clippy
- [x] 5. README/docs, Action
- [ ] 6. Release v0.2.0, tap (ingrain updated, decision-ledger formula removed)
- [ ] 7. wisp: ingrain only (settings, CI, docs, local checkout)
- [ ] 8. Remove decision-ledger repo (after confirmation)

- 2026-11-16: merged; 80 tests green, clippy clean; live Claude Code check: SessionStart context + PreToolUse deny work with `ingrain hook`.
- Merge review (Claude subagent): manifests skipped by pre-commit/CI/Stop, denied edits marked as edited, `-w` losing
  wrap-and-reindent corrections in mining, silent v0.1 hook breakage, over-broad legacy hook removal. All fixed with tests.
- Learned-rule commands moved under `ingrain rules`. 83 tests green, clippy clean.

---

# v0.2.1: skills and plugin (easy install)

Goal: a developer adds ingrain to their agent and repo in one or two commands.

## Decisions
- The ingrain repo is a Claude Code plugin and its own marketplace: `.claude-plugin/marketplace.json`
  (plugin source `./`) and `.claude-plugin/plugin.json`. `hooks/hooks.json` runs
  `ingrain hook <event> --from-plugin`; skills under `skills/`.
- The same `skills/<name>/SKILL.md` folders install into Codex, Cursor and others with `npx skills add WispAgents/ingrain`.
- Skills: `ingrain-setup` (install the binary, init, seed decisions from AGENTS.md, review with the user),
  `ingrain-decide` (record or change a decision from the conversation), `ingrain-rules` (mine and review learned rules).
- No double hooks: a `--from-plugin` hook does nothing when the repo's `.claude/settings.json` already runs ingrain.
- Agents without hooks: `ingrain init --agents-md` writes a marked, idempotent ingrain block into AGENTS.md.

## Steps
- [x] 1. hook --from-plugin; init --agents-md
- [x] 2. skills (3) + plugin + marketplace; `claude plugin validate .` passes
- [x] 3. tried both installs: `claude plugin marketplace add` + `install` (isolated config) OK;
  `npx skills add <path>` lists 3 skills, installs to .agents/skills (+ .claude/skills symlink).
  One live agent run with `--plugin-dir` on a scratch repo used ingrain-setup end to end: 2 decisions
  with checks, commit on a branch. Found: extract read ingrain's own AGENTS.md block → fixed (skipped).
- [ ] 4. README "Use it from your coding agent" (done); PR, review, merge; release v0.2.1; tap

# v0.2.2: Windows; one correction is enough

- `rules propose|eval --min-corrections N` (default: config `min_support`, now 1). On datasette
  (157 corrections, templates only): min 3 → 0 rules, min 1 → 13 rules. Real hits among them
  (`PluginSQL` → `PermissionSQL` rename), plus one-off noise (`r3`); review filters it.
- README no longer quotes the public-repo verdict; docs/validation.md keeps the study.

# v0.2.3: review fixes (PR #8)

- Enforcement: Bash hooks (ask before writing reviewed files or skipping hooks; check changed files after),
  protected config, CI/pre-commit pin rules and config to the base, override approval and CI annotations.
- Ledger: line-above override scope, dedupe polarity/numbers, file-name scopes, `*` within one directory.
- Rules: one correction still proposes a rule, without self-certification; single-fix one-offs held back
  (still common today, generic name, test-only). datasette: 33 candidates → 7 proposed, 12 held back.

# v0.2.4: ingrain inside agent sessions (PR #14)

- First setup waives approval for an allowlist of setup commands until a ledger is committed on any
  ref; live headless runs: 6/6 setups accepted and committed with no refusals.
- History rule: reviewed files that match history a command moved through (checkout, merge, rebase,
  fetched remotes, stash, in-progress operations) aren't tampering; rewinds and new content are.

# v0.3.0: personal stores (local and global)

- `--local` (`.git/ingrain/local/`, `L-` ids) and `--global` (`~/.config/ingrain/`, `G-` ids) keep
  decisions and learned rules out of the working tree; hooks and checks read all three stores.
- Personal stores get the same agent protections, with no first-setup exception; broken personal
  files are skipped so they can't fail checks in every repository.
