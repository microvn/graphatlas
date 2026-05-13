# Leaderboard: UC `impact` — regex

**Language:** rust

**Tasks:** 14 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-04-29

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.651 ⭐  | 0.571 ⭐    | 0.929       | 0.929 ⭐  | 0.028      | 2619   |   57.1%  | 0.571 ⭐      | 0.191    |
| bm25      | 0.357    | 0.107      | 1.000 ⭐     | 0.000    | 0.096 ⭐    | 0      |    0.0%  | 0.398        | 0.325 ⭐  |
| code-review-graph | 0.259    | 0.000      | 0.845       | 0.000    | 0.036      | 1859   |    0.0%  | 0.476        | 0.253    |
| random    | 0.089    | 0.173      | 0.060       | 0.000    | 0.011      | 0      |    0.0%  | 0.076        | 0.089    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.000        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.000        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.000        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo regex`
