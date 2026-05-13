//! Cluster C5 — AS-014 framework-aware route extractors: gin, Django urls.py,
//! Rails routes.rb, axum, NestJS @Controller.

use super::types::AffectedRoute;
use crate::common;
use ga_core::{Error, Result};
use ga_index::Store;
use std::collections::HashSet;

/// AS-014 — scan all files in the repo for framework-aware route mounts
/// whose handler matches `seed_symbol`. Covers gin (Go), Django (Python
/// urls.py), Rails (Ruby routes.rb), axum (Rust), nest (TS controllers).
///
/// Read happens via filesystem — repo root from `Store::metadata().repo_root`.
/// Files that don't match any extractor's file-extension gate are skipped.
///
/// Deterministic output: sorted by `(path, method, source_file)`.
pub(super) fn collect_affected_routes(
    store: &Store,
    seed_symbol: &str,
) -> Result<Vec<AffectedRoute>> {
    if !common::is_safe_ident(seed_symbol) {
        return Ok(Vec::new());
    }

    let repo_root = store.metadata().repo_root.clone();
    if repo_root.is_empty() {
        return Ok(Vec::new()); // defensive — repo_root should always be set post-index
    }
    let repo_root = std::path::PathBuf::from(repo_root);

    let conn = store
        .connection()
        .map_err(|e| Error::Other(anyhow::anyhow!("connection: {e}")))?;
    let rs = conn
        .query("MATCH (f:File) RETURN f.path")
        .map_err(|e| Error::Other(anyhow::anyhow!("routes file-scan query: {e}")))?;

    let mut seen: HashSet<(String, String, String)> = HashSet::new();
    let mut out: Vec<AffectedRoute> = Vec::new();
    for row in rs {
        let Some(lbug::Value::String(rel_path)) = row.into_iter().next() else {
            continue;
        };
        let Ok(bytes) = std::fs::read(repo_root.join(&rel_path)) else {
            continue;
        };
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        extract_routes_for_file(&rel_path, text, seed_symbol, &mut seen, &mut out);
    }

    // Rails + config/routes.rb: ga-parser currently doesn't emit File nodes
    // for Ruby, so we miss routes.rb via the graph path. Fall back to a
    // direct filesystem probe for the canonical Rails path.
    let rails_path = "config/routes.rb";
    if std::fs::metadata(repo_root.join(rails_path)).is_ok() {
        if let Ok(bytes) = std::fs::read(repo_root.join(rails_path)) {
            if let Ok(text) = std::str::from_utf8(&bytes) {
                extract_routes_for_file(rails_path, text, seed_symbol, &mut seen, &mut out);
            }
        }
    }

    out.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.method.cmp(&b.method))
            .then_with(|| a.source_file.cmp(&b.source_file))
    });
    Ok(out)
}

fn extract_routes_for_file(
    rel_path: &str,
    text: &str,
    seed: &str,
    seen: &mut HashSet<(String, String, String)>,
    out: &mut Vec<AffectedRoute>,
) {
    let push =
        |method: String, path: String, out: &mut Vec<AffectedRoute>, seen: &mut HashSet<_>| {
            let key = (method.clone(), path.clone(), rel_path.to_string());
            if seen.insert(key) {
                out.push(AffectedRoute {
                    method,
                    path,
                    source_file: rel_path.to_string(),
                });
            }
        };

    if rel_path.ends_with(".go") {
        for (m, p) in extract_gin(text, seed) {
            push(m, p, out, seen);
        }
    } else if rel_path.ends_with(".py") {
        for (m, p) in extract_django(text, seed) {
            push(m, p, out, seen);
        }
    } else if rel_path.ends_with(".rb") {
        for (m, p) in extract_rails(text, seed) {
            push(m, p, out, seen);
        }
    } else if rel_path.ends_with(".rs") {
        for (m, p) in extract_axum(text, seed) {
            push(m, p, out, seen);
        }
    } else if rel_path.ends_with(".ts") || rel_path.ends_with(".tsx") {
        for (m, p) in extract_nest(text, seed) {
            push(m, p, out, seen);
        }
    }
}

/// Gin: `r.METHOD("/path", Handler)` or `r.METHOD("/path", h.Handler)`.
/// Method upper-cased (GET/POST/…). Final identifier after `.` is the
/// handler name — matches seed.
fn extract_gin(text: &str, seed: &str) -> Vec<(String, String)> {
    const METHODS: &[&str] = &["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        for m in METHODS {
            let needle = format!(".{m}(");
            let Some(pos) = trimmed.find(&needle) else {
                continue;
            };
            let after = &trimmed[pos + needle.len()..];
            let Some(path) = extract_quoted(after) else {
                continue;
            };
            let Some(rest_comma) = after.find(',') else {
                continue;
            };
            let handler_part = after[rest_comma + 1..].trim();
            let handler = last_ident_segment(handler_part);
            if handler == seed {
                out.push((m.to_string(), path));
            }
        }
    }
    out
}

/// Django `path("url", views.handler, ...)`.
fn extract_django(text: &str, seed: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(pos) = line.find("path(") else {
            continue;
        };
        let after = &line[pos + 5..];
        let Some(url) = extract_quoted(after) else {
            continue;
        };
        let Some(comma) = after.find(',') else {
            continue;
        };
        let handler_part = after[comma + 1..].trim();
        let handler = last_ident_segment(handler_part);
        if handler == seed {
            out.push(("ANY".to_string(), url));
        }
    }
    out
}

