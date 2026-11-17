# Decisions

One YAML file per decision, `decisions/D-0001.yml`, reviewed like code. See the [decision file format](../reference/decision-format.md) for every field.

## Record a decision

```sh
ingrain decisions add "Billing code calls the payments client, never raw fetch." \
  --reason "the client adds idempotency keys" \
  --scope "src/billing/**" \
  --forbid-pattern 'TypeScript:fetch($$$A)'
```

Agents run the same command with `--propose`; the proposal waits in `decisions/pending/` until a
person runs `ingrain decisions review`. To see what applies to a file before editing it:
`ingrain decisions for <path>`.

## Scope and changes

A glob scope matches the path; a bare directory means everything under it. A symbol matches files
whose path names it or whose content mentions it. No scope, or `**`, is repo-wide.

To keep a decision for yourself instead of committing it, write it to a personal store:
`ingrain --local decisions add …` (this repo, ids `L-`) or `ingrain --global decisions add …`
(every repo, ids `G-`). See [local and global stores](personal-stores.md).

To change one: `ingrain decisions supersede D-0002 --decision "..." --reason "..."`, then
`ingrain decisions review`. The old decision stays in force until a person accepts the new one.

`ingrain decisions extract` reads texts it hasn't read before, oldest first, 400 per run: your
`--doc` files (AGENTS.md, CONTRIBUTING.md), commit messages, GitHub PRs, reviews, issues and
comments by the repo's owners, members and collaborators, and what you typed in Claude Code
sessions for this repo. A model returns proposals; near-duplicates of existing, pending or rejected
decisions are dropped. Measured on public repos, 82% of extracted records were real, durable
decisions ([docs/ledger-study](https://github.com/WispAgents/ingrain/blob/main/docs/ledger-study/report.md)).
