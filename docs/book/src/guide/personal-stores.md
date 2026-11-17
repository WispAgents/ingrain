# Just for you: local and global stores

Decisions and learned rules don't have to be committed. Next to the project's `decisions/` and
`.corrections/`, ingrain has two personal stores. Nothing in them is written to the working tree:

| Store | Where | Applies to | Decision ids |
|---|---|---|---|
| project (default) | `decisions/`, `.corrections/` | the repository, shared with the team | `D-0001` |
| `--local` | `.git/ingrain/local/` | this repository, for you only | `L-0001` |
| `--global` | `~/.config/ingrain/` | every repository on this machine | `G-0001` |

The global store is `$INGRAIN_HOME` when set, else `$XDG_CONFIG_HOME/ingrain`, else
`%APPDATA%\ingrain` on Windows and `~/.config/ingrain` elsewhere. Both variables must be absolute
paths; relative ones are ignored.

## Use ingrain in a repo without committing anything

```sh
ingrain init --local
ingrain decisions add "Use the fetch wrapper, not axios." --forbid-dep axios   # → L-0001
```

`init --local` creates `.git/ingrain/local/`, writes the Claude Code hooks to
`.claude/settings.local.json` and adds that file to `.git/info/exclude`. It also installs the git
pre-commit hook, which is local anyway. `git status` stays clean. All worktrees of the repository
share the local store. `--agents-md` is refused, because AGENTS.md is committed.

In a repo set up this way, commands without a flag write to the local store. A later plain
`ingrain init` still sets up the project store, if the team adopts ingrain.

## Your own rules, everywhere

```sh
ingrain init --global
ingrain --global decisions add "Prefer early returns over nested ifs."   # → G-0001
ingrain --global rules propose                                            # learned rules go there too
```

`init --global` works outside a git repository. It creates the global store and adds hooks to your
own Claude Code settings: `~/.claude/settings.json`, or `$CLAUDE_CONFIG_DIR/settings.json` when
that variable is set. Like the [plugin's](../agents.md) hooks, these:

- answer only in repositories set up for ingrain. Any global decision or rule counts, so they act
  in every repository once you have one.
- step aside where the repository's own `.claude/settings.json` or `settings.local.json` already
  runs ingrain, so nothing runs twice.

`init --global` doesn't add the hooks when your settings file already enables the ingrain plugin.
If you enabled the plugin somewhere else, pass `--no-claude`.

## How the stores combine

- **Reads see everything.** Hooks, `ingrain check`, `decisions for`, `decisions list` and
  `decisions stale` read the project, local and global stores together. If two stores have a rule
  with the same id, the first in project → local → global wins. `rules accept` refuses an id
  another store already has.
- **Writes go to one store.** `--local` / `--global` go right after `ingrain`, before the command
  (`ingrain --global decisions add …`). Without a flag, ingrain writes to the project store when
  the repo has `decisions/` or `.corrections/`. Otherwise it writes to the local store once
  `init --local` has run. Learned-rule commands (`rules propose`, `review`, `accept`, `reject`,
  `mode`, `promote`) use the flag the same way. `rules mine` stays per repository.
- **Ids pick the store.** `decisions supersede G-0001` puts the proposal in the global store.
  `decisions accept`, `reject` and `show` find the store from the id, so they need no flag.
  `decisions review` shows each proposal's store.
- **Config.** Each store has its own `corrections/config.yml`. A command uses the config of the
  store it writes to, which in a team repo is the project's.

## Checks and protection

- Personal stores aren't in git, so the pre-commit check reads them as they are. The project's
  rules and decisions still come from the base revision. CI never sees personal stores.
- A repository only gets the pre-commit hook from `ingrain init` or `ingrain init --local` run in
  it. Elsewhere, global decisions are enforced by the Claude Code hooks alone.
- Agents can't edit accepted files in a personal store: `decisions/*.yml`, `corrections/rules/`,
  `corrections/config.yml` and `rejected.yml`. They can't accept, reject or retune them either,
  and can't run `init --local` / `init --global`, without asking you. Proposals in `pending/` stay
  free. There is no [first-setup exception](hooks.md) for personal stores: nothing ever commits
  them, so nobody would review what an agent accepted.
- A repository that only sees your global decisions keeps its own `decisions/` folder (ADRs, say)
  editable.
- At session start, the agent is told that `L-` and `G-` ids are your personal decisions, and to
  record a decision with `--global` when you state a preference for every repository.
