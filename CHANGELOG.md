# Changelog

All notable changes to GraphAtlas will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - TBD

Initial public release. Pre-1.0 alpha — API may change between minor
versions.

### Added
- Rust workspace: `ga-core`, `ga-parser`, `ga-index`, `ga-query`,
  `ga-mcp`, `ga-bench`.
- CLI binary `graphatlas` with subcommands: `index`, `query`, `mcp`,
  `init`, `doctor`, `install`, `list`, `bench`, `update`, `cache`.
- MCP server (hand-rolled JSON-RPC) exposing graph-query tools to AI
  coding agents.
- Per-language tree-sitter parsers covering Rust, TypeScript, Python,
  Go, Java, Kotlin, Ruby, C, JavaScript.
- LadybugDB-backed graph index (file + symbol nodes, DEFINES / IMPORTS
  / IMPORTS_NAMED / CALLS / REFERENCES / OVERRIDES edges).
- Bench harness with M1 (callers/importers), M2 (impact, git-mined GT)
  and M3 (dead_code, hubs, rename_safety, minimal_context) gates.
- Dual MIT / Apache-2.0 licensing.
