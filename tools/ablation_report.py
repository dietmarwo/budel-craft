#!/usr/bin/env python3
"""Markdown tables for the 0.2.0 ablation, island and patient-budget records.

Each record file is produced from single-run directories and keeps the full solver
configuration of every run, so arms measured with different binaries or policies
remain distinguishable. Coverage is the strict validator's value for the saved answer.
"""
import argparse
from collections import defaultdict
import json
from pathlib import Path
import statistics


def rows(paths):
    out = []
    for path in paths:
        out.extend(json.loads(Path(path).read_text(encoding="utf-8"))["runs"])
    return out


def describe(config):
    f = config.get("features", {})
    p = config.get("policy", {})
    parts = [config.get("algorithm", "?"), f"w{config.get('workers', 1)}"]
    off = [k for k in ("slide", "pockets", "separate", "share") if f and not f.get(k, True)]
    if off:
        parts.append("off:" + ",".join(off))
    if f and not f.get("late_acceptance", True):
        parts.append("strict")
    elif p.get("late_length"):
        parts.append(f"late={p['late_length']}")
    # Records made before the field existed ran pockets and separation every round.
    parts.append(f"rescue={p.get('rescue_after', 0)}")
    if p.get("separate_moves") is not None:
        parts.append(f"sep={p['separate_moves']}")
    return " ".join(parts)


def table(records):
    groups = defaultdict(list)
    for r in records:
        groups[(r["case"], r["requested_seconds"], r["arm"])].append(r)
    lines = [
        "| Case | Seconds | Arm | Configuration | Seeds | Coverage per seed | Mean | Valid |",
        "| --- | ---: | --- | --- | --- | --- | ---: | ---: |",
    ]
    for (case, seconds, arm), members in sorted(groups.items()):
        members.sort(key=lambda r: r["seed"])
        cov = [r["coverage_percent"] for r in members]
        valid = sum(r["strict_valid"] for r in members)
        lines.append(
            f"| {case} | {seconds:g} | {arm} | {describe(members[0]['config'])} | "
            f"{','.join(str(r['seed']) for r in members)} | "
            f"{' / '.join(f'{c:.3f}' for c in cov)} | {statistics.mean(cov):.3f}% | "
            f"{valid}/{len(members)} |"
        )
    return "\n".join(lines)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("records", type=Path, nargs="+")
    args = p.parse_args()
    print(table(rows(args.records)))


if __name__ == "__main__":
    main()
