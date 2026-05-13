# Leaderboard: UC `impact` — tokio

**Language:** rust

**Tasks:** 13 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-04-29

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| bm25      | 0.540 ⭐  | 0.615 ⭐    | 0.944 ⭐     | 0.000    | 0.074      | 0      |    0.0%  | 0.467 ⭐      | 0.220 ⭐  |
| ga        | 0.533    | 0.462      | 0.769       | 0.769 ⭐  | 0.015      | 3838   |   46.2%  | 0.236        | 0.038    |
| code-review-graph | 0.320    | 0.077      | 0.879       | 0.000    | 0.169 ⭐    | 529    |    0.0%  | 0.138        | 0.178    |
| random    | 0.055    | 0.038      | 0.123       | 0.000    | 0.020      | 0      |    0.0%  | 0.093        | 0.039    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.077        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.077        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.077        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo tokio`
