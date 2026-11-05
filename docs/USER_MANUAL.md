# Budel Craft user manual

## Build and inspect

Run commands from the extracted `budel-craft` directory. Use Rust 1.88 or later. The 0.2.0 measurements used Rust 1.97.0 on Linux x86-64; the 0.1.0 records used Rust 1.88.0.

```bash
cargo build --release --locked
B=target/release/budel
"$B" inspect cases/medium.json
```

The inspector prints the usable rectangle, permitted poses, spacing, item and vertex counts, file hash, budget names, and the maximum coverage available in the supplied stock. Use `--json PATH` to save the report.

## Example 1. Solve and draw the tiny case

```bash
"$B" solve cases/tiny.json --time-limit 1 --seed 1 \
  --output out/tiny.answer.json --run-dir out/tiny
"$B" validate cases/tiny.json out/tiny.answer.json
"$B" render cases/tiny.json out/tiny.answer.json --output out/tiny.svg
```

The result contains two figures and 5% sheet coverage. Open the SVG in a browser. The red rectangle marks the usable sheet.

An answer contains the case name and a list of poses. This hand-written tiny answer also passes validation.

```json
{
  "case": "tiny",
  "placements": [
    {"id": "a", "x": 20, "y": 20, "rot": 0, "mirror": false},
    {"id": "b", "x": 45, "y": 20, "rot": 0, "mirror": false}
  ]
}
```

Angles are degrees around the original local origin. The engine negates local x before rotation when reflection is requested. The translations are in the case's coordinate units.

## Example 2. Seat every figure in the small case

```bash
"$B" solve cases/small.json --budget interactive --seed 1 --workers 1 \
  --output out/small.answer.json --run-dir out/small
"$B" validate cases/small.json out/small.answer.json --json out/small-strict.json
```

The case's interactive budget is six seconds. Its 40 required figures give 23.2899% coverage in any complete feasible answer. Search stops after certification. Open the packaged `examples/small.svg` for a checked layout.

For the assignment's reference score, install NumPy and Pillow in your Python environment.

```bash
python -m pip install numpy pillow
python tools/reference_score.py cases/small.json out/small.answer.json \
  --json out/small-judge.json --render out/small.png
```

On complex cases, the reference judge can take much longer than the solver. For example, the recorded `uniform-hard` judge took over a minute. Use the supervised campaign when you need separate time limits for solving and judging.

## Example 3. Improve optional stock under a budget

```bash
"$B" solve cases/medium.json --budget interactive --seed 7 \
  --workers 2 --memory-limit-mib 1024 \
  --output out/medium.answer.json --run-dir out/medium
```

The goal is to maximize selected area. A layout with fewer figures may score higher if it seats larger ones. `available_fraction` is the fraction of all supplied stock area seated. When that fraction reaches one, the score reaches its stock upper bound and the solver stops.

The default `lns` algorithm runs a few construction attempts and then keeps improving one layout for the rest of the budget: it slides figures toward the lower-left, fills gaps, rebuilds neighbourhoods around omitted figures, and after a stall tries free-space pockets and overlap-tolerant insertion. With two or more workers the lanes share their best layout.

| Option | Meaning |
| --- | --- |
| `--budget interactive`, `patient`, `background` | Resolve seconds from the case. A case with no default named budget uses one second. |
| `--time-limit SECONDS` | Explicit solve budget, mutually exclusive with a named budget |
| `--eval-limit N` | Global proposal cap, shared by all workers |
| `--seed N`, `--workers N` | Search seed and worker count. Use one worker for a repeatable work-limited experiment. |
| `--memory-limit-mib N` | Unix process address-space ceiling |
| `--cache-mib N` or `--no-cache` | Pose cache capacity or disabled cache |
| `--scalar`, `--no-pairs` | Disable the edge hierarchy fast path or the repeated-body reflection hint |
| `--policy FILE` | Use a JSON policy; omitted fields take their defaults |
| `--no-slide` | Disable slide-to-contact compaction |
| `--no-pockets` | Disable raster pocket insertion |
| `--no-separate` | Disable overlap-tolerant insertion |
| `--strict-accept` | Force improvement-only acceptance even if the policy sets `late_length` |
| `--no-share` | Keep worker lanes independent (no shared incumbent) |

Algorithm choices are `baseline` for one construction, `restart` for multiple constructions, `local` for compaction and gap filling, `multistart` for the 0.1.0 behaviour (up to 512 independent attempts, each with a short rebuilding burst), and the default `lns` for persistent search. `repair` and the optional `fcmaes-block` arm add small-block coordinate repair (DE for the latter) inside the persistent search.

Policy fields and defaults:

