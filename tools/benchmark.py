#!/usr/bin/env python3
"""Run a resumable Budel campaign and independently check the resulting answers."""
import argparse
import hashlib
import importlib.metadata
import itertools
import json
import math
from pathlib import Path
import sys
import time

from supervise import atomic_json, run

ROOT = Path(__file__).resolve().parents[1]
NAMES = ("tiny", "small", "medium", "large", "uniform-hard", "layered-hard", "extreme")
CPU_OFFSET = 0


def digest(path):
    checksum = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            checksum.update(block)
    return checksum.hexdigest()


def load(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def finish_validation(result, directory, source, binary, judge, deadline):
    answer = directory / "answer.json"
    expected = result["answer_sha256"]
    if expected is None:
        return
    if not answer.is_file() or digest(answer) != expected:
        raise ValueError(f"answer changed after the run: {answer}")
    spec = result["spec"]
    strict = directory / "independent-strict.json"
    reference = directory / "judge.json"
    checks = [
        ("strict", strict, 15.0, [binary, "validate", source, answer, "--json", strict]),
    ]
    if spec["reference"]:
        checks.append(("judge", reference, spec["judge_seconds"],
                       [sys.executable, judge, source, answer, "--quiet", "--json", reference]))
    pending = False
    for key, path, limit, argv in checks:
        status = key + "_execution"
        if result.get(key) is not None or result.get(status, {}).get("terminal", False):
            continue
        remaining = deadline - time.monotonic()
        if remaining < 0.15:
            pending = True
            continue
        attempt = run(argv, cwd=ROOT, wall_s=min(limit, remaining),
                      memory_mib=spec["memory_mib"], log=directory / (key + ".log"),
                      cpu_slots=1 if spec["pin_cpu"] else None, cpu_offset=CPU_OFFSET)
        result[key] = load(path) if path.is_file() else None
        attempt["terminal"] = (result[key] is not None or
                               attempt["reason"] != "wall-limit" or remaining >= limit)
        result[status] = attempt
        pending |= not attempt["terminal"]
    result["validation_pending"] = pending
    result["eligible"] = bool(
        result.get("strict") and result["strict"]["strict_valid"] and
        (not spec["reference"] or (result.get("judge") and result["judge"]["valid"]))
    )


def campaign(args):
    binary = args.binary.resolve()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    judge = ROOT / "tools" / "reference_score.py"
    adapter = ROOT / "tools" / "packingsolver_adapter.py"
    binary_hash = digest(binary)
    judge_hash = digest(judge)
    external = args.engine == "packingsolver"
    if external and (args.seeds != [1] or args.eval_limit is not None or args.policy or args.solver_flag):
        raise SystemExit("PackingSolver uses its own seed and has no native policy/work-limit flags")
    version = importlib.metadata.version("packingsolver") if external else None
    plan = []
    algorithms = ["packingsolver"] if external else args.algorithms
    for case, algorithm, seed in itertools.product(args.cases, algorithms, args.seeds):
        source = ROOT / "cases" / f"{case}.json"
        data = load(source)
        seconds = args.seconds if args.seconds is not None else (
            data["rules"].get("runtime", {}).get("budgets", {}).get(args.budget, 1.0)
        )
        if not math.isfinite(seconds) or not 0.001 <= seconds <= 86400:
            raise ValueError("solver budget must be finite and in 0.001..=86400 seconds")
        key = f"{case}-{algorithm}-s{seed}-w{args.workers}"
        spec = {
            "case": case, "case_sha256": digest(source), "binary_sha256": binary_hash,
            "judge_sha256": judge_hash, "algorithm": algorithm, "seed": None if external else seed,
            "workers": args.workers, "seconds": seconds, "memory_mib": args.memory_mib,
            "judge_seconds": args.judge_seconds, "eval_limit": args.eval_limit,
            "solver_flags": args.solver_flag, "policy_sha256": digest(args.policy) if args.policy else None,
            "reference": not args.skip_reference, "engine": args.engine, "pin_cpu": args.pin_cpu,
            "external_version": version, "adapter_sha256": digest(adapter) if external else None,
        }
        plan.append((out / key, source, spec))
    if len(set(directory for directory, _, _ in plan)) != len(plan):
        raise ValueError("repeated campaign entries")
    new_seconds = sum(spec["seconds"] for directory, _, spec in plan if not (directory / "result.json").is_file())
    print(f"{len(plan)} runs, up to {new_seconds:.2f}s new solver budget, aggregate wall cap {args.aggregate_s:.1f}s",
          flush=True)
    deadline = time.monotonic() + args.aggregate_s
    rows = []
    for directory, source, spec in plan:
        result_path = directory / "result.json"
        if result_path.is_file():
            result = load(result_path)
            if result["spec"] != spec:
                raise SystemExit(f"existing result differs from requested configuration: {directory}")
            if result.get("validation_pending"):
                finish_validation(result, directory, source, binary, judge, deadline)
                atomic_json(result_path, result)
            rows.append(result)
            continue
        if deadline - time.monotonic() < spec["seconds"] + 0.75:
            break
        directory.mkdir(parents=True, exist_ok=True)
        answer = directory / "answer.json"
        previous = [p for p in directory.iterdir() if p.name != "attempts"]
        if previous:
            old = directory / "attempts" / str(time.time_ns())
            old.mkdir(parents=True)
            for path in previous:
                path.rename(old / path.name)
        if external:
            argv = [sys.executable, adapter, source, "--output", answer, "--run-dir", directory,
                    "--time-limit", spec["seconds"]]
        else:
            argv = [binary, "solve", source, "--output", answer, "--run-dir", directory,
                    "--time-limit", spec["seconds"], "--algorithm", spec["algorithm"],
                    "--seed", spec["seed"], "--workers", spec["workers"]]
            if args.eval_limit is not None:
                argv.extend(["--eval-limit", args.eval_limit])
            if args.policy is not None:
                argv.extend(["--policy", args.policy.resolve()])
            argv.extend(args.solver_flag)
        print(f"Running {directory.name}", flush=True)
        execution = run(argv, cwd=ROOT, wall_s=spec["seconds"] + 0.6, memory_mib=args.memory_mib,
                        log=directory / "solver.log", cpu_slots=args.workers if args.pin_cpu else None,
                        cpu_offset=args.cpu_offset)
        metrics = directory / ("external.json" if external else "packing_metrics.json")
        result = {
            "spec": spec, "execution": execution, "metrics": load(metrics) if metrics.is_file() else None,
            "strict": None, "judge": None, "answer_sha256": digest(answer) if answer.is_file() else None,
            "validation_pending": False, "eligible": False,
        }
        # Commit the solve record before starting the separately bounded judges.
        result["validation_pending"] = answer.is_file()
        atomic_json(result_path, result)
        finish_validation(result, directory, source, binary, judge, deadline)
        atomic_json(result_path, result)
        rows.append(result)
        strict = result["strict"] or {}
        ref = result["judge"] or {}
        print(f"{directory.name} wall={execution['wall_s']:.3f}s "
              f"seated={strict.get('seated','—')} coverage={strict.get('coverage','—')} "
              f"strict={strict.get('strict_valid','—')} judge={ref.get('valid','—')}", flush=True)
    pending = sum(bool(row.get("validation_pending")) for row in rows)
    atomic_json(out / "campaign.json", {"planned_runs": len(plan), "completed_runs": len(rows),
                                       "pending_validations": pending, "runs": rows})
    if len(rows) != len(plan) or pending:
        print("Campaign paused at its aggregate cap. Rerun the same command to continue.", flush=True)
        return 3
    return 0


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--binary", type=Path, default=ROOT / "target" / "release" / "budel")
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--cases", nargs="+", choices=NAMES, default=["tiny", "small"])
    p.add_argument("--algorithms", nargs="+", default=["baseline", "lns"])
    p.add_argument("--engine", choices=["native", "packingsolver"], default="native")
    p.add_argument("--seeds", nargs="+", type=int, default=[1])
    p.add_argument("--workers", type=int, default=1)
    p.add_argument("--pin-cpu", action="store_true")
    p.add_argument("--cpu-offset", type=int, default=0,
                   help="first CPU (within this process's affinity) used for pinned children")
    p.add_argument("--seconds", type=float)
    p.add_argument("--budget", choices=["interactive", "patient", "background"], default="interactive")
    p.add_argument("--eval-limit", type=int)
    p.add_argument("--memory-mib", type=int, default=1024)
    p.add_argument("--judge-seconds", type=float, default=120)
    p.add_argument("--aggregate-s", type=float, default=900)
    p.add_argument("--skip-reference", action="store_true")
    p.add_argument("--solver-flag", action="append", default=[])
    p.add_argument("--policy", type=Path)
    args = p.parse_args()
    if any(not math.isfinite(x) or x <= 0 for x in (args.aggregate_s,args.judge_seconds)):
        p.error("positive finite aggregate and judge limits required")
    if not 1 <= args.workers <= 16 or args.memory_mib < 32:
        p.error("workers must be 1..=16, memory limit must be at least 32 MiB")
    global CPU_OFFSET
    CPU_OFFSET = args.cpu_offset
    return campaign(args)


if __name__ == "__main__":
    sys.exit(main())
