# Budel Craft 0.2.0 implementation log

5 November 2026

Version 0.2.0 turns the 0.1.0 multi-start heuristic into a persistent search on the incumbent and implements every item of the improvement review:

- slide-to-contact compaction;
- raster free-space pockets;
- overlap-tolerant insertion with guided local search;
- late acceptance;
- shared incumbents between workers;
- multi-seed fcmaes policy tuning;
- a non-blocking certification path.

Each component has a switch and a policy field. Defaults follow the measurements: late acceptance is off, and pockets and separation run only after the search stalls.

On the same host and binary, the default raises mean sheet coverage from 47.34% to 51.51% on medium and from 46.26% to 50.51% on layered-hard, compared with the 0.1.0 search (`multistart`). The final campaign covers seven cases: the six original ones and `extreme` (3,309 figures, added in I36). All 21 answers pass the native strict validator and the Python judge, and every run exited normally within its budget. Medium reaches its stock bound on two of three seeds. Five cases reach their stock bound on every seed and finish in about 1 s or less. See [the results](docs/PERFORMANCE.md).

| Case | Seated, seeds 1/2/3 | Sheet coverage, seeds 1/2/3 | Stock bound |
| --- | --- | --- | ---: |
| tiny | 2/2/2 of 2 | 5.0000% (all) | 5.0000% |
| small | 40/40/40 of 40 | 23.2899% (all) | 23.2899% |
| medium | 312/312/310 of 312 | 51.5850 / 51.5850 / 51.3630% | 51.5850% |
| large | 2,127/2,127/2,127 of 2,127 | 47.8051% (all) | 47.8051% |
| uniform-hard | 1,008/1,008/1,008 of 1,008 | 26.9808% (all) | 26.9808% |
| layered-hard | 1,155/1,165/1,159 of 1,184 | 50.0790 / 50.9213 / 50.5413% | 51.9847% |
| extreme | 3,309/3,309/3,309 of 3,309 | 21.8973% (all) | 21.8973% |

The 0.1.0 log is in [docs/IMPLEMENTATION_LOG_0.1.0.md](docs/IMPLEMENTATION_LOG_0.1.0.md).

## Iterations

| Iteration | Change and evidence |
| --- | --- |
| I24 | **Diagnosis of 0.1.0.** Each attempt rebuilt the layout from scratch, then ran two compaction sweeps and at most 64 rebuilds before it was discarded unless it was better. In the campaign traces the large gains came from fresh attempts. On the sandbox, medium seed 1 reproduced 47.5282% at 2.93 s and then stayed flat for 10.7 s. Medium left out 74 figures, mostly small; layered-hard left out 169 mid-size ones. The rendering showed a grid of bounding boxes with wide channels. |
| I25 | **Environment-switch prototype on the 0.1.0 binary.** Medium at 14 s rose from 47.53 / 47.19% to 50.96 / 51.10% with persistent search, and to 51.18 / 51.19% with slide compaction. Layered-hard seed 1 rose from 46.26% to 49.94%. Both validators accepted the medium answer and the layered-hard seed-1 answer. |
| I26 | **Configurable persistent phase.** Added `Algorithm::Multistart` (0.1.0 behaviour) and made `lns` persistent after `policy.restarts` attempts. Added `Features` switches with CLI flags. New `Policy` fields have serde defaults, so 0.1.0 policy and config files load (unit test). A resume first improves its checkpoint, then runs the restarts and the persistent phase. |
| I27 | **Slide compaction and best-of-k placement.** Items slide toward the lower-left by step halving on `fits`, so every intermediate pose is feasible; a unit test checks validity and unchanged area. `find` can compare `best_of` feasible proposals after sliding each one. Tuning found no gain for `best_of` > 1, so the default stays first-fit. |
| I28 | **Interior poles and separator.** Each shape gets lazily computed interior poles: a greedy cover on a 22×22 sample grid, at most 12 discs, each inset by four guards. Overlap of a conflicting pair is the summed pole penetration plus a base term. Separation moves colliding items by sampling plus coordinate descent under guided-local-search pair weights (×1.3 per colliding sweep, decay 0.95). It ends feasible or restores every moved item exactly. A unit test checks both outcomes against the strict validator. Nothing is published while overlaps exist. |
| I29 | **Pockets.** About 160,000 raster cells with vertex marking for thin curls. An exact Felzenszwalb–Huttenlocher distance transform with an occupied border gives a conservative clearance, checked against brute force in a unit test. Missing figures are tried where the clearance holds their largest pole plus the gap. |
| I30 | **Late acceptance and stagnation reset.** LAHC on area judges only rebuilds that seated their target. The lane best is kept separately and restored after six stalled rounds. It was consistently harmful: medium at 14 s averaged 51.47% without it and 50.87% with it, and all tuning candidates with it scored lower. The default is `late_length = 0`. |
| I31 | **Islands.** Lanes offer improvements to a shared incumbent and adopt a better one between rounds. At equal wall time with two workers, layered-hard reached 50.65 / 50.88% shared and 50.20 / 50.71% independent. |
| I32 | **Runtime.** A single-slot mailbox replaces the bounded channel, so workers never wait for validation; the metrics count superseded candidates. The final record reuses the accepted report instead of validating again, and the initial check is deadline-aware. The supervisor pinned every child to the first CPU of its affinity, so a pinned solve could share a core with any other pinned process; added `--cpu-offset`. This is a likely explanation for the 0.1.0 medium watchdog stop (8.65 s CPU in 15.04 s wall); the 0.1.0 records cannot confirm it. |
| I33 | **Ablations** (medium 14 s and 6 s, layered-hard 65 s, three seeds). Slide compaction was the largest component (1.3 points on medium). Pockets and separation were neutral. Added `rescue_after` to run them only after stalled rounds; the gated variant tied on layered-hard and gave the best 6 s medium mean. A patient 216 s layered-hard check gave 50.74 / 51.31% gated against 50.60 / 51.31% without them. |
| I34 | **Policy tuning.** The tuner covers ten search-control fields, averages 5 training seeds per candidate, always evaluates the default first, and checks the selected and default policies on 5 held-out seeds. BiteOpt (fcmaes-core 0.1.3) selected a policy at 98.27% training, but it reached 97.82% held-out against 98.16% for the default. Random search found nothing better. The default stays. |
| I35 | **Final checks.** Applied rustfmt and the Clippy 1.97 fixes (`is_multiple_of`, let-chains, both available at the declared Rust 1.88). Reran the six-case campaign with three seeds and the same-host `multistart` arm using the final binary. Updated the architecture, manual, README and performance documents and packaged the source and evidence. |
| I36 | **`extreme` case and memory measurement.** Added `cases/extreme.json` (3,309 optional figures, 606,594 vertices, 2.8-unit gap, 1800×1273 sheet, 184 s interactive budget) to the manifest and the harness case lists. No solver change; the rebuilt binary has the 0.2.0 hash. Every seed seats all figures (the 21.8973% stock bound) in about 1 s; the judge needs about 72 s per answer. The rerun showed about 172 MiB peak RSS even for tiny, because `wait4`'s `ru_maxrss` counts the forked copy of the harness before `exec`, and the harness held the parsed `extreme` case. The supervisor now records the solver's own `VmHWM`, sampled every 2 ms for the first 200 ms and every 25 ms afterwards, and uses only post-`exec` samples for the RSS limit; the raw value stays as `wait4_maxrss_bytes`. The seven-case campaign was rerun with the fixed harness. A first rerun with the old harness gave the same verdicts but inflated memory figures and was discarded. |

