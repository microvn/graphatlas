# Leaderboard: UC `impact` — faraday

**Language:** ruby

**Tasks:** 13 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-04-29

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.646 ⭐  | 0.692 ⭐    | 0.782 ⭐     | 0.692 ⭐  | 0.207      | 4940   |   46.2%  | 0.585 ⭐      | 0.294    |
| code-review-graph | 0.361    | 0.000      | 0.762       | 0.000    | 0.885 ⭐    | 12     |    0.0%  | 0.154        | 0.904 ⭐  |
| bm25      | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.077        | 0.000    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.077        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.077        | 0.000    |
| random    | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.077        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.077        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo faraday`