/// Rails `verb "/path" => "controller#action"`.
fn extract_rails(text: &str, seed: &str) -> Vec<(String, String)> {
    const VERBS: &[&str] = &["get", "post", "put", "patch", "delete", "head", "options"];
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        for v in VERBS {
            if !trimmed.starts_with(&format!("{v} ")) && !trimmed.starts_with(&format!("{v}\t")) {
                continue;
            }
            let after = &trimmed[v.len()..];
            let Some(path) = extract_quoted(after) else {
                continue;
            };
            let Some(arrow) = after.find("=>") else {
                continue;
            };
            let spec_part = &after[arrow + 2..];
            let Some(target) = extract_quoted(spec_part) else {
                continue;
            };
            let action = target.rsplit('#').next().unwrap_or("");
            if action == seed {
                out.push((v.to_uppercase(), path));
            }
        }
    }
    out
}

/// Axum `.route("/path", method(handler))`.
fn extract_axum(text: &str, seed: &str) -> Vec<(String, String)> {
    const VERBS: &[&str] = &["get", "post", "put", "delete", "patch", "head", "options"];
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(pos) = line.find(".route(") else {
            continue;
        };
        let after = &line[pos + 7..];
        let Some(path) = extract_quoted(after) else {
            continue;
        };
        let Some(comma) = after.find(',') else {
            continue;
        };
        let verb_part = after[comma + 1..].trim_start();
        for v in VERBS {
            let needle = format!("{v}(");
            if verb_part.starts_with(&needle) {
                let handler_part = &verb_part[needle.len()..];
                let handler = first_ident(handler_part);
                if handler == seed {
                    out.push((v.to_uppercase(), path.clone()));
                }
            }
        }
    }
    out
}

/// NestJS `@Controller('/prefix')` + `@Method('/path')` preceding a
/// function whose name matches the seed. Path joined with controller prefix.
fn extract_nest(text: &str, seed: &str) -> Vec<(String, String)> {
    const METHODS: &[&str] = &["Get", "Post", "Put", "Delete", "Patch", "Head", "Options"];
    // Find controller prefix (once per file).
    let mut prefix = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(after) = trimmed.strip_prefix("@Controller(") {
            if let Some(p) = extract_quoted(after) {
                prefix = p;
            }
            break;
        }
    }

    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        for m in METHODS {
            let needle = format!("@{m}(");
            if !trimmed.starts_with(&needle) {
                continue;
            }
            let after = &trimmed[needle.len()..];
            let method_path = extract_quoted(after).unwrap_or_default();
            // Find next method name in the following 8 lines.
            let mut handler = None;
            for look in lines.iter().skip(i + 1).take(8) {
                let tl = look.trim_start();
                if let Some(name) = method_name_from_line(tl) {
                    handler = Some(name);
                    break;
                }
            }
            if let Some(h) = handler {
                if h == seed {
                    out.push((m.to_uppercase(), join_paths(&prefix, &method_path)));
                }
            }
        }
    }
    out
}

/// Parse the first quoted string (single or double) from `s`.
fn extract_quoted(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let quote = bytes.iter().find(|b| matches!(**b, b'"' | b'\''))?;
    let qc = *quote as char;
    let first = s.find(qc)?;
    let rest = &s[first + 1..];
    let end = rest.find(qc)?;
    Some(rest[..end].to_string())
}

/// Final identifier segment of `"foo.bar.Baz"` → `"Baz"`. Handles trailing
/// `)`, `,`, or whitespace.
fn last_ident_segment(s: &str) -> String {
    let mut end = 0;
    for (i, c) in s.char_indices() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '.' {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    let token = &s[..end];
    token.rsplit('.').next().unwrap_or("").to_string()
}

/// First identifier token at the start of `s`.
fn first_ident(s: &str) -> String {
    let mut end = 0;
    for (i, c) in s.char_indices() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    s[..end].to_string()
}

/// Attempt to pull a method name from a nest controller body line. Matches
/// `async foo(...)`, `foo(...)`, `public foo(...)` etc. Returns None if the
/// line isn't a method declaration.
fn method_name_from_line(line: &str) -> Option<String> {
    let line = line
        .trim_start_matches("public ")
        .trim_start_matches("private ")
        .trim_start_matches("protected ")
        .trim_start_matches("static ")
        .trim_start_matches("async ");
    if line.starts_with('@') {
        return None;
    }
    let name = first_ident(line);
    if name.is_empty() {
        return None;
    }
    if line[name.len()..].trim_start().starts_with('(') {
        Some(name)
    } else {
        None
    }
}

/// Join controller prefix + method path with a single `/`.
fn join_paths(prefix: &str, method_path: &str) -> String {
    let p = prefix.trim_end_matches('/');
    let m = method_path.trim_start_matches('/');
    if p.is_empty() {
        return if m.is_empty() {
            String::new()
        } else {
            format!("/{m}")
        };
    }
    if m.is_empty() {
        return p.to_string();
    }
    format!("{p}/{m}")
}
