# Leaderboard: UC `impact` — gin

**Language:** go

**Tasks:** 14 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-04-28

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.693 ⭐  | 0.768 ⭐    | 0.841       | 0.857 ⭐  | 0.032      | 5737   |   71.4%  | 0.679        | 0.034    |
| bm25      | 0.596    | 0.616      | 0.992 ⭐     | 0.000    | 0.349 ⭐    | 0      |    0.0%  | 0.679        | 0.354 ⭐  |
| random    | 0.088    | 0.126      | 0.115       | 0.000    | 0.022      | 0      |    0.0%  | 0.571        | 0.022    |
| code-review-graph | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.571        | 0.000    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.571        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.571        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.571        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo gin`
