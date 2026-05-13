# Leaderboard: UC `impact` — axum

**Language:** rust

**Tasks:** 4 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-04-28

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.692 ⭐  | 0.750 ⭐    | 0.792       | 1.000 ⭐  | 0.030      | 6344   |   50.0%  | 0.767 ⭐      | 0.115    |
| bm25      | 0.479    | 0.500      | 0.792       | 0.000    | 0.275 ⭐    | 0      |    0.0%  | 0.383        | 0.330 ⭐  |
| random    | 0.078    | 0.000      | 0.250       | 0.000    | 0.023      | 0      |    0.0%  | 0.250        | 0.023    |
| code-review-graph | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.250        | 0.000    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.250        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.250        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.250        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo axum`
