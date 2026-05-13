//! Entry-point detection helpers shared between `ga_query::dead_code`
//! (production analysis) and the M3 bench `Hd-ast` rule.
//!
//! Extracted as a public module so the bench rule can import without
//! pulling in `ga_query::dead_code` (forbidden by anti-tautology policy
//! per spec §C1).
//!
//! These are pure file-system + line-pattern scanners — they NEVER call
//! the graph layer. Behaviour pinned by `ga_query::dead_code`'s existing
//! tests (route_handler / dunder_all / project_scripts cases). Any change
//! here is regression-safe iff those tests continue to pass.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

// ─────────────────────────────────────────────────────────────────────────
// Route-handler scan (parallels impact::routes but yields the handler set)
// ─────────────────────────────────────────────────────────────────────────

pub fn collect_route_handlers(repo_root: &Path) -> HashSet<String> {
    let mut out: HashSet<String> = HashSet::new();
    walk_files(repo_root, &mut |rel_path, text| {
        if rel_path.ends_with(".go") {
            extract_gin_handlers(text, &mut out);
        } else if rel_path.ends_with(".py") {
            extract_django_handlers(text, &mut out);
        } else if rel_path.ends_with(".rb") {
            extract_rails_handlers(text, &mut out);
        } else if rel_path.ends_with(".rs") {
            extract_axum_handlers(text, &mut out);
        } else if rel_path.ends_with(".ts") || rel_path.ends_with(".tsx") {
            extract_nest_handlers(text, &mut out);
        }
    });
    out
}

fn extract_gin_handlers(text: &str, out: &mut HashSet<String>) {
    const METHODS: &[&str] = &["GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS"];
    for line in text.lines() {
        let trimmed = line.trim();
        for m in METHODS {
            let needle = format!(".{m}(");
            let Some(pos) = trimmed.find(&needle) else {
                continue;
            };
            let after = &trimmed[pos + needle.len()..];
            let Some(comma) = after.find(',') else {
                continue;
            };
            let handler_part = after[comma + 1..].trim();
            let handler = last_ident_segment(handler_part);
            if !handler.is_empty() {
                out.insert(handler);
            }
        }
    }
}

fn extract_django_handlers(text: &str, out: &mut HashSet<String>) {
    for line in text.lines() {
        let Some(pos) = line.find("path(") else {
            continue;
        };
        let after = &line[pos + 5..];
        let Some(after_url) = skip_quoted(after) else {
            continue;
        };
        let Some(comma) = after_url.find(',') else {
            continue;
        };
        let handler_part = after_url[comma + 1..].trim();
        let handler = last_ident_segment(handler_part);
        if !handler.is_empty() {
            out.insert(handler);
        }
    }
}

fn extract_rails_handlers(text: &str, out: &mut HashSet<String>) {
    const VERBS: &[&str] = &["get", "post", "put", "patch", "delete", "head", "options"];
    for line in text.lines() {
        let trimmed = line.trim_start();
        for v in VERBS {
            if !trimmed.starts_with(&format!("{v} ")) && !trimmed.starts_with(&format!("{v}\t")) {
                continue;
            }
            let after = &trimmed[v.len()..];
            let Some(arrow) = after.find("=>") else {
                continue;
            };
            let spec_part = &after[arrow + 2..];
            if let Some(target) = first_quoted(spec_part) {
                if let Some(action) = target.rsplit('#').next() {
                    if !action.is_empty() {
                        out.insert(action.to_string());
                    }
                }
            }
        }
    }
}

fn extract_axum_handlers(text: &str, out: &mut HashSet<String>) {
    const VERBS: &[&str] = &["get", "post", "put", "delete", "patch", "head", "options"];
    for line in text.lines() {
        let Some(pos) = line.find(".route(") else {
            continue;
        };
        let after = &line[pos + 7..];
        let Some(comma) = after.find(',') else {
            continue;
        };
        let verb_part = after[comma + 1..].trim_start();
        for v in VERBS {
            let needle = format!("{v}(");
            if verb_part.starts_with(&needle) {
                let handler_part = &verb_part[needle.len()..];
                let handler = first_ident(handler_part);
                if !handler.is_empty() {
                    out.insert(handler);
                }
            }
        }
    }
}

