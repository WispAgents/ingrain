---
name: ingrain-decide
description: Record, change or look up a project decision in the repo's ingrain ledger (decisions/). Use when the user states a lasting rule for the codebase ("we don't use X", "always do Y", "from now on…"), asks to record, change or drop a decision, or when an edit was refused by an ingrain decision.
---

# Record or change a decision

Decisions live in `decisions/D-NNNN.yml`. Agents never edit those files; they propose, and a person
accepts. The `ingrain` binary must be installed (see the `ingrain-setup` skill).

## Look up what applies

```sh
ingrain decisions for <path>      # decisions that apply to a file
ingrain decisions list            # all live decisions
ingrain decisions show D-0012     # one in full
```

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

Other checks: `--forbid-import node-fetch`, `--forbid-dep redis`. `--category` takes `convention`,
`constraint`, `api_change`, `architecture` or `scope`.

Then show the user the proposal and ask them to accept it. If they say yes, run
`ingrain decisions accept <P-id>` (it stages the file; commit it with their other changes).
Don't accept without the user's explicit OK.

## Change or drop a decision

```sh
ingrain decisions supersede D-0012 --decision "<the new decision>" --reason "<why it changed>"
```

The old decision stays in force until the user accepts the replacement (`ingrain decisions accept
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
