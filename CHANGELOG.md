# Changelog

All notable changes to GraphAtlas will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-05-21

Initial public release. Pre-1.0 alpha — API may change between minor
versions.

### Added
- Rust workspace: `ga-core`, `ga-parser`, `ga-index`, `ga-query`,
  `ga-mcp`, `ga-bench`, `ga-server`.
- CLI binary `graphatlas` with subcommands: `init`, `reindex`, `query`,
  `mcp`, `doctor`, `install`, `ui`, `list`, `bench`, `cache`.
- MCP server exposing graph-query tools (`ga_impact`, `ga_callers`,
  `ga_callees`, `ga_importers`, `ga_minimal_context`, `ga_rename_safety`,
  `ga_dead_code`, `ga_hubs`, `ga_risk`, `ga_architecture`, `ga_bridges`,
  `ga_large_functions`, `ga_file_summary`, `ga_symbols`) to AI coding
  agents.
- Per-language tree-sitter parsers covering Rust, TypeScript,
  JavaScript, Python, Go, Java, Kotlin, C#, Ruby, PHP.
- `init` wires MCP config + skill files for Claude Code, Cursor, Cline,
  Codex, Gemini, Windsurf, Continue, Zed.
- LadybugDB-backed graph index (file + symbol nodes, DEFINES / IMPORTS
  / IMPORTS_NAMED / CALLS / REFERENCES / OVERRIDES edges).
- `ga-server` + `graphatlas ui` — local web dashboard for projects list,
  graph canvas (Sigma + ForceAtlas2), live reindex progress, native
  file watcher (FSEvents/inotify/RDCW + polling fallback), 2-step cache
  delete confirm.
- Bench harness with M1 (callers/importers), M2 (impact, git-mined GT)
  and M3 (dead_code, hubs, rename_safety, minimal_context) gates.
- Supply-chain CI gate via `cargo audit` + `cargo deny`.
- Dual MIT / Apache-2.0 licensing.
