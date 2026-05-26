# 30-Query Cross-Tool Bench v2 — Summary

**Fixtures**: preact, gin, tokio, django, php-symfony-console, kotlinx-coroutines
**UCs**: callers, callees, symbols
**Tools**: ga, ripgrep, codegraphcontext, codebase-memory, code-review-graph, gitnexus

## Cross-fixture average (per tool)

| Tool | Avg F1 | Avg F2 | Avg Recall | Avg Precision | Avg Pass % | Coverage | Avg payload tok | Tok / F1·100 |
|------|------:|------:|----------:|-------------:|----------:|----------|----------------:|-------------:|
| ga | 0.429 | 0.433 | 0.454 | 0.462 | 81.7% | 18/18 | 320 | 7.4 |
| ripgrep | 0.000 | — | 0.000 | 0.000 | 0.0% | 0/18 | — | — |
| codegraphcontext | 0.000 | — | 0.000 | 0.000 | 0.0% | 0/18 | — | — |
| codebase-memory | 0.112 | 0.142 | 0.236 | 0.104 | 8.3% | 18/18 | 48 | 4.3 |
| code-review-graph | 0.000 | 0.000 | 0.000 | 0.000 | 0.0% | 18/18 | 0 | — |
| gitnexus | 0.195 | 0.184 | 0.185 | 0.292 | 22.0% | 18/18 | 8 | 0.4 |

> **Tok / F1·100** = mean payload tokens divided by F1×100. Lower = more correctness per token. Caveat: file-set F1 ignores within-response detail; see `benches/cross-tool-mcp/` for real-MCP audit.

## CALLERS per fixture

| Fixture | ga | ripgrep | codegraphcontext | codebase-memory | code-review-graph | gitnexus |
|---|---|---|---|---|---|---|
| preact | 0.74 | — | — | 0.18 | 0.00 | 0.20 |
| gin | 0.68 | — | — | 0.21 | 0.00 | 0.38 |
| tokio | 0.56 | — | — | 0.06 | 0.00 | 0.25 |
| django | 0.59 | — | — | 0.21 | 0.00 | 0.34 |
| php-symfony-console | 0.64 | — | — | 0.12 | 0.00 | 0.32 |
| kotlinx-coroutines | 0.63 | — | — | 0.17 | 0.00 | 0.32 |

## CALLEES per fixture

| Fixture | ga | ripgrep | codegraphcontext | codebase-memory | code-review-graph | gitnexus |
|---|---|---|---|---|---|---|
| preact | 0.50 | — | — | 0.18 | 0.00 | 0.22 |
| gin | 0.51 | — | — | 0.21 | 0.00 | 0.33 |
| tokio | 0.67 | — | — | 0.13 | 0.00 | 0.21 |
| django | 0.75 | — | — | 0.18 | 0.00 | 0.23 |
| php-symfony-console | 0.66 | — | — | 0.16 | 0.00 | 0.40 |
| kotlinx-coroutines | 0.79 | — | — | 0.18 | 0.00 | 0.33 |

## SYMBOLS per fixture

| Fixture | ga | ripgrep | codegraphcontext | codebase-memory | code-review-graph | gitnexus |
|---|---|---|---|---|---|---|
| preact | 0.00 | — | — | 0.00 | 0.00 | 0.00 |
| gin | 0.00 | — | — | 0.00 | 0.00 | 0.00 |
| tokio | 0.00 | — | — | 0.00 | 0.00 | 0.00 |
| django | 0.00 | — | — | 0.00 | 0.00 | 0.00 |
| php-symfony-console | 0.00 | — | — | 0.00 | 0.00 | 0.00 |
| kotlinx-coroutines | 0.00 | — | — | 0.00 | 0.00 | 0.00 |

## Payload tokens per fixture (mean per task)

### CALLERS

| Fixture | ga | ripgrep | codegraphcontext | codebase-memory | code-review-graph | gitnexus |
|---|---|---|---|---|---|---|
| preact | 278 | — | — | 12 | 0 | 3 |
| gin | 364 | — | — | 26 | 0 | 9 |
| tokio | 355 | — | — | 7 | 0 | 5 |
| django | 780 | — | — | 71 | 0 | 13 |
| php-symfony-console | 368 | — | — | 180 | 0 | 20 |
| kotlinx-coroutines | 682 | — | — | 69 | 0 | 9 |

### CALLEES

| Fixture | ga | ripgrep | codegraphcontext | codebase-memory | code-review-graph | gitnexus |
|---|---|---|---|---|---|---|
| preact | 518 | — | — | 20 | 0 | 4 |
| gin | 320 | — | — | 31 | 0 | 6 |
| tokio | 655 | — | — | 8 | 0 | 3 |
| django | 368 | — | — | 82 | 0 | 6 |
| php-symfony-console | 377 | — | — | 247 | 0 | 8 |
| kotlinx-coroutines | 405 | — | — | 104 | 0 | 9 |

### SYMBOLS

| Fixture | ga | ripgrep | codegraphcontext | codebase-memory | code-review-graph | gitnexus |
|---|---|---|---|---|---|---|
| preact | 50 | — | — | 0 | 0 | 4 |
| gin | 41 | — | — | 0 | 0 | 12 |
| tokio | 46 | — | — | 0 | 0 | 8 |
| django | 48 | — | — | 0 | 0 | 1 |
| php-symfony-console | 43 | — | — | 0 | 0 | 10 |
| kotlinx-coroutines | 56 | — | — | 0 | 0 | 5 |

## Coverage gaps (tool × fixture × uc with F1=0)

