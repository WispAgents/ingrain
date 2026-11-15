# Judging instructions

Each item in your batch file is one real correction from a public GitHub repo: code an AI coding
agent wrote ("Agent code (before)") and what a human later replaced it with ("Human replacement
(after)"). Then:
- "Stated reason for the fix": the fix commit message, fix PR, and review comments. These were
  written AFTER the agent code. Use them only to understand WHY the human changed it.
- "Candidate prior records": texts dated BEFORE the agent commit (PR descriptions, review comments,
  issues, issue comments, commit messages, docs at the agent commit). Found by keyword search, so
  many are irrelevant.

For each item decide:

1. `kind`: what the correction is.
   - `decision`: the fix applies a durable project rule, convention, constraint, API change or scope
     choice that would apply to other code too (e.g. "use the @sentry/conventions constants", "use
     ClaudeAgentOptions not ClaudeCodeOptions", "don't add this dependency", "this belongs in X").
   - `bugfix`: a logic fix specific to this code.
   - `taste`: simplification, naming, readability.
   - `evolution`: requirements changed after the agent wrote it, a rename the agent couldn't know,
     content/text edits, feature removed.
   - `other`: can't tell.
2. `prior`: was the reason for this fix recorded BEFORE the agent commit, in one of the candidates?
   - `yes`: a candidate states the rule or intent specifically enough that an agent that had read it
     would have avoided this mistake.
   - `partial`: a candidate is clearly related, but you'd need inference to get from it to this fix.
   - `no`: nothing in the candidates records it.
   Judge only from the candidates shown. Don't use the "stated reason" texts as prior evidence.
3. `source`: the candidate id in brackets you relied on (e.g. `is18895`, `c1a2b3c4d5e`,
   `doc:CLAUDE.md`, `agent_pr`), or "" for no.
4. `where`: one of `agent_doc` (a doc tagged "agent-loaded doc"), `doc`, `commit`, `pr`, `review`,
   `issue`, `issue_comment`, `agent_pr`, `none`.
5. `decision`: if kind is `decision` or prior is not `no`, the rule in one imperative sentence
   (else "").
6. `why`: at most 20 words.

Be strict: "yes" needs a specific statement, not just the same topic.

Output one JSON object per item, in order, as JSON Lines:
{"key": "<the # heading>", "kind": ..., "prior": ..., "source": ..., "where": ..., "decision": ..., "why": ...}
Every item in the batch must get a line.