fn extract_nest_handlers(text: &str, out: &mut HashSet<String>) {
    const METHODS: &[&str] = &["Get", "Post", "Put", "Delete", "Patch", "Head", "Options"];
    let lines: Vec<&str> = text.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        for m in METHODS {
            let needle = format!("@{m}(");
            if !trimmed.starts_with(&needle) {
                continue;
            }
            for look in lines.iter().skip(i + 1).take(8) {
                let tl = look.trim_start();
                if let Some(name) = method_name_from_line(tl) {
                    out.insert(name);
                    break;
                }
            }
        }
    }
}

fn first_quoted(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let quote = bytes.iter().find(|b| matches!(**b, b'"' | b'\''))?;
    let qc = *quote as char;
    let first = s.find(qc)?;
    let rest = &s[first + 1..];
    let end = rest.find(qc)?;
    Some(rest[..end].to_string())
}

fn skip_quoted(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let quote = bytes.iter().find(|b| matches!(**b, b'"' | b'\''))?;
    let qc = *quote as char;
    let first = s.find(qc)?;
    let rest = &s[first + 1..];
    let end = rest.find(qc)?;
    Some(&rest[end + 1..])
}

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

// ─────────────────────────────────────────────────────────────────────────
// Library public API extractors
// ─────────────────────────────────────────────────────────────────────────

pub fn collect_dunder_all(repo_root: &Path) -> HashSet<(String, String)> {
    let mut out: HashSet<(String, String)> = HashSet::new();
    walk_files(repo_root, &mut |rel_path, text| {
        if !rel_path.ends_with("__init__.py") {
            return;
        }
        for name in parse_dunder_all(text) {
            if is_safe_ident(&name) {
                out.insert((rel_path.to_string(), name));
            }
        }
    });
    out
}

/// Conservative single-line `__all__ = [...]` extractor.
pub fn parse_dunder_all(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let after_eq = match trimmed.strip_prefix("__all__") {
            Some(rest) => rest.trim_start().strip_prefix('=').map(str::trim_start),
            None => None,
        };
        let Some(rhs) = after_eq else { continue };
        let inner = if let Some(s) = rhs
            .strip_prefix('[')
            .and_then(|s| s.find(']').map(|i| &s[..i]))
        {
            s
        } else if let Some(s) = rhs
            .strip_prefix('(')
            .and_then(|s| s.find(')').map(|i| &s[..i]))
        {
            s
        } else {
            continue;
        };
        for raw in inner.split(',') {
            let token = raw.trim().trim_matches(|c: char| c == '"' || c == '\'');
            if !token.is_empty() {
                out.push(token.to_string());
            }
        }
    }
    out
}

pub fn collect_project_scripts(repo_root: &Path) -> HashSet<String> {
    let mut out: HashSet<String> = HashSet::new();
    let pyproject = repo_root.join("pyproject.toml");
    let Ok(bytes) = std::fs::read(&pyproject) else {
        return out;
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return out;
    };
    let mut in_scripts = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_scripts = matches!(trimmed, "[project.scripts]" | "[tool.poetry.scripts]");
            continue;
        }
        if !in_scripts {
            continue;
        }
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        let Some((_, value)) = trimmed.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches(|c: char| c == '"' || c == '\'');
        let Some((_, func)) = value.rsplit_once(':') else {
            continue;
        };
        let func = func.trim();
        if is_safe_ident(func) {
            out.insert(func.to_string());
        }
    }
    out
}

/// Cargo `[[bin]]` declarations — return the `name` field of every
/// `[[bin]]` block in `Cargo.toml`. Used by the bench Hd-ast rule.
pub fn collect_cargo_bins(repo_root: &Path) -> HashSet<String> {
    let mut out = HashSet::new();
    let cargo = repo_root.join("Cargo.toml");
    let Ok(text) = std::fs::read_to_string(&cargo) else {
        return out;
    };
    let mut in_bin = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_bin = line == "[[bin]]";
            continue;
        }
        if !in_bin {
            continue;
        }
        if let Some(rest) = line.strip_prefix("name") {
            let after = rest.trim_start_matches(|c: char| c == ' ' || c == '=' || c == '"');
            let bin_name = after.trim_end_matches('"').trim();
            if !bin_name.is_empty() {
                out.insert(bin_name.to_string());
            }
        }
    }
    out
}

