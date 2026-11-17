# Install

```sh
brew install wispagents/tap/ingrain   # macOS and Linux
# or from source, any OS:
cargo install --locked --git https://github.com/WispAgents/ingrain --tag v0.2.4 ingrain
```

Prebuilt binaries for macOS, Linux and Windows (x64 and arm64) are on the
[releases page](https://github.com/WispAgents/ingrain/releases/latest). On Windows, unzip
`ingrain.exe` into a folder on your `PATH`. Claude Code runs hooks through Git Bash there, and
Git for Windows runs the pre-commit hook, so both work as on macOS and Linux.

Needs `git`. Optional: `gh` (logged in) to read GitHub PRs, reviews and issues; Claude Code
(`claude`) or `ANTHROPIC_API_KEY` for extraction and for rules templates can't express.
