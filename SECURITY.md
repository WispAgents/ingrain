# Security policy

## Supported versions

Only the latest release gets security fixes. Upgrade before reporting if you can.

| Version | Supported |
|---|---|
| latest `0.2.x` | yes |
| older | no |

## Reporting a vulnerability

**Do not open a public issue.** Report it privately through
[GitHub private vulnerability reporting](https://github.com/WispAgents/ingrain/security/advisories/new).

Include the ingrain version (`ingrain --version`), your OS, and steps or a repository that
reproduces it. We aim to reply within 3 working days, and to ship a fix or a mitigation within 30
days for confirmed issues. We'll credit you in the advisory unless you'd rather we didn't.

## Scope

In scope, among others:

- Ways to run code or commands through `ingrain` hooks, rule files or decision files that the
  user didn't approve
- Ways around enforcement that let an edit, commit or PR break a decision or rule set to `block`
- Leaking credentials or private repository data (ingrain never reads tokens itself; it shells out
  to `gh` and `claude`)
- Problems in the GitHub Action (`action/`) or the release artifacts

Out of scope: issues that need an attacker who already controls the repository's committed
`decisions/` or `.corrections/` files, or the machine running ingrain.
