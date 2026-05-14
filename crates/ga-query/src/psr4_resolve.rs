//! Composer PSR-4 autoload resolver with path-traversal hardening.
//!
//! Per v1.2-php S-001 AS-017 + Foundation-C16 + Foundation-C14 / THREAT_MODEL
//! AS-031 walker hardening (extended to the resolver layer).
//!
//! ## Security contract
//!
//! `composer.json` lives inside the indexed repo and is attacker-controlled
//! when GA is run against an untrusted source tree. PSR-4 autoload values are
//! filesystem paths; an attacker can write
//! `{"autoload": {"psr-4": {"X\\": "../../../etc/"}}}` to make the resolver
//! mount `/etc/` as a namespace root. Without canonicalization, downstream
//! file reads, symbol indexing, or summary output can leak content from
//! outside the repo.
//!
//! Every caller that reads composer.json MUST route through
//! [`canonicalize_psr4_root`] (single-entry helper) or [`read_composer_psr4`]
//! (composer-aware wrapper). Both reject paths that escape `repo_root` and
//! surface filtered entries as warnings for security audit.
//!
//! ## OWASP A01:2021 — Path Traversal
//!
//! Canonicalization is via `std::fs::canonicalize` which resolves symlinks +
//! `..` segments. The post-canonicalize check (`starts_with(repo_root_canon)`)
//! is the actual escape gate. Both sides must be canonicalized so a
//! tmp-dir-on-mac (`/private/var/...`) doesn't yield false positives.
//!
//! ## Today's wiring status
//!
//! No caller exists yet — Foundation-C16 Phase B reader for composer.json is
//! a future story. This module ships preemptively so the safety primitive
//! exists at the moment the reader lands. If a caller is added without going
//! through this helper, the canary test
//! `crates/ga-parser/tests/php_heredoc_no_phantom_edges.rs`-style approach
//! should be replicated for PSR-4 (TODO: add when reader ships).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug)]
pub enum Psr4ResolveError {
    /// The path is outside `repo_root` after canonicalization.
    EscapesRepoRoot {
        raw: String,
        canonical: PathBuf,
        repo_root: PathBuf,
    },
    /// The path doesn't exist on disk.
    NotFound { raw: String, source: io::Error },
    /// `repo_root` itself couldn't be canonicalized (broken symlink, missing dir).
    RepoRootInvalid {
        repo_root: PathBuf,
        source: io::Error,
    },
    /// composer.json read / parse failure.
    ComposerRead { path: PathBuf, source: io::Error },
    ComposerParse {
        path: PathBuf,
        source: serde_json::Error,
    },
}

