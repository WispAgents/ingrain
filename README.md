# ingrain

**Problem:** your coding agent keeps breaking what your team already settled. It re-adds the
dependency you removed, brings back the pattern you banned, and repeats the mistake you corrected
last week.

**Solution:** ingrain keeps those rules in your repo, puts the relevant ones in front of the agent
right before it edits a file, and blocks edits, commits and PRs that break them.

Rules come from two places:

- **Decisions** you record (or that `ingrain decisions extract` proposes from your AGENTS.md, PRs,
  reviews, commits and sessions): "billing calls the payments client, never raw fetch", with a
  reason, a scope and optional checks. They live in `decisions/`.
- **Lint rules learned from your corrections**: when you keep fixing the same thing in agent code,
  `ingrain rules mine` and `ingrain rules propose` turn it into an ast-grep rule, tested on your history. They
  live in `.corrections/rules/`.

Nothing becomes a rule without a person accepting it.

## Install

```sh
brew install wispagents/tap/ingrain
# or a prebuilt binary from https://github.com/WispAgents/ingrain/releases
# or from source:
cargo install --locked --git https://github.com/WispAgents/ingrain --tag v0.2.0 ingrain
```

Needs `git`. Optional: `gh` (logged in) to read GitHub PRs, reviews and issues; Claude Code
(`claude`) or `ANTHROPIC_API_KEY` for extraction and for rules templates can't express.

## Quick start

```sh
cd your-repo
ingrain init      # decisions/, .corrections/, Claude Code hooks, git pre-commit hook

ingrain decisions add "Billing code calls the payments client, never raw fetch." \
  --reason "the client adds idempotency keys" \
  --scope "src/billing/**" \
  --forbid-pattern 'TypeScript:fetch($$$A)'

ingrain decisions extract --doc AGENTS.md   # propose decisions from your rules doc, PRs, commits, sessions
ingrain decisions review                    # accept / edit / reject

ingrain rules mine --pr-refs --github       # capture corrections of agent code
ingrain rules propose --explain             # learn and backtest lint rules from them
ingrain rules review                        # accept / edit / reject

git commit decisions/ .corrections/ -m "ingrain rules"
```

## What the agent sees (Claude Code)

`ingrain init` adds one hook command per event to `.claude/settings.json` (`ingrain hook <event>`).
Each is a no-op for teammates who don't have ingrain installed, and all of them fail open.

| Hook | What happens |
|---|---|
| `SessionStart` | Repo-wide decisions go into the session, plus how to propose a new one |
| `PreToolUse` on Edit / Write / MultiEdit | The edit is simulated first. If an added line breaks a decision's check, the edit is **denied** with the decision and the way out. Otherwise the decisions scoped to that file are shown, each once per session |
| `PostToolUse` on Edit / Write / MultiEdit | Learned rules run on the edited file. Warn: a note to the agent. Block: the agent must fix it, with the past human fix shown |
| `Stop` | Rules and decisions are re-checked on the files this session edited. Something broken sends the turn back, up to `stop_max_attempts` times |

Agents can't edit accepted decisions or rules; they propose with `ingrain decisions add --propose`
or `ingrain decisions supersede`, and a person accepts.

Tested live with Claude Code 2.1.322: asked to write raw `fetch` in `src/billing/`, the write was
denied before the file changed, and the agent explained the decision instead of working around it.

## Where checks run

`ingrain check` runs learned rules and decision checks together, on **added lines only**, so
existing code never blocks.

| Path | Command | On a hit |
|---|---|---|
| Commit | `ingrain check --staged` (`.git/hooks/pre-commit`) | the commit fails |
| Pull request | `ingrain check --diff base...HEAD --format github` ([action](action/action.yml)) | annotations; failing check |
| By hand | `ingrain check [paths]` | listed; exit 1 if something blocks |

Pre-commit and CI judge a change by the decisions it started from, so a branch can't retire a
decision and break it in the same diff. CI also flags any change to the ledger.

To keep a line on purpose, add a comment with a reason, on the line or the line above:

```ts
// ingrain-ignore avoid-fetch-call: this endpoint streams the response body
// decisions-ignore D-0001: webhook replay needs raw fetch
const res = await fetch(url);
```

Without a reason it doesn't count. Overrides are logged; `ingrain rules promote` and
`ingrain decisions stale` flag rules and decisions that are overridden often.

