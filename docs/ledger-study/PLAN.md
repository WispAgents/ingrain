# Decision Ledger: validation plan

Working plan for validating Idea 2 before any product code. Re-read before each step and tick items as they land.

## Questions

1. **Prior record.** For a human correction of agent code, was the reason for the fix written down
   somewhere *before the agent wrote the code*? Places to look: repo docs (CLAUDE.md, AGENTS.md,
   CONTRIBUTING, docs/), commit messages, PR descriptions, PR review comments, issue bodies and comments.
2. **Extraction.** Can a model turn those texts into decision records (decision, reason, scope)
   accurately enough to inject at edit time?

## Bars (set before seeing results)

| Metric | Bar |
|---|---|
| A. Corrections whose reason was recorded before the agent commit | ≥ 20% of all sampled corrections |
| A2. Of those, recorded outside files the agent already loads (CLAUDE.md, AGENTS.md, README) | ≥ 50% (otherwise it's a "read your docs" problem, not a ledger) |
| B. Extraction recall: the extractor, given the gold source text only, produces the decision | ≥ 60% |
| C. Extraction precision: extracted records judged real, durable decisions | ≥ 70% |
| D. Scope: extracted scope covers the corrected file | ≥ 60% of recalled decisions |

Stop rule: A or C missed means don't build the ledger.

## Method

- **Sample:** 3 repos already hand-labeled for Ingrain (PyAirbyte, claude-agent-sdk-python, datasette)
  plus 4 new (langfuse, sentry-javascript, fastmcp, logfire). 40 corrections per repo → 280.
  Only corrections with ≥ 60 days of fetched GitHub history before the agent commit.
- **Corpus per repo (`fetch.py`):** PRs (title, body), PR review comments, issue comments, issues;
  newest ~3,000 of each via `gh api`, with created times. Git log and docs from the local clone.
- **Context pack per correction (`pack.py`):** the correction (before/after, fix commit message, fix PR
  and its review comments: the *stated* reason), plus prior candidates dated before the agent commit,
  ranked by keyword overlap: top discussion texts, commit messages, doc lines at the agent commit's parent.
  Also the agent commit's own PR body (if the human told the agent and it ignored that, it counts).
- **Judging:** independent subagents, not shown the hypothesis's bar. Per correction:
  `kind` (decision / bugfix / taste / evolution / other), `prior` (yes / partial / no) with the cited
  source id, `where` (doc / commit / pr / review / issue / agent_pr).
- **Extraction (`extract.py`):** `claude -p` with a fixed prompt returning JSON records
  `{decision, reason, scope[], supersedes?}`.
  - Recall: run on the gold source texts of `prior=yes` items; a judge checks a record matches.
  - Precision: run on 100 random corpus texts; a judge rates each record (real durable decision / noise).
- **Caveat to report:** keyword retrieval only finds candidates that share words; misses make A a lower bound.

## Steps

- [x] 1. fetch.py: corpus for 7 repos
- [x] 2. pack.py: sample + context packs
- [x] 3. Judge packs (parallel subagents)
- [x] 4. extract.py + recall and precision runs
- [x] 5. Judge extraction
- [x] 6. report.md with numbers, verdict

## Log
- 2026-11-15: Subagent tool unavailable, so judging runs as scripted `claude -p` calls (judge.py, judge_records.py).
- Coverage: logfire, sentry, langfuse refetched with 100 pages; sample needs 60 days of every stream before the agent commit.
- Results: A 11/245 (4.5%) MISSED (1.2% without agent-PR bodies); C precision 82%, B recall 11/11 (5 sources), D scope 7/11.
  Verdict: don't build. See report.md.
