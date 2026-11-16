---
name: ingrain-setup
description: Set up ingrain in the current repository so coding agents follow the team's recorded decisions and the lint rules learned from past corrections. Use when the user asks to set up or install ingrain, add a decision ledger, or stop agents from breaking conventions the team already agreed on.
---

# Set up ingrain in this repository

ingrain keeps the rules agents must follow in the repo (`decisions/` and `.corrections/rules/`),
shows the relevant ones before each edit, and blocks edits, commits and PRs that break them.
Follow these steps in order. Ask the user before installing software, accepting decisions,
committing or pushing.

## 1. Make sure the `ingrain` binary is installed

Run `ingrain --version`. If it works and prints 0.2.3 or later, go to step 2.

Otherwise ask the user, then install it with the first option that applies:

- macOS or Linux with Homebrew (`brew --version`): `brew install wispagents/tap/ingrain`
- Rust available (`cargo --version`): `cargo install --locked --git https://github.com/WispAgents/ingrain --tag v0.2.3 ingrain`
- Otherwise download the archive for this platform from https://github.com/WispAgents/ingrain/releases/latest
  and put the binary on the PATH:
  - macOS / Linux: `ingrain-<version>-<target>.tar.gz` (`aarch64-apple-darwin`, `x86_64-apple-darwin`,
    `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`)
  - Windows: `ingrain-<version>-x86_64-pc-windows-msvc.zip` (or `aarch64-pc-windows-msvc`); unzip
    `ingrain.exe` into a folder on the PATH

## 2. Initialize the repo

From the repository root:

```sh
ingrain init --agents-md
```

This creates `decisions/` and `.corrections/`, adds ingrain's hooks to `.claude/settings.json`
(they do nothing for teammates who don't have ingrain), installs a git pre-commit hook, and adds an
ingrain section to `AGENTS.md` for agents that don't run hooks (Codex, Cursor, Copilot…). Drop
`--agents-md` if the user doesn't want AGENTS.md touched.

## 3. Seed decisions from what the team already wrote down

If the repo has rule docs (`AGENTS.md`, `CLAUDE.md`, `CONTRIBUTING.md`, an ADR folder), propose
decisions from them. Use only the docs first: it's fast and cheap.

```sh
ingrain decisions extract --doc AGENTS.md --doc CONTRIBUTING.md --no-git --no-github --no-sessions
ingrain decisions list --all
```

Show the user the proposals, grouped by area, in a short list. Point out duplicates and anything
vague. Then:

- **Add checks before accepting**, where a decision can be checked mechanically. Edit the proposal
  file in `decisions/pending/` and add a `checks:` block:
  ```yaml
  checks:
    forbid_deps: [axios]            # package.json, pyproject.toml, requirements*.txt, Cargo.toml, go.mod
    forbid_imports: [node-fetch]
    forbid_patterns:
    - language: TypeScript
      pattern: fetch($$$A)          # ast-grep pattern
  ```
  Decisions without checks are still shown to agents; only checks can block.
- **Accept only what the user approves:** `ingrain decisions accept <P-ids…>`.
- **Reject the rest with a reason:** `ingrain decisions reject <P-id> --reason "<why>"`, so it isn't proposed again.

Accepted records can't be edited by agents afterwards (the hook refuses); changes go through
`ingrain decisions supersede`.

## 4. Add the CI check (GitHub)

If the repo is on GitHub, add `.github/workflows/ingrain.yml`:

```yaml
name: ingrain
on: pull_request
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }
      - uses: WispAgents/ingrain/action@v0.2.3
```

## 5. Commit on a branch

Create a branch and commit, by file name: `decisions/`, `.corrections/`, `.claude/settings.json`,
`AGENTS.md` and the workflow. Don't push or open a PR unless the user asks.

## 6. Show it working

Pick a file a decision covers and run `ingrain decisions for <path>`. Then tell the user, briefly:
how many decisions are live, which ones block, and how to add more (the `ingrain-decide` skill, or
`ingrain decisions add --propose "..."`). Learned rules come later, from corrections: see the
`ingrain-rules` skill.
