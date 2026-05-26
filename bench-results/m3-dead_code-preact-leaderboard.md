# M3 Gate — `dead_code` on `preact`

**Rule:** Hd-ast

**Policy bias:** Hd-ast cycle B' — entry-point detection sourced from `ga_query::entry_points` (shared with the production tool, definitionally aligned per spec C2): main/__main__, Python __all__ exports, pyproject [project.scripts] / [tool.poetry.scripts], Cargo [[bin]], framework route handlers (gin/django/rails/axum/nest line-pattern scan). Targeted side now per-file via import resolution: a call site `foo()` in F.py resolves to def (foo, F) intra-file, or to def (foo, G) iff F has `from <G's module> import foo` (matches production S-003 (name, file) identity). Remaining honest gaps: clap derive `#[command]` / Cobra command structs / Rust `pub use` re-exports / TS `export` re-exports / dynamic getattr / metaclass tricks — kept in GT, tools that know better under-score on those fixture cases. Candidate-pool note: parse_source emits every function/method/class incl. nested closures; ga's indexer stores fewer (top-level + methods only), so Hd-ast's expected_dead pool is systematically larger than ga's universe — drives FN up but doesn't affect precision.

| Status | Retriever | Score | Spec target | p95 latency (ms) |
|---|---|---|---|---|
| **FAIL** | ga | 0.721 | 0.850 | 33 |
| DEFERRED | codebase-memory | 0.000 | 0.850 | 0 |
| DEFERRED | code-review-graph | 0.000 | 0.850 | 0 |
| DEFERRED | gitnexus | 0.000 | 0.850 | 0 |

### Secondary metrics

**ga**:
- `actual_dead_count` = 233.000
- `expected_dead_aligned` = 557.000
- `expected_dead_raw` = 557.000
- `f1` = 0.425
- `f2` = 0.341
- `false_negatives` = 389.000
- `false_positives` = 65.000
- `ga_universe_size` = 1701.000
- `recall` = 0.302
- `true_positives` = 168.000

**codebase-memory**:
- `note_competitor_adapter_pending` = 0.000

**code-review-graph**:
- `note_competitor_adapter_pending` = 0.000

**gitnexus**:
- `note_competitor_adapter_pending` = 0.000

**SPEC GATE: 3 pass, 1 fail (target: all pass)**
