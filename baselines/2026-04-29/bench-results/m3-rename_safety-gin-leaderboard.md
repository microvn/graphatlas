# M3 Gate — `rename_safety` on `gin`

**Rule:** Hrn-static

**Policy bias:** Hrn-static cycle B — sites from raw AST (extract_calls + extract_references); per-def filtering: a call site `foo()` in file F resolves to def (foo, def_file) iff (a) F == def_file (intra-file), or (b) F has an import linking back to def_file's package (`from <pkg>.<def_module> import foo` or aliased equivalents). Cross-file calls without resolvable imports are dropped from `expected_sites` for ALL homonyms — under-counts expected sites rather than over-attributing to wrong def. Blockers from line-local string-literal scan: multi-line string blockers (triple-quoted Python, backtick-template TS) are NOT detected — false-negatives in GT documented honestly per C4. Polymorphic tier (≥2 def files for same name) scored separately.

| Status | Retriever | Score | Spec target | p95 latency (ms) |
|---|---|---|---|---|
| **FAIL** | ga | 0.750 | 0.900 | 68 |

### Secondary metrics

**ga**:
- `poly_target_count` = 1.000
- `recall_polymorphic` = 1.000
- `recall_unique` = 0.750
- `spec_target_polymorphic` = 0.700
- `unique_target_count` = 4.000

**SPEC GATE: 0 pass, 1 fail (target: all pass)**
