#!/usr/bin/env python3
"""Build a demo repository with decisions, proposals, learned rules and a hit log, for
screenshots and trying the viewer: `python3 viewer/scripts/demo-repo.py /tmp/harbor`."""

import json, os, shutil, subprocess, sys, time

root = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else "target/demo-harbor")
shutil.rmtree(root, ignore_errors=True)
os.makedirs(root)


def w(rel, text):
    p = os.path.join(root, rel)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "w") as f:
        f.write(text.lstrip("\n"))


def git(*args, date=None):
    env = dict(os.environ, GIT_AUTHOR_NAME="Rima Haddad", GIT_AUTHOR_EMAIL="rima@example.com",
               GIT_COMMITTER_NAME="Rima Haddad", GIT_COMMITTER_EMAIL="rima@example.com")
    if date:
        env["GIT_AUTHOR_DATE"] = env["GIT_COMMITTER_DATE"] = date + "T10:00:00Z"
    subprocess.run(["git", *args], cwd=root, env=env, check=True, capture_output=True)


git("init", "-q", "-b", "main")
w("src/api/client.ts", "export const apiClient = { get: (u: string) => fetch(u) };\n")
w("src/billing/charge.ts", "import { apiClient } from '../api/client';\nexport async function charge() { return apiClient.get('/charge'); }\n")
w("src/ui/Button.tsx", "export function Button() { return null; }\n")
w(".gitignore", "decisions/pending/\n.corrections/pending/\n")

D = "decisions/"
w(D + "D-0001.yml", """
id: D-0001
decision: Call the backend with `fetch` directly from components.
reason: Fewer layers while the API is small.
scope: ['src/**']
category: architecture
status: accepted
sources: ['https://github.com/harbor/web/pull/12']
author: rima
date: 2026-03-04
""")
git("add", "."); git("commit", "-qm", "docs: record D-0001 fetch from components", date="2026-03-04")

w(D + "D-0002.yml", """
id: D-0002
decision: 'Keep money as integer cents (`amountCents: number`), never floats.'
reason: Floats lost a cent on refunds in March; integers round-trip exactly.
rejected:
  - storing dollars as `number`
  - a decimal library everywhere
scope: ['src/billing/**', 'amountCents']
category: constraint
status: accepted
checks:
  forbid_patterns:
    - language: TypeScript
      pattern: parseFloat($A)
sources: ['https://github.com/harbor/web/pull/31', 'https://github.com/harbor/web/issues/29']
author: omar
date: 2026-04-18
""")
w(D + "D-0003.yml", """
id: D-0003
decision: Use the design system's `Button`; don't style raw `<button>` elements.
reason: Focus rings and dark mode come for free, and audits stay green.
scope: ['src/ui/**', '*.tsx']
category: convention
status: accepted
sources: ['docs/ui.md']
author: lena
date: 2026-06-02
""")
git("add", "."); git("commit", "-qm", "docs: money in cents, design-system buttons", date="2026-06-02")

w(D + "D-0001.yml", """
id: D-0001
decision: Call the backend with `fetch` directly from components.
reason: Fewer layers while the API is small.
scope: ['src/**']
category: architecture
status: superseded
superseded_by: D-0004
sources: ['https://github.com/harbor/web/pull/12']
author: rima
date: 2026-03-04
""")
w(D + "D-0004.yml", """
id: D-0004
decision: Go through `apiClient` for every backend call; no `fetch` in components.
reason: Retries, auth headers and tracing live in one place. Three outages traced to hand-rolled fetches.
rejected:
  - axios
  - keep fetch and add a lint rule
scope: ['src/**']
category: architecture
status: accepted
supersedes: D-0001
checks:
  forbid_imports: ['node-fetch', 'axios']
  forbid_deps: ['axios']
sources: ['https://github.com/harbor/web/pull/58', 'https://github.com/harbor/web/pull/58#discussion_r991']
author: rima
date: 2026-08-21
""")
git("add", "."); git("commit", "-qm", "docs: D-0004 replaces D-0001, all calls via apiClient", date="2026-08-21")

