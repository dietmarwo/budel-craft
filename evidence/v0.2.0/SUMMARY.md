# Budel campaign summary

| Case | Algorithm | Seconds | Workers | Work cap | Check | Eligible / runs | Median coverage | Median wall s |
| --- | --- | ---: | ---: | ---: | --- | ---: | ---: | ---: |
| large | lns | 92 | 1 | — | both | 3 / 3 | 47.8051% | 0.437 |
| layered-hard | lns | 65 | 1 | — | both | 3 / 3 | 50.2949% | 64.351 |
| layered-hard | multistart | 65 | 1 | — | strict | 3 / 3 | 46.2614% | 64.270 |
| medium | lns | 14 | 1 | — | both | 3 / 3 | 51.5658% | 13.650 |
| medium | multistart | 14 | 1 | — | strict | 3 / 3 | 47.3164% | 13.642 |
| small | lns | 6 | 1 | — | both | 3 / 3 | 23.2899% | 0.040 |
| tiny | lns | 1 | 1 | — | both | 3 / 3 | 5.0000% | 0.033 |
| uniform-hard | lns | 56 | 1 | — | both | 3 / 3 | 26.9808% | 0.216 |

Coverage medians use eligible layouts. The counts include failed runs. A watchdog can preserve an eligible checkpoint even when the process exits after its budget. See the raw execution and answer timing fields for those cases.
