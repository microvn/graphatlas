# Impact Benchmark — Cross-Fixture Aggregate

**Dataset:** 167 tasks across 14 repos (MQTTnet, Polly, axum, django, faraday, gin, jekyll, kotlinx-coroutines, kotlinx-serialization, mockito, nest, preact, regex, tokio)

**Split:** test

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-04-28

**Gate (AS-009):** composite ≥ 0.80 | test_recall ≥ 0.85 | completeness ≥ 0.80 | depth_F1 ≥ 0.80 | precision ≥ 0.70 | p95 ≤ 500ms

## Overall

| Retriever | Composite | Test Recall | Completeness | Precision | p95 ms | Pass Rate | Gate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|-----------|--------|-----------|------|-------------|----------|
| ga        | 0.345 ⭐  | 0.339 ⭐    | 0.438 ⭐     | 0.075      | 7005   |   24.6%  | — | 0.376        | 0.096    |
| bm25      | 0.208    | 0.232      | 0.334       | 0.101 ⭐    | 0      |    0.0%  | — | 0.386 ⭐      | 0.159 ⭐  |
| random    | 0.025    | 0.041      | 0.027       | 0.004      | 0      |    0.0%  | — | 0.256        | 0.011    |
| code-review-graph | 0.000    | 0.000      | 0.000       | 0.000      | 0      |    0.0%  | — | 0.228        | 0.000    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000      | 0      |    0.0%  | — | 0.210        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000      | 0      |    0.0%  | — | 0.210        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000      | 0      |    0.0%  | — | 0.240        | 0.000    |

## Summary

**Top composite:** `ga` (0.345)

**Top precision:** `bm25` (0.101)

**Top test recall:** `ga` (0.339)


## Per-repo composite

| Repo | Lang | Tasks | bm25 | code-review-graph | codebase-memory | codegraphcontext | ga | random | ripgrep |
|------|------|-------|-------|-------|-------|-------|-------|-------|-------|
| MQTTnet | csharp | 14 | 0.000 | 0.000 | 0.000 | 0.000 | 0.316 | 0.000 | 0.000 |
| Polly | csharp | 9 | 0.000 | 0.000 | 0.000 | 0.000 | 0.145 | 0.000 | 0.000 |
| axum | rust | 4 | 0.479 | 0.000 | 0.000 | 0.000 | 0.692 | 0.078 | 0.000 |
| django | python | 14 | 0.621 | 0.000 | 0.000 | 0.000 | 0.000 | 0.000 | 0.000 |
| faraday | ruby | 14 | 0.000 | 0.000 | 0.000 | 0.000 | 0.481 | 0.000 | 0.000 |
| gin | go | 14 | 0.596 | 0.000 | 0.000 | 0.000 | 0.693 | 0.088 | 0.000 |
| jekyll | ruby | 10 | 0.000 | 0.000 | 0.000 | 0.000 | 0.477 | 0.000 | 0.000 |
| kotlinx-coroutines | kotlin | 10 | 0.000 | 0.000 | 0.000 | 0.000 | 0.041 | 0.000 | 0.000 |
| kotlinx-serialization | kotlin | 11 | 0.000 | 0.000 | 0.000 | 0.000 | 0.484 | 0.000 | 0.000 |
| mockito | java | 12 | 0.000 | 0.000 | 0.000 | 0.000 | 0.417 | 0.000 | 0.000 |
| nest | typescript | 14 | 0.542 | 0.000 | 0.000 | 0.000 | 0.000 | 0.000 | 0.000 |
| preact | javascript | 14 | 0.300 | 0.000 | 0.000 | 0.000 | 0.428 | 0.089 | 0.000 |
| regex | rust | 14 | 0.039 | 0.000 | 0.000 | 0.000 | 0.280 | 0.081 | 0.000 |
| tokio | rust | 13 | 0.272 | 0.000 | 0.000 | 0.000 | 0.564 | 0.015 | 0.000 |

**Reproduce:** `graphatlas bench --uc impact`

**Methodology:** see [docs/guide/uc-impact-dataset-methodology.md](../docs/guide/uc-impact-dataset-methodology.md)
