---
name: ingrain-decide
description: Record, change or look up a project decision in the repo's ingrain ledger (decisions/). Use when the user states a lasting rule for the codebase ("we don't use X", "always do Y", "from now on…"), asks to record, change or drop a decision, or when an edit was refused by an ingrain decision.
---

# Record or change a decision

Decisions live in `decisions/D-NNNN.yml`. Agents never edit those files; they propose, and a person
accepts. The `ingrain` binary must be installed (see the `ingrain-setup` skill).

## Decision or learned rule?

ingrain has two kinds of rules. This skill covers **decisions**: what someone states up front
("we don't use Redis", "billing calls the payments client"), stored as a sentence with a reason,
a scope and optional checks. A decision without checks is shown before edits in scope; with checks,
a breaking edit is denied before it lands. **Learned rules** (`.corrections/rules/`) are ast-grep
lint rules mined from corrections people made to agent code. They run after edits in shadow, warn
or block mode. They belong to the `ingrain-rules` skill.

When the user states a rule, record a decision, even if it could also be a lint rule. Learned rules
only come from `ingrain rules mine` / `ingrain rules propose`.

## Look up what applies

```sh
ingrain decisions for <path>      # decisions that apply to a file
ingrain decisions list            # all live decisions
ingrain decisions show D-0012     # one in full
```

For the user, `ingrain viewer` opens the same decisions in a desktop window, with their history and
an inbox to accept or reject proposals. Suggest it to them; don't use it yourself.

These include the user's personal decisions: `L-` ids (this repository only, kept in
`.git/ingrain/local/`) and `G-` ids (every repository, kept in `~/.config/ingrain/`). Follow them like
`D-` ones; they are never committed, so don't look for them in `decisions/`.

## Record a new decision

When the user states something future code must follow, turn it into one record:

- **decision:** one imperative sentence an agent can follow ("Billing code calls the payments client, never raw fetch.")
- **reason:** why, in one short sentence, in the user's terms
- **scope:** where it applies: path globs (`src/billing/**`), a directory, or a symbol name (`apiClient`). Leave it out for repo-wide.
- **checks:** only when it can be checked mechanically; a decision without checks is shown to agents but never blocks

```sh
ingrain decisions add --propose "Billing code calls the payments client, never raw fetch." \
  --reason "the client adds idempotency keys" \
  --scope "src/billing/**" \
  --forbid-pattern 'TypeScript:fetch($$$A)'
```

Where it goes: by default, the repository's own store (`decisions/`, committed, ids `D-`). If the
user says it's their own preference for every repository ("I always want…", "in all my projects"),
add `--global` (ids `G-`, kept in `~/.config/ingrain/`). If they want it for this repository but not
committed, add `--local` (ids `L-`, kept in `.git/ingrain/local/`). Put the flag right after
`ingrain`: `ingrain --global decisions add --propose "…"`. In a repo set up with
`ingrain init --local`, plain `add` already goes to the local store.

Other checks: `--forbid-import node-fetch`, `--forbid-dep redis`. `--category` takes `convention`,
`constraint`, `api_change`, `architecture` or `scope`.

Then show the user the proposal and ask them to accept it. If they say yes, run
`ingrain decisions accept <P-id>` (it stages the file; commit it with their other changes).
Don't accept without the user's explicit OK. Once the repo's decisions are committed, Claude Code
asks the person to approve `accept` itself; in a session with no one to approve (headless, `-p`),
the hook refuses it: give the user the exact `ingrain decisions accept <P-id>` command to run.

## Change or drop a decision

```sh
ingrain decisions supersede D-0012 --decision "<the new decision>" --reason "<why it changed>"
```

This works the same for `L-` and `G-` ids: the proposal goes to the same store, and
`accept` finds it there. The old decision stays in force until the user accepts the replacement (`ingrain decisions accept
<P-id>`); accepting retires the old one. To drop a decision entirely, supersede it with one that
says so, so the history keeps the reason.

## When an edit is refused

If the hook refuses an edit because it breaks a decision:

1. Don't work around it (other syntax, another file, a script that edits the file).
2. Rewrite the code to follow the decision, if that does what the user asked.
3. If the user's request really conflicts with the decision, stop and tell them which decision and
   why it applies, and offer to propose a supersede.
4. If one specific line must break the decision on purpose, the user can approve a comment on that
   line: `decisions-ignore D-0012: <reason>`.

## Before finishing

Run `ingrain check` and fix what it reports.
