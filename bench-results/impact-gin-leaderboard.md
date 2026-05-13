# Leaderboard: UC `impact` — gin

**Language:** go

**Tasks:** 14 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-04-29

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.721 ⭐  | 0.839 ⭐    | 0.841       | 0.857 ⭐  | 0.027      | 665    |   78.6%  | 0.750 ⭐      | 0.032    |
| bm25      | 0.621    | 0.688      | 0.992 ⭐     | 0.000    | 0.321 ⭐    | 0      |    0.0%  | 0.607        | 0.326 ⭐  |
| code-review-graph | 0.524    | 0.594      | 0.917       | 0.000    | 0.077      | 244    |    0.0%  | 0.536        | 0.081    |
| random    | 0.118    | 0.174      | 0.151       | 0.000    | 0.022      | 0      |    0.0%  | 0.571        | 0.029    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.500        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.500        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.500        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo gin`
