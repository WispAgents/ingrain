# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] - 2026-11-17

### Added

- Personal stores, never written to the working tree. `--local` keeps decisions (`L-0001`) and
  learned rules in `.git/ingrain/local/` for one repository. `--global` keeps them (`G-0001`) in
  `~/.config/ingrain/` (`$INGRAIN_HOME`) for every repository. `ingrain init --local` puts the
  hooks in `.claude/settings.local.json` (git-excluded). `ingrain init --global` puts them in
  `~/.claude/settings.json` as plugin-style hooks. Hooks and `ingrain check` read all stores.
  Agents are kept out of personal stores the same way, with no first-setup exception.
- Docs site: an mdBook in `docs/book/`, deployed to GitHub Pages from `main`, with a CLI
  reference generated from the binary by the hidden `ingrain --markdown-help` flag.

## [0.2.4] - 2026-11-16

### Added

- First setup: until a decision, rule or ingrain config is committed on any ref, an agent can
  accept proposals and commit the setup without a person approving, so "use ingrain in this
  repo" finishes in headless sessions. Only plain setup commands are let through; a refusal
  says how to get through.
- `ingrain --help` and `ingrain init` list the setup steps for agents without the skills.
- Contribution guide, code of conduct, security policy, issue and PR templates, CODEOWNERS.
- Dependabot for Cargo and GitHub Actions, `cargo deny` checks in CI, declared MSRV.

### Fixed

- Checkout, merge, rebase (`--continue`, `--autostash`), pull and stash no longer block when
  they bring in reviewed decision changes. New content, rewound branches, restored older
  versions and the person's discarded decision edits are still reported.
- Committing a decision accepted with approval, a partly staged decision, or unstaging one is no
  longer reported as tampering. Same-size, same-mtime edits and index-only changes are.
- Fewer false asks: `ingrain decisions for`, `--help`, `--repo` after the subcommand,
  `git fetch`/`branch`/`remote` next to a decision path, reading `core.hooksPath`, line
  continuations and shell loops.
- Asks before removing ingrain's marker, reflogs or refs that show a ledger was committed.

## [0.2.3] - 2026-11-16

### Added

- A single fix can propose a rule; one-off-looking single-fix rules are held back with a reason.

### Fixed

- Enforcement: closed ways around decisions and rules (HEAD moves, hook skips, heredocs), safer backstop.
- Ledger: override scope, dedup polarity, file-name scopes, glob separators.
- Rules: no self-certifying rules, one chat counts as one fix, confirmed hits needed to leave shadow.

## [0.2.2] - 2026-11-16

### Added

- Windows support (x64 and arm64), tested in CI on `windows-latest`.
- `ingrain rules propose --min-corrections`, default 1.

### Fixed

- Windows: editor arguments, path-like programs, executable check, pwsh unzip, scoped write token.
- Single-fix rules need hits on agent code; eval repeat stat fixed at 3; report when config raises
  the minimum.

## [0.2.1] - 2026-11-16

### Added

- Decision ledger merged into ingrain (`ingrain decisions ...`).
- Agent skills and a Claude Code plugin for one-step setup.

### Changed

- Learned rules moved under `ingrain rules ...` subcommands.

### Fixed

- Plugin: acts only in set-up repos, detects legacy repo hooks, safe AGENTS.md markers, and reports
  a broken repo config instead of going silent.

## [0.1.0] - 2026-11-15

### Added

- First release: learn lint rules from human corrections of agent code, enforce them in Claude Code
  hooks, pre-commit and CI. Prebuilt binaries and a GitHub Action.

[Unreleased]: https://github.com/WispAgents/ingrain/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/WispAgents/ingrain/compare/v0.2.4...v0.3.0
[0.2.4]: https://github.com/WispAgents/ingrain/compare/v0.2.3...v0.2.4
[0.2.3]: https://github.com/WispAgents/ingrain/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/WispAgents/ingrain/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/WispAgents/ingrain/compare/v0.1.0...v0.2.1
[0.1.0]: https://github.com/WispAgents/ingrain/releases/tag/v0.1.0
