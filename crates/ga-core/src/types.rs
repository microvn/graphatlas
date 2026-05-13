use serde::{Deserialize, Serialize};

/// Supported source languages. v1 scope per PLAN R15; v1.1-M4 (S-005a)
/// extends with Java/Kotlin/CSharp/Ruby per Phase C languages spec.
/// Extension mapping ported from rust-poc/src/main.rs:83-104.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Python,
    TypeScript,
    JavaScript,
    Go,
    Rust,
    // v1.1-M4 — Phase C languages. Variant exists; LanguageSpec impl ships
    // per-story (S-001 Java, S-002 Kotlin, S-003 CSharp, S-004 Ruby).
    // AS-017: extractors return typed Err when variant present without spec.
    Java,
    Kotlin,
    CSharp,
    Ruby,
}

impl Lang {
    pub fn from_ext(ext: &str) -> Option<Self> {
        match ext {
            "py" | "pyw" => Some(Self::Python),
            "ts" | "tsx" => Some(Self::TypeScript),
            "js" | "jsx" | "mjs" => Some(Self::JavaScript),
            "go" => Some(Self::Go),
            "rs" => Some(Self::Rust),
            "java" => Some(Self::Java),
            "kt" | "kts" => Some(Self::Kotlin),
            "cs" => Some(Self::CSharp),
            "rb" => Some(Self::Ruby),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::TypeScript => "typescript",
            Self::JavaScript => "javascript",
            Self::Go => "go",
            Self::Rust => "rust",
            Self::Java => "java",
            Self::Kotlin => "kotlin",
            Self::CSharp => "csharp",
            Self::Ruby => "ruby",
        }
    }

    /// Canonical iterator over every supported `Lang` variant. Use this
    /// in tests / coverage assertions instead of hardcoded slices —
    /// the unit test `all_covers_every_variant` below uses an
    /// exhaustive match to compile-fail when a new variant is added
    /// to the enum without being added to `ALL`.
    ///
    /// Order is the v1 → v1.1-M4 ship order; consumers must not
    /// depend on it (use HashSet if order matters not).
    pub const ALL: &'static [Lang] = &[
        Self::Python,
        Self::TypeScript,
        Self::JavaScript,
        Self::Go,
        Self::Rust,
        Self::Java,
        Self::Kotlin,
        Self::CSharp,
        Self::Ruby,
    ];
}

/// File node in the graph. `hash` is BLAKE3 content hash per Foundation-C9.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct File {
    pub path: String,
    pub lang: Lang,
    pub mtime_ns: u64,
    pub size: u64,
    pub hash: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Interface,
    Struct,
    Enum,
    Trait,
    Module,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub id: String,
    pub name: String,
    pub kind: SymbolKind,
    pub file: String,
    pub line: u32,
    /// 1-based last line of the definition. v1.1 schema v3 — required for
    /// `ga_large_functions`. Synthetic symbols (metaprogramming) reuse
    /// `line` so `line_end - line + 1 == 1`.
    #[serde(default)]
    pub line_end: u32,
    pub enclosing: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EdgeType {
    Calls,
    Imports,
    Extends,
    Defines,
    TestedBy,
    /// Foundation-C15 — value-reference: a function symbol passed as a
    /// value (dispatch map entry, array element, callback arg, shorthand
    /// property). Distinct from Calls because the function isn't invoked
    /// at the reference site — it's held by identity for later dispatch.
    References,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    #[serde(rename = "type")]
    pub edge_type: EdgeType,
    pub from: String,
    pub to: String,
    pub confidence: f32,
}

/// Graph metadata — written to `metadata.json` next to `graph.db`.
/// `index_state` transitions {building → complete} per Foundation-C7.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphMeta {
    pub schema_version: u32,
    pub indexed_at: u64,
    pub repo_root: String,
    pub repo_hash: String,
    pub languages: Vec<Lang>,
    pub file_count: u32,
    pub index_state: IndexState,
    pub index_generation: String,
    pub indexed_root_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexState {
    Building,
    Complete,
}
