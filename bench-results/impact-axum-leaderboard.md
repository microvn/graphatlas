# Leaderboard: UC `impact` — axum

**Language:** rust

**Tasks:** 4 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-05-13

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.572 ⭐  | 0.750 ⭐    | 0.583       | 0.625 ⭐  | 0.020      | 650    |   50.0%  | 0.450        | 0.080    |
| bm25      | 0.479    | 0.500      | 0.792 ⭐     | 0.000    | 0.275 ⭐    | 0      |    0.0%  | 0.383        | 0.330 ⭐  |
| code-review-graph | 0.178    | 0.000      | 0.583       | 0.000    | 0.020      | 228    |    0.0%  | 0.950 ⭐      | 0.176    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.250        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.250        | 0.000    |
| gitnexus  | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.250        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.250        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo axum`
