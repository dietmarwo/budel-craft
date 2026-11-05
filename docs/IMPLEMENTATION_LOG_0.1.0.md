# Budel Craft 0.1.0 implementation log

Archived log of version 0.1.0. Its numbers, answers and commands refer to 0.1.0; the 0.1.0 answers are now in `evidence/v0.1.0-answers/`. See the [0.2.0 log](../IMPLEMENTATION_LOG.md).

5 November 2026

The implementation is a Rust 1.88 project with a standalone solver, independent strict validation, optional fcmaes experiments and a supervised comparison harness. The ZIP includes the six original cases, checked example answers, architecture, manual, source, lockfile and raw benchmark records.

Four cases reach their stock coverage upper bound. All six packaged layouts pass both validators. Medium and layered-hard retain 92.14% and 88.99% of their available figure area respectively. The selected medium answer is a certified checkpoint saved before its watchdog deadline. A separate work-limited run exits normally with 47.2537% sheet coverage. See [the results table](PERFORMANCE.md).

| Case | Figures seated | Sheet coverage | Python judge |
| --- | ---: | ---: | --- |
| tiny | 2/2 | 5.0000% | valid |
| small | 40/40 | 23.2899% | valid |
| medium | 238/312 | 47.5282% | valid |
| large | 2,127/2,127 | 47.8051% | valid |
| uniform-hard | 1,008/1,008 | 26.9808% | valid |
| layered-hard | 1,015/1,184 | 46.2614% | valid |

## Iterations

| Iteration | Change and evidence |
| --- | --- |
| I00 | Froze the six inputs and plan, hashed the case bytes, transcribed the full pasted judge and reproduced its five audited boundary behaviors. The originally published reference answers were unavailable. |
| I01 | Implemented typed case inspection, finite coordinate and simple-ring checks, ID validation, area recomputation and original input SHA-256. All six cases parsed with the expected counts and area ceilings. |
| I02 | Implemented rigid poses and JSON export. Checked mirror–rotate–translate order on an asymmetric figure, exact eight-pose angles and floating-point round trips. Rendered original artwork to SVG. |
| I03 | Implemented scalar segment and polygon predicates with exact-dyadic orientation fallback. Verified 10,000 independent integer distance decisions plus collinear, near-parallel, containment and contact fixtures. |
| I04 | Added strict answer validation and area accounting. Tested paper holes and envelopes, mandatory completeness and finite input. Verified tiny at 5% coverage. |
| I05 | Added monotonic deadlines, shared proposal limits, atomic answer and checkpoint publication, and a bounded subprocess supervisor. Verified interrupted output, resume, staged input checks, wall timeout, hard allocation and output limits. |
| I06 | Added the dynamic spatial grid, oversized-query fallback and deduplication. Checked insertion, deletion and rollback against known geometry. |
| I07 | Added immutable edge hierarchy topology, transformed bounds and bounded pose reuse. The scalar and accelerated paths agree on 700 rotated and mirrored actual-case pairs in the regression test. |
| I08 | Added free-rectangle construction and true-polygon gap proposals. The first pilot seated 34/40 in small, 227/312 in medium and all 2,127 in large. The incomplete small output was recorded as partial. |
| I09 | Added bounded randomized restarts with preserved incumbents. The short pilot obtained a complete small layout and preserved exact area ranking when placed count differed. |
| I10 | Added local translation, rotation, reflection and gap filling. Kept the local-only arm available to isolate compaction from rebuilding. |
| I11 | Added area-aware blocker selection and replacement. Optional improvement uses unrounded area. Mandatory progress uses count followed by area. A rejected exchange restores its original placements and area. |
| I12 | Added capped neighborhood rebuilding. Native LNS obtained a fully certified small layout and reached 47.1121% on the two-second medium pilot. |
| I13 | Added coordinate-based repair for small isolated blocks. It uses geometric conflict tests with an overlap proxy to guide search. Retained it as an optional ablation. |
| I14 | Pinned fcmaes-core 0.1.3 and implemented bounded policy search, fixed/random controls and separate held-out evaluations. Retained the fixed default because the pilot covers one held-out seed. |
| I15 | Implemented capped DE pose blocks with explicit actual evaluation counts. Exercised block evaluations on medium. Layered-hard consumed its short pilot work cap before that phase. |
| I16 | Added the bounded portfolio and cancellation with per-worker RNG and shared immutable geometry. Ran 1-, 2- and 4-worker medium trials and tested a shared 75-proposal limit. |
| I17 | Measured fixed-work scalar and edge-tree arms, added ray pruning and short-circuit distance checks, and reran the geometry checks. Tree versus scalar improved CPU cost on the measured workloads. The earlier-tree comparison was noisy. |
| I18 | Shared exact body geometry and used repeated-body reflection hints and multiple area/extent orders. The uniform-hard toggles all reached the stock bound. The one-seed timing differences do not justify a performance ranking for the hint. |
| I19 | Added resumable benchmark records and report generation. Recorded all six dual-validated layouts, the work-limited medium rerun, memory and timing, plus selected algorithm ablations. |
| I20 | Added the versioned PackingSolver adapter with continuous/discrete orientations, reflection and original-origin conversion. The asymmetric pose conversion and tiny answer passed. The larger attempts reached the watchdog with empty incumbents. |
| I21 | Wrote the architecture, user manual with working commands, requirements and performance documents. Included six answers, a normal-exit medium example, three rendered layouts and a stock-area chart. |
| I22 | Built the pinned optimization-jobs runner and executed a staged tiny job using its existing command schema. Checked changed case hashes, truncated checkpoints, cancellation, resume and supervisor limits. |
| I23 | Prepared the source archive and manifest. The package retains the original case and plan bytes and records which benchmark configuration produced each included answer. |