| Field | Default | Effect |
| --- | ---: | --- |
| `restarts` | 5 | Construction attempts before the persistent phase |
| `slide_sweeps` | 3 | Slide sweeps per persistent round (0 disables) |
| `rescue_after` | 2 | Stalled rounds before pocket and separation operators run |
| `pocket_trials` | 16 | Pocket candidates per missing figure (0 disables) |
| `separate_moves` | 600 | Item moves per overlap-tolerant insertion (0 disables) |
| `late_length` | 0 | Late-acceptance history in rebuild moves (0 accepts improvements only) |
| `best_of` | 1 | Feasible gap proposals compared before choosing |
| `gap_trials`, `rebuild_size`, `blocker_weight` | 120, 8, 0.36 | Gap sampling, blockers removed per rebuild, blocker count cost |
| `stock_area`, `box_area`, `extent`, `rect_waste`, `rect_height`, `contact_rate` | see source | Construction order and placement weights |

A policy file can contain only the fields to change, for example `{"late_length": 128}`. Policy files from 0.1.0 still load.

## Checkpoints and exit status

`solve` and `resume` return 0 when they write a certified answer, 1 when the mandatory case is incomplete, and 2 for a usage or processing error. `validate` returns 0 for a strict-valid answer and 1 for a violation. The benchmark records any external watchdog termination separately.

The run directory contains `packing_metrics.json`, `strict.json`, `progress.jsonl`, and `checkpoint.json`. An incomplete mandatory attempt writes `partial.answer.json` there for diagnostics. Use separate output paths for each attempt.

```bash
"$B" resume out/medium/checkpoint.json --additional-time 4 \
  --output out/medium-resumed.answer.json --run-dir out/medium-resumed
```

Use `--case PATH` with resume when the original case moved. The SHA-256 must still match. The metrics distinguish additional and cumulative runtime and proposals. Resume starts a new search from the last saved arrangement.

## Optional fcmaes experiments

```bash
cargo build --release --locked --features fcmaes
"$B" tune cases/medium.json --method bite --evaluations 16 --per-trial 1200000 \
  --seeds 5 --holdout-seeds 5 --time-limit 480 \
  --output out/medium.policy.json --json out/medium-tuning.json
"$B" solve cases/medium.json --policy out/medium.policy.json --budget interactive \
  --output out/tuned.answer.json --run-dir out/tuned
```

Methods are `fixed`, `random`, `de` and `bite` (BiteOpt). Each candidate is scored by its mean loss over `--seeds` training seeds at a fixed proposal limit; the default policy is always evaluated first. The report lists every evaluation, then compares the selected and default policies on `--holdout-seeds` separate seeds. Adopt a tuned policy only if it wins on the held-out seeds. The tuner varies ten search-control fields and keeps the construction weights. Use `--algorithm fcmaes-block` to test the local DE operator separately from policy tuning.

## Run a bounded campaign

```bash
python tools/benchmark.py --out out/campaign --cases tiny small \
  --algorithms baseline lns --seeds 1 2 3 --seconds 2 \
  --aggregate-s 60 --judge-seconds 30 --pin-cpu
python tools/report.py out/campaign/campaign.json --output out/campaign.md
```

The supervisor starts subprocesses without a shell and records time, CPU and RSS. With `--pin-cpu`, children are pinned to the first CPUs of the harness's affinity; when you run two campaigns at once, give them different `--cpu-offset` values or their pinned solves and judges share one core. Rerun the same campaign command to fill unfinished runs or validation records. The case, solver and judge hashes are checked before a result is reused. Use `--skip-reference` for explicitly native-only development pilots. Inspect the reports before comparing strict-only and dual-validated runs.

Install `packingsolver==0.1.1296` to enable the optional external arm.

```bash
python tools/benchmark.py --engine packingsolver --out out/external \
  --cases tiny small medium --budget interactive --workers 1 --pin-cpu \
  --aggregate-s 100 --judge-seconds 30
```

PackingSolver uses its own randomization and exposes no seed in this adapter. Conversion time is included. An incumbent saved before the deadline can be checked if the watchdog later stops the process.

## Run through optimization-jobs

On a node running `optimization-run` with trusted local command jobs enabled, stage a command from the current binary and case. This path was exercised with optimization-runner 0.4.0 at revision `eb232ae58d388b998d9da1eefc33643c939f4b38`.

```bash
python tools/job_spec.py cases/tiny.json --binary "$B" \
  --out out/tiny-job --seconds 1
optimization-run --root "$PWD/out/job-node" node init --capacity 1 --allow-commands
optimization-run --root "$PWD/out/job-node" validate --spec out/tiny-job/command.json
optimization-run --root "$PWD/out/job-node" run --spec out/tiny-job/command.json
```

The runner records its command result and exposes the answer, checkpoint and packing metrics as output artifacts. Build and run these binaries for the node's platform and library environment.
