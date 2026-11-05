# Budel packing in Rust — iterative implementation plan

Date: 5 November 2026  
Status: implementation specification; packing performance has not yet been measured  
Working project name: `budel-craft` (proposed, not an existing repository)

Build a native Rust solver for the six supplied budel cases. Own the geometry and packing search, use normal infrastructure crates, and add `fcmaes-core` where controlled experiments show a benefit. Specialized nesting libraries are optional comparison tools, not required runtime dependencies.

The implementation must preserve every supplied figure, obey the stated spacing, return judge-compatible answers, and retain useful progress when a time or memory limit is reached. Correctness, reproducible measurements, and bounded execution take priority over speculative optimization.

This plan is intended to be usable as the starting context for another implementation session. It records the actual case data, decisions derived from the supplied judge, module boundaries, 24 ordered iterations, acceptance checks, benchmark policy, and delivery requirements.

## 1. Decisions that govern the implementation

1. **Use the original polygons.** Rotation, reflection and translation are the only placement transformations. No shape scaling, smoothing, redrawing or vertex removal in exported artwork.
2. **Separate artwork, collision geometry and scoring area.** Internal accelerators may approximate geometry conservatively; final acceptance uses original selected bodies.
3. **Implement direct polygon collision and distance queries first.** No-fit polygons, Minkowski-sum offsets and raster collision masks are not initial requirements.
4. **Pack solid outer bodies.** Closed holes do not provide space for another item under the supplied judge. Open recesses in an outer boundary remain usable.
5. **Use the nominal gap.** The default solver targets `min_channel` plus a small documented numerical guard, rather than deliberately consuming the judge's tolerance allowance.
6. **Keep the supplied judge unchanged.** Archive its complete source and run it as an external compatibility check. Reproductions of its defects belong in audit tests, not in the optimizer's acceptance policy.
7. **Distinguish mandatory from optional stock.** Mandatory cases are feasibility problems under the existing coverage metric. Optional cases maximize selected area subject to geometric feasibility.
8. **Start with one worker and reproducible work limits.** Introduce parallelism only after the serial solver and resource accounting work.
9. **Use fcmaes for policies and small local blocks.** A single unconstrained search over all positions, angles and reflection bits is not the initial architecture.
10. **Treat all quality and speed improvements as hypotheses until measured.** An unsuccessful optimization experiment can be completed and documented without becoming the default.

## 2. Frozen input and observed case properties

### 2.1 Case matrix

Coverage below is the total available scored area divided by the whole sheet area. It is an upper bound on valid answers, not a packing result or a proof that every figure fits.

| Case | Sheet | Usable rectangle | Figures | All required | Poses | Gap / tolerance | Available coverage |
| --- | --- | --- | ---: | --- | --- | --- | ---: |
| `tiny` | 100 × 100 | 90 × 90 | 2 | yes | eight | 2 / 0.01 | 5.000000% |
| `small` | 1800 × 1273 | 1748 × 1221 | 40 | yes | free | 3 / 0.05 | 23.289928% |
| `medium` | 1600 × 1800 | 1548 × 1748 | 312 | no | free | 3 / 0.05 | 51.585016% |
| `large` | 1273 × 1800 | 1221 × 1748 | 2,127 | no | free | 2.8 / 0.05 | 47.805073% |
| `uniform-hard` | 1600 × 1800 | 1548 × 1748 | 1,008 | no | free | 3 / 0.05 | 26.980758% |
| `layered-hard` | 1600 × 1800 | 1548 × 1748 | 1,184 | no | free | 3 / 0.05 | 51.984676% |

The margin is 5 in `tiny` and 26 in the other cases. All cases allow mirroring and use `body="rest"`. None supplies an envelope. The five nontrivial cases use `holes_are="ink"`; `tiny` defaults to paper holes but contains none.

| Case | Outer vertices min / median / max | Total outer vertices | Hole rings | Distinct exact outer rings |
| --- | --- | ---: | ---: | ---: |
| `tiny` | 4 / 4 / 4 | 8 | 0 | 2 |
| `small` | 101 / 214 / 358 | 8,552 | 87 | 40 |
| `medium` | 72 / 161.5 / 363 | 52,699 | 626 | 312 |
| `large` | 49 / 90 / 369 | 221,049 | 4,260 | 2,127 |
| `uniform-hard` | 69 / 167 / 329 | 171,622 | 2,048 | 508 |
| `layered-hard` | 69 / 166.5 / 332 | 203,404 | 2,379 | 1,184 |

Across the supplied files there are 4,673 figures and 657,334 outer vertices. All outer rings passed finite-coordinate, nonzero-edge, nonadjacent-contact and self-intersection checks in the preparatory audit. This is an input audit, not a certificate for future transformed layouts.

The five nontrivial cases use clockwise outer rings; `tiny` uses counterclockwise rings. Keep source geometry immutable and normalize any internal winding consistently. Reflection reverses winding and must be supported explicitly.

`uniform-hard` has 500 exact outer-ring pairs and eight singleton rings. Share immutable collision geometry for these pairs while retaining both original item IDs and their artwork metadata. The other files have no exact duplicate outer rings under the direct coordinate comparison used here.

The `area` fields are rounded to one decimal place. Recompute areas from rings for evaluation and judge comparisons. Size class, growth step and reach are descriptive fields; none permits generating extra stock or resizing an item.

At their supplied orientations, bounding rectangles occupy approximately 109.53%, 126.29% and 132.05% of the usable area for `small`, `medium` and `layered-hard`, before spacing. This does not prove anything about other orientations, but rules out treating the current boxes as a satisfactory final geometry model.

### 2.2 Runtime budgets and their provenance

| Case | Interactive seconds | Patient seconds | Background seconds | Stated lab seconds | Stated provenance |
| --- | ---: | ---: | ---: | ---: | --- |
| `tiny` | — | — | — | — | handwritten format example |
| `small` | 6 | 60 | 600 | 1.9 | measured |
| `medium` | 14 | 60 | 600 | 4.8 | measured |
| `large` | 92 | 308 | 1,848 | 30.8 | measured |
| `uniform-hard` | 56 | 185 | 1,110 | 18.5 | estimated from an earlier curve |
| `layered-hard` | 65 | 216 | 1,296 | 21.6 | estimated from an earlier curve |

These lab times are supplied metadata, not timings reproduced in this work. Hardware, thread counts and reference placements are not supplied. Do not turn them into speedup claims.

### 2.3 Immutable input fingerprints

