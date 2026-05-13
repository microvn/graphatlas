# Leaderboard: UC `impact` — mockito

**Language:** java

**Tasks:** 13 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-04-29

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.463 ⭐  | 0.179 ⭐    | 0.846       | 0.885 ⭐  | 0.031 ⭐    | 12197  |   15.4%  | 0.710        | 0.068 ⭐  |
| code-review-graph | 0.287    | 0.000      | 0.949 ⭐     | 0.000    | 0.013      | 4375   |    0.0%  | 0.944 ⭐      | 0.036    |
| bm25      | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.385        | 0.000    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.385        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 24     |    0.0%  | 0.385        | 0.000    |
| random    | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.385        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.385        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo mockito`
