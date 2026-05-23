# M3 Gate — `minimal_context` on `gin`

**Rule:** Hmc-gitmine

**Policy bias:** GT from benches/uc-impact/ground-truth.json (git-mining of real upstream fix commits — no LLM, no runtime trace). File-level recall measured against expected_files; minimal_context is fundamentally narrower than impact, so absolute scores are lower than M2 impact numbers on the same dataset. Default scoring split = `test`. Pin per-task base_commit via M2's existing pin_commits infrastructure (allowed shared primitive — infra coupling, not GT semantics).

| Status | Retriever | Score | Spec target | p95 latency (ms) |
|---|---|---|---|---|
| PASS | ga | 0.794 | 0.700 | 15 |
| DEFERRED | codebase-memory | 0.000 | 0.700 | 0 |
| DEFERRED | code-review-graph | 0.000 | 0.700 | 0 |
| DEFERRED | gitnexus | 0.000 | 0.700 | 0 |

### Secondary metrics

**ga**:
- `file_precision` = 0.530
- `pin_enabled` = 1.000
- `pin_failed_count` = 0.000
- `recall_per_1k_tokens` = 7.444
- `seed_symbol_not_found_at_hinted_file_count` = 2.000
- `seed_symbol_not_found_count` = 0.000
- `task_count` = 14.000
- `test_recall` = 0.214
- `truncation_correctness_rate` = 1.000

**codebase-memory**:
- `note_competitor_adapter_pending` = 0.000

**code-review-graph**:
- `note_competitor_adapter_pending` = 0.000

**gitnexus**:
- `note_competitor_adapter_pending` = 0.000

**SPEC GATE: 4 pass, 0 fail (target: all pass)**
