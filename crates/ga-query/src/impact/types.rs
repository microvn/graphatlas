//! Wire-format types for `ga_impact`. Serde-shaped structs + typed enums.

use serde::{Deserialize, Serialize};

/// AS-016 — default traversal depth when the caller does not set `max_depth`.
pub(super) const DEFAULT_MAX_DEPTH: u32 = 3;

/// Input to `ga_impact`. At least one of `symbol`, `changed_files`, or `diff`
/// must be set — validated in cluster C1 (AS-015). `file` narrows symbol
/// resolution (Tools-C11 hint). `max_depth` caps transitive BFS (default 3).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImpactRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed_files: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<u32>,

    // EXP-M2-02 — opt-out flags for expensive subcomponents that don't
    // contribute to composite bench score. Defaults preserve backward
    // compatibility (treat `None` as "include").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_break_points: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_routes: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_configs: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_risk: Option<bool>,
    /// EXP-M2-11 — co-change + importers intersection (mimics GT
    /// `should_touch_files` Phase C per `extract-seeds.ts:491-538`). Lifts
    /// `blast_radius_coverage` by +38% (0.451→0.624) but blows p95 latency
    /// 4× (422ms→1867ms) due to git-subprocess fan-out. **Default: false**
    /// — opt-in only for callers that can absorb latency for blast-radius
    /// completeness (e.g. LLM-agent reviews, not fast-path MCP queries).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_co_change_importers: Option<bool>,
}

impl ImpactRequest {
    /// `true` iff the flag is unset or explicitly `true`. Used by dispatch
    /// to decide whether to run the corresponding subcomponent.
    pub(crate) fn wants_break_points(&self) -> bool {
        self.include_break_points.unwrap_or(true)
    }
    pub(crate) fn wants_routes(&self) -> bool {
        self.include_routes.unwrap_or(true)
    }
    pub(crate) fn wants_configs(&self) -> bool {
        self.include_configs.unwrap_or(true)
    }
    pub(crate) fn wants_risk(&self) -> bool {
        self.include_risk.unwrap_or(true)
    }
    pub(crate) fn wants_co_change_importers(&self) -> bool {
        // EXP-M2-11 default false: +blast_radius gain but p95 4× latency cost.
        // Opt-in via Some(true) for callers that tolerate git-subprocess cost.
        self.include_co_change_importers.unwrap_or(false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

impl Default for RiskLevel {
    fn default() -> Self {
        RiskLevel::Low
    }
}

/// 4-dim composite risk. Populated in cluster C7.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Risk {
    pub score: f32,
    pub level: RiskLevel,
    pub reasons: Vec<String>,
}

/// Which traversal step surfaced a file into `impacted_files`.
///
/// Serialized as lowercase string (`"seed"` | `"caller"` | `"callee"`) so the
/// wire format stays LLM-ergonomic. `#[non_exhaustive]` because later
/// clusters (C5 routes, C8 multi-file) may add variants like `"route"` or
/// `"changed"` — external matches must use a wildcard arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum ImpactReason {
    /// File is where the seed symbol is defined (depth 0).
    Seed,
    /// File reached through an incoming CALLS / REFERENCES edge (someone
    /// calls / references a symbol the BFS frontier currently holds).
    Caller,
    /// File reached through an outgoing CALLS / REFERENCES edge (the
    /// frontier symbol calls / references something defined there).
    Callee,
    /// EXP-M2-11 — surfaced via co-change + importer intersection (mimics
    /// GT `should_touch_files` derivation). Structural blast radius, not
    /// graph-reachable from seed via CALLS/REFERENCES.
    CoChange,
}

/// A file reached during BFS from the seed symbols. `depth` is min hops from
/// any seed; `reason` says which edge class surfaced it.
///
/// Self-explaining fields (additive — bench reads only `path`):
/// - `confidence` (Tools-C11): 1.0 when seed has a single definition or when
///   `file` hint matches; 0.6 when same-named definition exists in another
///   file (polymorphic blast radius).
/// - `relation_to_seed`: short universal English token an LLM consumer can
///   read without GA's spec context. Vocabulary: `"changed_directly"`,
///   `"shares_function_name"`, `"calls_seed_directly"`,
///   `"called_by_seed_directly"`, `"shared_dependency"`,
///   `"sibling_in_same_class"`, `"co_changes_with_seed"`.
/// - `explanation`: one short sentence in plain English for human/LLM
///   readers. Never references GA-internal taxonomy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactedFile {
    pub path: String,
    pub depth: u32,
    pub reason: ImpactReason,
    #[serde(default = "default_confidence")]
    pub confidence: f32,
    #[serde(default)]
    pub relation_to_seed: String,
    #[serde(default)]
    pub explanation: String,
}

