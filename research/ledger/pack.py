#!/usr/bin/env python3
"""Sample corrections and build a context pack for each: the stated reason for the fix,
and candidate texts written *before* the agent commit that might have recorded it.

    python3 -I pack.py <repos_dir> <corpus_dir> <out_dir> <per_repo> owner_name [owner_name ...]

Writes <out_dir>/packs.jsonl (structured) and <out_dir>/batch-<n>.md (for judges).
"""
import json
import pathlib
import random
import re
import subprocess
import sys

STOP = set("""
self this that with from import return const function async await true false none null undefined
string number boolean void type interface class def none args kwargs value values data item items
result results error errors test tests expect assert should when then else elif while for print
none list dict str int float bool any object array promise export default public private static
name names path file files line lines code json response request options config params param key
keys index count length new get set add update create delete remove make use used using also
""".split())
AGENT_DOCS = ("claude.md", "agents.md", ".cursorrules", "copilot-instructions.md", ".cursor/rules")
IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]{3,}")


def git(repo, *args):
    out = subprocess.run(["git", *args], cwd=repo, capture_output=True, text=True)
    return out.stdout if out.returncode == 0 else ""


def gh(path):
    out = subprocess.run(["gh", "api", path], capture_output=True, text=True)
    return json.loads(out.stdout) if out.returncode == 0 and out.stdout.strip() else []


def changed(s):
    lines = s["text"].split("\n")
    r = s["changed"]
    return "\n".join(lines[r["start"] - 1 : r["end"]]) if r["start"] <= r["end"] else ""


def keywords(c):
    before, after = set(IDENT.findall(changed(c["before"]))), set(IDENT.findall(changed(c["after"])))
    kw = {}
    for t in before ^ after:
        if t.lower() not in STOP:
            kw[t.lower()] = 2
    for t in before & after:
        if t.lower() not in STOP:
            kw.setdefault(t.lower(), 1)
    for t in IDENT.findall(c.get("note") or ""):
        if len(t) >= 5 and t.lower() not in STOP:
            kw.setdefault(t.lower(), 1)
    stem = pathlib.Path(c["path"]).stem.lower()
    if len(stem) >= 4 and stem not in STOP:
        kw.setdefault(stem, 1)
    return kw


def score(text, kw):
    low = text.lower()
    hits = {k for k in kw if re.search(r"\b" + re.escape(k) + r"\b", low)}
    return sum(kw[k] for k in hits), hits


