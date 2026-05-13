# M3 Gate — `dead_code` on `tokio`

**Rule:** Hd-ast

**Policy bias:** Hd-ast cycle B' — entry-point detection sourced from `ga_query::entry_points` (shared with the production tool, definitionally aligned per spec C2): main/__main__, Python __all__ exports, pyproject [project.scripts] / [tool.poetry.scripts], Cargo [[bin]], framework route handlers (gin/django/rails/axum/nest line-pattern scan). Targeted side now per-file via import resolution: a call site `foo()` in F.py resolves to def (foo, F) intra-file, or to def (foo, G) iff F has `from <G's module> import foo` (matches production S-003 (name, file) identity). Remaining honest gaps: clap derive `#[command]` / Cobra command structs / Rust `pub use` re-exports / TS `export` re-exports / dynamic getattr / metaclass tricks — kept in GT, tools that know better under-score on those fixture cases. Candidate-pool note: parse_source emits every function/method/class incl. nested closures; ga's indexer stores fewer (top-level + methods only), so Hd-ast's expected_dead pool is systematically larger than ga's universe — drives FN up but doesn't affect precision.

| Status | Retriever | Score | Spec target | p95 latency (ms) |
|---|---|---|---|---|
| PASS | ga | 0.992 | 0.850 | 74 |

### Secondary metrics

**ga**:
- `actual_dead_count` = 797.000
- `expected_dead_aligned` = 3185.000
- `expected_dead_raw` = 3185.000
- `f1` = 0.397
- `false_negatives` = 2394.000
- `false_positives` = 6.000
- `ga_universe_size` = 6299.000
- `recall` = 0.248
- `true_positives` = 791.000

**SPEC GATE: 1 pass, 0 fail (target: all pass)**
