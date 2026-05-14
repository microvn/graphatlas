# Leaderboard: UC `impact` — php-monolog

**Language:** php

**Tasks:** 12 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-05-13

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.719 ⭐  | 0.683 ⭐    | 0.972       | 0.917 ⭐  | 0.110      | 3592   |   66.7%  | 0.825 ⭐      | 0.162 ⭐  |
| code-review-graph | 0.525    | 0.517      | 1.000 ⭐     | 0.000    | 0.124 ⭐    | 175    |    0.0%  | 0.667        | 0.161    |
| bm25      | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.417        | 0.000    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.417        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.417        | 0.000    |
| random    | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.417        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.417        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo php-monolog`