## Review decisions

- **Feasibility is unchanged.** Every acceptance still goes through the 0.1.0 conflict predicate, strict validation of each recorded candidate, and the Python judge in campaigns. Pole discs and raster maps only propose positions or rank overlapping states.
- **Default composition:** persistent search, slide compaction, pockets and separation gated by `rescue_after = 2`, strict acceptance, first-fit gap filling, and island sharing when there are two or more workers. Pockets and separation are on because they target stagnation and never measured clearly worse; the measurements do not establish a gain.
- **`multistart` stays** as the comparison arm for the 0.1.0 search. On the same host it reproduces the 0.1.0 medium numbers (47.53% for seed 1, 47.19% for seed 2).
- **Resume** keeps cumulative accounting and now continues with restarts plus persistent search. The checkpoint format is unchanged; older configs load with default features.
- **The tuner reports held-out results.** A training gain that does not hold on held-out seeds is not adopted.
- **Medium is saturated at its interactive budget.** It reaches the stock bound on some seeds, so differences there are mostly visible at shorter budgets or on layered-hard.

## Verification record

- `cargo fmt --check` passes. `cargo clippy --all-targets --all-features -- -D warnings` passes with Clippy 1.97.
- `cargo test --locked --all-features` passes 14 tests: 5 new unit tests, 3 accelerator tests and 6 contract tests, including the 10,000 rational-threshold decisions and the 700 transformed-body comparisons.
- `python tools/robustness.py` passes, also after the I36 supervisor change: resume, cumulative accounting, changed input, truncated checkpoint, protected input, shared work limit, SIGTERM preservation, process-group timeout, allocation and log limits.
- `python tools/judge_audit.py` reproduces the five recorded judge behaviours.
- Every answer of both final campaigns passes the native strict validator and the Python judge. The six original example answers in `examples/answers` come from the first final campaign, `extreme.answer.json` from the seven-case campaign, and all keep their recorded hashes.
- The packaged ZIP was extracted into a clean directory. There, `cargo build --release --locked`, `cargo test --locked --all-features` (offline, from the lockfile), `cargo fmt --check`, the judge audit and a bounded tiny/small smoke campaign all passed. The six example answers validated, and their SHA-256 values match the campaign records.

## Reproduction

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo build --release --locked --features fcmaes
python tools/judge_audit.py
python tools/robustness.py --binary target/release/budel
python tools/benchmark.py --out out/final-a --cases medium layered-hard --algorithms lns \
  --seeds 1 2 3 --budget interactive --workers 1 --pin-cpu --cpu-offset 0 --judge-seconds 240 &
python tools/benchmark.py --out out/final-b --cases tiny small large uniform-hard extreme --algorithms lns \
  --seeds 1 2 3 --budget interactive --workers 1 --pin-cpu --cpu-offset 1 --judge-seconds 240
wait
python tools/report.py out/final-a/campaign.json out/final-b/campaign.json
python tools/ablation_report.py evidence/v0.2.0/ablations.json
```

The ablation, island and patient-budget records (in `evidence/v0.2.0/`) and the tuning runs, whose reports are not in the repository, came from development builds made before the final rustfmt, Clippy and resume changes. Each was a single run (`budel solve ... --seed N` plus the switches shown in its record's `config`), run two at a time and not pinned. Each record keeps its full configuration. Only the final campaign and the `multistart` arm used the packaged source. See [architecture](docs/ARCHITECTURE.md), [user manual](docs/USER_MANUAL.md) and [performance](docs/PERFORMANCE.md).
