# Leaderboard: UC `impact` — php-symfony-console

**Language:** php

**Tasks:** 14 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-05-13

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.681 ⭐  | 0.679      | 0.857       | 0.857 ⭐  | 0.161 ⭐    | 3542   |   57.1%  | 0.746        | 0.179 ⭐  |
| code-review-graph | 0.592    | 0.786 ⭐    | 0.917 ⭐     | 0.000    | 0.019      | 247    |    0.0%  | 0.964 ⭐      | 0.043    |
| bm25      | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.429        | 0.000    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.429        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.429        | 0.000    |
| random    | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.429        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.429        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo php-symfony-console`
