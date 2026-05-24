# M3 Gate — `hubs` on `php-symfony-console`

**Rule:** Hh-gitmine

**Policy bias:** Hh-gitmine — file-level oracle. Counts non-binary file touches in `git log --name-only` over the last 12 months BEFORE HEAD's committer timestamp (NOT relative to wall-clock now — fixtures are pinned at base_commit per CLAUDE.md, so a wall-clock window would silently exclude older fixtures). Ranks files by touch frequency. Engine output (symbol-level hubs) is projected to its file set and scored by Spearman rank correlation. Bias 1: file-granularity — a file with one giant hub function ties with a file holding 20 small symbols (rank is per-file, not per-symbol). Bias 2: pre-merge churn (rebases, squashed PRs) doesn't always reflect long-term architectural pressure — fixtures with squashy histories under-represent hubs. Bias 3: HEAD-anchored window means very-young fixtures (HEAD < 12 months after first commit) have a smaller effective window.

| Status | Retriever | Score | Spec target | p95 latency (ms) |
|---|---|---|---|---|
| **FAIL** | ga | 0.450 | 0.700 | 45 |
| DEFERRED | codebase-memory | 0.000 | 0.700 | 0 |
| DEFERRED | code-review-graph | 0.000 | 0.700 | 0 |
| DEFERRED | gitnexus | 0.000 | 0.700 | 0 |

### Secondary metrics

**ga**:
- `common_files` = 16.000
- `engine_size` = 33.000
- `gt_size` = 50.000
- `total_hubs_with_edges` = 50.000

**codebase-memory**:
- `note_competitor_adapter_pending` = 0.000

**code-review-graph**:
- `note_competitor_adapter_pending` = 0.000

**gitnexus**:
- `note_competitor_adapter_pending` = 0.000

**SPEC GATE: 3 pass, 1 fail (target: all pass)**
