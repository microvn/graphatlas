# Leaderboard: UC `impact` — axum

**Language:** rust

**Tasks:** 4 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-05-13

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.647 ⭐  | 0.750 ⭐    | 0.708 ⭐     | 0.875 ⭐  | 0.025 ⭐    | 859    |   50.0%  | 0.517        | 0.105    |
| bm25      | 0.328    | 0.500      | 0.417       | 0.000    | 0.018      | 0      |    0.0%  | 0.300        | 0.040    |
| code-review-graph | 0.178    | 0.000      | 0.583       | 0.000    | 0.020      | 233    |    0.0%  | 0.950 ⭐      | 0.176 ⭐  |
| random    | 0.078    | 0.000      | 0.250       | 0.000    | 0.023      | 0      |    0.0%  | 0.000        | 0.023    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.250        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.250        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.000        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo axum`
