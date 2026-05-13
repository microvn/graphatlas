## Summary

<!-- One paragraph: what changes, why. Skip if the title says it. -->

## Test plan

<!-- How you verified this works. Commands run, scenarios covered. -->

- [ ] `cargo build --release` clean
- [ ] `cargo test --workspace` pass
- [ ] `cargo fmt --check` clean
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [ ] `bash scripts/check-no-placeholders.sh` clean

## Notes for reviewers

<!-- Trade-offs, follow-ups, anything not obvious from the diff. Delete if none. -->
