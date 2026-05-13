//! infra:S-004 — class-method seed resolution (v1.1-M0).
//!
//! `ga_impact {symbol: "User.set_password"}` previously degenerated to
//! `WHERE s.name = "User.set_password"` which never matched anything,
//! because `Symbol.name` stores the unqualified method name and the
//! class-to-method relationship lives on `CONTAINS` edges (per KG-9).
//!
//! Resolver logic (per Infra-C5):
//! 1. Exact match — `WHERE s.name = <raw>` (for unusual langs that do
//!    store qualified names; currently none in v1, but preserved for
//!    future-lang compatibility).
//! 2. Split on `::` (Rust) or `.` (Python/TS): `Router::new` → enclosing
//!    `Router` + method `new`; `User.set_password` → enclosing `User` +
//!    method `set_password`. Last-2 semantic for multi-dot names:
//!    `a.b.c` → enclosing `b`, method `c`.
//! 3. Query `MATCH (cls)-[:CONTAINS]->(m) WHERE cls.name = enclosing AND
//!    m.name = method RETURN m.file LIMIT 1` — returns unqualified `last`
//!    plus the file where it lives (becomes file_hint for downstream).
//! 4. No match → `Error::InvalidParams` with top-3 Levenshtein suggestions
//!    on `Symbol.name` values — matches AS-014 spec.

use crate::common;
use ga_core::{Error, Result};
use ga_index::Store;

/// Outcome of seed resolution — passed downstream to BFS + break_points +
/// affected_tests + routes + configs. All downstream Cypher still matches
/// on unqualified `name` only, so the caller swaps `raw_symbol` for
/// `resolved.name` before proceeding.
pub(super) struct ResolvedSeed {
    pub name: String,
    #[allow(dead_code)] // reserved for Tools-C11 file-hint narrowing, used
    // when the raw seed was qualified — see fill_from_symbol.
    pub file_hint: Option<String>,
}

/// Resolve a possibly-qualified seed symbol into an unqualified name + file
/// hint.
///
/// Return values:
/// - `Ok(Some(ResolvedSeed))` — resolved; caller proceeds with downstream
///   queries using `resolved.name`.
/// - `Ok(None)` — short-circuit per Tools-C9-d: raw symbol contains
///   disallowed characters (e.g. `fo'o`) AND has no qualified separator.
///   Caller returns empty `ImpactResponse` silently, preserving legacy
///   allowlist behavior.
/// - `Err(InvalidParams)` — qualified seed (contains `.` or `::`) that
///   could not be resolved via CONTAINS traversal. Error message includes
///   top-3 Levenshtein suggestions per AS-014. Distinguishes "seed not
///   found" from "seed found, no impact".
pub(super) fn resolve_seed(
    store: &Store,
    raw: &str,
    file_hint: Option<&str>,
) -> Result<Option<ResolvedSeed>> {
    // Fast path: unqualified. Preserve Tools-C9-d silent-empty contract —
    // non-ident input (e.g. `fo'o`) short-circuits to `Ok(None)`, matching
    // pre-S-004 behavior. Only qualified seeds get the strict error path.
    if !raw.contains("::") && !raw.contains('.') {
        if !common::is_safe_ident(raw) {
            return Ok(None);
        }
        return Ok(Some(ResolvedSeed {
            name: raw.to_string(),
            file_hint: file_hint.map(String::from),
        }));
    }

    // Qualified split — prefer `::` (Rust) over `.` (Python/TS); if both
    // present, use whichever is right-most so we get the last segment as
    // the method name.
    let (enclosing, method) = split_qualified(raw);

    // M-3 review (2026-04-24): well-known roots (crate, self, super, this,
    // cls) are NOT class/module symbols — they're scope markers in Rust
    // and Python. Treat qualified names with these as enclosing as
    // unqualified lookups on the last segment; avoids a pointless
    // CONTAINS traversal that would always miss.
    if is_well_known_root(enclosing) {
        if !common::is_safe_ident(method) {
            return Err(Error::InvalidParams(format!(
                "Qualified symbol method part contains disallowed characters: {raw}"
            )));
        }
        return Ok(Some(ResolvedSeed {
            name: method.to_string(),
            file_hint: file_hint.map(String::from),
        }));
    }

    // Guard each segment against Cypher injection + weird chars before
    // interpolating into the query string. `is_safe_ident` allows `.` for
    // ADT-style names — but for the split parts we want tighter check: no
    // separator chars should survive here.
    if enclosing.contains("::")
        || enclosing.contains('.')
        || method.contains("::")
        || method.contains('.')
    {
        return Err(Error::InvalidParams(format!(
            "Qualified symbol parts still contain separators after split: {raw}"
        )));
    }
    if !common::is_safe_ident(enclosing) || !common::is_safe_ident(method) {
        return Err(Error::InvalidParams(format!(
            "Qualified symbol parts contain disallowed characters: {raw}"
        )));
    }

    // CONTAINS traversal: class `enclosing` contains method `method`.
    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("seed resolve connection: {e}")))?;
    let cypher = format!(
        "MATCH (cls:Symbol)-[:CONTAINS]->(m:Symbol) \
         WHERE cls.name = '{enclosing}' AND m.name = '{method}' AND m.kind <> 'external' \
         RETURN m.name, m.file LIMIT 1"
    );
    let rs = conn
        .query(&cypher)
        .map_err(|e| Error::Other(anyhow::anyhow!("seed resolve query: {e}")))?;
    for row in rs {
        let cols: Vec<lbug::Value> = row.into_iter().collect();
        if let (Some(lbug::Value::String(name)), Some(lbug::Value::String(file))) =
            (cols.first(), cols.get(1))
        {
            return Ok(Some(ResolvedSeed {
                name: name.clone(),
                file_hint: Some(file.clone()),
            }));
        }
    }

    // Fallback — maybe the last segment alone exists even without the
    // enclosing class matching. Only honor when caller supplied `file_hint`
    // (Tools-C11 polymorphic confidence pattern).
    if file_hint.is_some() {
        let fallback = format!(
            "MATCH (s:Symbol) WHERE s.name = '{method}' AND s.kind <> 'external' \
             RETURN s.file LIMIT 1"
        );
        let rs = conn
            .query(&fallback)
            .map_err(|e| Error::Other(anyhow::anyhow!("seed fallback query: {e}")))?;
        for row in rs {
            if let Some(lbug::Value::String(_file)) = row.into_iter().next() {
                return Ok(Some(ResolvedSeed {
                    name: method.to_string(),
                    file_hint: file_hint.map(String::from),
                }));
            }
        }
    }

    // Gather suggestions for not-found error (top-3 Levenshtein on
    // Symbol.name across the index).
    let suggestions = find_nearest_symbol_names(&conn, method, 3);
    let suggest_str = if suggestions.is_empty() {
        String::new()
    } else {
        format!(". Suggestions: {}", suggestions.join(", "))
    };
    Err(Error::InvalidParams(format!(
        "Symbol not found: {raw}{suggest_str}"
    )))
}

