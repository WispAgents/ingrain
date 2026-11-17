# What the agent sees (Claude Code)

`ingrain init` adds one hook command per event to `.claude/settings.json` (`ingrain hook <event>`).
Each is a no-op for teammates who don't have ingrain installed, and all of them fail open.

| Hook | What happens |
|---|---|
| `SessionStart` | Repo-wide decisions go into the session, plus how to propose a new one |
| `PreToolUse` on Edit / Write / MultiEdit / NotebookEdit | The edit is simulated first. If an added line breaks a decision's check, the edit is **denied** with the decision and the way out. An edit that adds an `ingrain-ignore` / `decisions-ignore` comment asks the person first. Otherwise the decisions scoped to that file are shown, each once per session |
| `PreToolUse` on Bash | A command that could write accepted decisions, rules or `.corrections/config.yml` (`sed -i`, `>`, `mv`, `rm`, …), or that accepts, rejects or retunes them (`ingrain rules mode`, `ingrain decisions accept`, …), or `git commit --no-verify`, asks the person first |
| `PostToolUse` on Edit / Write / MultiEdit / NotebookEdit / Bash | Learned rules run on the edited file, or on the files the shell command changed (decisions too, since a command can't be simulated). Warn: a note to the agent. Block: the agent must fix it, with the past human fix shown |
| `Stop` | Rules and decisions are re-checked on the files this session edited. Something broken sends the turn back, up to `stop_max_attempts` times; after that the turn ends and the person is told |

Agents can't edit accepted decisions, rules or `.corrections/config.yml` (a broken config falls back
to the defaults in hooks, so it can't switch them off); they propose with `ingrain decisions add --propose`
or `ingrain decisions supersede`, and a person accepts. The one exception is first setup: until a
commit on any branch holds a decision, rule or ingrain config, the agent setting ingrain up may accept
proposals and commit them, so "set up ingrain in this repo" finishes in a headless session too. People
review that commit like any other; `--no-verify` is still asked.

Tested live with Claude Code 2.1.322: asked to write raw `fetch` in `src/billing/`, the write was
denied before the file changed, and the agent explained the decision instead of working around it.