| Tool | Fixture | UC | Note |
|------|---------|----|----|
| ga | preact | symbols | F1=0 (not indexed or unsupported) |
| ga | gin | symbols | F1=0 (not indexed or unsupported) |
| ga | tokio | symbols | F1=0 (not indexed or unsupported) |
| ga | django | symbols | F1=0 (not indexed or unsupported) |
| ga | php-symfony-console | symbols | F1=0 (not indexed or unsupported) |
| ga | kotlinx-coroutines | symbols | F1=0 (not indexed or unsupported) |
| ripgrep | preact | callers | leaderboard missing |
| ripgrep | gin | callers | leaderboard missing |
| ripgrep | tokio | callers | leaderboard missing |
| ripgrep | django | callers | leaderboard missing |
| ripgrep | php-symfony-console | callers | leaderboard missing |
| ripgrep | kotlinx-coroutines | callers | leaderboard missing |
| ripgrep | preact | callees | leaderboard missing |
| ripgrep | gin | callees | leaderboard missing |
| ripgrep | tokio | callees | leaderboard missing |
| ripgrep | django | callees | leaderboard missing |
| ripgrep | php-symfony-console | callees | leaderboard missing |
| ripgrep | kotlinx-coroutines | callees | leaderboard missing |
| ripgrep | preact | symbols | leaderboard missing |
| ripgrep | gin | symbols | leaderboard missing |
| ripgrep | tokio | symbols | leaderboard missing |
| ripgrep | django | symbols | leaderboard missing |
| ripgrep | php-symfony-console | symbols | leaderboard missing |
| ripgrep | kotlinx-coroutines | symbols | leaderboard missing |
| codegraphcontext | preact | callers | leaderboard missing |
| codegraphcontext | gin | callers | leaderboard missing |
| codegraphcontext | tokio | callers | leaderboard missing |
| codegraphcontext | django | callers | leaderboard missing |
| codegraphcontext | php-symfony-console | callers | leaderboard missing |
| codegraphcontext | kotlinx-coroutines | callers | leaderboard missing |
| codegraphcontext | preact | callees | leaderboard missing |
| codegraphcontext | gin | callees | leaderboard missing |
| codegraphcontext | tokio | callees | leaderboard missing |
| codegraphcontext | django | callees | leaderboard missing |
| codegraphcontext | php-symfony-console | callees | leaderboard missing |
| codegraphcontext | kotlinx-coroutines | callees | leaderboard missing |
| codegraphcontext | preact | symbols | leaderboard missing |
| codegraphcontext | gin | symbols | leaderboard missing |
| codegraphcontext | tokio | symbols | leaderboard missing |
| codegraphcontext | django | symbols | leaderboard missing |
| codegraphcontext | php-symfony-console | symbols | leaderboard missing |
| codegraphcontext | kotlinx-coroutines | symbols | leaderboard missing |
| codebase-memory | preact | symbols | F1=0 (not indexed or unsupported) |
| codebase-memory | gin | symbols | F1=0 (not indexed or unsupported) |
| codebase-memory | tokio | symbols | F1=0 (not indexed or unsupported) |
| codebase-memory | django | symbols | F1=0 (not indexed or unsupported) |
| codebase-memory | php-symfony-console | symbols | F1=0 (not indexed or unsupported) |
| codebase-memory | kotlinx-coroutines | symbols | F1=0 (not indexed or unsupported) |
| code-review-graph | preact | callers | F1=0 (not indexed or unsupported) |
| code-review-graph | gin | callers | F1=0 (not indexed or unsupported) |
| code-review-graph | tokio | callers | F1=0 (not indexed or unsupported) |
| code-review-graph | django | callers | F1=0 (not indexed or unsupported) |
| code-review-graph | php-symfony-console | callers | F1=0 (not indexed or unsupported) |
| code-review-graph | kotlinx-coroutines | callers | F1=0 (not indexed or unsupported) |
| code-review-graph | preact | callees | F1=0 (not indexed or unsupported) |
| code-review-graph | gin | callees | F1=0 (not indexed or unsupported) |
| code-review-graph | tokio | callees | F1=0 (not indexed or unsupported) |
| code-review-graph | django | callees | F1=0 (not indexed or unsupported) |
| code-review-graph | php-symfony-console | callees | F1=0 (not indexed or unsupported) |
| code-review-graph | kotlinx-coroutines | callees | F1=0 (not indexed or unsupported) |
| code-review-graph | preact | symbols | F1=0 (not indexed or unsupported) |
| code-review-graph | gin | symbols | F1=0 (not indexed or unsupported) |
| code-review-graph | tokio | symbols | F1=0 (not indexed or unsupported) |
| code-review-graph | django | symbols | F1=0 (not indexed or unsupported) |
| code-review-graph | php-symfony-console | symbols | F1=0 (not indexed or unsupported) |
| code-review-graph | kotlinx-coroutines | symbols | F1=0 (not indexed or unsupported) |
| gitnexus | preact | symbols | F1=0 (not indexed or unsupported) |
| gitnexus | gin | symbols | F1=0 (not indexed or unsupported) |
| gitnexus | tokio | symbols | F1=0 (not indexed or unsupported) |
| gitnexus | django | symbols | F1=0 (not indexed or unsupported) |
| gitnexus | php-symfony-console | symbols | F1=0 (not indexed or unsupported) |
| gitnexus | kotlinx-coroutines | symbols | F1=0 (not indexed or unsupported) |

**Generated**: aggregator script `aggregate-30q-v2.py` over /Volumes/Data/projects/me/graphatlas/bench-results/