/// M-3 review (2026-04-24): scope markers that are NOT class/module
/// symbols. `crate::Foo` means "Foo at crate root" — enclosing "crate"
/// never contains `Foo`. Python `self.method` inside a def is similar
/// at call-site level.
fn is_well_known_root(enclosing: &str) -> bool {
    matches!(enclosing, "crate" | "self" | "super" | "this" | "cls")
}

/// Split a qualified name into (enclosing, method). `::` takes precedence
/// over `.` — Rust code like `foo::bar.baz` (rare) treats `bar.baz` as one
/// method name, not split. Last-2 semantic: `a.b.c` → (`b`, `c`).
fn split_qualified(raw: &str) -> (&str, &str) {
    if let Some(idx) = raw.rfind("::") {
        let method = &raw[idx + 2..];
        let before = &raw[..idx];
        let enclosing = before.rsplit("::").next().unwrap_or(before);
        let enclosing = enclosing.rsplit('.').next().unwrap_or(enclosing);
        (enclosing, method)
    } else {
        // `.` separator
        let idx = raw.rfind('.').expect("caller checks for separator");
        let method = &raw[idx + 1..];
        let before = &raw[..idx];
        let enclosing = before.rsplit('.').next().unwrap_or(before);
        (enclosing, method)
    }
}

/// Top-N Levenshtein neighbours on `Symbol.name` — used only for error
/// messages, so we accept the full-scan cost (expected to fire rarely).
fn find_nearest_symbol_names(conn: &lbug::Connection<'_>, target: &str, k: usize) -> Vec<String> {
    let rs = match conn
        .query("MATCH (s:Symbol) WHERE s.kind <> 'external' RETURN DISTINCT s.name LIMIT 10000")
    {
        Ok(rs) => rs,
        Err(_) => return Vec::new(),
    };
    let mut names: Vec<String> = Vec::new();
    for row in rs {
        if let Some(lbug::Value::String(n)) = row.into_iter().next() {
            names.push(n);
        }
    }
    let mut scored: Vec<(usize, String)> = names
        .into_iter()
        .map(|n| (levenshtein(target, &n), n))
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().take(k).map(|(_, n)| n).collect()
}

/// Small Levenshtein distance — O(m×n), fine for short symbol names. Kept
/// inline to avoid pulling a new crate dependency for a one-shot error
/// message path.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr: Vec<usize> = vec![0; n + 1];
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_python_dot() {
        assert_eq!(
            split_qualified("User.set_password"),
            ("User", "set_password")
        );
    }

    #[test]
    fn split_rust_double_colon() {
        assert_eq!(split_qualified("Router::new"), ("Router", "new"));
    }

    #[test]
    fn split_multi_dot_takes_last_two() {
        assert_eq!(split_qualified("a.b.c"), ("b", "c"));
    }

    #[test]
    fn split_multi_colon_takes_last_two() {
        assert_eq!(split_qualified("a::b::c"), ("b", "c"));
    }

    #[test]
    fn levenshtein_matches_spec_examples() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("foo", "foo"), 0);
        assert_eq!(levenshtein("", "abc"), 3);
    }
}
