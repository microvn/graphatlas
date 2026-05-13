# M3 Gate — `risk` on `axum`

**Rule:** Hr-text

**Policy bias:** Hr-text cycle B — risk labels mined from a SINGLE repo-wide `git log --name-only` pass anchored to fixture HEAD's committer date (not wall-clock). Drops the cycle-A per-file git_log loop that took 13+ min on django (50k files × 2 subprocesses). Cost now scales with commits-in-window, not files. Files with zero commits in window are NOT emitted as GT entries (avoids a flood of vacuous `expected_risky=false` rows inflating denominator). Bug-keyword set: fix/bug/error/crash/regression (whole-word). Shallow fixtures (httpx, radash — 1-commit) yield ≤1 commit total → empty GT; `git fetch --unshallow` recommended before bench.

| Status | Retriever | Score | Spec target | p95 latency (ms) |
|---|---|---|---|---|
| PASS | ga | 0.800 | 0.800 | 79272 |

### Secondary metrics

**ga**:
- `expected_risky_count` = 10.000
- `false_negatives` = 2.000
- `false_positives` = 2.000
- `pr_at_0.20_precision` = 0.474
- `pr_at_0.20_recall` = 0.900
- `pr_at_0.30_precision` = 0.800
- `pr_at_0.30_recall` = 0.800
- `pr_at_0.40_precision` = 1.000
- `pr_at_0.40_recall` = 0.100
- `pr_at_0.50_precision` = 1.000
- `pr_at_0.50_recall` = 0.000
- `precision` = 0.800
- `predicted_risky_count` = 10.000
- `recall` = 0.800
- `scored_files` = 20.000
- `true_positives` = 8.000

**SPEC GATE: 1 pass, 0 fail (target: all pass)**
