# Leaderboard: UC `impact` — django

**Language:** python

**Tasks:** 10 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-04-29

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.604 ⭐  | 0.550      | 0.850       | 0.550 ⭐  | 0.312 ⭐    | 1833   |   40.0%  | 0.338        | 0.324 ⭐  |
| bm25      | 0.549    | 0.717 ⭐    | 0.833       | 0.000    | 0.080      | 0      |    0.0%  | 0.837 ⭐      | 0.314    |
| code-review-graph | 0.263    | 0.000      | 0.867 ⭐     | 0.000    | 0.021      | 1086   |    0.0%  | 0.267        | 0.029    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.200        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.200        | 0.000    |
| random    | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.200        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.200        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo django`
