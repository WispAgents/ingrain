# CLI reference

This document contains the help content for the `ingrain` command-line program.

## `ingrain`

Keep coding agents in line with what your team decided: recorded decisions and rules learned from your corrections, enforced at edit time, before commit and in CI

**Usage:** `ingrain [OPTIONS] <COMMAND>`

```text
Setting up a repo (for coding agents):
  1. ingrain init --agents-md          hooks, pre-commit, and an AGENTS.md section for hookless agents
  2. ingrain decisions extract --doc AGENTS.md --no-git --no-github --no-sessions
                                       propose decisions from the rule docs (add --doc for each)
  3. Add a `checks:` block (forbid_deps, forbid_imports, forbid_patterns) to each proposal in
     decisions/pending/ that can be checked mechanically; without checks a decision never blocks
  4. Show the user the proposals, then `ingrain decisions accept <P-ids>` for the ones they approve
     and `ingrain decisions reject <P-id> --reason ...` for the rest
  5. On GitHub, add .github/workflows/ingrain.yml using WispAgents/ingrain/action
  6. Commit decisions/, .corrections/, .claude/settings.json and AGENTS.md on a branch
Until that first commit, ingrain's hooks let the agent accept and commit, when each runs as its
own command (no `cd`, variables or other programs in it); after it, agents only propose
(`ingrain decisions add --propose`, `ingrain decisions supersede`) and a person accepts.
Just for the user, nothing committed: `ingrain init --local` (this repo, .git/ingrain/local) or
`ingrain init --global` (every repo, ~/.config/ingrain); then `ingrain --local|--global decisions
add --propose ...`. A person accepts; there is no first-setup exception for personal stores.
Guide: https://github.com/WispAgents/ingrain/blob/main/skills/ingrain-setup/SKILL.md
```

**Subcommands:**

* `init` — Create .corrections/ and decisions/, and install the Claude Code and git hooks (`--local`: for you only, nothing committed; `--global`: for every repository)
* `check` — Check added lines against learned rules and decisions (pre-commit, CI)
* `decisions` — The decision ledger: record, extract, review, supersede and list decisions
* `rules` — Learned lint rules: mine corrections, propose, review, promote and evaluate rules
* `view` — Open the desktop viewer: browse, review and trace decisions and rules (`--global`: only your personal rules)
* `hook` — Claude Code hook entry point (reads the hook JSON on stdin)

**Options:**

* `--repo <DIR>` — Repository to work on (defaults to the current directory)
* `--local` — Keep decisions and rules for this repository only, inside `.git/`: never committed
* `--global` — Keep decisions and rules for every repository on this machine (`~/.config/ingrain`, or `$INGRAIN_HOME`)



## `ingrain init`

Create .corrections/ and decisions/, and install the Claude Code and git hooks (`--local`: for you only, nothing committed; `--global`: for every repository)

**Usage:** `ingrain init [OPTIONS]`

**Options:**

* `--no-claude` — Don't touch the Claude Code settings
* `--no-git-hook` — Don't install the git pre-commit hook
* `--agents-md` — Also add an ingrain section to AGENTS.md, for agents that read it but run no hooks (Codex, Cursor, Copilot…)



## `ingrain check`

Check added lines against learned rules and decisions (pre-commit, CI)

**Usage:** `ingrain check [OPTIONS] [PATHS]...`

**Arguments:**

* `<PATHS>` — Files to check (default: every file with added lines)

**Options:**

* `--staged` — Check staged changes (pre-commit)
* `--diff <RANGE>` — Check added lines in a revision range, e.g. `origin/main...HEAD` (CI)
* `--format <FORMAT>`

  Default value: `text`

  Possible values: `text`, `json`, `github`




## `ingrain decisions`

The decision ledger: record, extract, review, supersede and list decisions

**Usage:** `ingrain decisions <COMMAND>`

**Subcommands:**

* `add` — Record a decision (accepted, or `--propose` for review)
* `extract` — Propose decisions from rule docs, commits, GitHub PRs/reviews/issues and Claude Code sessions
* `review` — Go through proposed decisions one by one: accept, edit, reject or skip
* `accept` — Accept proposals; a superseding proposal retires the decision it replaces
* `reject` — Reject a proposal so extraction doesn't bring it back
* `supersede` — Propose a replacement for an accepted decision
* `list` — List decisions (`--all` includes superseded ones and proposals)
* `show` — Show one decision in full
* `for` — Decisions that apply to a file
* `stale` — Decisions whose scope no longer exists, that are often overridden, or that are broken



