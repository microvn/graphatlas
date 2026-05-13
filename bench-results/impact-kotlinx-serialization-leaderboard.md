# Leaderboard: UC `impact` — kotlinx-serialization

**Language:** kotlin

**Tasks:** 14 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-04-29

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.583 ⭐  | 0.571 ⭐    | 0.675       | 0.571 ⭐  | 0.443 ⭐    | 4300   |   21.4%  | 0.508        | 0.464 ⭐  |
| code-review-graph | 0.300    | 0.167      | 0.761 ⭐     | 0.000    | 0.032      | 1338   |    0.0%  | 0.629 ⭐      | 0.084    |
| bm25      | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.286        | 0.000    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.286        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.286        | 0.000    |
| random    | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.286        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.286        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo kotlinx-serialization`
