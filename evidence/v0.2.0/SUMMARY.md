# Budel campaign summary

| Case | Algorithm | Seconds | Workers | Work cap | Check | Eligible / runs | Median coverage | Median wall s |
| --- | --- | ---: | ---: | ---: | --- | ---: | ---: | ---: |
| extreme | lns | 184 | 1 | — | both | 3 / 3 | 21.8973% | 1.059 |
| large | lns | 92 | 1 | — | both | 3 / 3 | 47.8051% | 0.282 |
| layered-hard | lns | 65 | 1 | — | both | 3 / 3 | 50.5413% | 64.269 |
| layered-hard | multistart | 65 | 1 | — | strict | 3 / 3 | 46.2614% | 64.270 |
| medium | lns | 14 | 1 | — | both | 3 / 3 | 51.5850% | 9.279 |
| medium | multistart | 14 | 1 | — | strict | 3 / 3 | 47.3164% | 13.642 |
| small | lns | 6 | 1 | — | both | 3 / 3 | 23.2899% | 0.023 |
| tiny | lns | 1 | 1 | — | both | 3 / 3 | 5.0000% | 0.009 |
| uniform-hard | lns | 56 | 1 | — | both | 3 / 3 | 26.9808% | 0.127 |

Coverage medians use eligible layouts. The counts include failed runs. A watchdog can preserve an eligible checkpoint even when the process exits after its budget. See the raw execution and answer timing fields for those cases.