fn default_confidence() -> f32 {
    1.0
}

impl Default for ImpactedFile {
    fn default() -> Self {
        Self {
            path: String::new(),
            depth: 0,
            reason: ImpactReason::Seed,
            confidence: default_confidence(),
            relation_to_seed: String::new(),
            explanation: String::new(),
        }
    }
}

/// How a test file was surfaced as affected.
///
/// `#[non_exhaustive]` — future indexer work may add variants (e.g. `Coverage`
/// when a coverage map is available) without breaking external match arms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum AffectedTestReason {
    /// Reached via a `TESTED_BY` rel in the graph (highest confidence).
    Edge,
    /// Matched a per-language test-naming convention plus a path-relevance
    /// heuristic (seed name or seed file stem appears in the test path).
    Convention,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectedTest {
    pub path: String,
    pub reason: AffectedTestReason,
}

/// AS-014 — framework-detected route mount.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectedRoute {
    pub method: String,
    pub path: String,
    pub source_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectedConfig {
    pub path: String,
    pub line: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreakPoint {
    pub file: String,
    pub line: u32,
    pub caller_symbols: Vec<String>,
}

/// Tools-C5 truncation summary — one (was-capped, total-available) row per
/// capped list in the response. Empty when nothing was truncated.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TruncationMeta {
    #[serde(default, skip_serializing_if = "is_false")]
    pub impacted_files: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub affected_tests: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub affected_routes: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub affected_configs: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub break_points: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TotalAvailable {
    pub impacted_files: u32,
    pub affected_tests: u32,
    pub affected_routes: u32,
    pub affected_configs: u32,
    pub break_points: u32,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Tools-C1 + AS-016 + Tools-C5/C10 — response metadata.
///
/// - `transitive_completeness` reports the max depth actually reached by BFS.
/// - `max_depth` reports the configured cap.
/// - `truncated` / `total_available` expose Tools-C5 output-cap info per list.
/// - `warning` is Tools-C10 — set when any surfaced path comes from a
///   vendored / excluded directory (LLM clients should flag / quarantine).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImpactMeta {
    pub transitive_completeness: u32,
    pub max_depth: u32,
    #[serde(default, skip_serializing_if = "TruncationMeta::is_empty")]
    pub truncated: TruncationMeta,
    #[serde(default, skip_serializing_if = "TotalAvailable::is_zero")]
    pub total_available: TotalAvailable,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

impl TruncationMeta {
    fn is_empty(&self) -> bool {
        !self.impacted_files
            && !self.affected_tests
            && !self.affected_routes
            && !self.affected_configs
            && !self.break_points
    }
}

impl TotalAvailable {
    fn is_zero(&self) -> bool {
        self.impacted_files == 0
            && self.affected_tests == 0
            && self.affected_routes == 0
            && self.affected_configs == 0
            && self.break_points == 0
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImpactResponse {
    pub impacted_files: Vec<ImpactedFile>,
    pub affected_tests: Vec<AffectedTest>,
    pub affected_routes: Vec<AffectedRoute>,
    pub affected_configs: Vec<AffectedConfig>,
    pub risk: Risk,
    pub break_points: Vec<BreakPoint>,
    pub meta: ImpactMeta,
}
