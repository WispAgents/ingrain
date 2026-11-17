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

## Where to go next

- New here? [Install](install.md) ingrain and follow the [quick start](quick-start.md).
- Using Claude Code, Codex, Cursor or another agent? See [Use it from your coding agent](agents.md).
- Not sure which kind of rule you need? Read [Decisions or learned rules?](guide/decisions-vs-rules.md).
- Looking for a flag? The [CLI reference](reference/cli.md) is generated from the binary itself.
