#!/usr/bin/env python3
"""Fetch the discussion corpus of a GitHub repo: PRs, review comments, issues, issue comments.

    python3 -I fetch.py owner/name out_dir [max_pages]

Writes <out_dir>/<owner>_<name>.jsonl, one record per text:
{"id", "kind": pr|review|issue|issue_comment, "time", "author", "text", "url", "pr"?, "path"?}
"""
import json
import pathlib
import subprocess
import sys
from datetime import datetime


def ts(s):
    return int(datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp())


def gh(path):
    out = subprocess.run(["gh", "api", path], capture_output=True, text=True)
    if out.returncode != 0:
        print(f"  gh api {path}: {out.stderr.strip()[:200]}", file=sys.stderr)
        return []
    return json.loads(out.stdout)


def pages(path, max_pages):
    sep = "&" if "?" in path else "?"
    for page in range(1, max_pages + 1):
        batch = gh(f"{path}{sep}per_page=100&page={page}")
        yield from batch
        if len(batch) < 100:
            return


def login(x):
    return (x.get("user") or {}).get("login", "")


def main(repo, out_dir, max_pages=30):
    out = []
    for p in pages(f"repos/{repo}/pulls?state=all&sort=created&direction=desc", max_pages):
        text = f"{p['title']}\n\n{p.get('body') or ''}".strip()
        out.append(dict(id=f"pr{p['number']}", kind="pr", time=ts(p["created_at"]), author=login(p),
                        text=text, url=p["html_url"], pr=p["number"]))
    for c in pages(f"repos/{repo}/pulls/comments?sort=created&direction=desc", max_pages):
        pr = int(c["pull_request_url"].rsplit("/", 1)[1])
        out.append(dict(id=f"rc{c['id']}", kind="review", time=ts(c["created_at"]), author=login(c),
                        text=c.get("body") or "", url=c["html_url"], pr=pr, path=c.get("path")))
    for i in pages(f"repos/{repo}/issues?state=all&sort=created&direction=desc", max_pages):
        if "pull_request" in i:
            continue
        text = f"{i['title']}\n\n{i.get('body') or ''}".strip()
        out.append(dict(id=f"is{i['number']}", kind="issue", time=ts(i["created_at"]), author=login(i),
                        text=text, url=i["html_url"]))
    for c in pages(f"repos/{repo}/issues/comments?sort=created&direction=desc", max_pages):
        num = int(c["issue_url"].rsplit("/", 1)[1])
        out.append(dict(id=f"ic{c['id']}", kind="issue_comment", time=ts(c["created_at"]), author=login(c),
                        text=c.get("body") or "", url=c["html_url"], pr=num))
    path = pathlib.Path(out_dir) / (repo.replace("/", "_") + ".jsonl")
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w") as f:
        for r in out:
            f.write(json.dumps(r) + "\n")
    kinds = {}
    for r in out:
        kinds.setdefault(r["kind"], []).append(r["time"])
    summary = {k: (len(v), datetime.fromtimestamp(min(v)).date().isoformat()) for k, v in kinds.items()}
    print(repo, summary)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2], int(sys.argv[3]) if len(sys.argv) > 3 else 30)