w(D + "D-0004.yml", open(os.path.join(root, D + "D-0004.yml")).read().replace(
    "scope: ['src/**']", "scope: ['src/**', 'scripts/**']"))
git("add", "."); git("commit", "-qm", "docs: D-0004 also covers scripts/", date="2026-09-30")

w(D + "D-0005.yml", """
id: D-0005
decision: Log with `logger` from `src/obs`, never `console.log` in shipped code.
reason: Logs need request ids to be searchable.
scope: ['src/**']
category: convention
status: accepted
checks:
  forbid_patterns:
    - language: TypeScript
      pattern: console.log($$$A)
sources: ['AGENTS.md']
author: omar
date: 2026-10-07
""")
w(D + "D-0006.yml", """
id: D-0006
decision: Feature flags are read once per request in middleware and passed down.
reason: Reading flags in components made renders inconsistent within one page.
scope: ['src/server/**', 'useFlag']
category: api_change
status: accepted
author: lena
date: 2026-11-03
""")
git("add", "."); git("commit", "-qm", "docs: logging and flags decisions", date="2026-11-03")

# Proposals waiting for review (gitignored).
w(D + "pending/P-3a9c01de.yml", """
id: P-3a9c01de
decision: Write new UI tests with Testing Library queries by role, not by test id.
reason: Role queries also check accessibility; test ids drift from what users see.
scope: ['src/**/*.test.tsx']
category: convention
status: proposed
sources: ['https://github.com/harbor/web/pull/77#discussion_r1204']
date: 2026-11-15
""")
w(D + "pending/P-77b2e410.yml", """
id: P-77b2e410
decision: Keep money as integer cents and format it only at the edge with `formatMoney`.
reason: Two screens formatted cents differently; one helper keeps them the same.
scope: ['src/billing/**', 'amountCents']
category: constraint
status: proposed
supersedes: D-0002
sources: ['https://github.com/harbor/web/pull/80']
date: 2026-11-16
""")

# Learned rules.
w(".corrections/config.yml", "min_support: 1\n")
w(".corrections/rules/no-raw-fetch.yml", """
id: no-raw-fetch
language: TypeScript
rule:
  pattern: fetch($$$ARGS)
message: Use apiClient instead of fetch
metadata:
  ingrain:
    mode: warn
    support: 6
    origin: template
    evidence:
      - https://github.com/harbor/web/commit/4be1f0a
      - https://github.com/harbor/web/pull/58#discussion_r991
    backtest: { before_hits: 6, befores: 6, after_hits: 0, afters: 6, kept_hits: 1, kept: 40, head_hits: 2 }
    example:
      before: "const r = await fetch(`/api/users/${id}`)"
      after: "const r = await apiClient.get(`/users/${id}`)"
""")
w(".corrections/rules/no-console-log.yml", """
id: no-console-log
language: TypeScript
rule:
  pattern: console.log($$$A)
message: Use logger.info, not console.log
metadata:
  ingrain:
    mode: shadow
    support: 3
    origin: template
    backtest: { before_hits: 3, befores: 3, after_hits: 0, afters: 3, kept_hits: 0, kept: 40, head_hits: 5 }
    example:
      before: "console.log('charged', amount)"
      after: "logger.info('charged', { amount })"
""")
w(".corrections/rules/no-float-money.yml", """
id: no-float-money
language: TypeScript
rule:
  pattern: parseFloat($A)
message: Money is integer cents; don't parseFloat it
metadata:
  ingrain:
    mode: block
    support: 4
    origin: llm
    backtest: { before_hits: 4, befores: 4, after_hits: 0, afters: 4, kept_hits: 0, kept: 40, head_hits: 0 }
""")
git("add", "."); git("commit", "-qm", "rules: accept no-raw-fetch, no-console-log, no-float-money", date="2026-11-10")

