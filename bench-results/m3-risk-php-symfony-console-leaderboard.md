# M3 Gate — `risk` on `php-symfony-console`

**Rule:** Hr-text

**Policy bias:** Hr-text cycle B — risk labels mined from a SINGLE repo-wide `git log --name-only` pass anchored to fixture HEAD's committer date (not wall-clock). Drops the cycle-A per-file git_log loop that took 13+ min on django (50k files × 2 subprocesses). Cost now scales with commits-in-window, not files. Files with zero commits in window are NOT emitted as GT entries (avoids a flood of vacuous `expected_risky=false` rows inflating denominator). Bug-keyword set: fix/bug/error/crash/regression (whole-word). Shallow fixtures (httpx, radash — 1-commit) yield ≤1 commit total → empty GT; `git fetch --unshallow` recommended before bench.

| Status | Retriever | Score | Spec target | p95 latency (ms) |
|---|---|---|---|---|
| **FAIL** | ga | 0.700 | 0.800 | 119825 |
| DEFERRED | codebase-memory | 0.000 | 0.800 | 0 |
| DEFERRED | code-review-graph | 0.000 | 0.800 | 0 |
| DEFERRED | gitnexus | 0.000 | 0.800 | 0 |

### Secondary metrics

**ga**:
- `expected_risky_count` = 10.000
- `f1_at_0.30_cutoff` = 0.690
- `f2_at_0.30_cutoff` = 0.847
- `false_negatives_at_cutoff` = 0.000
- `false_positives_at_cutoff` = 9.000
- `max_f1_threshold` = 0.400
- `max_f2` = 0.847
- `max_f2_threshold` = 0.200
- `pr_at_0.20_f1` = 0.690
- `pr_at_0.20_f2` = 0.847
- `pr_at_0.20_precision` = 0.526
- `pr_at_0.20_recall` = 1.000
- `pr_at_0.30_f1` = 0.690
- `pr_at_0.30_f2` = 0.847
- `pr_at_0.30_precision` = 0.526
- `pr_at_0.30_recall` = 1.000
- `pr_at_0.40_f1` = 0.700
- `pr_at_0.40_f2` = 0.700
- `pr_at_0.40_precision` = 0.700
- `pr_at_0.40_recall` = 0.700
- `pr_at_0.50_f1` = 0.000
- `pr_at_0.50_f2` = 0.000
- `pr_at_0.50_precision` = 0.000
- `pr_at_0.50_recall` = 0.000
- `precision_at_0.30_cutoff` = 0.526
- `predicted_risky_count` = 19.000
- `recall_at_0.30_cutoff` = 1.000
- `scored_files` = 19.000
- `true_positives_at_cutoff` = 10.000

**codebase-memory**:
- `note_competitor_adapter_pending` = 0.000

**code-review-graph**:
- `note_competitor_adapter_pending` = 0.000

**gitnexus**:
- `note_competitor_adapter_pending` = 0.000

**SPEC GATE: 3 pass, 1 fail (target: all pass)**
