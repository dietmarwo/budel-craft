#!/usr/bin/env python3
"""Optional external reference arm using PackingSolver's irregular Python API."""
import argparse
import importlib.metadata
import json
import math
from pathlib import Path
import sys
import threading
import time

from supervise import atomic_json


def signed_area(ring):
    x0, y0 = ring[0]
    return 0.5 * math.fsum(
        (x - x0) * (v - y0) - (y - y0) * (u - x0)
        for (x, y), (u, v) in zip(ring, ring[1:] + ring[:1])
    )


def pack(case, limit, started, publish):
    import packingsolver.irregular as ps

    plane, rules = case["plane"], case["rules"]
    margin = rules.get("margin", plane.get("margin", 0))
    guard = max(1e-6, 64 * sys.float_info.epsilon * max(plane["w"], plane["h"]))
    builder = ps.InstanceBuilder()
    # Single stock-limited sheet in both cases. For mandatory cases the same
    # independent validator requires all supplied IDs.
    builder.set_objective(ps.Objective.Knapsack)
    builder.add_bin_type(
        ps.build_rectangle(margin, plane["w"] - margin, margin, plane["h"] - margin),
        copies=1, item_bin_minimum_spacing=guard * 4,
    )
    builder.set_item_item_minimum_spacing(rules.get("min_channel", 0) + guard * 4)
    flags = [False, True] if rules.get("mirror_allowed", True) else [False]
    if rules.get("poses", "free") == "eight":
        rotations = [(float(a), float(a), flip) for flip in flags for a in range(0, 360, 45)]
    else:
        rotations = [(0.0, 360.0, flip) for flip in flags]
    ids = []
    for sh in case["shapes"]:
        if time.monotonic() - started > limit:
            return [], {"reason": "conversion-deadline"}
        ring = (sh.get("envelope") or sh["ring"]) if rules.get("body") == "envelope" else sh["ring"]
        area = signed_area(ring)
        profit = abs(area)
        if rules.get("holes_are", "paper") == "paper":
            profit -= sum(abs(signed_area(h)) for h in sh.get("holes", []))
        if area < 0:
            ring = ring[::-1]
        item_id = builder.add_item_type(ps.build_shape(ring), allowed_rotations=rotations,
                                       copies=1, copies_min=0, profit=profit)
        if item_id != len(ids):
            raise RuntimeError("unexpected item-type reordering")
        ids.append(sh["id"])
    instance = builder.build()
    params = ps.OptimizeParameters()
    params.time_limit = max(0.001, limit - (time.monotonic() - started) - 0.025)
    params.memory_limit_megabytes = 512
    params.verbosity_level = 0
    def placements_for(solution):
        if solution.number_of_different_bins() > 1:
            raise RuntimeError("baseline returned more than one sheet")
        placements = []
        if solution.number_of_different_bins():
            for item in solution.bin(0).items:
                # Original local origin, including reflection before rotation.
                placements.append({"id": ids[item.item_type_id], "x": item.bl_corner.x,
                                   "y": item.bl_corner.y, "rot": item.angle % 360,
                                   "mirror": item.mirror})
        return placements

    def callback(output):
        solution = output.solution
        publish(placements_for(solution), solution.profit())

    params.new_solution_callback = callback
    output = ps.optimize(instance, params)
    solution = output.solution
    return placements_for(solution), {"reason": "returned", "engine_time_s": output.time,
                                       "reported_profit": solution.profit(),
                                       "bins": solution.number_of_different_bins()}


def main():
    started = time.monotonic()
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("case", type=Path)
    p.add_argument("--time-limit", type=float, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--run-dir", type=Path, required=True)
    args = p.parse_args()
    if not math.isfinite(args.time_limit) or args.time_limit <= 0:
        p.error("positive finite time limit required")
    case = json.loads(args.case.read_text(encoding="utf-8"))
    version = importlib.metadata.version("packingsolver")
    best_profit = -1.0
    best_count = 0
    lock = threading.Lock()

    def publish(placements, profit):
        nonlocal best_profit, best_count
        with lock:
            elapsed = time.monotonic() - started
            if elapsed > args.time_limit or profit <= best_profit:
                return
            answer = {"case": case["case"], "placements": placements, "runtime_s": elapsed,
                      "author": f"PackingSolver {version}"}
            atomic_json(args.output, answer)
            best_profit, best_count = profit, len(placements)

    placements, info = pack(case, args.time_limit, started, publish)
    if "reported_profit" in info:
        publish(placements, info["reported_profit"])
    atomic_json(args.run_dir / "external.json", dict(info, version=version,
                seated=best_count, saved_profit=best_profit, runtime_s=time.monotonic() - started))
    return 0


if __name__ == "__main__":
    sys.exit(main())
