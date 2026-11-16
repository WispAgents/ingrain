# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Contribution guide, code of conduct, security policy, issue and PR templates, CODEOWNERS.
- Dependabot for Cargo and GitHub Actions, `cargo deny` checks in CI, declared MSRV.

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

[Unreleased]: https://github.com/WispAgents/ingrain/compare/v0.2.3...HEAD
[0.2.3]: https://github.com/WispAgents/ingrain/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/WispAgents/ingrain/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/WispAgents/ingrain/compare/v0.1.0...v0.2.1
[0.1.0]: https://github.com/WispAgents/ingrain/releases/tag/v0.1.0
