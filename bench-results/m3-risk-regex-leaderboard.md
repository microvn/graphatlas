# M3 Gate — `risk` on `regex`

**Rule:** Hr-text

**Policy bias:** Hr-text cycle B — risk labels mined from a SINGLE repo-wide `git log --name-only` pass anchored to fixture HEAD's committer date (not wall-clock). Drops the cycle-A per-file git_log loop that took 13+ min on django (50k files × 2 subprocesses). Cost now scales with commits-in-window, not files. Files with zero commits in window are NOT emitted as GT entries (avoids a flood of vacuous `expected_risky=false` rows inflating denominator). Bug-keyword set: fix/bug/error/crash/regression (whole-word). Shallow fixtures (httpx, radash — 1-commit) yield ≤1 commit total → empty GT; `git fetch --unshallow` recommended before bench.

| Status | Retriever | Score | Spec target | p95 latency (ms) |
|---|---|---|---|---|
| **FAIL** | ga | 0.778 | 0.800 | 258380 |

### Secondary metrics

**ga**:
- `expected_risky_count` = 10.000
- `f1_at_0.30_cutoff` = 0.769
- `false_negatives_at_cutoff` = 0.000
- `false_positives_at_cutoff` = 6.000
- `max_f1_threshold` = 0.400
- `pr_at_0.20_f1` = 0.741
- `pr_at_0.20_precision` = 0.588
- `pr_at_0.20_recall` = 1.000
- `pr_at_0.30_f1` = 0.769
- `pr_at_0.30_precision` = 0.625
- `pr_at_0.30_recall` = 1.000
- `pr_at_0.40_f1` = 0.778
- `pr_at_0.40_precision` = 0.875
- `pr_at_0.40_recall` = 0.700
- `pr_at_0.50_f1` = 0.000
- `pr_at_0.50_precision` = 0.000
- `pr_at_0.50_recall` = 0.000
- `precision_at_0.30_cutoff` = 0.625
- `predicted_risky_count` = 16.000
- `recall_at_0.30_cutoff` = 1.000
- `scored_files` = 20.000
- `true_positives_at_cutoff` = 10.000

**SPEC GATE: 0 pass, 1 fail (target: all pass)**
