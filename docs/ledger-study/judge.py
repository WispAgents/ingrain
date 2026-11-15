#!/usr/bin/env python3
"""Judge context packs with `claude -p`, following JUDGE.md.

    python3 -I judge.py <packs.jsonl> <out.jsonl> [per_call]
"""
import concurrent.futures as cf
import importlib.util
import json
import pathlib
import re
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).parent
MODEL = "claude-opus-5-5"
spec = importlib.util.spec_from_file_location("pack", HERE / "pack.py")
pack = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pack)


def ask(prompt):
    with tempfile.TemporaryDirectory() as d:
        out = subprocess.run(
            ["claude", "-p", "--tools", "", "--no-session-persistence", "--output-format", "text", "--model", MODEL],
            input=prompt, capture_output=True, text=True, cwd=d, timeout=1200)
    if out.returncode != 0:
        raise RuntimeError(out.stderr.strip()[:300])
    return out.stdout


def judge(batch, instructions):
    items = "".join(pack.render(p) for p in batch)
    prompt = (f"{instructions}\n\nInstead of writing a file, reply with only the JSON Lines, one line per item, "
              f"for these {len(batch)} items:\n\n{items}")
    for _ in range(2):
        try:
            lines = [json.loads(l) for l in ask(prompt).splitlines() if l.strip().startswith("{")]
            got = {l.get("key"): l for l in lines}
            if all(p["key"] in got for p in batch):
                return [got[p["key"]] for p in batch]
        except Exception as e:  # noqa: BLE001
            print("retry:", e, file=sys.stderr)
    return [{"key": p["key"], "error": True} for p in batch]


def main(src, dst, per_call=5):
    instructions = (HERE / "JUDGE.md").read_text()
    packs = [json.loads(l) for l in open(src)]
    done = set()
    if pathlib.Path(dst).exists():
        done = {json.loads(l)["key"] for l in open(dst) if '"error"' not in l}
    todo = [p for p in packs if p["key"] not in done]
    batches = [todo[i:i + per_call] for i in range(0, len(todo), per_call)]
    with cf.ThreadPoolExecutor(4) as pool, open(dst, "a") as f:
        for res in pool.map(lambda b: judge(b, instructions), batches):
            for r in res:
                f.write(json.dumps(r) + "\n")
                f.flush()
    print("judged", len(todo))


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2], int(sys.argv[3]) if len(sys.argv) > 3 else 5)
