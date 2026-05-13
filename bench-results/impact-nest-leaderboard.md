# Leaderboard: UC `impact` — nest

**Language:** typescript

**Tasks:** 14 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-04-29

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.646 ⭐  | 0.686 ⭐    | 0.798       | 0.786 ⭐  | 0.097      | 584    |   57.1%  | 0.586 ⭐      | 0.109    |
| bm25      | 0.486    | 0.479      | 0.738       | 0.000    | 0.485 ⭐    | 0      |    0.0%  | 0.544        | 0.598 ⭐  |
| code-review-graph | 0.309    | 0.107      | 0.863 ⭐     | 0.000    | 0.051      | 622    |    0.0%  | 0.445        | 0.057    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.357        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.357        | 0.000    |
| random    | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.398        | 0.010    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.357        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo nest`
