# Install

```sh
brew install wispagents/tap/ingrain   # macOS and Linux
# or from source, any OS:
cargo install --locked --git https://github.com/WispAgents/ingrain --tag v0.4.0 ingrain
# on Linux without the GUI libraries, or Rust older than 1.99: add --no-default-features
```

`ingrain viewer`, the desktop window, is in the macOS and Windows binaries. The Linux prebuilt binaries leave the viewer out, so `ingrain` runs in any container or CI
image with nothing extra installed. For the viewer on a Linux desktop, build from source (Rust 1.99
or newer, plus `libxkbcommon-dev` and `libwayland-dev`):
`cargo install --locked --git https://github.com/WispAgents/ingrain ingrain`.
Building from source with the viewer needs Rust 1.99; `--no-default-features` builds without it on
Rust 1.90.

Prebuilt binaries for macOS, Linux and Windows (x64 and arm64) are on the
[releases page](https://github.com/WispAgents/ingrain/releases/latest). On Windows, unzip
`ingrain.exe` into a folder on your `PATH`. Claude Code runs hooks through Git Bash there, and
Git for Windows runs the pre-commit hook, so both work as on macOS and Linux.

Needs `git`. Optional: `gh` (logged in) to read GitHub PRs, reviews and issues; Claude Code
(`claude`) or `ANTHROPIC_API_KEY` for extraction and for rules templates can't express.
