# Impact Benchmark — Cross-Fixture Aggregate

**Dataset:** 170 tasks across 14 repos (MQTTnet, axum, django, faraday, gin, kotlinx-coroutines, kotlinx-serialization, mockito, nest, php-monolog, php-symfony-console, preact, regex, tokio)

**Split:** test

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-05-13

**Gate (AS-009):** composite ≥ 0.80 | test_recall ≥ 0.85 | completeness ≥ 0.80 | depth_F1 ≥ 0.80 | precision ≥ 0.70 | p95 ≤ 500ms

## Overall

| Retriever | Composite | Test Recall | Completeness | Precision | p95 ms | Pass Rate | Gate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|-----------|--------|-----------|------|-------------|----------|
| ga        | 0.589 ⭐  | 0.554 ⭐    | 0.773       | 0.137 ⭐    | 3749   |   44.7%  | — | 0.568 ⭐      | 0.178 ⭐  |
| code-review-graph | 0.375    | 0.265      | 0.837 ⭐     | 0.121      | 1095   |    0.0%  | — | 0.563        | 0.167    |
| bm25      | 0.242    | 0.243      | 0.432       | 0.103      | 0      |    0.0%  | — | 0.433        | 0.166    |
| random    | 0.026    | 0.039      | 0.033       | 0.005      | 0      |    0.0%  | — | 0.287        | 0.016    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000      | 0      |    0.0%  | — | 0.265        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000      | 0      |    0.0%  | — | 0.265        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000      | 0      |    0.0%  | — | 0.265        | 0.000    |

## Token cost vs lexical IR baseline

**GA vs BM25:** resolves 1.71× more regression-causing changes (68.2% vs 40.0% reach 100% recall) using 29% fewer tokens per successful retrieval (12571 vs 17715).

Token cost = bytes/4 of files an agent reads, walking the retriever's ranked list, to reach the recall threshold. Means are **conditional on success** — failures aren't folded in, since a retriever that returns fewer files would otherwise look cheaper just for missing more.

| Retriever | reached 50% | tokens→50% (when reached) | reached 100% | tokens→100% (when reached) | files returned |
|-----------|------------:|--------------------------:|-------------:|---------------------------:|---------------:|
| ga        |       81.2% |                     13055 |        68.2% |                      12571 |           36.8 |
| bm25      |       44.7% |                     17174 |        40.0% |                      17715 |            8.3 |
| random    |        3.5% |                     14478 |         2.4% |                      13460 |            6.1 |
| ripgrep   |        0.0% |                         0 |         0.0% |                          0 |            0.0 |

## Summary

**Top composite:** `ga` (0.589)

**Top precision:** `ga` (0.137)

**Top test recall:** `ga` (0.554)


## Per-repo composite

| Repo | Lang | Tasks | bm25 | code-review-graph | codebase-memory | codegraphcontext | ga | random | ripgrep |
|------|------|-------|-------|-------|-------|-------|-------|-------|-------|
| MQTTnet | csharp | 14 | 0.000 | 0.524 | 0.000 | 0.000 | 0.290 | 0.000 | 0.000 |
| axum | rust | 4 | 0.479 | 0.178 | 0.000 | 0.000 | 0.692 | 0.078 | 0.000 |
| django | python | 10 | 0.549 | 0.263 | 0.000 | 0.000 | 0.604 | 0.000 | 0.000 |
| faraday | ruby | 13 | 0.000 | 0.361 | 0.000 | 0.000 | 0.646 | 0.000 | 0.000 |
| gin | go | 14 | 0.621 | 0.524 | 0.000 | 0.000 | 0.721 | 0.118 | 0.000 |
| kotlinx-coroutines | kotlin | 9 | 0.000 | 0.214 | 0.000 | 0.000 | 0.432 | 0.000 | 0.000 |
| kotlinx-serialization | kotlin | 14 | 0.000 | 0.300 | 0.000 | 0.000 | 0.576 | 0.000 | 0.000 |
| mockito | java | 13 | 0.000 | 0.287 | 0.000 | 0.000 | 0.463 | 0.000 | 0.000 |
| nest | typescript | 14 | 0.486 | 0.309 | 0.000 | 0.000 | 0.646 | 0.000 | 0.000 |
| php-monolog | php | 12 | 0.000 | 0.525 | 0.000 | 0.000 | 0.719 | 0.000 | 0.000 |
| php-symfony-console | php | 14 | 0.000 | 0.592 | 0.000 | 0.000 | 0.681 | 0.000 | 0.000 |
| preact | javascript | 12 | 0.524 | 0.378 | 0.000 | 0.000 | 0.622 | 0.044 | 0.000 |
| regex | rust | 14 | 0.357 | 0.259 | 0.000 | 0.000 | 0.651 | 0.089 | 0.000 |
| tokio | rust | 13 | 0.540 | 0.320 | 0.000 | 0.000 | 0.533 | 0.055 | 0.000 |

**Reproduce:** `graphatlas bench --uc impact`

**Methodology:** see [docs/guide/uc-impact-dataset-methodology.md](../docs/guide/uc-impact-dataset-methodology.md)