w(".corrections/pending/prefer-role-queries.yml", """
id: prefer-role-queries
language: Tsx
rule:
  pattern: getByTestId($A)
message: Query by role (getByRole) instead of test id
metadata:
  ingrain:
    mode: shadow
    support: 2
    origin: llm
    evidence: ['https://github.com/harbor/web/pull/77#discussion_r1204']
    backtest: { before_hits: 2, befores: 2, after_hits: 0, afters: 2, kept_hits: 3, kept: 52, head_hits: 14 }
    example:
      before: "screen.getByTestId('submit')"
      after: "screen.getByRole('button', { name: 'Submit' })"
""")

# A hit log spread over the last weeks.
now = int(time.time())
hits = []
for i, (rule, days, path, ev) in enumerate([
    ("no-raw-fetch", 1, "src/ui/Profile.tsx", "hit"), ("no-raw-fetch", 3, "src/ui/Cart.tsx", "hit"),
    ("no-raw-fetch", 9, "src/ui/Orders.tsx", "hit"), ("no-raw-fetch", 16, "src/ui/Search.tsx", "hit"),
    ("no-raw-fetch", 23, "src/ui/Admin.tsx", "hit"), ("no-raw-fetch", 24, "src/ui/Admin2.tsx", "hit"),
    ("no-raw-fetch", 40, "src/ui/Old.tsx", "hit"), ("no-raw-fetch", 2, "src/ui/Cart.tsx", "override"),
    ("no-console-log", 0, "src/billing/charge.ts", "hit"), ("no-console-log", 5, "src/server/a.ts", "hit"),
    ("no-console-log", 12, "src/server/b.ts", "hit"), ("no-console-log", 6, "src/server/a.ts", "override"),
    ("no-console-log", 7, "src/server/c.ts", "override"),
]):
    hits.append({"time": now - days * 86400 - i, "rule": rule, "path": path, "line": 3, "mode": "warn",
                 "event": ev, "context": "hook", "matched": f"m{i}"})
os.makedirs(os.path.join(root, ".git/ingrain"), exist_ok=True)
with open(os.path.join(root, ".git/ingrain/hits.jsonl"), "w") as f:
    f.write("\n".join(json.dumps(h) for h in hits) + "\n")

# Personal stores: this repository only (`--local`, inside .git), and every repository
# (`--global`, in $INGRAIN_HOME when set; skipped otherwise so a real ~/.config is never touched).
L = ".git/ingrain/local/"
w(L + "decisions/L-0001.yml", """
id: L-0001
decision: Run `pnpm test --filter billing` before pushing billing changes.
reason: The full suite takes nine minutes; billing's takes forty seconds.
scope: ['src/billing/**']
category: convention
status: accepted
date: 2026-11-12
""")
w(L + "decisions/pending/P-local01.yml", """
id: P-local01
decision: Keep scratch scripts in `scripts/tmp/`, never in `src/`.
status: proposed
date: 2026-11-16
""")
w(L + "corrections/rules/no-only-tests.yml", """
id: no-only-tests
language: TypeScript
rule:
  pattern: it.only($$$A)
message: Don't leave it.only in tests
metadata:
  ingrain:
    mode: warn
    support: 2
    origin: template
""")
home = os.environ.get("INGRAIN_HOME")
if home:
    def wh(rel, text):
        p = os.path.join(home, rel)
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w") as f:
            f.write(text.lstrip("\n"))
    wh("decisions/G-0001.yml", """
id: G-0001
decision: Prefer early returns over nested ifs.
reason: Flat code is easier to review.
category: convention
status: accepted
date: 2026-10-01
""")
    wh("decisions/G-0002.yml", """
id: G-0002
decision: Never commit `.env` files.
reason: Secrets leaked once from a demo branch.
scope: ['.env']
category: constraint
status: accepted
date: 2026-09-14
""")
    wh("corrections/rules/no-any.yml", """
id: no-any
language: TypeScript
rule:
  pattern: 'let $A: any'
message: Type it; don't use any
metadata:
  ingrain:
    mode: shadow
    support: 3
    origin: llm
""")

print(root)
