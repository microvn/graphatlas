# Leaderboard: UC `impact` — jekyll

**Language:** ruby

**Tasks:** 10 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-04-28

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.477 ⭐  | 0.550 ⭐    | 0.550 ⭐     | 0.600 ⭐  | 0.014 ⭐    | 15884  |   10.0%  | 0.444 ⭐      | 0.072 ⭐  |
| bm25      | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.100        | 0.000    |
| code-review-graph | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.100        | 0.000    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.100        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.100        | 0.000    |
| random    | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.100        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.100        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo jekyll`
