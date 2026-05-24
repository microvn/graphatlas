# 5×5 MCP Cross-Tool Audit — 2026-05-24

Real-MCP companion to `bench-results/30q-v2-summary.md`. Each cell is
the **actual token cost** of one MCP `tools/call` response captured
over stdio. Token = `chars(result_json) / 4`, matching the offline
`payload_tokens` column in the 30q v2 leaderboard.

**Tools**: ga, semble

## preact

| # | UC | Seed | ga tok | semble tok |
|---|----|------|---:|---:|
| 0 | callers | render | 3551 | 7213 |
| 1 | callers | diff | 174 | 5813 |
| 2 | callees | render | 3550 | 6319 |
| 3 | impact | diffElementNodes | 4953 | 6413 |
| 4 | callers | createElement | 785 | 5661 |

## gin

| # | UC | Seed | ga tok | semble tok |
|---|----|------|---:|---:|
| 0 | callers | Default | 204 | 6815 |
| 1 | callers | New | 6704 | 6671 |
| 2 | callees | Engine | 170 | 6609 |
| 3 | impact | Engine | 194 | 6617 |
| 4 | callers | Run | 162 | 6889 |

## tokio

| # | UC | Seed | ga tok | semble tok |
|---|----|------|---:|---:|
| 0 | callers | block_on | 542 | 6142 |
| 1 | callers | JoinHandle | 2055 | 6503 |
| 2 | callees | Scheduler | 187 | 7235 |
| 3 | impact | block_on | 564 | 6774 |
| 4 | callers | spawn | 669 | 7474 |

## django

| # | UC | Seed | ga tok | semble tok |
|---|----|------|---:|---:|
| 0 | callers | get_object | 384 | 6846 |
| 1 | callers | render | 1255 | 5075 |
| 2 | callees | save | 1040 | 7139 |
| 3 | impact | save | 1064 | 7618 |
| 4 | callers | QuerySet | 69 | 6387 |

## kotlinx-coroutines

| # | UC | Seed | ga tok | semble tok |
|---|----|------|---:|---:|
| 0 | callers | launch | 261 | 10075 |
| 1 | callers | async | 204 | 7295 |
| 2 | callees | CoroutineScope | 183 | 7330 |
| 3 | impact | Dispatchers | 326 | 8438 |
| 4 | callers | withContext | 227 | 7018 |

## Cumulative tokens per tool

| Tool | Total | Mean / query | Errors | Skips |
|------|------:|-------------:|-------:|------:|
| ga | 29477 | 1179 | 0 | 0 |
| semble | 172369 | 6895 | 0 | 0 |

> See `benches/cross-tool-mcp/README.md` for methodology + per-tool caveats.
