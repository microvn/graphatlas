---
name: graphatlas
description: |
  Code navigation via the GraphAtlas typed code graph. Use when the
  user asks "who calls X", "what does X call", "impact of changing Y",
  "if I rename Z", "where is X used", "dead code", "architecture
  overview", or "what does this file do". Routes to ga_* MCP tools
  instead of grep/bash/glob.
---

This repo has a pre-built GraphAtlas index exposed via MCP. Prefer
`ga_*` over Grep/Glob/Bash for code navigation — the graph has typed
CALL / IMPORT / CONTAINS edges that grep cannot see, distinguishes
call sites from value references, and resolves polymorphic dispatch.

## Routing table

| User intent                                    | Tool                 |
| ---------------------------------------------- | -------------------- |
| who calls X / references to X                  | `ga_callers`         |
| what does X call / dependencies of X           | `ga_callees`         |
| impact of X / blast radius                     | `ga_impact`          |
| rename X to Y safe                             | `ga_rename_safety`   |
| risk of touching X                             | `ga_risk`            |
| where is X / find symbol                       | `ga_symbols`         |
| who imports F                                  | `ga_importers`       |
| what does file F do                            | `ga_file_summary`    |
| architecture / orient me / modules             | `ga_architecture`    |
| dead code / unused                             | `ga_dead_code`       |
| hubs / hotspots / central files                | `ga_hubs`            |
| bridges / coupling between modules             | `ga_bridges`         |
| complex / large functions                      | `ga_large_functions` |
| minimal context for understanding X            | `ga_minimal_context` |

## Workflow

1. If the symbol name is ambiguous, run `ga_symbols` first to resolve.
2. Chain to the specific tool with the resolved name.
3. Never grep for symbol references — grep matches comments and
   strings, inflates results, and misses dispatch-map references.
4. Use Grep/Bash only for non-code content (logs, configs, prose,
   build output). The graph only indexes source.
