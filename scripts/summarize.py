#!/usr/bin/env python3
"""Aggregate `ingrain eval --json` reports into a markdown table.

    python3 scripts/summarize.py <reports-dir>
"""
import json
import pathlib
import statistics
import sys


def pct(a, b):
    return f"{100 * a / b:.0f}%" if b else "-"


def main(directory):
    rows = []
    for path in sorted(pathlib.Path(directory).glob("*.json")):
        r = json.loads(path.read_text())
        rows.append((path.stem, r))
    print("| repo | corrections | agent hunks | in a 3+ cluster | rules | prevented (test) | true / false hits | precision |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|")
    recalls, precisions = [], []
    tot = dict(c=0, h=0, rep=0, rules=0, prev=0, test=0, tp=0, fp=0)
    for name, r in rows:
        prec = r.get("precision")
        recalls.append(r["recall"])
        if prec is not None:
            precisions.append(prec)
        tot["c"] += r["corrections"]
        tot["h"] += r["agent_hunks"]
        tot["rep"] += r["repeated"]
        tot["rules"] += len(r["rules"])
        tot["prev"] += r["prevented"]
        tot["test"] += r["test_corrections"]
        tot["tp"] += r["test_true_hits"]
        tot["fp"] += r["test_false_hits"]
        print(
            f"| {name} | {r['corrections']} | {r['agent_hunks']} | {pct(r['repeated'], r['corrections'])} "
            f"| {len(r['rules'])} | {r['prevented']}/{r['test_corrections']} ({100 * r['recall']:.1f}%) "
            f"| {r['test_true_hits']} / {r['test_false_hits']} | {'-' if prec is None else f'{100 * prec:.0f}%'} |"
        )
    print(
        f"| **all {len(rows)}** | {tot['c']} | {tot['h']} | {pct(tot['rep'], tot['c'])} | {tot['rules']} "
        f"| {tot['prev']}/{tot['test']} ({pct(tot['prev'], tot['test'])}) | {tot['tp']} / {tot['fp']} "
        f"| {pct(tot['tp'], tot['tp'] + tot['fp'])} |"
    )
    print()
    print(f"median recall per repo: {100 * statistics.median(recalls):.1f}%")
    if precisions:
        print(f"median precision per repo (repos with hits): {100 * statistics.median(precisions):.0f}% over {len(precisions)} repos")
    p95 = [r["scan_ms_p95"] for _, r in rows if r["scan_ms_p95"]]
    if p95:
        print(f"scan time per hunk, worst repo p95: {max(p95):.2f} ms")


if __name__ == "__main__":
    main(sys.argv[1])
