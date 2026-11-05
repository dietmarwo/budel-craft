#!/usr/bin/env python3
"""Summarize measured campaigns and optionally draw the placed-stock comparison."""
import argparse
from collections import defaultdict
import json
from pathlib import Path
import statistics


def collect(paths):
    rows = []
    for path in paths:
        campaign = json.loads(path.read_text(encoding="utf-8"))
        rows.extend(campaign["runs"])
    return rows


def eligible(row):
    strict = row.get("strict") or {}
    judge = row.get("judge") or {}
    return strict.get("strict_valid", False) and (
        not row["spec"]["reference"] or judge.get("valid", False)
    )


def table(rows):
    groups = defaultdict(list)
    for row in rows:
        spec = row["spec"]
        key = (spec["case"], spec["algorithm"], spec["seconds"], spec["workers"],
               spec["reference"], spec.get("eval_limit"))
        groups[key].append(row)
    lines = [
        "| Case | Algorithm | Seconds | Workers | Work cap | Check | Eligible / runs | Median coverage | Median wall s |",
        "| --- | --- | ---: | ---: | ---: | --- | ---: | ---: | ---: |",
    ]
    for (case, algorithm, limit, workers, reference, work), members in sorted(
        groups.items(), key=lambda pair: tuple(str(v) for v in pair[0])
    ):
        accepted = [r["strict"]["coverage"] for r in members if eligible(r)]
        area = f"{statistics.median(accepted):.4f}%" if accepted else "—"
        wall = statistics.median(r["execution"]["wall_s"] for r in members)
        check = "both" if reference else "strict"
        lines.append(f"| {case} | {algorithm} | {limit:g} | {workers} | {work or '—'} | {check} | "
                     f"{len(accepted)} / {len(members)} | {area} | {wall:.3f} |")
    return "\n".join(lines)


def plot(rows, path):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    best = {}
    for row in rows:
        if eligible(row):
            case = row["spec"]["case"]
            result = row["strict"]
            if case not in best or result["area"] > best[case]["area"]:
                best[case] = result
    if not best:
        raise ValueError("plot needs at least one eligible answer")
    order = ["tiny", "small", "medium", "large", "uniform-hard", "layered-hard"]
    labels = [key for key in order if key in best]
    values = [100.0 * min(1.0, best[key]["available_fraction"]) for key in labels]
    fig, ax = plt.subplots(figsize=(8, 3.8), layout="constrained")
    ax.barh(labels, [100] * len(labels), color="#e8e4db")
    ax.barh(labels, values, color="#256c64")
    for i, value in enumerate(values):
        r = best[labels[i]]
        ax.text(value - 1.5, i, f"{value:.1f}%  ·  {r['seated']:,}/{r['available']:,}",
                ha="right", va="center", color="white", fontsize=9)
    ax.invert_yaxis()
    ax.set_xlim(0, 100)
    ax.set_xlabel("Percent of available figure area")
    ax.set_title("Budel layouts accepted by the requested validators", loc="left", weight="bold")
    ax.spines[["top", "right", "left"]].set_visible(False)
    path.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(path, metadata={"Date": None} if path.suffix == ".svg" else None)
    plt.close(fig)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("campaigns", type=Path, nargs="+")
    p.add_argument("--output", type=Path)
    p.add_argument("--plot", type=Path)
    args = p.parse_args()
    rows = collect(args.campaigns)
    text = "# Budel campaign summary\n\n" + table(rows) + (
        "\n\nCoverage medians use eligible layouts. The counts include failed runs. "
        "A watchdog can preserve an eligible checkpoint even when the process exits after its budget. "
        "See the raw execution and answer timing fields for those cases.\n"
    )
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(text, encoding="utf-8")
    else:
        print(text, end="")
    if args.plot:
        plot(rows, args.plot)


if __name__ == "__main__":
    main()
