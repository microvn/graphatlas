# Leaderboard: UC `impact` — MQTTnet

**Language:** csharp

**Tasks:** 14 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-05-13

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| code-review-graph | 0.524 ⭐  | 0.814 ⭐    | 0.649 ⭐     | 0.000    | 0.027      | 355    |    0.0%  | 0.611 ⭐      | 0.068    |
| ga        | 0.290    | 0.167      | 0.411       | 0.500 ⭐  | 0.166 ⭐    | 2898   |   14.3%  | 0.295        | 0.184 ⭐  |
| bm25      | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.214        | 0.000    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.214        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.214        | 0.000    |
| random    | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.214        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.214        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo MQTTnet`
