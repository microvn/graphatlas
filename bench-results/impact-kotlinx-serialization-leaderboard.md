# Leaderboard: UC `impact` — kotlinx-serialization

**Language:** kotlin

**Tasks:** 14 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-05-13

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | F2 files | F2 tests | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|----------|----------|--------|-----------|-------------|----------|
| ga        | 0.583 ⭐  | 0.571 ⭐    | 0.675       | 0.571 ⭐  | 0.443 ⭐    | 0.336 ⭐     | 0.319 ⭐     | 403    |   21.4%  | 0.498        | 0.461 ⭐  |
| code-review-graph | 0.300    | 0.167      | 0.761 ⭐     | 0.000    | 0.032      | 0.131       | 0.076       | 456    |    0.0%  | 0.629 ⭐      | 0.084    |
| bm25      | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0.000       | 0.000       | 0      |    0.0%  | 0.286        | 0.000    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0.000       | 0.000       | 1      |    0.0%  | 0.286        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0.000       | 0.000       | 0      |    0.0%  | 0.286        | 0.000    |
| gitnexus  | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0.000       | 0.000       | 0      |    0.0%  | 0.286        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0.000       | 0.000       | 0      |    0.0%  | 0.286        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo kotlinx-serialization`