/// Collect directories whose `Cargo.toml` declares `crate-type` containing
/// `cdylib` or `staticlib`. Files under those directories are part of a C-ABI
/// library — every `pub` item is reachable via the C interface, so nothing
/// in those crates should be flagged as dead.
///
/// Returns relative directory paths (e.g. `"regex-capi"`) that contain the
/// cdylib `Cargo.toml`, normalised with trailing `/` stripped.
pub fn collect_cdylib_dirs(repo_root: &Path) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut stack: Vec<PathBuf> = vec![repo_root.to_path_buf()];
    let mut visited: HashMap<PathBuf, ()> = HashMap::new();
    while let Some(dir) = stack.pop() {
        if visited.insert(dir.clone(), ()).is_some() {
            continue;
        }
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut subdirs = Vec::new();
        let mut cargo_path = None;
        for entry in read.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if !name_str.starts_with('.')
                    && !matches!(
                        name_str.as_ref(),
                        "target" | "node_modules" | ".git" | "build" | "dist"
                    )
                {
                    subdirs.push(path);
                }
            } else if ft.is_file() && entry.file_name() == "Cargo.toml" {
                cargo_path = Some(path);
            }
        }
        for sub in subdirs {
            stack.push(sub);
        }
        let Some(cargo) = cargo_path else { continue };
        let Ok(text) = std::fs::read_to_string(&cargo) else {
            continue;
        };
        if is_cdylib_cargo(&text) {
            if let Ok(rel) = dir.strip_prefix(repo_root) {
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                out.insert(rel_str);
            }
        }
    }
    out
}

/// Returns true when a `Cargo.toml` text declares `cdylib` or `staticlib`
/// as a crate-type under `[lib]`.
fn is_cdylib_cargo(text: &str) -> bool {
    let mut in_lib = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_lib = trimmed == "[lib]";
            continue;
        }
        if !in_lib {
            continue;
        }
        if trimmed.starts_with("crate-type") {
            return trimmed.contains("cdylib") || trimmed.contains("staticlib");
        }
    }
    false
}

/// Two files share a Python package if they live under the same directory.
pub fn same_package(init_file: &str, candidate: &str) -> bool {
    let pkg_dir = Path::new(init_file)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    if pkg_dir.is_empty() {
        return true;
    }
    candidate.starts_with(&format!("{pkg_dir}/")) || candidate == format!("{pkg_dir}/__init__.py")
}

fn is_safe_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && s.chars().next().map_or(false, |c| !c.is_ascii_digit())
}

// ─────────────────────────────────────────────────────────────────────────
// Bounded recursive walk
// ─────────────────────────────────────────────────────────────────────────

fn walk_files<F: FnMut(&str, &str)>(repo_root: &Path, on_file: &mut F) {
    fn skip_dir(name: &str) -> bool {
        matches!(
            name,
            ".git"
                | ".hg"
                | ".svn"
                | "node_modules"
                | "target"
                | "dist"
                | "build"
                | "__pycache__"
                | ".graphatlas"
        )
    }
    let mut stack: Vec<PathBuf> = vec![repo_root.to_path_buf()];
    let mut visited: HashMap<PathBuf, ()> = HashMap::new();
    while let Some(dir) = stack.pop() {
        if visited.insert(dir.clone(), ()).is_some() {
            continue;
        }
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with('.') || skip_dir(&name_str) {
                    continue;
                }
                stack.push(path);
            } else if ft.is_file() {
                let Ok(rel) = path.strip_prefix(repo_root) else {
                    continue;
                };
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                let Ok(text) = std::str::from_utf8(&bytes) else {
                    continue;
                };
                on_file(&rel_str, text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_dunder_all_single_line_list() {
        let names = parse_dunder_all("__all__ = ['foo', 'bar', 'baz']\n");
        assert_eq!(names, vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn last_ident_segment_handles_dotted() {
        assert_eq!(last_ident_segment("views.list_users)"), "list_users");
        assert_eq!(last_ident_segment("h.HandleX,"), "HandleX");
    }

    #[test]
    fn same_package_top_level_init_covers_all() {
        assert!(same_package("__init__.py", "anything.py"));
    }

    #[test]
    fn same_package_pkg_init_covers_pkg_files_only() {
        assert!(same_package("mylib/__init__.py", "mylib/core.py"));
        assert!(!same_package("mylib/__init__.py", "other/core.py"));
    }
}
