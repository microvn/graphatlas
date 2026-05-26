# M3 Gate — `rename_safety` on `kotlinx-coroutines`

**Rule:** Hrn-static

**Policy bias:** Hrn-static cycle B — sites from raw AST (extract_calls + extract_references); per-def filtering: a call site `foo()` in file F resolves to def (foo, def_file) iff (a) F == def_file (intra-file), or (b) F has an import linking back to def_file's package (`from <pkg>.<def_module> import foo` or aliased equivalents). Cross-file calls without resolvable imports are dropped from `expected_sites` for ALL homonyms — under-counts expected sites rather than over-attributing to wrong def. Blockers from line-local string-literal scan: multi-line string blockers (triple-quoted Python, backtick-template TS) are NOT detected — false-negatives in GT documented honestly per C4. Polymorphic tier (≥2 def files for same name) scored separately.

| Status | Retriever | Score | Spec target | p95 latency (ms) |
|---|---|---|---|---|
| PASS | ga | 1.000 | 0.900 | 0 |
| DEFERRED | codebase-memory | 0.000 | 0.900 | 0 |
| DEFERRED | code-review-graph | 0.000 | 0.900 | 0 |
| DEFERRED | gitnexus | 0.000 | 0.900 | 0 |

### Secondary metrics

**ga**:
- `poly_target_count` = 0.000
- `recall_polymorphic` = 1.000
- `recall_unique` = 1.000
- `spec_target_polymorphic` = 0.700
- `unique_target_count` = 0.000

**codebase-memory**:
- `note_competitor_adapter_pending` = 0.000

**code-review-graph**:
- `note_competitor_adapter_pending` = 0.000

**gitnexus**:
- `note_competitor_adapter_pending` = 0.000

**SPEC GATE: 4 pass, 0 fail (target: all pass)**
