# Leaderboard: UC `impact` — django

**Language:** python

**Tasks:** 10 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-05-13

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| bm25      | 0.549 ⭐  | 0.717 ⭐    | 0.833 ⭐     | 0.000    | 0.080      | 0      |    0.0%  | 0.837 ⭐      | 0.314 ⭐  |
| ga        | 0.386    | 0.400      | 0.500       | 0.400 ⭐  | 0.108 ⭐    | 2024   |   40.0%  | 0.095        | 0.116    |
| code-review-graph | 0.187    | 0.000      | 0.617       | 0.000    | 0.014      | 1127   |    0.0%  | 0.067        | 0.023    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.200        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.200        | 0.000    |
| random    | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.200        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.200        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo django`
