# Decisions or learned rules?

|                | Decision                                                        | Learned rule                                                        |
|----------------|-----------------------------------------------------------------|---------------------------------------------------------------------|
| Comes from     | Someone saying what code must do, recorded or extracted         | A mistake people kept fixing in agent code, mined from history      |
| Holds          | A sentence, a reason, a scope, optional checks                  | An ast-grep pattern, backtested on past corrections                 |
| Covers         | Anything: architecture, conventions, dependencies, scope        | Only code patterns a lint rule can match                            |
| Enforced       | Shown before edits in scope; with checks, a breaking edit is denied before it lands | Runs after edits, in shadow → warn → block mode, promoted from its hit log |
| Changed with   | `ingrain decisions supersede`                                   | `ingrain rules mode`, `ingrain rules promote`                       |
| Per-line override | A comment naming the decision id ([how](checks.md))      | A comment naming the rule id ([how](checks.md))             |

Use a decision when you can say the rule up front, especially when it isn't about code shape
("don't add Redis", "billing goes through the payments client"). Learned rules cover the rest: the
mistakes nobody wrote down until they had fixed them a few times. Both are accepted by a person,
can't be edited by agents, and are checked together by `ingrain check`, the pre-commit hook and CI.
