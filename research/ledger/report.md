# Decision Ledger: validation report

**Verdict: don't build it.** Extraction works: the model turns PR and review text into decision
records with 82% precision. But the premise fails. Only **4.5%** of human corrections of agent code
(11 of 245) had their reason written down anywhere before the agent wrote the code, against a 20% bar.
Most of those trace to a single PR. The stop rule (bar A missed) applies.

Date: 2026-11-15. Plan and pre-set bars: [PLAN.md](PLAN.md). Data: [data/](data/).

## Results against the bars

| Metric | Result | Bar | |
|---|---|---|---|
| A. Reason recorded before the agent commit | **11 / 245 (4.5%)** | ≥ 20% | missed |
| A, counting distinct fix events | 5 events (7 of the 11 are hunks of one PR) | | |
| A, excluding the agent PR's own description | 3 / 245 (1.2%) | | |
| A, upper bound counting "partial" | 60 / 245 (24%) | | see below |
| A2. Recorded outside agent-loaded docs | 11 / 11 | ≥ 50% | met |
| B. Extraction recall on gold sources | 11 / 11 (5 distinct sources) | ≥ 60% | met, tiny n |
| C. Extraction precision on random texts | **61 / 74 records (82%)** durable; 13 one-off; 0 wrong | ≥ 70% | met |
| D. Scope covers the corrected file | 7 / 11 (64%) | ≥ 60% | met |

## What the corrections are

245 corrections, 35 each from PyAirbyte, claude-agent-sdk-python, datasette, fastmcp, logfire,
sentry-javascript and langfuse. Each correction had at least 60 days of fetched GitHub history (PRs,
reviews, issues, comments) before the agent commit.

| kind | count | prior record yes / partial / no |
|---|---:|---|
| bugfix | 80 (33%) | 0 / 27 / 53 |
| taste | 66 (27%) | 0 / 10 / 56 |
| **decision** (fix applies a durable project rule) | **50 (20%)** | **11 / 7 / 32** |
| evolution | 48 (20%) | 0 / 5 / 43 |
| other | 1 | 0 / 0 / 1 |

This matches the Ingrain hand-labels from a different angle. Most corrections are local (bug fixes,
taste) or new requirements. When a correction does enforce a project rule (20%), the rule usually
**wasn't written anywhere yet**: in 32 of 50 cases the decision was made at review time, by the human
fixing the code.

The 49 "partial" items are mostly topic-related PR descriptions with no specific rule ("adds X
support" when the fix was about how X must be done). They wouldn't have stopped an agent.

## Why the gold set is weaker than it looks

- **7 of the 11 come from one fix event:** claude-agent-sdk-python PR #545 ("don't expose SessionStart
  / SessionEnd / Setup hook types"), one PR with 7 corrected hunks.
- **8 of 11 cite the agent commit's own PR description.** GitHub returns the *current* PR body, which
  can be edited after the fix, so these may leak the answer. Without them A is 1.2%.
- The 3 clean cases are a datasette PR on SQLite identifier quoting, and two sentry-javascript PRs:
  use `@sentry/conventions` constants, and don't call `getClient()` when a client is passed in.

## Caveats

- **A is a lower bound.** Candidates come from keyword search, so a prior record that uses different
  words is missed. Doubling A still misses the bar.
- **Coverage.** The newest 3,000–10,000 items per stream. Busy repos (sentry, langfuse) have only
  2–7 months of history before each agent commit. Slack, design docs and private channels aren't
  visible at all, and a private team may record more there.
- **Judges.** Labels come from `claude-opus-5-5` via `judge.py` / `judge_records.py`, with written
  rubrics, no human audit. The extractor and the precision judge are the same model family, so C may
  be optimistic.

## What this says about the space

Two ideas, one root cause. In public repos, human corrections of agent code are mostly **not memory
failures**. Neither a lint rule learned from past fixes (Ingrain: 10.6% rule-shaped) nor a recorded
decision (this study: 4.5% recorded beforehand) would have prevented them. They are bug fixes (33%)
and taste (27%) caught in review, plus requirements that changed (20%). The extractor itself is good
and would be worth reusing if a different product needs decision records.

## Reproduce

```sh
python3 -I fetch.py owner/name /tmp/ledger/corpus 100        # per repo
python3 -I pack.py <clones> /tmp/ledger/corpus /tmp/ledger/packs 35 <repo dirs...>
python3 -I judge.py /tmp/ledger/packs/packs.jsonl labels.jsonl 5
python3 -I extract.py texts.jsonl extracted.jsonl 8
```

Corrections come from `ingrain mine --pr-refs --github` (the Ingrain branch).
