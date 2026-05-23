# Leaderboard: UC `impact` — MQTTnet

**Language:** csharp

**Tasks:** 14 (split: test)

**Spec:** S-004 AS-009

**GT source:** git-mining-2026-05-13

**Gate:** composite ≥ 0.80 (S-004 AS-009)

| Retriever | Composite | Test Recall | Completeness | Depth_F1 | Precision | p95 ms | Pass Rate | BlastRadius | AdjPrec |
|-----------|-----------|-------------|--------------|----------|-----------|--------|-----------|-------------|----------|
| code-review-graph | 0.208 ⭐  | 0.286 ⭐    | 0.307 ⭐     | 0.000    | 0.012      | 195    |    0.0%  | 0.329 ⭐      | 0.026    |
| ga        | 0.173    | 0.100      | 0.221       | 0.286 ⭐  | 0.157 ⭐    | 329    |    7.1%  | 0.200        | 0.169 ⭐  |
| bm25      | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.214        | 0.000    |
| codebase-memory | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.000        | 0.000    |
| codegraphcontext | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.000        | 0.000    |
| random    | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.000        | 0.000    |
| ripgrep   | 0.000    | 0.000      | 0.000       | 0.000    | 0.000      | 0      |    0.0%  | 0.214        | 0.000    |

**Reproduce:** `graphatlas bench --uc impact --repo MQTTnet`
