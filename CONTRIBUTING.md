# Contributing to ingrain

Thanks for helping. This file covers how to report problems, propose changes and get a pull
request merged.

## Ground rules

- Be kind. This project follows the [Code of Conduct](CODE_OF_CONDUCT.md).
- Security issues go through [SECURITY.md](SECURITY.md), never a public issue.
- For anything bigger than a small fix, open an issue first so we can agree on the approach before
  you write the code. [PLAN.md](PLAN.md) lists the design decisions that are fixed unless that file
  changes.

## Development setup

You need Rust (stable, at least the `rust-version` in [Cargo.toml](Cargo.toml)) and `git`. `gh` and
`claude` are optional and only needed for the GitHub and LLM code paths.

```sh
git clone https://github.com/WispAgents/ingrain
cd ingrain
cargo build
cargo run -- --help
```

Crates:

| Crate | What it holds |
|---|---|
| `crates/core` | Data model, rule files, store layout, git helpers |
| `crates/engine` | ast-grep matching |
| `crates/learn` | Capture, clustering, rule writing, backtest, eval |
| `crates/ledger` | Decisions, scope, checks, extraction |
| `crates/cli` | The `ingrain` binary, hooks, end-to-end tests |

## Before you push

CI runs these on Linux, macOS and Windows, so run them locally first:

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all --locked
```

If you change dependencies, also run [`cargo deny check`](https://github.com/EmbarkStudios/cargo-deny)
(`cargo install cargo-deny`). It checks licenses, advisories and duplicate sources against
[deny.toml](deny.toml).

## Docs

The user docs are an [mdBook](https://rust-lang.github.io/mdBook/) in `docs/book/`, published to
GitHub Pages from `main`. `docs/book/src/reference/cli.md` is generated from the `clap`
definitions; regenerate it whenever you change a command, flag or help text (CI fails otherwise):

```sh
cargo run -q -p ingrain -- --markdown-help > docs/book/src/reference/cli.md
mdbook serve docs/book   # http://localhost:3000, needs `brew install mdbook` or `cargo install mdbook`
```

## Pull requests

- Branch from `main`. One topic per PR. Keep unrelated refactors out.
- Add or update tests for behaviour changes. End-to-end CLI tests live in `crates/cli/tests/`.
- Update [README.md](README.md) and the matching page in `docs/book/` when you change a command,
  flag, file layout or hook behaviour, and add a line under `Unreleased` in [CHANGELOG.md](CHANGELOG.md).
- `ingrain check` must never call a model, and checks only flag added lines. Keep it that way.
- Windows is supported: use `std::path` APIs rather than `/`-joined strings, and don't pass
  `\\?\` verbatim paths to git.
- Fill in the PR template. A maintainer from [CODEOWNERS](.github/CODEOWNERS) reviews every PR, and
  CI must be green before it merges.

### Commit messages

We use [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<optional scope>): <summary in the imperative, lower case>
```

Types: `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `ci`, `chore`, `release`. Common scopes:
`rules`, `ledger`, `enforce`, `plugin`, `windows`. Example: `fix(ledger): match file-name scopes`.

## Releasing (maintainers)

1. Bump the version in `Cargo.toml`, `Cargo.lock` (`cargo check` updates it),
   `.claude-plugin/plugin.json`, `action/action.yml`, `examples/ingrain-pr-check.yml`,
   `skills/ingrain-setup/SKILL.md` and `README.md`. `grep -rn <old version> --exclude-dir=target .`
   finds them all.
2. Move the `Unreleased` entries in `CHANGELOG.md` under the new version.
3. Commit as `release: vX.Y.Z`, open a PR and merge it.
4. Tag the merge commit `vX.Y.Z` and push the tag. The [release workflow](.github/workflows/release.yml)
   builds the binaries and publishes the GitHub release.
5. Update the formula in the Homebrew tap (`wispagents/tap`) with the new URLs and sha256 sums.

## License

By contributing, you agree that your contributions are licensed under the
[Apache License 2.0](LICENSE), the same license as the project.