Retain byte-identical copies and record these SHA-256 values in a machine-readable manifest. A changed file is a changed benchmark.

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `tiny.json` | 932 | `f0181bafa8f64a8fc78d37578a69c48d4e54855b87715c149830435a33b0135c` |
| `small.json` | 148430 | `44ffd9746de20b92288665b528029d7834f0d72c366ac1ed356120c9f9f611e0` |
| `medium.json` | 917857 | `269b7289e2bf5d865af8aad923db8edc272a3f30f547ed94aae741134cd499c9` |
| `large.json` | 4281064 | `99d8d8135395435e91510dbce52b9d7679879dc06a6bc19e21bf4bfee67d7123` |
| `uniform-hard.json` | 2941630 | `2a823069a98be1f7c2fb4461853956b4857eb8cbbce3f929a124fcaa5223862a` |
| `layered-hard.json` | 3363866 | `f60887e32e825d46e2b74bac12ed6fded060bf20a8fdf9d2e3ee9797e83d0688` |

## 3. Solver contract and objective

### 3.1 Placement representation

For local point `p`, reflection flag `b`, angle `theta` in degrees and translation `t`:

`T(p) = R(theta) * diag(if b then -1 else 1, 1) * p + t`.

Reflection is across the local Y axis. Rotation is about the input coordinate origin, not the centroid. If internal recentering is used, the exporter must compensate for it, and a round-trip test must compare transformed original vertices.

An answer contains `case`, `placements`, and optional author/runtime metadata. Each placement writes explicit `id`, `x`, `y`, `rot`, and `mirror` values. Use sufficient floating-point precision for round trips; do not round translations to the input's two-decimal coordinate grid. Output angles in `[0,360)`, using exact multiples of 45 degrees for the eight-pose case.

For the proposed parser, require finite positive sheet dimensions, a nonnegative margin and channel, a valid usable rectangle, simple positive-area body rings, valid unique IDs, and correctly typed flags. Reject unsupported rule values clearly. Validate hole geometry where it affects area or drawing. Strict parsing intentionally rejects malformed inputs that the supplied Python code may crash on or coerce unexpectedly.

Honor the supplied general ring/envelope selection rule in the adapter, although the initial six cases only use rings. Preserve holes for scoring and rendering. Resolve the effective margin once, with `rules.margin` taking precedence over `plane.margin`.

### 3.2 Validity and numerical policy

A certified answer must satisfy identity/count rules, pose permissions, containment in the usable rectangle, no body overlap or contact, and nominal inter-item spacing. For a nominal gap `g`, search for `distance >= g + guard`; the guard is scale-aware and independently recorded. Start by testing a guard such as `max(1e-6, 64 * machine_epsilon * coordinate_scale)`, then validate it against the supported scale range. This is an engineering tolerance, not a formal exact-arithmetic guarantee.

The supplied judge accepts non-overlapping gaps down to `g - tolerance`. Keep its acceptance result separate from the stricter solver validity result. The guard should not silently add half the item gap to the sheet margin.

For zero-gap synthetic tests, touching must still be rejected. Use explicit boundary-contact and containment checks rather than assuming positive clearance will hide overlap defects. If a floating-point predicate is indeterminate, conservatively reject or use a higher-precision fallback; never accept an uncertain collision query merely for speed.

Use squared distances in acceptance queries. Two disjoint 2D segments have their minimum distance at an endpoint projection; combine endpoint-to-segment checks with proper intersection, endpoint contact and collinear-overlap classification. Ring boundary distance alone does not detect one body contained in another: containment is a separate necessary test.

### 3.3 Ranking and termination

| Situation | Primary decision | Secondary policy |
| --- | --- | --- |
| Optional stock | Maximize unrounded selected scoring area among valid layouts | Stable deterministic tie-break; a documented layout heuristic may guide equal-area exploration |
| Mandatory stock, incomplete search | Reduce missing stock using an internal construction/repair objective | Partial arrangements are progress artifacts, not valid answers |
| Mandatory stock, all placed | Record a complete feasible answer and time to first feasibility | Stop by default; optional compaction is a separately labeled experiment |
| All optional stock placed | Record that the available-area upper bound is attained | Stop by default unless a secondary objective was explicitly requested |

For optional stock, the score is proportional to the sum of selected outer areas, minus paper-hole areas where applicable. Count is a diagnostic, not a substitute for area. A replacement of several small items by one large item can improve coverage even if count decreases.

Coverage on a fixed sheet does not depend on how compactly a fixed selected set is arranged. Compaction matters only as a means to admit more items or as an explicitly separate aesthetic objective. `border_free_share` is reported but is not silently promoted to an official tie-breaker.

Internal working states may temporarily be incomplete or overlapping during repair. They must never replace the best certified incumbent. Failure to find a complete mandatory layout is reported as no feasible solution found within the budget, not as proof of infeasibility.

### 3.4 Known judge issues and compatibility fixtures

| Issue reproduced during review | Treatment in this project |
| --- | --- |
| Zero-gap corner contact can pass | Strict validator rejects it |
| Coincident squares can pass with suitable starting vertices and report 200% coverage | Regression fixture; never exploit this behavior |
| `plane.margin` fallback differs between validation and measurement | Resolve margin consistently internally; retain original judge behavior in external reports |
| Eight-pose tolerance differs just below and above zero | Export canonical exact permitted angles |
| `tightest` only covers a limited neighborhood | Label judge diagnostic honestly; provide a separate global minimum only if actually computed |
| Runtime declarations do not affect validity | Enforce deadlines in the solver and supervising runner |
| Malformed/non-finite input can escape graceful handling | Validate before building geometry or indexes |

All supplied cases have positive effective clearance and matching margin fields, so the first and third issues are not the main obstacle for these files. The full pasted judge includes CLI/rendering code; the temporary audit extracts used in the discussion are not a replacement for that full source.

## 4. Architecture and dependency boundaries

Begin with one library crate and one CLI binary in a small Cargo project. Split into multiple crates only if measured build/API needs justify it.

| Module or path | Responsibility |
| --- | --- |
| `src/input.rs`, `src/model.rs` | Typed JSON adapter, immutable source shapes, rules and per-item metadata |
| `src/geometry/{predicates,distance,transform,bounds}.rs` | Scalar reference geometry and numeric policy |
| `src/geometry/{grid,edge_tree,cache}.rs` | Spatial acceleration and bounded pose caches |
| `src/layout.rs` | Placements, unplaced IDs, transactional move/rollback and objective totals |
| `src/search/{construct,local,select,lns,repair}.rs` | Construction and search operators |
| `src/search/fcmaes.rs` | Optional policy and local-block optimizers |
| `src/runtime/{budget,checkpoint,portfolio}.rs` | Cancellation, deadlines, recovery and workers |
| `src/validate.rs`, `src/export.rs` | Original-geometry validation, answer serialization and SVG rendering |
| `src/metrics.rs`, `src/bin/budel.rs` | Counters, run metadata and CLI |
| `tools/` | Bounded external judge runner, independent oracle, report generation |
| `tests/`, `benches/`, `docs/`, `cases/manifest.json` | Regression fixtures, profiling corpus, documentation and data identities |

### 4.1 Core data model

