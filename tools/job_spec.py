#!/usr/bin/env python3
"""Stage a Budel solve using the upstream execution.command/0.1 contract."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import shutil

from supervise import atomic_json


def stage(source, target, name, executable):
    source = source.resolve(strict=True)
    if not source.is_file() or source.stat().st_size > 64 * 1024 * 1024:
        raise ValueError(f"invalid command input: {source}")
    shutil.copyfile(source, target / name)
    data = (target / name).read_bytes()
    return {"path": name, "file": name, "sha256": hashlib.sha256(data).hexdigest(),
            "bytes": len(data), "executable": executable}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("case", type=Path)
    p.add_argument("--binary", type=Path, required=True)
    p.add_argument("--out", type=Path, required=True, help="new staging directory")
    p.add_argument("--seconds", type=float, required=True)
    p.add_argument("--workers", type=int, default=1)
    p.add_argument("--seed", type=int, default=1)
    p.add_argument("--eval-limit", type=int)
    args = p.parse_args()
    if (not math.isfinite(args.seconds) or not 0.001 <= args.seconds <= 86400
            or not 1 <= args.workers <= 16 or not 0 <= args.seed < 2**64
            or (args.eval_limit is not None and not 0 <= args.eval_limit < 2**64)):
        p.error("invalid time, worker, seed or proposal limit")
    args.out.mkdir(parents=True, exist_ok=False)
    inputs = [
        stage(args.binary, args.out, "budel-bin", True),
        stage(args.case, args.out, "case.json", False),
    ]
    argv = ["./budel-bin", "solve", "case.json", "--time-limit", str(args.seconds),
            "--workers", str(args.workers), "--seed", str(args.seed),
            "--memory-limit-mib", "1024", "--output", "answer.json", "--run-dir", "run"]
    if args.eval_limit is not None:
        argv += ["--eval-limit", str(args.eval_limit)]
    spec = {
        "schema_version": "execution.command/0.1",
        "argv": argv,
        "env": {},
        "resources": {"cpu_slots": args.workers},
        "limits": {"wall_seconds": args.seconds + 0.75, "grace_seconds": 1},
        "replay": "never",
        "inputs": inputs,
        "outputs": [
            {"name": "answer", "path": "answer.json", "max_bytes": 16 << 20, "required": False},
            {"name": "metrics", "path": "run/packing_metrics.json", "max_bytes": 1 << 20, "required": True},
            {"name": "strict", "path": "run/strict.json", "max_bytes": 1 << 20, "required": True},
            {"name": "checkpoint", "path": "run/checkpoint.json", "max_bytes": 16 << 20, "required": True},
            {"name": "progress", "path": "run/progress.jsonl", "max_bytes": 1 << 20, "required": True},
        ],
    }
    spec["request_key"] = "budel-" + hashlib.sha256(json.dumps(spec, sort_keys=True).encode()).hexdigest()[:24]
    atomic_json(args.out / "command.json", spec)
    print(args.out / "command.json")


if __name__ == "__main__":
    main()
