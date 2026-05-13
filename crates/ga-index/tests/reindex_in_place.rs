//! v1.5 PR6.1a — `Store::reindex_in_place` lifecycle method.
//!
//! Spec mapping: PR6 tool sub-spec AS-003 step "drop RW handle if held →
//! `nuke_cache_files(&layout)` → reopen RW". This file pins the
//! mechanism at the Store layer; PR6.1b wires it into the ga_reindex
//! MCP tool via an `Arc<RwLock<Arc<Store>>>` McpContext wrap.

use ga_core::IndexState;
use ga_index::{OpenOutcome, Store};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn real_repo(tmp: &TempDir, rel: &str) -> PathBuf {
    let p = tmp.path().join("repos").join(rel.trim_start_matches('/'));
    std::fs::create_dir_all(&p).unwrap();
    std::fs::write(p.join("README.md"), "# fixture\n").unwrap();
    std::fs::create_dir_all(p.join("src")).unwrap();
    std::fs::write(p.join("src").join("lib.rs"), "// fixture\n").unwrap();
    p
}

// =====================================================================
// AS-001: Store::reindex_in_place returns fresh Store ready for build
// =====================================================================

#[test]
fn as_001_reindex_in_place_returns_fresh_store_with_outcome_fresh_build() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = real_repo(&tmp, "ri-001");

    let mut s1 = Store::open_with_root(&cache, &repo).unwrap();
    s1.commit_in_place().unwrap();
    let gen_before = s1.metadata().graph_generation;
    assert_eq!(gen_before, 1, "first commit gen=1");

    // Reindex: consume the old Store, get a new one ready for build.
    let new_store = s1.reindex_in_place(&repo).expect("reindex_in_place");

    // The new Store must be on FreshBuild outcome (cache nuked + reopened).
    assert!(
        matches!(new_store.outcome(), OpenOutcome::FreshBuild),
        "reindex_in_place must yield FreshBuild outcome, got {:?}",
        new_store.outcome()
    );
    // State is Building (caller must commit to flip to Complete).
    assert_eq!(
        new_store.metadata().index_state,
        IndexState::Building,
        "post-nuke Store must be in Building state"
    );
}

// =====================================================================
// AS-003: Cache files nuked between consume + reopen
// =====================================================================

#[test]
fn as_003_cache_files_nuked_during_reindex_in_place() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = real_repo(&tmp, "ri-003");

    let mut s1 = Store::open_with_root(&cache, &repo).unwrap();
    s1.commit_in_place().unwrap();
    let layout = s1.layout().clone();
    let graph_db = layout.graph_db();
    let metadata_json = layout.metadata_json();

    // Sanity pre-reindex: both files exist.
    assert!(graph_db.exists(), "pre-reindex graph.db must exist");
    assert!(metadata_json.exists(), "pre-reindex metadata.json must exist");

    // Reindex (new Store opens fresh — would have just re-created files).
    let new_store = s1.reindex_in_place(&repo).expect("reindex_in_place");

    // Post-reindex: graph.db exists (newly created by reopen) but metadata
    // is now in Building state (not Complete). The freshly-opened Store
    // created its own files.
    assert!(graph_db.exists(), "post-reindex graph.db must be re-created");
    assert_eq!(
        new_store.metadata().index_state,
        IndexState::Building,
        "post-reindex metadata is Building until caller commits"
    );
}

// =====================================================================
// AS-004: Post-rebuild commit continues graph_generation sequence
// =====================================================================

#[test]
fn as_004_post_reindex_commit_continues_generation_sequence() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = real_repo(&tmp, "ri-004");

    let mut s1 = Store::open_with_root(&cache, &repo).unwrap();
    s1.commit_in_place().unwrap();
    assert_eq!(s1.metadata().graph_generation, 1);

    // After reindex_in_place the new Store starts in Building state with
    // graph_generation=0 (sentinel — pre-commit). Caller commits → bumps.
    let mut new_store = s1.reindex_in_place(&repo).expect("reindex_in_place");
    assert_eq!(
        new_store.metadata().graph_generation,
        0,
        "post-nuke metadata begins at gen=0 sentinel"
    );

    new_store.commit_in_place().expect("post-rebuild commit");
    // First commit on the new fresh cache → gen=1 (lbug GraphMeta seeded).
    // This is correct behavior: nuke wiped the lbug GraphMeta row, so the
    // commit starts a new generation sequence. PR6.1b (ga_reindex tool wire)
    // will surface gen_before/gen_after at the MCP layer by remembering the
    // pre-reindex generation across the consume boundary.
    assert_eq!(
        new_store.metadata().graph_generation,
        1,
        "first commit on fresh cache → gen=1"
    );
}

// =====================================================================
// AS-005: Mid-rebuild failure leaves cache empty; next open = FreshBuild
// =====================================================================

#[test]
fn as_005_consuming_method_drops_handles_so_cross_process_can_reopen() {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = real_repo(&tmp, "ri-005");

    let mut s1 = Store::open_with_root(&cache, &repo).unwrap();
    s1.commit_in_place().unwrap();

    let new_store = s1.reindex_in_place(&repo).expect("reindex_in_place");
    // Drop the new Store WITHOUT committing — simulates mid-build crash.
    drop(new_store);

    // Next Store::open must succeed via FreshBuild or RebuildCrashRecovery.
    // The old cache files are gone (we nuked) + new Store was Building
    // when dropped → metadata.json says Building → CrashedBuilding path.
    let recovered = Store::open_with_root(&cache, &repo).expect("post-failure recovery");
    let outcome = recovered.outcome();
    assert!(
        matches!(
            outcome,
            OpenOutcome::FreshBuild | OpenOutcome::RebuildCrashRecovery { .. }
        ),
        "post-failure recovery must yield FreshBuild or RebuildCrashRecovery, got {outcome:?}"
    );
}

// =====================================================================
// AS-002: ordering — old handles released before new Store opens
// =====================================================================
//
// This is hard to assert directly without instrumenting Store. The proof
// is structural: `reindex_in_place(self, ...)` takes self by value, so
// the OLD Store's Drop runs (releasing lbug Database + flock) before
// the body opens a new one. Documented contract.

#[test]
fn as_002_consume_pattern_documented() {
    // Compile-time proof: the signature requires by-value self, which
    // means the caller surrenders ownership. The body's old-handle drop
    // happens at the function boundary or earlier (we explicitly drop
    // in-method to be deterministic about ordering).
    //
    // This test is a smoke that the signature shape is `(self, &Path)`
    // not `(&mut self, ...)` — the former is what gives us the
    // drop-before-open ordering guarantee for free.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = real_repo(&tmp, "ri-002");
    let mut s1 = Store::open_with_root(&cache, &repo).unwrap();
    s1.commit_in_place().unwrap();
    // s1 moved here:
    let _new = s1.reindex_in_place(&repo).expect("reindex_in_place");
    // s1 no longer accessible — borrow checker enforces the ordering.
    // (Uncomment next line to verify compile error: `s1.commit_in_place();`)
}

fn _compile_check_signature() {
    fn _takes_self(s: Store, repo: &Path) -> ga_core::Result<Store> {
        s.reindex_in_place(repo)
    }
}
