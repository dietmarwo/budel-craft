# Budel case contract

The supplied JSON files (the six original cases and the later `extreme` case) define a fixed stock of polygons and one rectangular sheet per case. Each ID can appear once. The selected collision body is the rest ring or the supplied envelope if requested. The solver treats that body as solid. Paper holes subtract from scoring area and appear in the drawing. Ink holes stay part of the scored area. Open curls and recesses in the outer ring can contain parts of another figure when the clearance check passes.

Coordinates use the case's own units. A pose reflects local x when `mirror` is true, rotates around the original local origin in degrees, then translates by `(x,y)`. Every output uses the original case geometry. The `eight` mode permits exact multiples of 45 degrees. The `free` mode allows arbitrary angles.

Use `rules.margin` when present and otherwise `plane.margin`. The usable rectangle is inset by that amount on each side. The nominal spacing applies to figure pairs. The strict validator enforces the nominal spacing with a scale-aware guard. Boundary contact between figures also fails at zero nominal spacing.

| Case | Items | Required | Nominal gap | Available sheet coverage |
| --- | ---: | --- | ---: | ---: |
| tiny | 2 | all | 2.0 | 5.0000% |
| small | 40 | all | 3.0 | 23.2899% |
| medium | 312 | optional | 3.0 | 51.5850% |
| large | 2,127 | optional | 2.8 | 47.8051% |
| uniform-hard | 1,008 | optional | 3.0 | 26.9808% |
| layered-hard | 1,184 | optional | 3.0 | 51.9847% |

Coverage is unrounded selected area divided by the full sheet area. The sum of all stock areas is an upper bound. Every complete valid mandatory layout has the same coverage. For that reason the solver stops when it obtains a complete certified layout. Size classes are descriptive. The score uses area computed from polygon coordinates.

## Reference judge provenance

`tools/reference_score.py` transcribes the user-pasted judge, including its load, geometry, score, render, comparison, and reproduction paths. Formatting and comments in the transcription differ in places. The six source files and the transcription have recorded SHA-256 hashes. The published reference-answer directory needed by its `--reproduce` command was absent from the supplied material.

`python tools/judge_audit.py` reproduces the legacy corner-touch and coincident-polygon cases, the relaxed gap threshold, the margin fallback discrepancy, and the eight-pose wrap discrepancy. The Rust validator checks original polygons independently of this script's AABB grid and vectorized distance routine. The regression suite includes the corresponding strict checks.

The legacy judge accepts a gap as low as `min_channel - tolerance`. Its `tightest` statistic covers its examined neighbor pairs. Runtime values in an answer are declarations for its report. The native process and benchmark supervisor separately enforce their requested time and work limits.
