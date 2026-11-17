# Decision file format

One YAML file per decision, `decisions/D-0001.yml`, reviewed like code:

```yaml
id: D-0001
decision: Billing code calls the payments client, never raw fetch.
reason: the client adds idempotency keys
rejected:
- a shared fetch wrapper per module
scope:
- src/billing/**          # path globs, or symbol / module names like `apiClient`
category: convention      # convention | constraint | api_change | architecture | scope
status: accepted          # accepted | superseded (proposals live in decisions/pending/)
checks:                   # optional; without checks a decision is shown, not enforced
  forbid_imports: [axios]
  forbid_deps: [redis]    # package.json, pyproject.toml, requirements*.txt, Cargo.toml, go.mod
  forbid_patterns:
  - language: TypeScript
    pattern: fetch($$$A)  # ast-grep pattern
sources:
- https://github.com/acme/app/pull/214
```

| Field | Meaning |
|---|---|
| `id` | `D-NNNN` once accepted; proposals in `decisions/pending/` use `P-…` ids |
| `decision` | The rule, as one sentence |
| `reason` | Why; shown to the agent next to the decision |
| `rejected` | Alternatives considered and turned down |
| `scope` | Path globs, or symbol / module names. No scope, or `**`, is repo-wide |
| `category` | `convention`, `constraint`, `api_change`, `architecture` or `scope` |
| `status` | `accepted` or `superseded` |
| `checks` | Optional. Without checks a decision is shown, not enforced |
| `sources` | Where the decision came from (PRs, docs, commits) |

Agents never edit these files. They propose with `ingrain decisions add --propose` or
`ingrain decisions supersede`, and a person accepts with `ingrain decisions review`.
