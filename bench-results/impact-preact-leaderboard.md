# Leaderboard: UC `impact` — preact

**Language:** javascript

**Tasks:** 12 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-04-29

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.622 ⭐  | 0.639 ⭐    | 0.792       | 0.750 ⭐  | 0.110      | 2674   |   58.3%  | 0.544        | 0.156    |
| bm25      | 0.524    | 0.528      | 0.958 ⭐     | 0.000    | 0.172 ⭐    | 0      |    0.0%  | 0.751 ⭐      | 0.286 ⭐  |
| code-review-graph | 0.378    | 0.250      | 0.903       | 0.000    | 0.047      | 1409   |    0.0%  | 0.645        | 0.078    |
| random    | 0.044    | 0.111      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.239        | 0.029    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.167        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.167        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.167        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo preact`
