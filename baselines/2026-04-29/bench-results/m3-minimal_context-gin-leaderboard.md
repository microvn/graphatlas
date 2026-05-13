# M3 Gate — `minimal_context` on `gin`

**Rule:** Hmc-gitmine

**Policy bias:** GT from benches/uc-impact/ground-truth.json (git-mining of real upstream fix commits — no LLM, no runtime trace). File-level recall measured against expected_files; minimal_context is fundamentally narrower than impact, so absolute scores are lower than M2 impact numbers on the same dataset. Default scoring split = `test`. Pin per-task base_commit via M2's existing pin_commits infrastructure (allowed shared primitive — infra coupling, not GT semantics).

| Status | Retriever | Score | Spec target | p95 latency (ms) |
|---|---|---|---|---|
| PASS | ga | 0.782 | 0.700 | 11 |

### Secondary metrics

**ga**:
- `file_precision` = 0.302
- `pin_enabled` = 1.000
- `pin_failed_count` = 0.000
- `recall_per_1k_tokens` = 6.590
- `seed_symbol_not_found_at_hinted_file_count` = 1.000
- `seed_symbol_not_found_count` = 0.000
- `task_count` = 14.000
- `test_recall` = 0.219
- `truncation_correctness_rate` = 1.000

**SPEC GATE: 1 pass, 0 fail (target: all pass)**
