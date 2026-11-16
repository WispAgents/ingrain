# AGENTS.md

<!-- ingrain:start -->
## Decisions and learned rules (ingrain)

This repo keeps rules agents must follow, managed by [ingrain](https://github.com/WispAgents/ingrain):
accepted decisions in `decisions/` and lint rules learned from past corrections in `.corrections/rules/`.

- Before editing a file, run `ingrain decisions for <path>` and follow what it lists.
- Before finishing, run `ingrain check` and fix what it reports. If a flagged line is right as written,
  keep it and add `decisions-ignore <id>: <reason>` or `ingrain-ignore <rule>: <reason>` on that line.
- Never edit `decisions/*.yml` or `.corrections/rules/*.yml`. When the user states a lasting decision,
  propose it: `ingrain decisions add --propose "<decision>" --reason "<why>" --scope <glob>`.
  To change one: `ingrain decisions supersede <id> --decision "<new>" --reason "<why>"`. A person accepts.
<!-- ingrain:end -->
