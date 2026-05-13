#!/usr/bin/env bash
# v1.5 Tier B — subprocess smoke test for the reindex lifecycle.
#
# Thin wrapper: the real work lives in
# `tests/smoke_reindex_subprocess.rs::smoke_reindex_full_lifecycle`.
# This script just invokes it with the right flags so the user (or CI)
# has a single command to run.
#
# The Rust test boots `target/release/graphatlas mcp` as a child
# process, drives newline-delimited JSON-RPC over its stdin/stdout,
# and asserts:
#   - initialize handshake (protocolVersion = 2025-11-25)
#   - tools/list includes ga_reindex
#   - baseline ga_callers succeeds
#   - edit a tracked file → next ga_callers returns STALE_INDEX (-32010)
#   - ga_reindex full returns `reindexed: true` + graph_generation_after
#   - post-reindex ga_callers is fresh
#   - external `git commit` triggers the L1 watcher, stderr contains
#     "L1 watcher: reindex complete" within 30s
#
# Run locally:
#   bash scripts/smoke-reindex.sh
#
# CI: this script runs in `.github/workflows/ci.yml::lbug-empirical`
# matrix (ubuntu + macos) so regressions cross-platform are caught
# before merge.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

echo "smoke-reindex.sh — driving tests/smoke_reindex_subprocess.rs"
echo
exec cargo test --release \
    --test smoke_reindex_subprocess \
    -- --ignored --nocapture --test-threads=1