## Review decisions

- A native original-ring validator certifies the nominal gap plus a guard. The Python judge is a second compatibility check with its original tolerance semantics. A saved strict certificate gives a clearance threshold across every pair.
- The reference checker was transcribed from the pasted source and has its own recorded hash. Scoring regressions verify its known behavior. Published reference placements and matched lab hardware were unavailable.
- The default algorithm is native LNS. The fcmaes policy and block pilots remain optional because the trials do not establish a general quality gain.
- The supervisor enforces a per-process address-space ceiling and records sampled group RSS plus the leader's measured peak RSS. A 160 MiB allocation failed under a 64 MiB ceiling in the bounded test. The recorded case results used older pilot settings where noted by their per-run limit fields.
- Atomic publication uses a flushed temporary file and a same-directory rename. Extra synchronous disk writes were removed from the hot checkpoint path. Process interruption recovery passed. Resume is a fresh search from the last certified arrangement with cumulative accounting.
- The medium time-limit trials preserve the earlier valid answer through a watchdog stop. The work-limited run demonstrates normal completion within the interactive allowance. Exact elapsed times are specific to the observed scheduling and filesystem conditions.
- The architecture reuses the existing optimization-jobs command protocol. The runner owns the execution result and Budel provides packing metrics and answers as typed-by-purpose artifacts.
- The measured campaign covers the interactive case settings and bounded pilots. The background matrix remains an additional experiment. The remaining density gaps on medium and layered-hard are against the available-stock ceiling.

## Verification record

- Rust formatting and Clippy passed with all targets and optional features. Nine focused tests passed, including the 10,000 rational-threshold decisions and 700 transformed-body differential comparisons.
- The source from checkpoint 67879e9 was extracted into a clean directory and passed the full offline all-feature Cargo test run. Later changes add reports and packaging metadata.
- The bounded robustness script passed after the hard allocation test was separated from sampled RSS observation. It exercises the native binary's actual solve, resume and validate commands.
- All six representative answer files were scored by both validators. The clean-package check verifies their hashes and the original input hashes against the raw campaign records.
- Source checkpoints are d6302f7 for the initial integrated solver and 67879e9 for profiling, job integration and guides.

## Reproduction

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo build --release --locked --features fcmaes
python tools/judge_audit.py
python tools/robustness.py --binary target/release/budel
python tools/benchmark.py --out out/recheck --cases tiny small \
  --algorithms lns --seconds 2 --aggregate-s 45 --judge-seconds 30
```

See [architecture](ARCHITECTURE.md), [user manual](USER_MANUAL.md), [requirements](REQUIREMENTS.md) and [performance](PERFORMANCE.md). The original plan is included for traceability. The evidence directory retains run configuration, source-data hashes and validator outputs.
