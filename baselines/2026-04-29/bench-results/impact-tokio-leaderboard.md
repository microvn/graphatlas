# Leaderboard: UC `impact` — tokio

**Language:** rust

**Tasks:** 13 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-04-28

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.564 ⭐  | 0.538 ⭐    | 0.769 ⭐     | 0.769 ⭐  | 0.015      | 7992   |   53.8%  | 0.282        | 0.040    |
| bm25      | 0.272    | 0.308      | 0.469       | 0.000    | 0.053 ⭐    | 0      |    0.0%  | 0.333 ⭐      | 0.183 ⭐  |
| random    | 0.015    | 0.038      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.005        | 0.005    |
| code-review-graph | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.077        | 0.000    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.077        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.077        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.000        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo tokio`
