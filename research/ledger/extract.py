#!/usr/bin/env python3
"""Decision extractor: the candidate product component under test.

    python3 -I extract.py <in.jsonl> <out.jsonl> [batch_size]

Input records: {"id", "repo", "kind", "author", "time", "text"}. Output: one line per input
id with {"id", "records": [{"decision", "reason", "scope", "category"}]}. Uses `claude -p`
(the user's Claude Code login) with no tools, in an empty temp dir.
"""
import concurrent.futures as cf
import datetime
import json
import re
import subprocess
import sys
import tempfile

MODEL = "claude-opus-5-5"
PROMPT = """You maintain a "decision ledger" for a software repository: short records of durable project
decisions that future code changes (often written by AI coding agents) must follow.

Below are texts from the repository {repo}: PR descriptions, review comments, issues, commit messages
or docs. For each text, extract the durable decisions it states, if any. A decision is something
future code should keep doing or avoid, for example:
- a convention ("use `apiClient` instead of raw fetch", "tests use anyio.run, not @pytest.mark.anyio")
- a constraint ("billing must not import from api/", "support Python 3.10")
- an API change that callers must follow ("ClaudeCodeOptions is renamed to ClaudeAgentOptions")
- an architecture or scope choice ("we stay on SQLite", "this feature is out of scope for the SDK")

Not decisions: bug reports, questions, status updates, thanks, one-off fixes that only apply to the
change under discussion, descriptions of what a PR does with no lasting rule.

For each decision give:
- "decision": one imperative sentence an agent can follow
- "reason": why, in one short sentence (empty if not stated)
- "scope": list of path globs, module or symbol names it applies to (best guess; ["**"] if repo-wide)
- "category": convention | constraint | api_change | architecture | scope

Most texts contain no decision; return an empty list for them. Reply with only JSON:
{{"results": [{{"id": "<text id>", "records": [...]}}, ...]}} covering every id below.

{texts}"""


def ask(prompt):
    with tempfile.TemporaryDirectory() as d:
        out = subprocess.run(
            ["claude", "-p", "--tools", "", "--no-session-persistence", "--output-format", "text", "--model", MODEL],
            input=prompt, capture_output=True, text=True, cwd=d, timeout=900)
    if out.returncode != 0:
        raise RuntimeError(out.stderr.strip()[:300])
    return out.stdout


def parse(reply):
    m = re.search(r"\{.*\}", reply, re.S)
    return json.loads(m.group(0))["results"] if m else []


def run_batch(batch):
    repo = batch[0]["repo"]
    texts = "\n\n".join(
        f"### id: {t['id']} · {t['kind']} · {t.get('author','')} · "
        f"{datetime.date.fromtimestamp(t['time']).isoformat() if t.get('time') else ''}\n{t['text'][:3000]}"
        for t in batch)
    for attempt in range(2):
        try:
            results = {r["id"]: r.get("records", []) for r in parse(ask(PROMPT.format(repo=repo, texts=texts)))}
            return [{"id": t["id"], "records": results.get(t["id"], []), "missing": t["id"] not in results} for t in batch]
        except Exception as e:  # noqa: BLE001 - report and retry once
            err = str(e)
    return [{"id": t["id"], "records": [], "error": err} for t in batch]


def main(src, dst, batch_size=8):
    items = [json.loads(l) for l in open(src)]
    by_repo = {}
    for it in items:
        by_repo.setdefault(it["repo"], []).append(it)
    batches = [v[i:i + batch_size] for v in by_repo.values() for i in range(0, len(v), batch_size)]
    with cf.ThreadPoolExecutor(4) as pool, open(dst, "w") as f:
        for res in pool.map(run_batch, batches):
            for r in res:
                f.write(json.dumps(r) + "\n")
                f.flush()
    print("done", len(items), "texts in", len(batches), "calls")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2], int(sys.argv[3]) if len(sys.argv) > 3 else 8)
