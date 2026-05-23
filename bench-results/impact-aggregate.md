# Impact Benchmark — Cross-Fixture Aggregate

**Dataset:** 170 tasks across 14 repos (MQTTnet, axum, django, faraday, gin, kotlinx-coroutines, kotlinx-serialization, mockito, nest, php-monolog, php-symfony-console, preact, regex, tokio)

**Split:** test

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-05-13

**Gate (AS-009):** composite ≥ 0.80 | test_recall ≥ 0.85 | completeness ≥ 0.80 | depth_F1 ≥ 0.80 | precision ≥ 0.70 | p95 ≤ 500ms

## Overall

| Retriever | Composite | Test Recall | Completeness | Precision | p95 ms | Pass Rate | Gate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|-----------|--------|-----------|------|-------------|----------|
| ga        | 0.511 ⭐  | 0.485 ⭐    | 0.665       | 0.116      | 1097   |   40.0%  | — | 0.531 ⭐      | 0.157    |
| code-review-graph | 0.345    | 0.221      | 0.795 ⭐     | 0.120      | 631    |    0.0%  | — | 0.528        | 0.163    |
| bm25      | 0.324    | 0.344      | 0.542       | 0.157 ⭐    | 0      |    1.8%  | — | 0.510        | 0.250 ⭐  |
| random    | 0.033    | 0.045      | 0.046       | 0.007      | 0      |    0.0%  | — | 0.272        | 0.021    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000      | 0      |    0.0%  | — | 0.247        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000      | 0      |    0.0%  | — | 0.247        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000      | 0      |    0.0%  | — | 0.259        | 0.000    |

## Token cost vs lexical IR baseline

**GA vs BM25:** resolves 1.19× more regression-causing changes (58.8% vs 49.4% reach 100% recall) using 18% fewer tokens per successful retrieval (12901 vs 15718).

Token cost = bytes/4 of files an agent reads, walking the retriever's ranked list, to reach the recall threshold. Means are **conditional on success** — failures aren't folded in, since a retriever that returns fewer files would otherwise look cheaper just for missing more.

| Retriever | reached 50% | tokens→50% (when reached) | reached 100% | tokens→100% (when reached) | files returned |
|-----------|------------:|--------------------------:|-------------:|---------------------------:|---------------:|
| ga        |       68.8% |                     13228 |        58.8% |                      12901 |           32.3 |
| bm25      |       55.9% |                     15203 |        49.4% |                      15718 |            9.4 |
| random    |        5.3% |                     11710 |         2.9% |                      13236 |            7.9 |
| ripgrep   |        0.0% |                         0 |         0.0% |                          0 |            0.0 |

## Summary

**Top composite:** `ga` (0.511)

**Top precision:** `bm25` (0.157)

**Top test recall:** `ga` (0.485)


## Per-repo composite

| Repo | Lang | Tasks | bm25 | code-review-graph | codebase-memory | codegraphcontext | ga | random | ripgrep |
|------|------|-------|-------|-------|-------|-------|-------|-------|-------|
| MQTTnet | csharp | 14 | 0.000 | 0.208 | 0.000 | 0.000 | 0.173 | 0.000 | 0.000 |
| axum | rust | 4 | 0.328 | 0.178 | 0.000 | 0.000 | 0.647 | 0.078 | 0.000 |
| django | python | 10 | 0.549 | 0.187 | 0.000 | 0.000 | 0.386 | 0.000 | 0.000 |
| faraday | ruby | 13 | 0.000 | 0.361 | 0.000 | 0.000 | 0.646 | 0.000 | 0.000 |
| gin | go | 14 | 0.621 | 0.524 | 0.000 | 0.000 | 0.721 | 0.118 | 0.000 |
| kotlinx-coroutines | kotlin | 9 | 0.000 | 0.214 | 0.000 | 0.000 | 0.270 | 0.000 | 0.000 |
| kotlinx-serialization | kotlin | 14 | 0.000 | 0.300 | 0.000 | 0.000 | 0.557 | 0.000 | 0.000 |
| mockito | java | 13 | 0.000 | 0.287 | 0.000 | 0.000 | 0.455 | 0.000 | 0.000 |
| nest | typescript | 14 | 0.486 | 0.309 | 0.000 | 0.000 | 0.115 | 0.000 | 0.000 |
| php-monolog | php | 12 | 0.536 | 0.525 | 0.000 | 0.000 | 0.706 | 0.047 | 0.000 |
| php-symfony-console | php | 14 | 0.571 | 0.592 | 0.000 | 0.000 | 0.692 | 0.041 | 0.000 |
| preact | javascript | 12 | 0.524 | 0.378 | 0.000 | 0.000 | 0.622 | 0.044 | 0.000 |
| regex | rust | 14 | 0.357 | 0.259 | 0.000 | 0.000 | 0.651 | 0.089 | 0.000 |
| tokio | rust | 13 | 0.540 | 0.320 | 0.000 | 0.000 | 0.533 | 0.055 | 0.000 |

**Reproduce:** `graphatlas bench --uc impact`

**Methodology:** see [docs/guide/uc-impact-dataset-methodology.md](../docs/guide/uc-impact-dataset-methodology.md)
