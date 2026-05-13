# Leaderboard: UC `impact` — regex

**Language:** rust

**Tasks:** 14 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-04-28

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.280 ⭐  | 0.214 ⭐    | 0.429 ⭐     | 0.429 ⭐  | 0.009      | 8686   |   21.4%  | 0.271 ⭐      | 0.095    |
| random    | 0.081    | 0.155      | 0.060       | 0.000    | 0.011      | 0      |    0.0%  | 0.038        | 0.051    |
| bm25      | 0.039    | 0.000      | 0.119       | 0.000    | 0.020 ⭐    | 0      |    0.0%  | 0.076        | 0.142 ⭐  |
| code-review-graph | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.000        | 0.000    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.000        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.000        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.000        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo regex`
