# Changelog

All notable changes to GraphAtlas will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.2] - 2026-06-03

Extend `ga_architecture` module-dependency authority to C#, PHP, and Ruby —
the last three languages with statically extractable build manifests. Java
and Kotlin remain honest-SKIP (Gradle resolves the dependency graph only at
build-runtime, so no sound static ground truth exists).

### Added
- **C#** — `.csproj` `<ProjectReference>` dependency-graph ground truth +
  `.sln`/`.csproj` module markers. New `ProjectScope` (analogue of
  `CrateScope`) restricts name-fallback resolution to directly-referenced
  projects, cutting cross-project over-link.
- **PHP** — `composer.json` PSR-4 autoload resolution (longest-namespace-prefix
  wins) for `use Ns\Class` imports + `composer.json` module marker.
- **Ruby** — `require` / `require_relative` surfaced as imports (parser); gem
  `lib/` load-path resolution + `.gemspec` module marker.

### Fixed
- `version_flag` smoke test now asserts the live `CARGO_PKG_VERSION` instead of
  a hard-coded literal, closing the drift that followed the 0.1.1 bump.

## [0.1.1] - 2026-05-26

Multi-MCP reindex correctness — fix concurrent reindex bug + recovery
escape hatch. See `docs/specs/graphatlas-v1.5/graphatlas-v1.5-reindex-multi-mcp.md`.

### Added
- `graphatlas reset <repo>` CLI for stuck/corrupt cache recovery. Default
  refuses if the per-repo flock is held by a live process; `--force`
  bypasses the probe (only after operator confirms holder is dead).
  Bench fixture paths refused in both modes.
- `McpContext::refresh_if_stale` invoked at every MCP tool dispatch
  entry (except `ga_reindex`) so long-running readers reopen the lbug
  handle when peer writers bump generation. Bounds inode pinning.
- `Store::open_with_root_and_schema_with_lock` variant that consumes
  a caller-provided exclusive flock — eliminates the drop-then-reacquire
  race window in `reindex_in_place`.
- Multi-process integration tests in `crates/ga-index/tests/` driven by
  new helper binaries `ga_index_lock_holder` and `ga_index_lbug_opener`.
- Spike examples documenting POSIX inode persistence + cross-process
  lbug coexistence on macOS APFS.

### Changed
- `reindex_in_place` now acquires the exclusive flock BEFORE
  `nuke_cache_files` (was after), preventing cache corruption when
  the acquire fails. Peer-held case returns `Ok(read_only_store)` so
  the MCP store cell stays populated.
- `Store::open_read_only` no longer holds a long-lived shared flock —
  cross-process steady state has zero application-level flock held.
  Boot-race against an in-progress initial build now polls with
  exponential backoff (100ms..2s, 30s budget) before returning Err.
- `seal_for_serving` releases the exclusive flock entirely instead of
  downgrading to shared. Post-seal writer holds no flock so peers can
  freely transition to writer for their own reindex.
- `LockFile::downgrade_to_shared` marked `#[deprecated]` (retained as
  potential Windows / NFS fallback API).
- MCP `ga_reindex` handler returns `-32014 ALREADY_REINDEXING` when a
  peer is reindexing; client retries after the peer commits.

### Fixed
- **agentfolk-e1c32d stuck-cache bug (2026-05-25):** suspended writer
  + read-only MCP peer + watcher-triggered reindex left
  `~/.graphatlas/<repo>/lock.pid` orphaned with `graph.db` unlinked
  forever. Root cause: pre-PR6.1 `reindex_in_place` nuked the cache
  before re-acquiring the exclusive lock; the long-lived shared flock
  from the reader blocked any exclusive upgrade. Investigation report:
  `docs/investigate/ga-multi-terminal-reindex-stuck-lock-2026-05-26.md`.
- **Concurrent reindex traps loser forever (2026-05-26):** two MCP
  processes firing `ga_reindex` simultaneously caused the loser's
  `build_index` to run on a read-only Store (lbug refused with "Cannot
  execute write operations in a read-only database"), leaving the
  process unable to reindex. Handler now checks `OpenOutcome::AttachedReadOnly`
  and short-circuits to `ALREADY_REINDEXING` without invoking `build_index`,
  preserving the cell for recovery.

### CI
- New `reset-no-kill` job greps `src/cmd_reset.rs` to enforce the
  no-kill constraint declared in the multi-mcp spec.

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
