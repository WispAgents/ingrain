# Contributing

```sh
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all --locked
```

Crates: `core` (data model, rule files, git), `engine` (ast-grep), `learn` (capture, cluster, rule
writing, backtest, eval), `ledger` (decisions, scope, checks, extraction), `cli` (the `ingrain`
binary). [PLAN.md](https://github.com/WispAgents/ingrain/blob/main/PLAN.md) is the working plan.

See [CONTRIBUTING.md](https://github.com/WispAgents/ingrain/blob/main/CONTRIBUTING.md) before opening a PR, [SECURITY.md](https://github.com/WispAgents/ingrain/blob/main/SECURITY.md) to report a
vulnerability, and [CHANGELOG.md](https://github.com/WispAgents/ingrain/blob/main/CHANGELOG.md) for release notes. This project follows a
[Code of Conduct](https://github.com/WispAgents/ingrain/blob/main/CODE_OF_CONDUCT.md).

License: Apache-2.0.

## Docs

This book lives in `docs/book/`. The [CLI reference](reference/cli.md) is generated from the
`clap` definitions; regenerate it after changing a command or flag:

```sh
cargo run -q -p ingrain -- --markdown-help > docs/book/src/reference/cli.md
mdbook serve docs/book   # http://localhost:3000
```

CI fails when `reference/cli.md` is out of date. Pushes to `main` deploy the book to GitHub Pages.
