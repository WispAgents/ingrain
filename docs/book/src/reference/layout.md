# Repository layout

```
decisions/D-NNNN.yml       accepted decisions          committed
decisions/pending/         proposed decisions          gitignored
.corrections/rules/        accepted learned rules      committed
.corrections/pending/      proposed rules              gitignored
.corrections/config.yml    thresholds, agent markers   committed
.git/ingrain/              corrections, hit logs, hook sessions, extraction state (local)
```

Personal stores ([local and global](../guide/personal-stores.md)) use the same layout under their
own directory, with `corrections/` in place of `.corrections/`. They are never committed:

```
.git/ingrain/local/                   --local: this repository only (shared by its worktrees)
  decisions/L-NNNN.yml                accepted decisions
  decisions/pending/                  proposed decisions
  corrections/rules/                  accepted learned rules
  corrections/pending/                proposed rules
  corrections/config.yml              thresholds, agent markers
~/.config/ingrain/                    --global: every repository ($INGRAIN_HOME overrides)
  decisions/G-NNNN.yml
  corrections/rules/
  ...
.claude/settings.local.json           hooks from `init --local` (added to .git/info/exclude)
~/.claude/settings.json               hooks from `init --global` ($CLAUDE_CONFIG_DIR overrides)
```
