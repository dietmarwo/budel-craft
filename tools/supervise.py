"""Linux process-group supervision for bounded benchmark and judge runs."""
import json
import math
import os
from pathlib import Path
import resource
import signal
import subprocess
import time


def atomic_json(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{os.getpid()}.tmp")
    with temporary.open("w", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, allow_nan=False)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)


def group_rss(group):
    pages = 0
    for path in Path("/proc").iterdir():
        if not path.name.isdecimal():
            continue
        try:
            fields = (path / "stat").read_text().rsplit(")", 1)[1].split()
            if int(fields[2]) == group:
                pages += max(0, int(fields[21]))
        except (OSError, IndexError, ValueError):
            continue
    return pages * os.sysconf("SC_PAGE_SIZE")


def run(command, *, cwd, wall_s, log, memory_mib=1024, env=None, log_mib=4, cpu_slots=None, cpu_offset=0):
    """Run argv without a shell. Collect wait4 CPU/RSS and sampled group RSS."""
    if not Path("/proc/self/stat").exists():
        raise RuntimeError("the benchmark supervisor requires Linux /proc")
    log = Path(log)
    log.parent.mkdir(parents=True, exist_ok=True)
    memory = memory_mib * 1024 * 1024
    if not math.isfinite(wall_s) or wall_s <= 0 or memory <= 0:
        raise ValueError("positive wall and memory limits are required")
    cpus = sorted(os.sched_getaffinity(0))
    if cpu_slots is not None and not (1 <= cpu_slots and 0 <= cpu_offset
                                      and cpu_offset + cpu_slots <= len(cpus)):
        raise ValueError("CPU offset and slots must fit the current process affinity")
    # Parallel campaigns must use different offsets, or their pinned children share CPUs.
    pinned = cpus[cpu_offset:cpu_offset + cpu_slots] if cpu_slots is not None else None
    child_env = dict(os.environ, OPENBLAS_NUM_THREADS="1", OMP_NUM_THREADS="1", MKL_NUM_THREADS="1")
    child_env.update(env or {})

    def address_limit():
        # A second limit covers large allocations between RSS samples. Virtual
        # address space includes libraries and stacks and is reported separately.
        current, maximum = resource.getrlimit(resource.RLIMIT_AS)
        bound = memory
        for inherited in (current, maximum):
            if inherited != resource.RLIM_INFINITY:
                bound = min(bound, inherited)
        resource.setrlimit(resource.RLIMIT_AS, (bound, maximum))
        if pinned is not None:
            os.sched_setaffinity(0, pinned)

    started = time.monotonic()
    peak = 0
    reason = "exit"
    with log.open("wb") as output:
        child = subprocess.Popen(
            list(map(str, command)), cwd=cwd, env=child_env, stdout=output,
            stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, start_new_session=True,
            preexec_fn=address_limit,
        )

        def kill_group():
            try:
                os.killpg(child.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass

        try:
            while True:
                pid, status, usage = os.wait4(child.pid, os.WNOHANG)
                if pid:
                    child.returncode = os.waitstatus_to_exitcode(status)
                    break
                peak = max(peak, group_rss(child.pid))
                if reason == "exit":
                    if time.monotonic() - started >= wall_s:
                        reason = "wall-limit"
                    elif peak > memory:
                        reason = "rss-limit"
                    elif log.stat().st_size > log_mib * 1024 * 1024:
                        reason = "output-limit"
                    if reason != "exit":
                        kill_group()
                        kill_deadline = time.monotonic() + 2.0
                elif time.monotonic() >= kill_deadline:
                    raise TimeoutError(f"could not reap supervised group {child.pid}")
                time.sleep(0.025)
        finally:
            kill_group()
            if child.returncode is None:
                try:
                    child.wait(timeout=1)
                except subprocess.TimeoutExpired:
                    pass

    peak = max(peak, usage.ru_maxrss * 1024)
    if reason == "exit" and peak > memory:
        reason = "rss-limit"
    if reason == "exit" and log.stat().st_size > log_mib * 1024 * 1024:
        reason = "output-limit"
    return {
        "reason": reason, "exit_code": child.returncode,
        "wall_s": time.monotonic() - started,
        "cpu_s": usage.ru_utime + usage.ru_stime,
        "leader_peak_rss_bytes": usage.ru_maxrss * 1024,
        "sampled_group_peak_rss_bytes": peak,
        "rss_limit_bytes": memory, "address_space_cap_per_process_bytes": memory,
        "cpu_slots": cpu_slots,
        "pinned_cpus": pinned,
    }
