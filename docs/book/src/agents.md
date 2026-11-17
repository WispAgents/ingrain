# Use it from your coding agent

With `ingrain` installed, give your agent the ingrain skills, then ask it to **"set up ingrain in
this repo"**. The `ingrain-setup` skill runs `ingrain init` (and installs the binary first if it's
missing, after asking), turns the rules in your AGENTS.md / CONTRIBUTING.md into decisions for
you to approve, and commits on a branch.

**Claude Code** (plugin: skills + hooks):

```
/plugin marketplace add WispAgents/ingrain
/plugin install ingrain@ingrain
```

**Any agent** (Claude Code, Codex, Cursor, Copilot, Gemini CLI, opencode and
[more](https://github.com/vercel-labs/skills)):

```sh
npx skills add WispAgents/ingrain
```

| Skill | Use it for |
|---|---|
| `ingrain-setup` | Install and set up ingrain in a repo, seed decisions from your docs |
| `ingrain-decide` | "From now on we don't use X": record, change or look up a decision |
| `ingrain-rules` | Learn rules from past corrections, review them, set shadow / warn / block |

The plugin's hooks act only in repos set up with `ingrain init`, and step aside where the repo's
own `.claude/settings.json` already runs ingrain (no double checks). Without the `ingrain` binary
they do nothing. Agents without hooks get an ingrain section in AGENTS.md (`ingrain init --agents-md`) and are caught by pre-commit and CI.
