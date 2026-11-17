# Quick start

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

Next: [what the agent sees](guide/hooks.md) once ingrain is set up, and [where checks run](guide/checks.md).
