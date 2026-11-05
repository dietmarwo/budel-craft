#!/usr/bin/env python3
"""Bounded recovery, accounting and process-limit checks for the release binary."""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
from tempfile import TemporaryDirectory

from supervise import run

ROOT = Path(__file__).resolve().parents[1]


def load(path):
    return json.loads(path.read_text(encoding="utf-8"))


def invoke(binary, *args, code=0):
    r = subprocess.run([binary, *map(str, args)], text=True, capture_output=True, timeout=8)
    assert r.returncode == code, (r.returncode, r.stdout, r.stderr)
    return r


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/budel")
    args = parser.parse_args()
    binary = args.binary.resolve()
    with TemporaryDirectory(prefix="budel-robustness-") as tmp:
        tmp = Path(tmp)
        tiny = ROOT / "cases/tiny.json"
        small = ROOT / "cases/small.json"
        medium = ROOT / "cases/medium.json"

        partial = tmp / "partial"
        invoke(binary, "solve", small, "--time-limit", 1, "--eval-limit", 3,
               "--output", tmp/"absent.json", "--run-dir", partial, code=1)
        assert not (tmp/"absent.json").exists()
        cp = partial/"checkpoint.json"
        old = load(cp)
        assert old["total_proposals"] == 3
        result = tmp/"resumed"
        invoke(binary, "resume", cp, "--additional-time", 4,
               "--output", tmp/"small.answer.json", "--run-dir", result)
        metrics = load(result/"packing_metrics.json")
        assert metrics["cumulative_proposals"] == old["total_proposals"] + metrics["proposals"]
        assert metrics["best"]["complete"] and metrics["best"]["strict_valid"]

        changed = tmp/"changed.json"
        changed.write_bytes(small.read_bytes() + b"\n")
        invoke(binary, "resume", cp, "--case", changed, "--additional-time", 2,
               "--output", tmp/"invalid.json", "--run-dir", tmp/"invalid", code=2)
        broken = tmp/"broken.json"
        broken.write_text('{"version":1', encoding="utf-8")
        invoke(binary, "resume", broken, "--additional-time", 2,
               "--output", tmp/"invalid.json", "--run-dir", tmp/"invalid", code=2)
        invoke(binary, "solve", tiny, "--time-limit", 1, "--output", tiny,
               "--run-dir", tmp/"overwrite", code=2)

        parallel = tmp/"parallel"
        invoke(binary, "solve", medium, "--workers", 4, "--eval-limit", 75, "--time-limit", 3,
               "--output", tmp/"parallel.answer.json", "--run-dir", parallel)
        assert load(parallel/"packing_metrics.json")["proposals"] <= 75

        interrupted = tmp/"interrupt"
        path = tmp/"interrupted.answer.json"
        with (tmp/"interrupt.log").open("wb") as log:
            child = subprocess.Popen([binary, "solve", medium, "--workers", "2", "--time-limit", "15",
                                      "--output", path, "--run-dir", interrupted], stdout=log,
                                     stderr=subprocess.STDOUT, start_new_session=True)
            try:
                until = time.monotonic() + 4
                while time.monotonic() < until and child.poll() is None:
                    if path.is_file() and load(path)["placements"]:
                        break
                    time.sleep(0.03)
                assert child.poll() is None and path.is_file() and load(path)["placements"]
                os.killpg(child.pid, signal.SIGTERM)
                assert child.wait(timeout=4) == 0
            finally:
                if child.poll() is None:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait(timeout=2)
        assert load(interrupted/"packing_metrics.json")["termination"] == "interrupted"
        invoke(binary, "validate", medium, path)

        child = run([sys.executable, "-c",
                     "import subprocess,time,sys; subprocess.Popen([sys.executable,'-c','import time;time.sleep(20)']);time.sleep(20)"],
                    cwd=tmp, wall_s=0.25, memory_mib=128, log=tmp/"wall.log")
        assert child["reason"] == "wall-limit" and child["exit_code"] < 0
        memory = run([sys.executable, "-c", "a=bytearray(160<<20)"],
                     cwd=tmp, wall_s=3, memory_mib=64, log=tmp/"memory.log")
        assert memory["exit_code"] != 0
        assert "MemoryError" in (tmp/"memory.log").read_text()
        output = run([sys.executable, "-c", "print('a'*32768)"], cwd=tmp,
                     wall_s=2, log_mib=0.01, log=tmp/"output.log")
        assert output["reason"] == "output-limit"
    print("Passed: bounded resume, cumulative accounting, changed input, truncated checkpoint, "
          "protected input, shared work limit, SIGTERM preservation, process-group timeout, "
          "hard allocation and log limits.")


if __name__ == "__main__":
    main()