impl std::fmt::Display for Psr4ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EscapesRepoRoot {
                raw,
                canonical,
                repo_root,
            } => write!(
                f,
                "psr4_escape: '{raw}' canonicalizes to {} which is outside repo_root {}",
                canonical.display(),
                repo_root.display()
            ),
            Self::NotFound { raw, source } => {
                write!(f, "psr4_not_found: '{raw}': {source}")
            }
            Self::RepoRootInvalid { repo_root, source } => write!(
                f,
                "psr4_repo_root_invalid: {} ({source})",
                repo_root.display()
            ),
            Self::ComposerRead { path, source } => {
                write!(f, "composer_read_failed: {}: {source}", path.display())
            }
            Self::ComposerParse { path, source } => {
                write!(f, "composer_parse_failed: {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for Psr4ResolveError {}

/// Canonicalize a PSR-4 autoload root path against `repo_root` and confirm it
/// does NOT escape.
///
/// Returns the canonicalized absolute path on success. Returns
/// `EscapesRepoRoot` if the path resolves outside `repo_root` (even via
/// symlink), `NotFound` if the path doesn't exist on disk, or
/// `RepoRootInvalid` if `repo_root` itself can't be canonicalized.
///
/// The raw value `raw` is interpreted relative to `repo_root` unless it's
/// absolute. Trailing slashes are tolerated.
pub fn canonicalize_psr4_root(repo_root: &Path, raw: &str) -> Result<PathBuf, Psr4ResolveError> {
    let repo_canon = repo_root
        .canonicalize()
        .map_err(|e| Psr4ResolveError::RepoRootInvalid {
            repo_root: repo_root.to_path_buf(),
            source: e,
        })?;

    let raw_path = Path::new(raw);
    let joined = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        repo_canon.join(raw_path)
    };

    let resolved = joined
        .canonicalize()
        .map_err(|e| Psr4ResolveError::NotFound {
            raw: raw.to_string(),
            source: e,
        })?;

    if !resolved.starts_with(&repo_canon) {
        return Err(Psr4ResolveError::EscapesRepoRoot {
            raw: raw.to_string(),
            canonical: resolved,
            repo_root: repo_canon,
        });
    }

    Ok(resolved)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Psr4Entry {
    pub namespace: String,
    pub canonical_root: PathBuf,
}

#[derive(Debug)]
pub struct ComposerReadResult {
    /// Entries whose paths pass canonicalization + escape check.
    pub entries: Vec<Psr4Entry>,
    /// Diagnostic strings surfacing rejected entries. Format:
    /// `"psr4_escape: <namespace> => <raw_path>: <reason>"`.
    /// Security consumers should log/surface these — silently dropping a
    /// rejected entry hides the attempt.
    pub warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ComposerManifest {
    #[serde(default)]
    autoload: Option<ComposerAutoload>,
}

#[derive(Debug, Deserialize)]
struct ComposerAutoload {
    #[serde(rename = "psr-4", default)]
    psr_4: std::collections::BTreeMap<String, ComposerPsr4Value>,
}

/// composer.json `psr-4` value may be a single string OR an array of strings
/// (for multi-root namespaces). We accept either.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ComposerPsr4Value {
    Single(String),
    Multi(Vec<String>),
}

impl ComposerPsr4Value {
    fn into_paths(self) -> Vec<String> {
        match self {
            Self::Single(s) => vec![s],
            Self::Multi(v) => v,
        }
    }
}

/// Read `composer.json`, parse `autoload.psr-4`, canonicalize each entry, and
/// return safe entries + warnings for rejected ones.
///
/// `repo_root` is the trust boundary — entries whose canonical path escapes
/// it are dropped (NOT returned in `entries`) and reported as warnings.
pub fn read_composer_psr4(
    composer_path: &Path,
    repo_root: &Path,
) -> Result<ComposerReadResult, Psr4ResolveError> {
    let raw = fs::read_to_string(composer_path).map_err(|e| Psr4ResolveError::ComposerRead {
        path: composer_path.to_path_buf(),
        source: e,
    })?;

    let manifest: ComposerManifest =
        serde_json::from_str(&raw).map_err(|e| Psr4ResolveError::ComposerParse {
            path: composer_path.to_path_buf(),
            source: e,
        })?;

    let mut entries = Vec::new();
    let mut warnings = Vec::new();

    let Some(autoload) = manifest.autoload else {
        return Ok(ComposerReadResult { entries, warnings });
    };

    for (namespace, value) in autoload.psr_4 {
        for raw_path in value.into_paths() {
            match canonicalize_psr4_root(repo_root, &raw_path) {
                Ok(canonical_root) => entries.push(Psr4Entry {
                    namespace: namespace.clone(),
                    canonical_root,
                }),
                Err(Psr4ResolveError::EscapesRepoRoot { canonical, .. }) => {
                    warnings.push(format!(
                        "psr4_escape: {namespace} => {raw_path} (canonicalizes to {} — outside repo_root)",
                        canonical.display()
                    ));
                }
                Err(Psr4ResolveError::NotFound { .. }) => {
                    warnings.push(format!("psr4_not_found: {namespace} => {raw_path}"));
                }
                Err(other) => return Err(other),
            }
        }
    }

    Ok(ComposerReadResult { entries, warnings })
}