- `SourceItem`: external ID, original ring, holes, optional envelope, descriptive metadata.
- `Geometry`: immutable selected collision body, area, edges, local bounds, optional acceleration structure; sharable across identical bodies.
- `Pose`: translation, normalized angle, reflection bit; never a scale factor.
- `PlacedItem`: item index, pose, transformed conservative bounds and cached geometry generation.
- `Layout`: placements plus missing IDs and a dynamic spatial index.
- `LayoutTransaction`: temporary removals/insertions with exact commit or rollback.
- `Incumbent`: independently accepted snapshot with objective, validity and discovery time.
- `Budget`: shared monotonic deadline, work counters, cancellation and reserved finalization allowance.

Provide distinct query methods for `violates_clearance`, `is_inside_usable_plane`, and optional diagnostic distance. A boolean query can stop early. A global minimum-distance report cannot reuse a limited-radius query and pretend it examined every pair.

### 4.2 Geometry acceleration rules

Broad-phase filtering must be conservative: expand query bounds by the target clearance and account for numerical roundoff. Overlapping boxes are candidates, not proof of polygon overlap. Clearly separated enclosing boxes can prove separation.

Use a mutable item-level spatial grid first and immutable edge hierarchies when profiling supports them. Transform a moving shape into reusable scratch storage; avoid allocating a vertex-pair matrix. Pose caches have explicit byte limits and keys including geometry identity, angle and reflection. Rounded-angle cache keys cannot stand in for exact poses without a proven error bound and final recomputation.

If conservative polygon simplification is later introduced, the simplified body must contain the original body. Separation of enclosing surrogates proves separation; surrogate overlap may require refinement. Generic polyline simplification is not automatically conservative.

### 4.3 Dependencies and integrations

Normal infrastructure crates for JSON, CLI handling, RNG, hashing and diagnostics are acceptable. Keep the geometry and packing implementation native Rust. The core should compile and solve with fcmaes integration disabled; enable it through a feature after a baseline exists.

