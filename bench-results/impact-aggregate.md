# Impact Benchmark — Cross-Fixture Aggregate

**Dataset:** 144 tasks across 12 repos (MQTTnet, axum, django, faraday, gin, kotlinx-coroutines, kotlinx-serialization, mockito, nest, preact, regex, tokio)

**Split:** test

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-04-29

**Gate (AS-009):** composite ≥ 0.80 | test_recall ≥ 0.85 | completeness ≥ 0.80 | depth_F1 ≥ 0.80 | precision ≥ 0.70 | p95 ≤ 500ms

## Overall

| Retriever | Composite | Test Recall | Completeness | Precision | p95 ms | Pass Rate | Gate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|-----------|--------|-----------|------|-------------|----------|
| ga        | 0.569 ⭐  | 0.531 ⭐    | 0.750       | 0.137 ⭐    | 5128   |   41.7%  | — | 0.539 ⭐      | 0.181    |
| code-review-graph | 0.342    | 0.193      | 0.816 ⭐     | 0.131      | 1592   |    0.0%  | — | 0.515        | 0.179    |
| bm25      | 0.286    | 0.287      | 0.510       | 0.122      | 0      |    0.0%  | — | 0.435        | 0.196 ⭐  |
| random    | 0.031    | 0.046      | 0.039       | 0.006      | 0      |    0.0%  | — | 0.262        | 0.019    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000      | 0      |    0.0%  | — | 0.236        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000      | 0      |    0.0%  | — | 0.236        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000      | 0      |    0.0%  | — | 0.236        | 0.000    |

## Token cost vs lexical IR baseline

**GA vs BM25:** resolves 1.40× more regression-causing changes (66.0% vs 47.2% reach 100% recall) using 19% fewer tokens per successful retrieval (14308 vs 17715).

Token cost = bytes/4 of files an agent reads, walking the retriever's ranked list, to reach the recall threshold. Means are **conditional on success** — failures aren't folded in, since a retriever that returns fewer files would otherwise look cheaper just for missing more.

| Retriever | reached 50% | tokens→50% (when reached) | reached 100% | tokens→100% (when reached) | files returned |
|-----------|------------:|--------------------------:|-------------:|---------------------------:|---------------:|
| ga        |       79.2% |                     15105 |        66.0% |                      14308 |           35.5 |
| bm25      |       52.8% |                     17174 |        47.2% |                      17715 |            9.8 |
| random    |        4.2% |                     14478 |         2.8% |                      13460 |            7.2 |
| ripgrep   |        0.0% |                         0 |         0.0% |                          0 |            0.0 |

## Summary

**Top composite:** `ga` (0.569)

**Top precision:** `ga` (0.137)

**Top test recall:** `ga` (0.531)


## Per-repo composite

| Repo | Lang | Tasks | bm25 | code-review-graph | codebase-memory | codegraphcontext | ga | random | ripgrep |
|------|------|-------|-------|-------|-------|-------|-------|-------|-------|
| MQTTnet | csharp | 14 | 0.000 | 0.524 | 0.000 | 0.000 | 0.290 | 0.000 | 0.000 |
| axum | rust | 4 | 0.479 | 0.178 | 0.000 | 0.000 | 0.692 | 0.078 | 0.000 |
| django | python | 10 | 0.549 | 0.263 | 0.000 | 0.000 | 0.604 | 0.000 | 0.000 |
| faraday | ruby | 13 | 0.000 | 0.361 | 0.000 | 0.000 | 0.646 | 0.000 | 0.000 |
| gin | go | 14 | 0.621 | 0.524 | 0.000 | 0.000 | 0.721 | 0.118 | 0.000 |
| kotlinx-coroutines | kotlin | 9 | 0.000 | 0.214 | 0.000 | 0.000 | 0.432 | 0.000 | 0.000 |
| kotlinx-serialization | kotlin | 14 | 0.000 | 0.300 | 0.000 | 0.000 | 0.583 | 0.000 | 0.000 |
| mockito | java | 13 | 0.000 | 0.287 | 0.000 | 0.000 | 0.463 | 0.000 | 0.000 |
| nest | typescript | 14 | 0.486 | 0.309 | 0.000 | 0.000 | 0.646 | 0.000 | 0.000 |
| preact | javascript | 12 | 0.524 | 0.378 | 0.000 | 0.000 | 0.622 | 0.044 | 0.000 |
| regex | rust | 14 | 0.357 | 0.259 | 0.000 | 0.000 | 0.651 | 0.089 | 0.000 |
| tokio | rust | 13 | 0.540 | 0.320 | 0.000 | 0.000 | 0.533 | 0.055 | 0.000 |

**Reproduce:** `graphatlas bench --uc impact`

**Methodology:** see [docs/guide/uc-impact-dataset-methodology.md](../docs/guide/uc-impact-dataset-methodology.md)
