# Contributing to GraphAtlas

Solo-dev OSS project. External contributions welcome once v1 ships.

## Dev setup

Requirements:
- Rust stable toolchain (rustup installs the pinned version from `rust-toolchain.toml`)
- `cmake` (lbug 0.15 builds a C++ graph engine via cmake-rs)
  - macOS: `brew install cmake`
  - Linux: `sudo apt-get install cmake` or distro equivalent

Build + test:

```sh
cargo build --release
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/check-no-placeholders.sh
```

All five must pass locally before pushing a PR — CI enforces the same gates.

## Workspace layout

```
Cargo.toml              # workspace root + thin binary
src/main.rs             # CLI dispatcher (8 subcommands per Foundation-C6)
crates/
  ga-core              # shared types + error enum (zero runtime deps beyond std+serde)
  ga-parser            # LanguageSpec trait + per-lang impls (bodies in S-004)
  ga-index             # lbug 0.15 wrapper (bodies in S-003)
  ga-query             # 6 tool impls (bodies in Tools spec)
  ga-mcp               # hand-rolled JSON-RPC MCP server (bodies in S-006; rmcp swap v1.1)
```

Per Foundation-C2, `ga-core` must stay dep-minimal. New crates should
reference workspace-level dependencies via `dep = { workspace = true }`.

## Licensing

Dual-licensed: MIT OR Apache-2.0 (Foundation-C10). Contributions must be
acceptable under both.
