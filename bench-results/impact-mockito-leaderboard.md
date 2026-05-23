# Leaderboard: UC `impact` — mockito

**Language:** java

**Tasks:** 13 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-05-13

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| ga        | 0.455 ⭐  | 0.179 ⭐    | 0.821       | 0.885 ⭐  | 0.030 ⭐    | 661    |   15.4%  | 0.736        | 0.075 ⭐  |
| code-review-graph | 0.287    | 0.000      | 0.949 ⭐     | 0.000    | 0.013      | 512    |    0.0%  | 0.944 ⭐      | 0.036    |
| bm25      | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.385        | 0.000    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.385        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.385        | 0.000    |
| random    | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.385        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.385        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo mockito`
