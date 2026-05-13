//! Foundation-C16 Phase B — per-file import map builder.
//!
//! Extracted from `indexer.rs` (M-2 refactor) to keep that file under
//! review-threshold as v1.1 continues to add resolver complexity.
//!
//! Map keyed on `(caller_file, imported_local_name)` →
//! `(target_file, original_name)`. Consumed by CALLS resolution between
//! step 1 (same-file) and step 3 (repo-wide fallback).
//!
//! infra:S-002 AS-005: `original_name` differs from `local` for aliased
//! imports (`from X import Foo as F` → local=F, original=Foo). At call
//! site `F()`, callee_name=F; indexer looks up import_map[(file, "F")]
//! = (X.py, "Foo") and resolves via symbol_by_file_name[(X.py, "Foo")].

use crate::import_resolve::{resolve_import_path, PendingImport};
use std::collections::{HashMap, HashSet};

/// Build the Phase B import map from parser-emitted pending imports.
///
/// Each `PendingImport.imported_names` lists the local names the caller
/// file binds. `PendingImport.imported_aliases` lists `(local, original)`
/// pairs for aliased forms. This function folds both into a single map
/// indexer::build_index consumes during CALLS resolution.
///
/// Complexity O(N imports + M imported_names per import). No lbug access.
pub(crate) fn build_import_map(
    pending_imports: &[PendingImport],
    file_paths: &HashSet<String>,
) -> HashMap<(String, String), (String, String)> {
    let mut import_map: HashMap<(String, String), (String, String)> = HashMap::new();
    for pi in pending_imports {
        let Some(dst) = resolve_import_path(&pi.target_path, pi.src_lang, &pi.src_file, file_paths)
        else {
            continue;
        };
        // Build alias lookup once per import so we can resolve original
        // names in O(1) while iterating imported_names.
        let alias_lookup: HashMap<&str, &str> = pi
            .imported_aliases
            .iter()
            .map(|(local, original)| (local.as_str(), original.as_str()))
            .collect();
        for name in &pi.imported_names {
            let original = alias_lookup
                .get(name.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| name.clone());
            import_map
                .entry((pi.src_file.clone(), name.clone()))
                .or_insert_with(|| (dst.clone(), original));
        }
    }
    import_map
}