## Decisions

One YAML file per decision, `decisions/D-0001.yml`, reviewed like code:

```yaml
id: D-0001
decision: Billing code calls the payments client, never raw fetch.
reason: the client adds idempotency keys
rejected:
- a shared fetch wrapper per module
scope:
- src/billing/**          # path globs, or symbol / module names like `apiClient`
category: convention      # convention | constraint | api_change | architecture | scope
status: accepted          # accepted | superseded (proposals live in decisions/pending/)
checks:                   # optional; without checks a decision is shown, not enforced
  forbid_imports: [axios]
  forbid_deps: [redis]    # package.json, pyproject.toml, requirements*.txt, Cargo.toml, go.mod
  forbid_patterns:
  - language: TypeScript
    pattern: fetch($$$A)  # ast-grep pattern
sources:
- https://github.com/acme/app/pull/214
```

A glob scope matches the path; a bare directory means everything under it. A symbol matches files
whose path names it or whose content mentions it. No scope, or `**`, is repo-wide.

To change one: `ingrain decisions supersede D-0002 --decision "..." --reason "..."`, then
`ingrain decisions review`. The old decision stays in force until a person accepts the new one.

`ingrain decisions extract` reads texts it hasn't read before, oldest first, 400 per run: your
`--doc` files (AGENTS.md, CONTRIBUTING.md), commit messages, GitHub PRs, reviews, issues and
comments by the repo's owners, members and collaborators, and what you typed in Claude Code
sessions for this repo. A model returns proposals; near-duplicates of existing, pending or rejected
decisions are dropped. Measured on public repos, 82% of extracted records were real, durable
decisions ([docs/ledger-study](docs/ledger-study/report.md)).

## Learned rules

| Source | Signal |
|---|---|
| Git fix-ups (`ingrain rules mine`) | A human commit rewrites lines an agent commit added, within `window_days` |
| PR refs (`--pr-refs`) | Same, on `refs/pull/*/head`, so fix-ups inside PRs survive squash merges |
| Review suggestions (`--github`) | A ```` ```suggestion ```` comment on agent-written lines |
| Claude Code transcripts | The agent edits, you reply, the agent rewrites what it just wrote |

`ingrain rules propose` clusters corrections by what they removed, writes rules (templates first, then a
model), and keeps a rule only if it catches the past mistakes, spares the fixed versions, and is
precise on agent code humans kept. Accepted rules start in shadow mode; `ingrain rules promote` moves them
to warn and block from their hit log. `ingrain rules eval` measures the whole loop on your history.

Expect few rules at first: on 17 public repos, most human fixes of agent code were one-off bug fixes
and taste, not repeated mistakes ([docs/validation.md](docs/validation.md)). Decisions you record
are the main lever; learned rules catch what repeats.

## Commands

| | |
|---|---|
| `init` | Create `decisions/` and `.corrections/`, install hooks |
| `check [paths] [--staged \| --diff RANGE] [--format text\|json\|github]` | Rules and decisions on added lines |
| `decisions add \| extract \| review \| accept \| reject \| supersede \| list \| show \| for \| stale` | The decision ledger |
| `rules mine \| propose [--explain] \| review \| accept \| reject \| list \| mode \| promote \| eval` | Learned rules |
| `hook <session-start\|pre-tool-use\|post-tool-use\|stop>` | Claude Code hook entry point |

## CI

```yaml
# .github/workflows/ingrain.yml
on: pull_request
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }
      - uses: WispAgents/ingrain/action@v0.2.0
```

## Layout

```
decisions/D-NNNN.yml       accepted decisions          committed
decisions/pending/         proposed decisions          gitignored
.corrections/rules/        accepted learned rules      committed
.corrections/pending/      proposed rules              gitignored
.corrections/config.yml    thresholds, agent markers   committed
.git/ingrain/              corrections, hit logs, hook sessions, extraction state (local)
```

## Development

```sh
cargo test --all
cargo clippy --all-targets -- -D warnings
```

Crates: `core` (data model, rule files, git), `engine` (ast-grep), `learn` (capture, cluster, rule
writing, backtest, eval), `ledger` (decisions, scope, checks, extraction), `cli` (the `ingrain`
binary). [PLAN.md](PLAN.md) is the working plan.

License: Apache-2.0.
