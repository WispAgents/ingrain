#!/usr/bin/env python3
"""Judge extracted decision records against their source text.

    python3 -I judge_records.py <records.md> <out.jsonl>

Labels per record: durable (a real project decision future code should follow, supported by the
text), one_off (true, but only about that one change), wrong (not supported by the text).
"""
import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from judge import ask  # noqa: E402

RUBRIC = """You are auditing a "decision ledger" extractor. For each source text below, it extracted
records meant to be durable project decisions that future code changes (often by AI agents) must follow.

Rate every record:
- "durable": a real decision about how this project's code should be written going forward
  (a convention, constraint, API change, architecture or scope choice), clearly supported by the text.
- "one_off": true to the text, but only about the specific change under discussion; injecting it into
  future agent sessions would be noise.
- "wrong": not supported by the text, or misstates it.
Also rate "scope": "ok" if its scope plausibly names where the decision applies, else "bad".

Reply with only JSON Lines, one per record:
{"id": "<source id>", "index": <record number>, "label": "durable|one_off|wrong", "scope": "ok|bad", "why": "<max 15 words>"}

"""


def main(src, dst):
    text = pathlib.Path(src).read_text()
    chunks = text.split("\n---\n")
    half = len(chunks) // 2
    out = []
    for part in ("\n---\n".join(chunks[:half]), "\n---\n".join(chunks[half:])):
        reply = ask(RUBRIC + part)
        out += [json.loads(l) for l in reply.splitlines() if l.strip().startswith("{")]
    with open(dst, "w") as f:
        for r in out:
            f.write(json.dumps(r) + "\n")
    print("judged", len(out))


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
