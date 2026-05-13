# Leaderboard: UC `impact` — kotlinx-coroutines

**Language:** kotlin

**Tasks:** 9 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-04-29

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.432 ⭐  | 0.361 ⭐    | 0.506       | 0.778 ⭐  | 0.131 ⭐    | 541    |   11.1%  | 0.496        | 0.167    |
| code-review-graph | 0.214    | 0.028      | 0.617 ⭐     | 0.000    | 0.116      | 633    |    0.0%  | 0.689 ⭐      | 0.195 ⭐  |
| bm25      | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.333        | 0.000    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.333        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.333        | 0.000    |
| random    | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.333        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.333        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo kotlinx-coroutines`
