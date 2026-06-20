//! Regression: `graphatlas reindex -- --full` — docs/investigate/graphatlas-reindex-full-flag-2026-06-19.md
//!
//! The `reindex` subcommand has no `--full` flag (reindex is always a full
//! rebuild). When invoked as `reindex -- --full`, clap's `--` end-of-options
//! marker binds `--full` to the positional `repo` PATH arg. Pre-fix,
//! `cmd_reindex` passed the literal "--full" straight to the indexer, which
//! reported the misleading "config corrupt at --full: repo_root does not exist"
//! — making a fat-fingered flag look like a corrupt cache.
//!
//! Post-fix: a flag-shaped positional (leading '-') is rejected up front with a
//! clear message that does NOT mention "config corrupt".

use graphatlas::cmd_reindex::cmd_reindex;
use std::path::PathBuf;

#[test]
fn reindex_rejects_flag_shaped_positional_with_clear_message() {
    // Regression: `reindex -- --full` — cmd_reindex.rs bound "--full" as repo path.
    let err = cmd_reindex(Some(PathBuf::from("--full")), false)
        .expect_err("a flag-shaped repo arg must be rejected, not treated as a path");
    let msg = format!("{err:#}");

    // Must NOT surface the misleading cache-corruption category.
    assert!(
        !msg.contains("config corrupt"),
        "flag-shaped arg must not be reported as cache corruption; got: {msg}"
    );
    // Must clearly tell the user `--full` is not a flag / point at the real usage.
    assert!(
        msg.contains("--full") && (msg.contains("not a flag") || msg.contains("always a full")),
        "error must explain `--full` is not a flag (reindex is always full); got: {msg}"
    );
}