`fcmaes-core` supplies native optimizers and retry/batch interfaces; use it as a dependency rather than copying optimizer implementations. Pin an actual release or commit and commit the lockfile during implementation. API signatures and cancellation behavior must be checked at that pinned version. [Source: fcmaes-rust repository](https://github.com/dietmarwo/fcmaes-rust).

Before emitting integration-level `run.json` records, inspect the current shared result contract and existing runner implementation. Reuse the appropriate existing schema and requested-versus-actual evaluation semantics. The historical `tutorials/RESULT_SCHEMA.md` path must be resolved against the pinned checkout, not assumed unchanged. Keep packing-specific metrics in a documented artifact rather than inventing a competing generic job protocol.

Use shell-free supervised subprocesses for the Python judge and external solvers. Existing process-group and durable-artifact support in the optimization-jobs ecosystem should be reviewed before writing a second implementation. This is a later adapter; it does not place JSON or subprocess boundaries inside a geometry evaluation. [Source: optimization-job-api](https://docs.rs/optimization-job-api/latest/optimization_job_api/).

## 5. Iteration procedure

Every iteration has a small coherent implementation scope and a finite verification budget:

1. Record its starting commit, relevant input hashes and the exact hypothesis.
2. Implement the scope, preserving the best previous baseline configuration.
3. Run focused correctness tests first. Geometry, state mutation and concurrency changes require invariant and differential tests.
4. Run only the bounded pilot specified for the iteration. Record failed and cancelled attempts too.
5. Accept a change as the default only if the applicable correctness and evidence gates pass. A negative optimization result is documented and left disabled or removed.
6. Update the implementation log, milestone checklist and any documentation whose claims changed.
7. Commit the coherent change and checkpoint the next step. Rebase patch delivery onto the actual target revision when a repository already exists.

Each log entry records files changed, commands, results, elapsed time, memory observations, before/after metrics, remaining limitations, and the next iteration. Do not mark an unrun test or benchmark as passed.

No iteration loops indefinitely until it gets a desired score. A failed gate produces a reproducible blocker or a bounded follow-up hypothesis. Continue independent work where possible, but do not promote algorithms on top of an unresolved geometry correctness failure.

## 6. Milestones and ordered iterations

| Milestone | Iterations | Reviewable outcome |
| --- | --- | --- |
| M0: contract and trusted geometry | I00–I04 | Parsed cases, exact transform contract, strict validator and judge audit fixtures |
| M1: bounded baseline | I05–I09 | Deadline-aware constructor, original-geometry validation and reproducible results |
| M2: useful packing search | I10–I13 | Local improvement, stock selection, neighborhood rebuilding and isolated repair |
| M3: measured fcmaes integration | I14–I16 | Policy/block experiments and controlled parallel portfolio |
| M4: performance and comparison | I17–I20 | Profiled kernel, case-aware tactics, reproducible benchmark suite and external comparison |
| M5: usable delivery | I21–I23 | Documentation, runner integration, robustness checks and packaged source/evidence |

### I00 — Freeze the specification and evidence

**Goal:** establish exactly which problem is being solved before creating a search algorithm.

**Implementation:** create `docs/requirements.md`, the case manifest, `docs/judge-audit.md`, and an iteration log. Preserve the complete supplied `score.py` as `tools/reference_score.py` with a recorded hash and provenance. Resolve the output ranking as specified in Section 3, including mandatory-case time to feasibility. Record optional aesthetics as separate experiments. Locate the current fcmaes result/runner contracts without yet adding a dependency.

**Verification:** reproduce case counts, area ceilings and fingerprints. Recreate the coincident-square, contact, tolerance, margin and angle-wrap judge examples. Check that the archived judge includes its original CLI and rendering functions rather than just the earlier audit extracts.

**Acceptance:** requirements and data manifest agree with all six files; judge deviations are explicit. Missing reference answers are recorded as unavailable rather than reconstructed or assumed.

**Budget and checkpoint:** data inspection and tiny synthetic tests only; no packing search. Checkpoint the source/manifest hashes and unresolved external contract paths.

### I01 — Create the Rust project and typed input adapter

**Goal:** inspect every case reliably with no optimizer present.

**Implementation:** create the library, `budel inspect` CLI and initial README. Parse source shapes into immutable records; build an external-ID-to-index map. Resolve effective rules, choose the body, calculate unrounded areas and retain descriptive metadata. Detect invalid dimensions, non-finite coordinates, degenerate edges/rings, invalid hole data and duplicate IDs with actionable errors. Hash the original input bytes. Do not trust the rounded `area` field as the objective value.

**Verification:** load all supplied cases and compare statistics against Section 2. Use malformed synthetic fixtures for missing fields, wrong booleans, duplicate IDs, empty rings and impossible margins. Check that normalization does not alter original exported coordinates.

**Acceptance:** every supplied case loads, invalid fixtures fail cleanly, and the inspector produces deterministic machine-readable statistics.

**Budget and checkpoint:** one pass through each file under a timeout. Record parsing/preprocessing time separately and select a pinned toolchain and dependency lockfile.

### I02 — Implement rigid poses and answer round trips

**Goal:** make geometry transformations and output conventions unambiguous.

**Implementation:** implement reflection, rotation and translation in the stated order. Add eight-pose canonicalization, free-angle normalization, pose permissions, transformed AABBs and JSON answer import/export. Add SVG rendering directly from original geometry with item identity retained. If geometry is recentered internally, implement and document translation compensation in the exporter.

**Verification:** use an asymmetric polygon with a nonzero local origin. Compare transformed vertices against the supplied Python function for reflected and unreflected poses, negative angles and wraparound. Include mirror-only placement fixtures so mirroring cannot accidentally become an ignored flag. Reload serialized answers and recompute their vertices and area.

**Acceptance:** allowed poses survive round trips, disallowed poses are rejected, and the source-to-output transform preserves size and shape within the stated floating-point tolerance.

**Budget and checkpoint:** hundreds of tiny deterministic poses; no large-case search. Save an illustrative tiny answer and its renderer output as a format fixture, not as a performance result.

### I03 — Build the scalar geometry reference kernel

**Goal:** have a simple, auditable correctness baseline before adding acceleration.

**Implementation:** implement orientation classification, point-on-segment, segment intersections including contact/collinearity, point containment and squared segment distance. Handle zero-length segments defensively even though the loader normally rejects them. A polygon pair must be checked for both boundary interactions and containment. Give uncertain numeric decisions an explicit conservative outcome. Avoid the judge's vertex-pair array allocation pattern.

**Verification:** analytic rectangles and triangles; coincident/reversed/cyclically shifted rings; containment without edge crossings; parallel and near-parallel edges; endpoint-to-edge contact; nested concavities; threshold gaps. Run bounded randomized differential tests against an independently expressed endpoint-projection/intersection implementation. For ambiguous cases, use higher precision or an optional test-only robust geometry oracle. The supplied judge alone is not a correctness oracle.

**Acceptance:** no unexplained disagreement, symmetry under swapping the pair, and invariance under common rigid transforms. Preserve failing seeds as small regression fixtures.

**Budget and checkpoint:** start with 1,000 random small pairs, then 10,000; cap each test process. Retain this scalar implementation as a reference for all later accelerators.

### I04 — Add strict layout validation and objective accounting

**Goal:** independently certify answers and distinguish certification from judge acceptance.

**Implementation:** validate IDs, completeness, poses, sheet containment, overlap/contact and nominal clearance on original selected bodies. Calculate scoring area, coverage and fraction of available area placed. Implement explicit mandatory/optional ranking. Store `strict_valid`, `judge_valid` and the judge invocation status separately; unavailable, timed-out or crashed judge checks are not successful checks.

**Verification:** all judge defect fixtures plus valid arbitrary-order layouts. Check that paper holes change area but do not enable nesting. Check envelope selection on a synthetic case. Verify that moving an unchanged selected set preserves its coverage, and that invalid layouts cannot become incumbents even when their reported area is high.

**Acceptance:** the tiny case has a fully validated complete answer; an incomplete mandatory answer is clearly marked unsuccessful. Before/after serialization checks agree. General distance diagnostics state whether their search was global or radius-limited.

**Budget and checkpoint:** tiny and small synthetic layouts. Keep validation timings separate from solver timings until the budget controller is implemented.

### I05 — Add deadlines, atomic checkpoints and process supervision

**Goal:** prevent hanging searches, lost progress and uncontrolled memory use from the start.

**Implementation:** introduce monotonic deadlines, work limits, cancellation and a reserved finalization allowance. Save the best certified answer atomically and keep a small bounded set of working checkpoints separately. Add seed/configuration/input hashes and cumulative work/time on resume. The supervisor starts shell-free process groups, enforces wall and memory limits, limits captured output, and terminates the whole child group on failure. Reuse existing ecosystem supervision code where its API and behavior fit.

**Verification:** inject cancellation during construction, local search, serialization and external judging. Exercise truncated checkpoint files, a stalled child and a memory-cap violation using safe synthetic workloads. Verify that an earlier valid incumbent survives and that restarting never silently resets the accounting of a resumed experiment.

**Acceptance:** each run ends with a useful status and recoverable artifacts. No complete mandatory solution means no misleading valid answer. Deadline overshoot and unverified finalization are visible.

**Budget and checkpoint:** second-scale interruption tests, never intentional OOM of the host. Start with the limits in Section 7 and calibrate finalization reserves from actual measurements.

### I06 — Add a conservative item-level spatial grid

**Goal:** reduce polygon comparisons to nearby candidates without changing answers.

**Implementation:** index each transformed AABB in all covered grid cells, query with clearance-expanded bounds, deduplicate candidates using reusable stamps, and handle negative cell coordinates correctly. Support insert, remove, move and rollback. Bound cell enumeration for pathological inputs and retain a fallback list for unusually large bodies. Do not build a global all-pairs set on every move.

**Verification:** compare candidate detection and final validity against brute force over random layouts. Test edges on cell boundaries, different body sizes, mirror/rotation updates and stale index entries after rejected moves. Include mixed-size subsets from `layered-hard`.

**Acceptance:** no missed violating pair in the differential corpus; rollback restores both geometry and index state exactly. Counters expose candidates per query and grid occupancy.

**Budget and checkpoint:** synthetic layouts of 10, 100 and then 1,000 bodies under fixed work limits. Retain several cell-size choices for later measured tuning; avoid selecting one solely from the case name.

### I07 — Add allocation-free edge queries and optional edge hierarchies

**Goal:** reduce the cost of complex 100–369-vertex body comparisons.

**Implementation:** reuse transformed-coordinate and candidate buffers. First reject edge pairs by conservative edge AABBs; then add a compact immutable edge hierarchy if profiling supports it. The moving body's local hierarchy may be queried through a conservative transformed representation, with exact segment evaluation at leaves. Separate threshold queries from full diagnostic distances. Add bounded pose caching for frequent discrete angles.

**Verification:** compare all accelerated results to I03 for actual-case ring pairs at randomly generated poses and threshold-sensitive positions. Check cache invalidation on reflection and geometry changes. Measure allocations and peak scratch usage. Include long thin curls so aggressive bounding approximations cannot hide collisions.

**Acceptance:** accelerated and scalar predicates agree throughout the corpus. Keep an accelerator only if measured cost improves its intended workload; small rings may continue using a simple loop.

**Budget and checkpoint:** fixed query batches with a short timeout, not an unbounded microbenchmark. Record throughput separately for easy rejection, near contact, containment and fully separated overlapping boxes.

### I08 — Implement a deterministic constructive baseline

**Goal:** produce actual placements through a comprehensible algorithm.

**Implementation:** order items by several documented keys such as area, bounding extent and concavity proxy. For each item, generate candidate positions near the sheet boundary and existing placed bounds, plus a bounded spatial sampling fallback. Test permitted angles and both reflection states. Use real polygon checks; boxes only prioritize candidates. Maintain separate optional-stock and mandatory-progress behavior.

**Verification:** solve `tiny` fully; run short bounded attempts on `small` and a subset of `medium`. Verify every emitted answer through strict validation and the original judge. Check unknown/duplicate ID handling and ensure a failed insertion does not discard earlier placements.

**Acceptance:** a reproducible baseline exists with candidate counts, seated area, missing stock, time and validation outcomes. A partial `small` layout is logged honestly and is not a passed mandatory case.

**Budget and checkpoint:** one seed/configuration, at most the interactive allowance per attempted case. Preserve this constructor as a permanent comparison arm before adding search.

### I09 — Add randomized construction and bounded restart selection

**Goal:** explore placement orders and pose sampling without relying on one lucky construction.

**Implementation:** perturb deterministic priority keys, vary insertion strategies and sample additional free angles. Always preserve the selected items' original geometry. Allocate a fixed attempt budget among restarts and carry the best certified layout forward. Keep random streams explicit and reproducible for evaluation-limited runs. Document that wall-clock cutoffs and parallel scheduling can change the number of evaluated candidates.

**Verification:** compare the constructor and restart portfolio on matched seeds, worker counts and time limits. Log all runs, including failed mandatory attempts. Verify that the retained optional-stock incumbent's unrounded area never decreases.

**Acceptance:** the multi-start arm is selectable and reproducible; promote it by evidence rather than by the best single seed. No invalid candidate is admitted to the incumbent archive.

**Budget and checkpoint:** three small pilots before longer runs. Use one worker and cap total pilot time; do not multiply the full background budget by every experimental ordering.

### I10 — Add local translation, rotation and reflection moves

**Goal:** open useful space around placed items and improve future insertion opportunities.

**Implementation:** add translation steps, free-angle refinement or exact discrete orientation changes, reflection flips and move-to-gap proposals. Treat each proposal as a transaction with fast local revalidation. For equal-area layouts, use explicitly internal guidance such as occupied extent or insertion opportunities; keep the best official objective separately. Repeatedly retry a bounded set of unplaced items after useful rearrangements.

**Verification:** mirrored asymmetric fixtures, tight gaps, sheet-edge moves and invalid transactions. Compare fixed work and fixed wall-time arms against I09. Check that the objective increases only when additional scored area is placed, not just because the picture looks more compact.

**Acceptance:** accepted moves preserve the strict rules, rejected moves leave state unchanged, and improvement counters identify the operations that create actual area gains.

**Budget and checkpoint:** start on `small` and `medium` subsets; cap iterations and stagnation periods. Stop mandatory runs once complete unless a separate secondary experiment is selected.

### I11 — Add area-aware stock selection and replacement

**Goal:** solve the optional-stock objective rather than maximizing item count accidentally.

**Implementation:** maintain an unplaced-item pool with area, size and candidate-gap information. Try inserting omitted items, replacing one placed item by several others, and replacing a local group by a more valuable group. Evaluate the net change in exact scoring area; use area-to-placement-cost features only for proposal priority. No class quota is inferred from descriptive metadata.

**Verification:** synthetic cases where fewer large items win, more small items win, and rounded `area` fields would select the wrong alternative. Check ID uniqueness and full rollback of removed groups. Test bounded runs on `medium` and `layered-hard`.

**Acceptance:** replacement decisions follow unrounded area and strict feasibility. Report both placed count and area so improvements cannot be misdescribed.

**Budget and checkpoint:** bounded group sizes and candidate pools. Keep a no-replacement ablation and avoid enumerating every subset of the stock.

### I12 — Add large-neighborhood rebuilding

**Goal:** escape configurations that cannot improve through isolated moves.

**Implementation:** select a spatial neighborhood, a congested boundary, or a group near an unplaced valuable item; remove a capped number of figures and reconstruct locally. Start with 4–16 items, increasing only after profiling. Vary group size and selection by measured operator utility. Keep the untouched layout as fixed obstacles. Full rollback is mandatory on failure.

**Verification:** known trap fixtures where a group must move together; random transaction sequences; stale-cache and missing-ID tests. Compare area-versus-time curves against I11 and count successful rebuilding operations.

**Acceptance:** the search can make joint changes while retaining a valid incumbent, and uses bounded state/memory. Before declaring the solver complete on mandatory benchmarks, obtain a fully valid `small` answer; if it remains unsolved, record the failure as a quality blocker rather than weakening rules.

**Budget and checkpoint:** a per-neighborhood deadline as well as the global deadline. A failed neighborhood is abandoned cleanly; it cannot monopolize the run.

### I13 — Add isolated overlap repair for difficult neighborhoods

**Goal:** explore small temporary infeasible neighborhoods when feasible-only moves are trapped.

**Implementation:** keep a separate repair state for the selected local group. Use informative surrogate penalties for outside distance, crossings, interior samples and clearance deficits. A plain nonnegative boundary distance is flat inside overlap and is not sufficient as the sole repair objective. Sample-based penalties guide search but never certify feasibility. Return to the main layout only after all original-body checks pass and the accepted objective/tie policy permits the change.

**Verification:** containment, crossed thin curls, deeply overlapping starts and infeasible trapped neighborhoods. Verify that incomplete repair cannot overwrite a valid checkpoint. Compare repair enabled/disabled on identical starting layouts and budgets.

**Acceptance:** repair is optional and bounded, with explicit success/failure accounting. If it does not improve quality enough to justify its cost, complete the experiment and leave it disabled by default.

**Budget and checkpoint:** cap neighborhood size, penalty evaluations and repair wall time. No full-sheet all-overlap initialization with thousands of figures.

### I14 — Add fcmaes policy optimization

**Goal:** test whether optimizing a small placement policy improves the custom search.

**Implementation:** pin and inspect `fcmaes-core`, then add an optional adapter. Begin with roughly 8–16 bounded parameters controlling priority weights, pose-sampling allocation, restart fractions, insertion heuristics and neighborhood selection. Use deterministic seeds/scenarios during each policy comparison. Keep the complete geometry and objective evaluation in Rust. Inspect existing result schemas and record requested/actual evaluations and cancellation without inventing incompatible generic statuses.

**Verification:** compare an untuned policy, a hand-configured policy, random policy search and an fcmaes method under equal tuning cost. Use disjoint development and validation seeds; include independent synthetic/subset instances and report if tuning used the six official cases. Evaluate selected policies on held-out runs before choosing defaults. All nested solves share the outer tuning deadline.

**Acceptance:** a reproducible policy experiment and a documented decision. An unsuccessful search does not justify a dependency becoming mandatory. Preserve the untuned baseline.

**Budget and checkpoint:** first use cheap, fixed-work subset runs. Bound total policy evaluations and cumulative solver work; do not nest hundreds of full background solves inside one objective.

### I15 — Add fcmaes optimization of small pose blocks

**Goal:** test joint continuous adjustment where a geometry-aware neighborhood supplies the structure.

**Implementation:** optimize 2–8 selected figures at a time, initially 6–24 continuous coordinates for free translation/rotation. Normalize coordinates to the neighborhood scale. Fix other figures as obstacles. Handle reflection and eight-pose choices through explicit discrete proposals or separate branches; do not pretend reflection is a smooth coordinate. Start with DE and, where useful, a CMA-ES refinement; verify the actual pinned APIs and stopping behavior.

**Verification:** compare joint block search against equal-budget coordinate moves, random block proposals and I13 repair from the same snapshots. Validate all returned poses independently. Check angle wraparound and that an unproductive optimizer obeys its local budget.

**Acceptance:** retain only methods that improve area, mandatory success rate or time to a target under a declared budget. Keep detailed operator statistics and the no-fcmaes ablation.

**Budget and checkpoint:** one small block before any batch; hard dimension and evaluation caps. Do not expand to thousands of pose variables merely because a generic optimizer accepts them.

### I16 — Add a controlled parallel search portfolio

**Goal:** use multiple CPU cores while preserving correctness, accounting and bounded memory.

**Implementation:** give workers independent mutable layouts and random streams, sharing immutable source geometry. Partition the global budget among serial search lanes; synchronize only improved certified incumbents and compact statistics. Publish snapshots atomically. Use small worker counts first. Avoid nested retry, population and external-solver thread pools exceeding the declared worker budget.

**Verification:** compare 1, 2 and 4 workers, then higher counts supported by the host. Test cancellation while workers publish results, duplicate-result races and late completion after the deadline. Track total CPU time, peak process-tree memory and actual worker count. Revalidate imported incumbents before use.

**Acceptance:** no concurrency-related validity failure or lost incumbent. Distinguish better quality at equal wall time with more cores from better efficiency at equal aggregate CPU work.

**Budget and checkpoint:** one short case per worker count before campaigns. Derive the maximum worker count from measured per-worker state and the configured memory cap.

### I17 — Profile and improve geometry throughput

**Goal:** optimize the measured bottleneck without sacrificing feasible space silently.

**Implementation:** profile transform cost, grid candidates, edge-tree traversal, exact distance work, allocations and cache misses. Address one bottleneck per sub-iteration: data layout, scratch reuse, better traversal ordering, bounded pose caches, or conservative simplification. SIMD is an optional measured experiment after the scalar architecture is sound. Retain an original-body validation path and a way to disable each accelerator.

**Verification:** replay a frozen query corpus and end-to-end search workloads. Compare predicate outcomes, memory, throughput and packing quality. Conservative simplification may reduce achievable quality even when safe; measure that tradeoff. Verify that early exits stop only when the query's requested condition is actually decided.

**Acceptance:** every speed claim has a workload, baseline, compiler/configuration and raw measurements. No accepted change introduces a false-negative collision or a hidden unbounded cache.

**Budget and checkpoint:** one candidate change and one bounded benchmark at a time. If geometry already accounts for little runtime, redirect effort to the search rather than chasing microbenchmark gains.

### I18 — Add tactics for repeated shapes and mixed sizes

**Goal:** exploit observed structure without hard-coding answers to case names.

**Implementation:** share exact duplicate collision bodies and orientation caches. Test paired placements for repeated shapes when the two-item arrangement is itself feasible. For mixed-size stock, compare large-first skeleton construction, small-item gap filling, and revisiting earlier large-item decisions. Any stored group pattern retains all original IDs and an internally validated rigid arrangement. A union or enclosing box must not silently replace the group's exact body for final checks.

**Verification:** use `uniform-hard` for repeat reuse and `layered-hard` for scale variation. Add synthetic layouts with no repeats and misleading size priorities. Compare preprocessing cost, memory, first-answer time and final area with tactics disabled.

**Acceptance:** use detected features, not a switch on `case == ...`. Geometry sharing must not merge stock identities. Repeated-pattern or size tactics become defaults only for supported measured conditions.

**Budget and checkpoint:** cap pattern count, candidate partners and cache bytes. Pattern generation belongs inside cold-run runtime accounting unless explicitly reported as a separate warm-start experiment.

### I19 — Complete the benchmark and reporting harness

**Goal:** turn development observations into reproducible case-level evidence.

**Implementation:** implement the benchmark matrix in Section 7, including seeds, time/work budgets, strict validation, original-judge results, resource accounting and raw artifacts. Generate tables and scientific plots from raw records. Include baseline construction, the best custom search, fcmaes ablations and chosen worker counts. Track time to first valid mandatory answer and to available-area completion.

**Verification:** replay saved answers independently; inspect failed and timed-out runs; deliberately feed an invalid high-coverage answer and ensure it cannot win a report. Verify that successful-run-only summaries cannot hide failures. A zero denominator or absent global distance must be reported explicitly.

**Acceptance:** a clean checkout can reproduce a bounded smoke campaign and regenerate its report. Published results say which budgets/configurations were actually run. All claims link to answer files, logs and input/configuration hashes.

**Budget and checkpoint:** start with interactive runs and three seeds. Promote to patient/background budgets selectively. Print planned total compute before launching a campaign and enforce an aggregate cap.

### I20 — Compare with external nesting baselines

**Goal:** determine whether the custom approach is useful relative to established alternatives.

**Implementation:** create adapters for at least one available solver, initially evaluating PackingSolver and/or Sparrow. Inspect the pinned version's actual feature behavior. Convert original bodies and units faithfully, preserve IDs, and validate the converted output with the same strict validator and supplied judge. Document strip-versus-fixed-sheet and optional-selection differences. Native solver objective values are not substitutes for our case objective.

**Verification:** test the adapter on tiny asymmetric and mirror-required examples before real cases. Check continuous/eight-pose restrictions, spacing-to-margin conventions and output origins. If a baseline does not search mirrored placements or lacks fixed-sheet stock selection, label the restricted comparison or implement a separately costed adapter. Include conversion, startup and preprocessing in end-to-end timing.

**Acceptance:** report quality and runtime honestly under comparable rules. If a dependency cannot be installed or a feature cannot be matched, record that baseline as unavailable/restricted, not as defeated. Compare an optional jagua-rs geometry adapter only if a measured kernel bottleneck makes it useful.

**Budget and checkpoint:** one tiny check and one short real-case run per baseline before expanding. Keep third-party dependencies outside the default native solver build.

### I21 — Write architecture, tutorials and visual diagnostics

**Goal:** make the implementation understandable and usable without conversation history.

**Implementation:** complete the README, architecture document, solver-contract guide, a tutorial using `tiny` then a real case, and a performance report. Explain local-origin transforms, reflection, score ceilings, stock selection, gap tolerance, deadlines and checkpoint recovery. Include rendering of original shapes, selected IDs and optional diagnostic overlays. Show per-class counts as descriptive statistics.

**Verification:** execute every documented command in a clean test directory. Check generated figures against original geometry and validated answers. Build report tables directly from stored results, not copied numbers. Review every Markdown file for stale benchmark claims or promises of unimplemented features.

**Acceptance:** a new user can inspect a case, solve it under a chosen budget, validate and render the result, and understand a failed mandatory run. Documentation distinguishes proposed, implemented and measured features.

**Budget and checkpoint:** small documentation examples only. Avoid rerunning full campaigns merely to regenerate a chart or change prose.

### I22 — Add the existing job interface and complete robustness review

**Goal:** integrate with the optimization ecosystem without duplicating protocols or compromising the standalone CLI.

**Implementation:** map the solver into the current optimization-jobs interface if required for the intended workflow. Read the actual pinned job/result schemas and reuse their status, artifact, cancellation and requested/actual work semantics. Keep packing-specific data in typed result artifacts. Add resumable checkpoints only with explicit case/configuration/version compatibility. A job worker launches or embeds a complete solve; it does not call a remote service for every geometric evaluation.

**Verification:** cancellation and resume, a changed input hash, a changed worker count, unexpected child exit, truncated logs, malformed answers and artifact write failures. Run the full focused geometry/state regression suite. Exercise actual resource limits on safe synthetic workloads.

**Acceptance:** standalone operation still works; integrations preserve accounting and answer validation. Any generic job status names used in source match the inspected shared schema rather than a guessed enumeration.

**Budget and checkpoint:** bounded local jobs only. If integration is unnecessary, complete the review and record the adapter as deferred with a stable CLI boundary; do not create an unnecessary MCP layer.

### I23 — Package the implementation and final evidence

**Goal:** deliver source, documentation and results that can be applied and checked later.

**Implementation:** run appropriate formatting, lint, build, unit/property/integration and bounded end-to-end checks. Record the final commit, lockfile, toolchain, case/judge hashes and enabled features. Produce a combined patch against the actual repository base when applicable; for a new project also produce a complete source archive. Include the implementation log, requirements/architecture/tutorial documents, representative answers, machine-readable benchmark records and reproduction commands. Exclude build products and bulky regenerable scratch data.

**Verification:** unpack into a clean directory, apply the patch where relevant, build with the lockfile, run the smoke suite, and independently validate the packaged answers. Verify links and that every performance number is generated from included evidence. Check that no failed or unrun benchmark is summarized as complete.

**Acceptance:** the deliverable is usable without hidden workspace state. Report completed iterations, optional experiments left disabled, remaining quality gaps, and the comparison's actual scope.

**Budget and checkpoint:** targeted final checks; rerun longer benchmarks only after substantive changes or unresolved failures. Publishing to GitHub or package registries is a separate deployment step, not implied by preparing an archive.

## 7. Resource limits and benchmark protocol

### 7.1 Default limits

The following are proposed development defaults, to be calibrated once I05–I08 produce measurements. They are not claims about required resources or achieved speed.

| Resource | Initial policy |
| --- | --- |
| Concurrent benchmark processes | 1; increase only after memory profiling |
| Solver workers | 1 initially; explicit bounded portfolio later |
| Solver process-tree memory cap | 1 GiB where enforceable; lower portable settings may be tested |
| Pose/pattern cache | Explicit byte cap, initially 64 MiB per process, shared across workers where safe |
| Captured child output | Bounded log size; retain termination reason and tail on overflow |
| Supervisor poll interval | Approximately 0.25–1 second |
| Progress record interval | About 5 seconds, plus improvements and phase changes |
| External judge timeout | Start at 120 seconds per answer; calibrate separately from solver budgets |
| Smoke campaign aggregate cap | 60 seconds of solver work plus bounded validation |
| Development campaign cap | 15 minutes per invocation by default; resumable chunks |

Prefer a process-tree/cgroup memory limit where available. A resident-memory watchdog is a fallback; an address-space limit is not the same measurement and may interact badly with thread stacks. Record which limit mechanism was used. Memory exhaustion must terminate the supervised run, not destabilize the host.

Avoid O(N²) persistent pair tables and O(Va×Vb) temporary distance matrices. Handle giant input dimensions, huge grid ranges and unlimited caches as explicit validation/resource cases. The supplied maximum ring size is modest enough for a bounded scalar fallback; thousands of simultaneous layouts are not needed.

### 7.2 Timing contract

Start the solver clock before case parsing. Include parsing, geometry preparation, search, strict final validation and answer preparation in the planned budget. Reserve finalization time based on measurements rather than using the entire deadline for search. The supervising runner records actual end-to-end process duration, exit reason, CPU time and memory.

The answer's `runtime_s` is sampled at final serialization; the runner's end-to-end duration also includes the final write and process exit and is authoritative for comparisons. Report the difference rather than inventing a runtime from the allotted budget. The original Python judge runs afterward with a separate bounded validation allowance; its time is recorded and not mislabeled as solver time.

Build time is excluded from solve timing and reported separately where relevant. Cold and warm-start experiments are different protocols. Any reusable preprocessing must be charged to a cold run or explicitly declared and measured as amortized work.

For deadlines, record requested time, observed time, overshoot and termination status. Cooperative checks should occur inside long candidate and geometry loops. A hard watchdog protects against an operator failing to cooperate.

### 7.3 Escalation order

1. Analytic geometry fixtures and fixed query corpora.
2. `tiny`, then small extracted neighborhoods with fixed evaluation limits.
3. One interactive real-case run, followed by three seeds if correctness passes.
4. All relevant interactive cases with one worker.
5. Selected patient-budget comparisons and ablations.
6. Selected background runs, only after evidence suggests the additional time is useful.
7. Controlled worker scaling with both wall-time and aggregate CPU accounting.

One configuration over the five nontrivial cases costs up to 233 seconds for interactive budgets, 829 seconds for patient budgets, or 5,454 seconds (90.9 minutes) for background budgets, before validation. Running all three budgets independently costs up to 108.6 minutes per configuration and seed. Do not silently launch that matrix for every experiment.

A long anytime run can emit checkpoints at several times, but those checkpoints are not automatically equivalent to independent budget-aware solves. A solver told it has 1,848 seconds may allocate exploration differently from one told it has 92 seconds. Label checkpoint curves and independently budgeted runs separately.

### 7.4 Required run records

Store the following as appropriate fields/artifacts within the reused result contract:

- Source revision, dirty-state indicator, compiler/toolchain, build flags and dependency lock hash.
- Case and original judge hashes, solver configuration hash, seed and worker count.
- Requested and actual evaluations/proposals, kernel queries and completed restarts.
- Requested and measured wall time, CPU seconds, peak process-tree RSS and termination reason.
- Seated count, unrounded area, coverage, fraction of available area placed and per-class counts.
- Mandatory completeness, strict validity, original-judge validity and judge execution status.
- Time to first complete valid mandatory answer and time to target placed-area fractions.
- Available-area upper-bound attainment, where all stock is actually placed and validated.
- Minimum-clearance diagnostic with its exact scope: global, threshold-certified, or radius-limited.
- Operator attempts/successes, candidate-neighbor counts, exact edge tests, cache size/hits and allocation counters.
- Paths and checksums for answer, validation report, checkpoint, progress log and rendering.

Report best and median quality across seeds, spread, success rate and failures. Do not publish a median over only successful runs without also showing failures. Invalid answers cannot win coverage comparisons; for expected-quality summaries, define their contribution explicitly, such as zero usable coverage, and retain their failure category. For mandatory cases, prioritize success probability and time to feasibility; coverage differences between complete valid answers are numerical noise or an accounting error.

### 7.5 Promotion and ablation rules

Correctness has an absolute gate: no accepted false-negative collision, malformed answer or lost identity in the regression/differential corpus. Performance changes must preserve that gate.

For search changes, use paired seeds and identical budgets/hardware. A three-seed pilot selects promising directions; it does not establish a universal improvement. Use more seeds for the final promoted comparison when run cost permits. Include representative data outside the tuning subset before making a generalization claim.

Always retain these comparison arms where relevant: deterministic construction, randomized construction, local search without LNS, best custom search without fcmaes, fcmaes policy/block variants, and serial versus parallel execution. Report quality at equal wall time and work at equal CPU resources as distinct questions.

If all stock is placed, coverage is already at its available-stock upper bound. Compare completion time, robustness, resource use or a separately declared secondary objective rather than claiming further coverage gains.

## 8. Planned CLI and artifacts

The commands below describe the target interface to implement; they are not commands available in the current workspace.

```bash
cargo build --release --locked

budel inspect cases/small.json --json out/small-inspect.json

budel solve cases/small.json \
  --budget interactive --seed 1 --workers 1 \
  --output out/small.answer.json --run-dir out/small-run

budel solve cases/large.json \
  --time-limit 92 --seed 1 --workers 4 \
  --memory-limit-mib 1024 \
  --output out/large.answer.json --run-dir out/large-run

budel validate cases/large.json out/large.answer.json --strict
budel render cases/large.json out/large.answer.json --output out/large.svg

python tools/reference_score.py cases/large.json out/large.answer.json \
  --json out/large-judge.json --validate-only

budel resume out/large-run/checkpoint.json \
  --additional-time 60 --output out/large-resumed.answer.json
```

`--time-limit` and named budgets need a documented precedence or mutual exclusion rule. An explicit time limit is mandatory for `tiny` unless a small CLI default is documented. Resume checks input and configuration compatibility and records both additional and cumulative cost. Invalid/partial mandatory artifacts live at a separate diagnostic path; they do not masquerade as certified `answer.json` files.

A complete run directory contains the shared-contract run record, packing-specific metrics, the best answer if one exists, strict and reference-judge reports, progress records, and a compatible checkpoint. Renderings are optional and generated outside the hot search loop. Keep full geometry in the original case file and refer to item IDs in snapshots to avoid repeatedly duplicating the case into every log entry.

## 9. Risks and explicit decision points

| Risk | Observable signal | Response |
| --- | --- | --- |
| Numeric predicates are unreliable | Disagreement with scalar/oracle tests; failures after JSON round trips | Stop promotion, minimize the case, improve predicates or use a narrowly scoped robust fallback |
| Collision checks dominate | Profiling shows most time in exact edge work | I07/I17 acceleration; optionally compare jagua-rs behind the same geometry boundary |
| Search quality dominates | Geometry is fast but area stagnates | Improve proposals, selection and neighborhood structure rather than micro-optimizing arithmetic |
| fcmaes adds overhead without quality | Equal-budget ablation loses or ties consistently | Keep the adapter experimental or disabled; retain the specialized packing loop |
| Mixed sizes defeat a single grid scale | Excessive candidate lists or cell enumeration | Hierarchical/size-segregated broad phase with the same conservative differential tests |
| All stock cannot be fitted within budget | Persistent missing IDs and no complete mandatory answer | Report the observed failure; try bounded structural changes; do not claim infeasibility |
| Baseline features differ | Missing reflection search, different sheet/selection objective or spacing semantics | Label restricted results or implement a measured adapter before comparing |
| Long campaigns consume the session | Matrix cost exceeds aggregate cap | Checkpoint completed runs and resume in bounded chunks; never repeat finished runs without reason |
| Reference answers remain unavailable | No actual placement/time-quality baseline | Compare available solvers and internal baselines; keep supplied lab times as metadata |

The core implementation is complete only when input/export semantics, geometry correctness, state rollback, resource limits and real-case execution are in place. A quality goal can remain unmet without pretending the implementation passed it. Optional experiments are complete when their bounded evaluation and decision are documented; adoption is not mandatory.

## 10. Final deliverable checklist

- [ ] Source builds from a clean checkout with the recorded lockfile and feature set.
- [ ] All six cases are identified by manifest hashes and inspected by the native adapter.
- [ ] Original rings are retained; exported transforms preserve scale and shape.
- [ ] Mirror search is exercised, not merely accepted by the file format.
- [ ] Strict original-geometry validation and unchanged external judging are both available.
- [ ] Mandatory completeness and optional-area objectives are reported correctly.
- [ ] Best valid answers survive cancellation and resume has cumulative accounting.
- [ ] Benchmarks have fixed seeds, bounded resources, raw records and honest failure reporting.
- [ ] Any fcmaes/performance/parallelism claims have appropriate ablations.
- [ ] README, architecture, requirements, tutorial and performance documents agree with final behavior.
- [ ] Final source/patch archive and implementation log reproduce the smoke checks and representative answers.
- [ ] Remaining gaps and unavailable external comparisons are explicit.

## 11. References and evidence boundaries

The governing inputs are the six user-supplied JSON files and the pasted `score.py`. Case counts, hashes, areas, geometry complexity and judge defects in this plan come from those inputs and the preparatory audit. No packing quality or speed result has been obtained for the proposed solver.

Relevant implementation and comparison sources:

- [fcmaes-rust](https://github.com/dietmarwo/fcmaes-rust): reusable native optimizer core; inspect pinned APIs before integrating.
- [optimization-job-api](https://docs.rs/optimization-job-api/latest/optimization_job_api/): existing job, process supervision and artifact interfaces; reuse appropriate contracts.
- [PackingSolver](https://github.com/fontanf/packingsolver) and its [irregular solver documentation](https://fontanf.github.io/packingsolver/irregular.html): external comparison candidate.
- [Sparrow](https://github.com/JeroenGar/sparrow): external strip-packing comparison candidate.
- [jagua-rs](https://github.com/JeroenGar/jagua-rs): optional geometry-backend comparison if profiling justifies it.
- [Spyrrow API documentation](https://spyrrow.readthedocs.io/en/latest/api.html): an example of why reflection permissions in an API must be distinguished from actual reflection exploration in a bundled solver.
- [An open-source heuristic to reboot 2D nesting research](https://arxiv.org/abs/2509.13329): reference for iterative feasibility and compression ideas.
- [Decoupling Geometry from Optimization in 2D Irregular Cutting and Packing Problems](https://arxiv.org/abs/2508.08341): reference for conservative geometry acceleration and simplification.

Versions and APIs can change. The implementation log must record what was actually checked out, installed and measured rather than treating these live links as version pins.


