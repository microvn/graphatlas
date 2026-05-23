# Leaderboard: UC `impact` — php-symfony-console

**Language:** php

**Tasks:** 14 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-05-13

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.692 ⭐  | 0.679      | 0.893       | 0.929 ⭐  | 0.093      | 515    |   57.1%  | 0.746        | 0.110    |
| code-review-graph | 0.592    | 0.786 ⭐    | 0.917 ⭐     | 0.000    | 0.019      | 154    |    0.0%  | 0.964        | 0.043    |
| bm25      | 0.571    | 0.643      | 0.857       | 0.000    | 0.380 ⭐    | 0      |   14.3%  | 0.994 ⭐      | 0.565 ⭐  |
| random    | 0.041    | 0.000      | 0.131       | 0.000    | 0.014      | 0      |    0.0%  | 0.441        | 0.024    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.429        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.429        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.429        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo php-symfony-console`
