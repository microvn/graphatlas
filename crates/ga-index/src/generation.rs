//! v1.5 PR4 Staleness Phase B — `graph_generation` atomic bump.
//!
//! Extracted from `store.rs`. Bumps the monotonic counter in the lbug
//! `GraphMeta` table (authoritative) and updates the in-memory mirror;
//! the on-disk metadata.json mirror is written by `Metadata::commit_in_place`.
//!
//! Order of writes (challenge C-2 contract):
//! 1. Compute next = metadata.graph_generation + 1.
//! 2. lbug `MERGE (m:GraphMeta {key: "graph_generation"}) SET m.value = "<next>"`.
//!    If this fails, generation is NOT bumped and the caller's
//!    `commit_in_place` aborts via `?` — partial state never persists.
//! 3. metadata.graph_generation = next (in-memory).
//! 4. Caller writes metadata.json mirror via `Metadata::commit_in_place`.
//!
//! If `db` is `None` (mid-seal transient state), the bump still updates
//! the in-memory counter so the metadata.json mirror reflects it; the
//! lbug write is skipped with a warning.

use crate::metadata::Metadata;
use ga_core::{Error, Result};

pub(crate) fn bump_graph_generation(
    metadata: &mut Metadata,
    db: Option<&lbug::Database>,
) -> Result<()> {
    let next = metadata.graph_generation.saturating_add(1);
    if let Some(db) = db {
        let conn = lbug::Connection::new(db)
            .map_err(|e| Error::Database(format!("bump_graph_generation: open conn: {e}")))?;
        // MERGE = upsert by primary key. The literal substitution is safe
        // because `next` is a u64 with no special characters.
        let q = format!(
            "MERGE (m:GraphMeta {{key: 'graph_generation'}}) SET m.value = '{next}'"
        );
        conn.query(&q)
            .map_err(|e| Error::Database(format!("bump_graph_generation: MERGE: {e}")))?;
    } else {
        eprintln!(
            "warn: bump_graph_generation called with db=None; lbug GraphMeta write skipped \
             (next gen would have been {next})"
        );
    }
    metadata.graph_generation = next;
    Ok(())
}