## `ingrain decisions add`

Record a decision (accepted, or `--propose` for review)

**Usage:** `ingrain decisions add [OPTIONS] <DECISION>`

**Arguments:**

* `<DECISION>` — The decision, as one imperative sentence

**Options:**

* `--reason <REASON>`
* `--scope <SCOPE>` — Path globs or symbol names it applies to (repeatable). Omit for repo-wide
* `--category <CATEGORY>`

  Default value: `convention`
* `--rejected <REJECTED>` — An alternative that was rejected (repeatable)
* `--forbid-import <FORBID_IMPORT>` — Module that must not be imported in scope (repeatable)
* `--forbid-dep <FORBID_DEP>` — Package that must not be added as a dependency (repeatable)
* `--forbid-pattern <FORBID_PATTERN>` — ast-grep pattern that must not appear in scope, as `Language:pattern` (repeatable)
* `--source <SOURCE>` — Link to where it was decided (repeatable)
* `--propose` — Write it as a proposal for a human to accept (what agents should use)



## `ingrain decisions extract`

Propose decisions from rule docs, commits, GitHub PRs/reviews/issues and Claude Code sessions

**Usage:** `ingrain decisions extract [OPTIONS]`

**Options:**

* `--since <SINCE>` — Only texts after this date (YYYY-MM-DD). Default: 90 days before the first run
* `--github <GITHUB>` — GitHub repo as `owner/name` (default: detected from the remote; skipped if there is none)
* `--no-github`
* `--no-git`
* `--no-sessions`
* `--doc <DOC>` — Also read a doc that states project rules, e.g. AGENTS.md (repeatable). Read once
* `--sessions-dir <SESSIONS_DIR>` — Claude Code projects directory (default ~/.claude/projects)
* `--pages <PAGES>` — Pages of 100 GitHub items per stream

  Default value: `10`
* `--max-texts <MAX_TEXTS>` — Read at most this many texts per run (oldest unread first)

  Default value: `400`
* `--llm <LLM>` — auto, claude, api or none

  Default value: `auto`
* `--model <MODEL>`
* `--dry-run` — Count the texts and stop before calling the model



## `ingrain decisions review`

Go through proposed decisions one by one: accept, edit, reject or skip

**Usage:** `ingrain decisions review`



## `ingrain decisions accept`

Accept proposals; a superseding proposal retires the decision it replaces

**Usage:** `ingrain decisions accept <IDS>...`

**Arguments:**

* `<IDS>`



## `ingrain decisions reject`

Reject a proposal so extraction doesn't bring it back

**Usage:** `ingrain decisions reject --reason <REASON> <ID>`

**Arguments:**

* `<ID>`

**Options:**

* `--reason <REASON>`



## `ingrain decisions supersede`

Propose a replacement for an accepted decision

**Usage:** `ingrain decisions supersede [OPTIONS] --decision <DECISION> --reason <REASON> <ID>`

**Arguments:**

* `<ID>` — The accepted decision to replace

**Options:**

