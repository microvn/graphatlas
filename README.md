# GraphAtlas

Graph-based code-context engine for AI coding agents. Rust workspace, MCP-native, dual-licensed MIT/Apache.

[![CI](https://github.com/microvn/GraphAtlas/actions/workflows/ci.yml/badge.svg)](https://github.com/microvn/GraphAtlas/actions)
[![License: MIT/Apache-2.0](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE-MIT)
[![MCP](https://img.shields.io/badge/MCP-compatible-green.svg)](https://modelcontextprotocol.io)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](rust-toolchain.toml)

> **GraphAtlas resolves 1.40× more regression-causing changes than BM25 using 19% fewer tokens** — on a 144-task git-mined benchmark across 12 OSS repos. Deterministic, MCP-native, no LLM in the loop.

## What it does

AI coding agents struggle with blast radius. Asked to change a function, the agent sees the file it lives in — not the 12 callers, 3 test files, 2 re-exports, and the override in another module. So fixes ship with regressions, or the agent dumps the whole repo into the prompt and prays.

GraphAtlas pre-computes the call / import / override / reference graph once, stores it in an embedded graph DB (LadybugDB), and exposes graph queries as MCP tools. The agent asks "who calls this?" — gets a precise answer in milliseconds, with a tight token footprint. No LLM in the engine, no hallucination, deterministic results.

## A real agent flow

**Without GraphAtlas — agent has only Grep:**

```
> rg compute_total
200 matches across 47 files — docs, tests, vendored deps, string literals.

Agent reads all 47 files (~85K tokens). Still misses the override in
subscription.rs because rg can't see the trait dispatch. Fix ships, the
nightly suite catches a regression.
```

**With GraphAtlas — agent has `ga_callers`:**

```jsonc
> ga_callers --symbol compute_total --file src/order.rs
{
  "callers": [
    { "symbol": "Order::finalize",                "file": "src/order.rs",        "line": 142, "edge": "call" },
    { "symbol": "SubscriptionOrder::compute_total","file": "src/subscription.rs", "line":  88, "edge": "override" },
    { "symbol": "test_order_total",               "file": "tests/order.rs",      "line":  23, "edge": "call" }
  ],
  "meta": { "max_depth": 2, "polymorphic_resolved": true }
}
12 callers in 14ms. Agent reads exactly the files that matter (~6K tokens).
The override edge surfaces the subscription override no grep would catch.
```

## Honest numbers

Git-mined benchmark across 12 OSS repos (axum, tokio, regex, django, gin, kotlinx-coroutines, kotlinx-serialization, mockito, nest, preact, MQTTnet, faraday). 144 tasks, each derived from a real fix commit. Full table in `bench-results/impact-aggregate.md`.

| Retriever | Composite | Reach 100% recall | Tokens→100% (when reached) |
|-----------|----------:|------------------:|---------------------------:|
| **graphatlas** | **0.569** | **66.0%** | **14,308** |
| BM25 | 0.286 | 47.2% | 17,715 |
| ripgrep | 0.000 | 0.0% | — |
| random | 0.031 | 2.8% | 13,460 |

The three numbers that matter:

- **2.0× BM25's composite quality** — composite = 0.4·test_recall + 0.3·completeness + 0.15·depth_F1 + 0.15·precision.
- **1.40× more changes resolved** at full recall (66.0% vs 47.2%).
- **19% fewer tokens** per successful retrieval (14,308 vs 17,715).

Token cost = bytes/4 of files an agent reads, walking the retriever's ranked list, to reach the recall threshold. Means are conditional on success — a retriever that returns fewer files would otherwise look cheaper just for missing more.

Reproducible from a fresh clone: `cargo test -p ga-bench --test m2_gate_impact -- --nocapture`. Pre-1.0 — gate pass rate is 41.7% at the strict composite ≥0.80 bar. Engine improvements track in `CHANGELOG.md`.

## Vs. the alternatives

|  | grep / ripgrep | BM25 / embeddings | AST-only | **GraphAtlas** |
|--|:--:|:--:|:--:|:--:|
| Resolves call / import edges | — | — | partial | ✓ |
| Polymorphic dispatch | — | — | — | ✓ |
| Re-export following | — | — | — | ✓ |
| Tests-of-symbol surfacing | — | partial | — | ✓ |
| Deterministic, no LLM | ✓ | ✓ | ✓ | ✓ |
| Token-efficient on impact | — | — | partial | ✓ |
| Indexes once, queries fast | ✓ | ✓ | ✓ | ✓ |
| MCP-native | — | — | — | ✓ |

## What it doesn't do

- No embeddings, no vector search.
- No LLM in the retrieval path.
- No cloud, no telemetry — runs entirely local.
- Not a code-review bot, not a linter, not a refactoring engine. It answers structural questions; downstream tools act on the answers.

## Install

```sh
git clone https://github.com/microvn/GraphAtlas
cd GraphAtlas
cargo install --path .

# Or one-shot installer (downloads release tarball + wires MCP config)
curl -fsSL https://raw.githubusercontent.com/microvn/GraphAtlas/main/install.sh | bash
```

Requires Rust stable (toolchain pinned by `rust-toolchain.toml`) and `cmake` for `lbug`'s embedded graph engine.

## Quickstart

```sh
cd /path/to/your/repo

graphatlas init claude-code   # wire MCP + skill + CLAUDE.md into Claude Code
graphatlas reindex             # build the graph index for this repo
graphatlas doctor              # verify health
```

`init` supports 8 platforms — `claude-code`, `cursor`, `cline`, `codex`, `gemini`, `windsurf`, `continue`, `zed`. Run `graphatlas init` with no args for an interactive picker, or `graphatlas init --all` to wire every detected agent.

`reindex` builds (or rebuilds) the on-disk graph for the current repo — the MCP server then serves queries against it. The agent sees `ga_impact`, `ga_callers`, `ga_minimal_context`, etc. as native tools; no extra prompting needed.

## The 12 tools

| Tool | What it answers |
|------|----------------|
| `ga_impact` | If I change this file/symbol, what breaks? (blast radius) |
| `ga_callers` / `ga_callees` | Who calls this? What does this call? |
| `ga_importers` | Who imports this module? |
| `ga_minimal_context` | Smallest context an LLM needs to safely edit this symbol |
| `ga_rename_safety` | Will renaming this break a caller's signature? |
| `ga_dead_code` | What's unreachable from entry points? |
| `ga_hubs` | High-fan-in files (the "everywhere" files in a codebase) |
| `ga_risk` | Heuristic risk score for touching a symbol |
| `ga_architecture` | Module-level dependency view |
| `ga_bridges` | Cross-module connectors |
| `ga_large_functions` | Functions over a complexity / line budget |
| `ga_file_summary` | Per-file symbol + edge density overview |

Plus `ga_symbols` (lookup), `ga_version`, `ga_query` (raw Cypher).

## Languages

Tree-sitter parsers ship for **Rust, TypeScript, JavaScript, Python, Go, Java, Kotlin, C#, Ruby, PHP** (10 languages). Each has a `LanguageSpec` impl in `crates/ga-parser/src/langs/` covering imports, calls, references, definitions, attributes/decorators, overrides, and re-exports.

Adding a language is a `LanguageSpec` impl plus a tree-sitter dependency.

## Status

Pre-1.0 — API and graph schema may change between minor versions until v1.0.0. See `CHANGELOG.md` for what's released vs in-flight, `CONTRIBUTING.md` for build / test gates, and `crates/` for the workspace breakdown.

Issues and PRs welcome.

## License

Dual-licensed under either [MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE) at your option. Contributions intentionally submitted for inclusion shall be dual-licensed as above, without any additional terms or conditions.
