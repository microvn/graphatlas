//! AS-003 demonstration artifact: feature-flagged test that always panics.
//! Normal `cargo test` = flag off = no-op. `cargo test --features demo-red` = RED.
//! CI gate proves merge-block — see CONTRIBUTING.md §Demonstrating AS-003.

#[cfg(feature = "demo-red")]
#[test]
fn demo_red_blocks_merge() {
    panic!(
        "AS-003 demo: this test fails on purpose when --features demo-red is enabled. \
         CI will block merge. Remove the feature flag or delete this test to un-red."
    );
}

// Placeholder test so the file compiles even with the flag off — keeps cargo happy.
#[cfg(not(feature = "demo-red"))]
#[test]
fn demo_red_dormant() {
    // No-op — AS-003 demo artifact is dormant without `--features demo-red`.
}