def snippet(text, hits, width=600):
    low = text.lower()
    pos = min((low.find(h) for h in hits if low.find(h) >= 0), default=0)
    start = max(0, pos - width // 3)
    return ("…" if start else "") + text[start : start + width].strip() + ("…" if start + width < len(text) else "")


def clip(t, n):
    t = (t or "").strip()
    return t if len(t) <= n else t[:n] + "…"


def clip_lines(t, n=15):
    l = t.split("\n")
    return "\n".join(l[:n]) + ("\n…" if len(l) > n else "")


def commit_log(repo):
    out = git(repo, "log", "--no-merges", "--format=%H%x1f%at%x1f%an%x1f%B%x1e", "--max-count=20000", "HEAD")
    rows = []
    for rec in out.split("\x1e"):
        f = rec.strip("\n").split("\x1f")
        if len(f) == 4:
            rows.append(dict(sha=f[0], time=int(f[1]), author=f[2], text=f[3].strip()))
    return rows


def doc_hits(repo, rev, kw):
    strong = [k for k, w in kw.items() if w == 2][:12] or list(kw)[:8]
    if not strong:
        return []
    args = ["grep", "-n", "-i", "-w", "-C2"]
    for k in strong:
        args += ["-e", k]
    args += [rev, "--", "*.md", "*.mdc", ".cursorrules", "*.txt"]
    out = git(repo, *args)
    blocks = {}
    for line in out.splitlines():
        m = re.match(r"^[0-9a-f]+:(.+?)[:-](\d+)[:-](.*)$", line)
        if m:
            blocks.setdefault(m.group(1), []).append(m.group(3))
    hits = []
    for path, lines in blocks.items():
        text = "\n".join(lines)
        s, h = score(text, kw)
        hits.append(dict(path=path, score=s, hits=sorted(h), text=snippet(text, h, 500),
                         agent_loaded=any(a in path.lower() for a in AGENT_DOCS)))
    return sorted(hits, key=lambda d: -d["score"])[:3]


def pr_of(repo_name, sha, cache):
    if sha not in cache:
        prs = gh(f"repos/{repo_name}/commits/{sha}/pulls")
        cache[sha] = prs[0]["number"] if prs else None
    return cache[sha]


def main(repos_dir, corpus_dir, out_dir, per_repo, names):
    rnd = random.Random(42)
    out = pathlib.Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    packs = []
    for name in names:
        repo = pathlib.Path(repos_dir) / name
        repo_name = name.replace("_", "/", 1)
        corpus = [json.loads(l) for l in open(pathlib.Path(corpus_dir) / f"{name}.jsonl")]
        by_kind = {}
        for r in corpus:
            by_kind.setdefault(r["kind"], []).append(r["time"])
        # Every stream must reach back 60 days before the agent commit.
        coverage = max(min(v) for v in by_kind.values())
        corrections = [json.loads(l) for l in open(repo / ".git/ingrain/corrections.jsonl")]
        eligible = [c for c in corrections if c["agent_time"] >= coverage + 60 * 86400 and changed(c["before"]).strip()]
        sample = rnd.sample(eligible, min(per_repo, len(eligible)))
        log = commit_log(repo)
        prs = {r["pr"]: r for r in corpus if r["kind"] == "pr"}
        cache = {}
        for c in sample:
            kw = keywords(c)
            src = c["source"]
            agent_sha = src.get("agent_commit")
            fix_sha = src.get("fix_commit")
            agent_pr = pr_of(repo_name, agent_sha, cache) if agent_sha else None
            fix_pr = pr_of(repo_name, fix_sha, cache) if fix_sha else None
            # Texts written after the agent commit on the fix PR state why it was fixed.
            fix_reviews = [r for r in corpus if r["kind"] in ("review", "issue_comment")
                           and r.get("pr") == fix_pr and r["time"] >= c["agent_time"]]
            prior = [r for r in corpus if r["time"] < c["agent_time"]]
            scored = []
            for r in prior:
                s, h = score(r["text"], kw)
                if s >= 3:
                    scored.append((s, r, h))
            scored.sort(key=lambda x: -x[0])
            commits = []
            for row in log:
                if row["time"] >= c["agent_time"] or row["sha"] in (agent_sha, fix_sha):
                    continue
                s, h = score(row["text"], kw)
                if s >= 3:
                    commits.append((s, row, h))
            commits.sort(key=lambda x: -x[0])
            parent = (agent_sha + "^") if agent_sha else "HEAD"
            docs = doc_hits(repo, parent, kw)
            fix_msg = git(repo, "log", "-1", "--format=%B", fix_sha) if fix_sha else (c.get("note") or "")
            pack = dict(
                key=f"{name}:{c['id']}", repo=repo_name, correction=c["id"], path=c["path"],
                agent_time=c["agent_time"], fix_time=c["time"], source_kind=src["kind"],
                before=clip_lines(changed(c["before"])), after=clip_lines(changed(c["after"])),
                fix_message=clip(fix_msg, 800),
                fix_pr=(dict(number=fix_pr, text=clip(prs[fix_pr]["text"], 1200)) if fix_pr in prs else None),
                fix_reviews=[clip(r["text"], 400) for r in fix_reviews[:5]],
                agent_pr=(dict(number=agent_pr, text=clip(prs[agent_pr]["text"], 1000)) if agent_pr in prs else None),
                candidates=[dict(id=r["id"], kind=r["kind"], url=r["url"], author=r["author"],
                                 days_before=round((c["agent_time"] - r["time"]) / 86400, 1),
                                 text=snippet(r["text"], h)) for s, r, h in scored[:6]],
                commits=[dict(id="c" + row["sha"][:10], days_before=round((c["agent_time"] - row["time"]) / 86400, 1),
                              text=clip(row["text"], 400)) for s, row, h in commits[:3]],
                docs=docs,
            )
            packs.append(pack)
        print(name, "eligible", len(eligible), "sampled", len(sample), file=sys.stderr)
    with (out / "packs.jsonl").open("w") as f:
        for p in packs:
            f.write(json.dumps(p) + "\n")
    rnd.shuffle(packs)
    batch_size = (len(packs) + 3) // 4
    for b in range(4):
        with (out / f"batch-{b + 1}.md").open("w") as f:
            for p in packs[b * batch_size : (b + 1) * batch_size]:
                f.write(render(p))
    print("packs", len(packs), file=sys.stderr)


def render(p):
    md = [f"# {p['key']}", f"file: `{p['path']}` · source: {p['source_kind']}",
          "## Agent code (before)", "```", p["before"], "```", "## Human replacement (after)", "```", p["after"] or "(deleted)", "```",
          "## Stated reason for the fix (written AFTER the agent code; NOT prior evidence)",
          f"fix commit message: {p['fix_message']}"]
    if p["fix_pr"]:
        md.append(f"fix PR #{p['fix_pr']['number']}: {p['fix_pr']['text']}")
    for r in p["fix_reviews"]:
        md.append(f"- review on fix PR: {r}")
    md.append("## Candidate prior records (all dated BEFORE the agent commit)")
    if p["agent_pr"]:
        md.append(f"- [agent_pr pr{p['agent_pr']['number']}] the agent commit's own PR description: {p['agent_pr']['text']}")
    for c in p["candidates"]:
        md.append(f"- [{c['id']} · {c['kind']} · {c['days_before']} days before · {c['author']}] {c['text']}")
    for c in p["commits"]:
        md.append(f"- [{c['id']} · commit · {c['days_before']} days before] {c['text']}")
    for d in p["docs"]:
        tag = "agent-loaded doc" if d["agent_loaded"] else "doc"
        md.append(f"- [doc:{d['path']} · {tag} at agent commit] {d['text']}")
    if not (p["agent_pr"] or p["candidates"] or p["commits"] or p["docs"]):
        md.append("(no candidates found)")
    return "\n".join(md) + "\n\n---\n\n"


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4]), sys.argv[5:])
