# Leaderboard: UC `impact` — php-monolog

**Language:** php

**Tasks:** 12 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-05-13

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.706 ⭐  | 0.683      | 0.931       | 0.917 ⭐  | 0.107      | 480    |   66.7%  | 0.864        | 0.170    |
| bm25      | 0.536    | 0.683      | 0.674       | 0.000    | 0.404 ⭐    | 0      |    8.3%  | 0.870 ⭐      | 0.632 ⭐  |
| code-review-graph | 0.525    | 0.517      | 1.000 ⭐     | 0.000    | 0.124      | 87     |    0.0%  | 0.667        | 0.161    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 2      |    0.0%  | 0.417        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.417        | 0.000    |
| gitnexus  | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.417        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.417        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo php-monolog`
