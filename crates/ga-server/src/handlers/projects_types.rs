//! Response shapes for the projects API (S-002 + S-003 forward compat).
//!
//! Kept in a separate module so handler logic and DTOs don't grow into
//! a single ~500-line file. `index_counts` + `health` are Optional —
//! S-003 migration is what populates them.

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ProjectRow {
    pub slug: String,
    pub name: String,
    pub repo_root: String,
    pub languages: Vec<LangCount>,
    pub last_indexed_unix: u64,
    pub index_state: ProjectIndexState,
    pub index_counts: Option<IndexCounts>,
    pub health: Option<HealthSummary>,
    pub watcher: WatcherStatus,
    pub watcher_queue_pending: u64,
    pub watcher_last_event_unix: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct LangCount {
    pub lang: String,
    pub file_count: u64,
}

/// Extended state — `ga_core::IndexState` only has Building/Complete;
/// Spec A adds Orphan (path missing), Corrupt (set by S-005 cancel),
/// Stale (set by S-006 staleness check).
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum ProjectIndexState {
    Fresh,
    Building,
    Orphan,
    #[allow(dead_code)]
    Corrupt,
    #[allow(dead_code)]
    Stale,
}

#[derive(Debug, Serialize)]
pub struct IndexCounts {
    pub node_count: u64,
    pub edge_count: u64,
    pub file_count: u64,
    pub last_index_duration_ms: u64,
    pub db_size_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct HealthSummary {
    pub computed_at_unix: u64,
    pub hubs_count: u64,
    pub bridges_count: u64,
    pub dead_code_count: u64,
    pub large_functions_count: u64,
    pub tested_count: u64,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum WatcherStatus {
    Running,
    Stopped,
    Errored,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
}
