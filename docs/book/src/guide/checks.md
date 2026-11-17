# Where checks run

`ingrain check` runs learned rules and decision checks together, on **added lines only**, so
existing code never blocks.

| Path | Command | On a hit |
|---|---|---|
| Commit | `ingrain check --staged` (`.git/hooks/pre-commit`) | the commit fails |
| Pull request | `ingrain check --diff base...HEAD --format github` ([action](https://github.com/WispAgents/ingrain/blob/main/action/action.yml)) | annotations; failing check |
| By hand | `ingrain check [paths]` | listed; exit 1 if something blocks |

Pre-commit and CI judge a change by the decisions, learned rules and config it started from, so a
branch can't retire a decision, delete a rule, set it to shadow or ignore its path, and break it in
the same diff. CI also flags any change to `decisions/` or `.corrections/`. Outside pull requests
the Action compares with the default branch. The pre-commit hook goes where git runs hooks
(`core.hooksPath`, or the main `.git/hooks` from a worktree).

To keep a line on purpose, add a comment with a reason, on the line or the line above:

```ts
// ingrain-ignore avoid-fetch-call: this endpoint streams the response body
// decisions-ignore D-0001: webhook replay needs raw fetch
const res = await fetch(url);
```

Without a reason it doesn't count. On the line above, only a line that is just the comment counts.
An agent adding one is asked to get the person's approval, and CI shows every override on added
lines as a warning annotation. Overrides are logged; `ingrain rules promote` and
`ingrain decisions stale` flag rules and decisions that are overridden often.
