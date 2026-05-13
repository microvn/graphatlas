//! Per-language `LanguageSpec` implementations.
//!
//! Each sub-module pins its tree-sitter grammar crate and ships the per-lang
//! node-kind checklist. Grammar crate versions are pinned in `Cargo.toml` +
//! Cargo.lock per AS-010 "AST-node-kinds checklist validated against pinned
//! tree-sitter-<lang> grammar SHA".

pub mod csharp;
pub mod go;
pub mod java;
pub mod js;
pub mod kotlin;
pub mod py;
pub mod rs;
pub mod ruby;
pub mod shared;
pub mod ts;
