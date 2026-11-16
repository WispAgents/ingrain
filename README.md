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
brew install wispagents/tap/ingrain   # macOS and Linux
# or from source, any OS:
cargo install --locked --git https://github.com/WispAgents/ingrain --tag v0.2.2 ingrain
```

Prebuilt binaries for macOS, Linux and Windows (x64 and arm64) are on the
[releases page](https://github.com/WispAgents/ingrain/releases/latest). On Windows, unzip
`ingrain.exe` into a folder on your `PATH`. Claude Code runs hooks through Git Bash there, and
Git for Windows runs the pre-commit hook, so both work as on macOS and Linux.

Needs `git`. Optional: `gh` (logged in) to read GitHub PRs, reviews and issues; Claude Code
(`claude`) or `ANTHROPIC_API_KEY` for extraction and for rules templates can't express.

## Use it from your coding agent

With `ingrain` installed, give your agent the ingrain skills, then ask it to **"set up ingrain in
this repo"**. The `ingrain-setup` skill runs `ingrain init` (and installs the binary first if it's
missing, after asking), turns the rules in your AGENTS.md / CONTRIBUTING.md into decisions for
you to approve, and commits on a branch.

**Claude Code** (plugin: skills + hooks):

```
/plugin marketplace add WispAgents/ingrain
/plugin install ingrain@ingrain
```

**Any agent** (Claude Code, Codex, Cursor, Copilot, Gemini CLI, opencode and
[more](https://github.com/vercel-labs/skills)):

```sh
npx skills add WispAgents/ingrain
```

| Skill | Use it for |
|---|---|
| `ingrain-setup` | Install and set up ingrain in a repo, seed decisions from your docs |
| `ingrain-decide` | "From now on we don't use X": record, change or look up a decision |
| `ingrain-rules` | Learn rules from past corrections, review them, set shadow / warn / block |

The plugin's hooks act only in repos set up with `ingrain init`, and step aside where the repo's
own `.claude/settings.json` already runs ingrain (no double checks). Without the `ingrain` binary
they do nothing. Agents without hooks get an ingrain section in AGENTS.md (`ingrain init --agents-md`) and are caught by pre-commit and CI.

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
precise on agent code humans kept. One correction is enough to propose a rule; pass
`--min-corrections 3` (or set `min_support` in `.corrections/config.yml`) to wait for a mistake to
repeat. Accepted rules start in shadow mode; `ingrain rules promote` moves them to warn and block
from their hit log. `ingrain rules eval` measures the whole loop on your history.

## Commands

| | |
|---|---|
| `init` | Create `decisions/` and `.corrections/`, install hooks |
| `check [paths] [--staged \| --diff RANGE] [--format text\|json\|github]` | Rules and decisions on added lines |
| `decisions add \| extract \| review \| accept \| reject \| supersede \| list \| show \| for \| stale` | The decision ledger |
| `rules mine \| propose [--explain] [--min-corrections N] \| review \| accept \| reject \| list \| mode \| promote \| eval` | Learned rules |
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
      - uses: WispAgents/ingrain/action@v0.2.2
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
