# Leaderboard: UC `impact` — preact

**Language:** javascript

**Tasks:** 14 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-04-28

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.428 ⭐  | 0.500 ⭐    | 0.500       | 0.429 ⭐  | 0.092      | 15874  |   50.0%  | 0.310        | 0.113    |
| bm25      | 0.300    | 0.310      | 0.524 ⭐     | 0.000    | 0.124 ⭐    | 0      |    0.0%  | 0.522 ⭐      | 0.229 ⭐  |
| random    | 0.089    | 0.167      | 0.071       | 0.000    | 0.007      | 0      |    0.0%  | 0.252        | 0.040    |
| code-review-graph | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.143        | 0.000    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.000        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.000        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.143        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo preact`