* `--decision <DECISION>` — The new decision
* `--reason <REASON>`
* `--scope <SCOPE>` — New scope (default: the old decision's scope)
* `--keep-checks` — Keep the old decision's deterministic checks



## `ingrain decisions list`

List decisions (`--all` includes superseded ones and proposals)

**Usage:** `ingrain decisions list [OPTIONS]`

**Options:**

* `--all`



## `ingrain decisions show`

Show one decision in full

**Usage:** `ingrain decisions show <ID>`

**Arguments:**

* `<ID>`



## `ingrain decisions for`

Decisions that apply to a file

**Usage:** `ingrain decisions for [OPTIONS] <PATH>`

**Arguments:**

* `<PATH>`

**Options:**

* `--json`



## `ingrain decisions stale`

Decisions whose scope no longer exists, that are often overridden, or that are broken

**Usage:** `ingrain decisions stale`



## `ingrain rules`

Learned lint rules: mine corrections, propose, review, promote and evaluate rules

**Usage:** `ingrain rules <COMMAND>`

**Subcommands:**

* `mine` — Capture corrections from git history, PR review suggestions and agent transcripts
* `propose` — Cluster corrections and write backtested rule proposals to .corrections/pending/
* `review` — Go through proposed rules one by one: accept, edit, reject or skip
* `accept` — Accept proposed rules (moves them to .corrections/rules/ and stages them)
* `reject` — Reject a proposed rule so it is never proposed again
* `list` — List accepted and proposed rules with their mode and hit counts
* `mode` — Set a rule's mode by hand
* `promote` — Suggest (or apply) promotions and demotions from the hit log
* `eval` — Time-split evaluation: learn from older corrections, score on newer ones



## `ingrain rules mine`

Capture corrections from git history, PR review suggestions and agent transcripts

**Usage:** `ingrain rules mine [OPTIONS]`

**Options:**

* `--github <GITHUB>` — Also read ```suggestion review comments from GitHub (`owner/name`, or detected from the origin remote when given without a value)
* `--review-pages <REVIEW_PAGES>` — Pages of 100 review comments to read, newest first

  Default value: `20`
* `--pr-refs` — Fetch `refs/pull/*/head` first, so fix-ups inside PRs survive squash merges
* `--transcripts <TRANSCRIPTS>` — Read Claude Code transcripts from this projects directory (default ~/.claude/projects)
* `--no-transcripts` — Skip transcripts
* `--since <SINCE>` — Only commits after this date (`git log --since`)
* `--max-commits <MAX_COMMITS>` — Only the newest N commits



## `ingrain rules propose`

Cluster corrections and write backtested rule proposals to .corrections/pending/

**Usage:** `ingrain rules propose [OPTIONS]`

**Options:**

* `--llm <LLM>` — Model backend for clusters templates can't express: auto, claude, api or none

  Default value: `auto`
* `--model <MODEL>` — Model id (default claude-opus-5-5)
* `--llm-clusters <LLM_CLUSTERS>` — Clusters sent to the model at most

  Default value: `10`
* `--min-corrections <MIN_CORRECTIONS>` — Corrections of the same pattern needed before a rule is proposed (default: `min_support` in .corrections/config.yml, 1 unless set). Raise it for fewer, better-supported rules
* `--explain` — Print every cluster and how each candidate rule scored
* `--all` — Also keep single-fix rules that look like one-offs (code still common today, or a generic name)



## `ingrain rules review`

Go through proposed rules one by one: accept, edit, reject or skip

**Usage:** `ingrain rules review`



## `ingrain rules accept`

Accept proposed rules (moves them to .corrections/rules/ and stages them)

**Usage:** `ingrain rules accept <IDS>...`

**Arguments:**

* `<IDS>`



## `ingrain rules reject`

Reject a proposed rule so it is never proposed again

**Usage:** `ingrain rules reject --reason <REASON> <ID>`

**Arguments:**

* `<ID>`

**Options:**

* `--reason <REASON>`



## `ingrain rules list`

List accepted and proposed rules with their mode and hit counts

**Usage:** `ingrain rules list`



## `ingrain rules mode`

Set a rule's mode by hand

**Usage:** `ingrain rules mode <ID> <MODE>`

**Arguments:**

* `<ID>`
* `<MODE>`



## `ingrain rules promote`

Suggest (or apply) promotions and demotions from the hit log

**Usage:** `ingrain rules promote [OPTIONS]`

**Options:**

* `--apply`



## `ingrain rules eval`

Time-split evaluation: learn from older corrections, score on newer ones

**Usage:** `ingrain rules eval [OPTIONS]`

**Options:**

* `--split <SPLIT>` — Share of corrections (oldest first) used to learn rules

  Default value: `0.6`
* `--llm <LLM>` — Model backend for clusters templates can't express: auto, claude, api or none

  Default value: `none`
* `--model <MODEL>`
* `--llm-clusters <LLM_CLUSTERS>`

  Default value: `10`
* `--min-corrections <MIN_CORRECTIONS>` — Corrections of the same pattern needed before a rule is proposed (default: `min_support` in .corrections/config.yml, 1 unless set). Raise it for fewer, better-supported rules
* `--json <JSON>` — Write the full report as JSON



## `ingrain view`

Open the desktop viewer: browse, review and trace decisions and rules (`--global`: only your personal rules)

**Usage:** `ingrain view [OPTIONS] [PATH]`

**Arguments:**

* `<PATH>` — A folder inside the repository to open (default: the current directory, or `--repo`)

**Options:**

* `--select <ID>` — Open on this item, e.g. `D-0004` or a rule id



## `ingrain hook`

Claude Code hook entry point (reads the hook JSON on stdin)

**Usage:** `ingrain hook [OPTIONS] <EVENT>`

**Arguments:**

* `<EVENT>`

  Possible values: `session-start`, `pre-tool-use`, `post-tool-use`, `stop`


**Options:**

* `--from-plugin` — Set by the Claude Code plugin; does nothing when the repo's own settings run ingrain



