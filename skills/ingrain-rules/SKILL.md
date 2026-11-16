---
name: ingrain-rules
description: Learn lint rules from the corrections people make to agent-written code with ingrain, review them with the user, and tune how strictly they apply (shadow, warn, block). Use when the user asks why agents keep repeating the same mistake, wants rules learned from the repo's history, or asks about ingrain rules or their modes.
---

# Learn rules from corrections

ingrain finds places where a person rewrote code an agent wrote, groups the repeats, writes an
ast-grep rule for each, and tests it on the repo's history. Rules live in `.corrections/rules/`;
agents never edit them. The `ingrain` binary must be installed (see the `ingrain-setup` skill).

A single correction is enough to propose a rule. Every proposal still has to pass the backtest,
and the user reviews each one, so expect some one-off fixes among them: reject those with a reason.

## 1. Capture corrections

```sh
ingrain rules mine --pr-refs --github
```

`--pr-refs` also reads fix-ups inside PRs, which squash merges would hide. `--github` reads
```` ```suggestion ```` review comments (needs `gh` logged in). Claude Code transcripts on this
machine are read too; add `--no-transcripts` to skip them. Report the counts it prints.

## 2. Propose rules

```sh
ingrain rules propose --explain
```

Add `--min-corrections 3` to propose only mistakes people fixed at least 3 times (fewer, better-supported rules).

Each proposal has passed a backtest: it catches the past mistakes, doesn't flag the fixed
versions, and is precise on agent code people kept. `--explain` shows why clusters were rejected.
If nothing is proposed, say so plainly and stop.

## 3. Review with the user

For each proposal in `.corrections/pending/`, show the rule, its message, the evidence (past
fixes) and its backtest numbers. Then:

```sh
ingrain rules accept <id…>                       # after the user approves
ingrain rules reject <id> --reason "<why>"       # never proposed again
```

Accepted rules start in **shadow** mode: hits are logged, nothing is shown or blocked.

## 4. Tighten over time

```sh
ingrain rules list                 # modes and hit counts
ingrain rules promote              # suggests shadow → warn → block from the hit log
ingrain rules promote --apply      # only with the user's OK
ingrain rules mode <id> block      # set one by hand
```

`ingrain rules eval` measures the loop on this repo's history: rules learned from older
corrections, scored on newer ones.

## When a rule blocks you

Fix the code the way the rule's message and past fix show. If the line is right as written, the
user can approve a comment on it: `ingrain-ignore <rule-id>: <reason>`.
