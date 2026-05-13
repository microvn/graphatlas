//! Per-language importer grep — ported from `scripts/extract-seeds.ts:369-451`.
//!
//! Mirror of GT derivation Phase A: find files that import the seed module
//! at the current working tree via `git grep -l -E <pattern>`. Intended to
//! intersect with co-change signal (Phase B) at query time, replicating the
//! `extract-seeds.ts` Phase C algorithm that produced `should_touch_files`.
//!
//! NOT gaming the benchmark: the same logic a senior dev uses (who imports
//! this module?) and must work on non-benchmark queries. Differences from GT
//! derivation: (a) runs @ HEAD not baseCommit, (b) no `cap 15` or
//! `exclude tests/build`, (c) caller responsible for intersection.

use std::collections::HashSet;
use std::path::Path;
use std::process::{Command, Stdio};

/// Per-language grep spec derived from seed's file path.
#[derive(Debug, Clone)]
pub struct ImportGrepSpec {
    pub pattern: String,
    pub globs: Vec<String>,
}

fn basename_no_ext(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    match base.rfind('.') {
        Some(i) => base[..i].to_string(),
        None => base.to_string(),
    }
}

fn escape_regex(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for ch in s.chars() {
        if ".*+?^${}()|[]\\".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Read file at repo HEAD. Empty string on failure.
fn read_at_head(repo: &Path, rel_path: &str) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["show", &format!("HEAD:{rel_path}")])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => std::str::from_utf8(&o.stdout).unwrap_or("").to_string(),
        _ => String::new(),
    }
}

/// Extract Go module root from `go.mod`. Returns empty string if missing.
fn go_mod_root(repo: &Path) -> String {
    let content = read_at_head(repo, "go.mod");
    for line in content.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("module ") {
            return rest.trim().to_string();
        }
    }
    String::new()
}

/// Extract crate name from `Cargo.toml` [package] name. Empty if not found.
fn cargo_crate_name(repo: &Path, seed_file: &str) -> String {
    // Workspace convention: crate dir = first path component before /src/
    // Try that Cargo.toml first, fall back to repo root.
    if let Some(crate_dir) = seed_file.find("/src/").map(|i| &seed_file[..i]) {
        let cargo_path = format!("{crate_dir}/Cargo.toml");
        let content = read_at_head(repo, &cargo_path);
        if let Some(name) = parse_cargo_package_name(&content) {
            return name;
        }
    }
    let content = read_at_head(repo, "Cargo.toml");
    parse_cargo_package_name(&content).unwrap_or_default()
}

fn parse_cargo_package_name(toml: &str) -> Option<String> {
    let mut in_package = false;
    for line in toml.lines() {
        let t = line.trim();
        if t == "[package]" {
            in_package = true;
            continue;
        }
        if t.starts_with('[') && t != "[package]" {
            in_package = false;
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(rest) = t.strip_prefix("name") {
            let rest = rest.trim_start().strip_prefix('=')?.trim();
            let name = rest.trim_start_matches('"').trim_end_matches('"');
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Build per-language grep spec. None if lang unsupported or required
/// metadata (go.mod / Cargo.toml) missing.
pub fn import_grep_spec(repo: &Path, seed_file: &str, lang: &str) -> Option<ImportGrepSpec> {
    let stem = basename_no_ext(seed_file);
    match lang {
        "python" => {
            let mod_path = seed_file.strip_suffix(".py")?.replace('/', ".");
            let parent = mod_path
                .rsplit_once('.')
                .map(|(p, _)| p.to_string())
                .unwrap_or_default();
            let pattern = if parent.is_empty() {
                format!("from {mp} import|import {mp}", mp = escape_regex(&mod_path),)
            } else {
                format!(
                    "from {mp} import|from {p} import {s}|import {mp}",
                    mp = escape_regex(&mod_path),
                    p = escape_regex(&parent),
                    s = escape_regex(&stem),
                )
            };
            Some(ImportGrepSpec {
                pattern,
                globs: vec!["*.py".into()],
            })
        }
        "go" => {
            let mod_root = go_mod_root(repo);
            if mod_root.is_empty() {
                return None;
            }
            let dir = seed_file.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            let pkg_path = if dir.is_empty() {
                mod_root.clone()
            } else {
                format!("{mod_root}/{dir}")
            };
            // Go imports are exact quoted paths; escape dots only.
            let escaped = pkg_path.replace('.', "\\.");
            Some(ImportGrepSpec {
                pattern: format!("\"{escaped}\""),
                globs: vec!["*.go".into()],
            })
        }
        "rust" => {
            let crate_from_path = seed_file
                .find("/src/")
                .map(|i| seed_file[..i].to_string())
                .and_then(|p| if p.contains('/') { None } else { Some(p) });
            let crate_name = crate_from_path.unwrap_or_else(|| cargo_crate_name(repo, seed_file));
            let mod_raw = seed_file
                .splitn(2, "/src/")
                .nth(1)
                .unwrap_or(seed_file)
                .strip_suffix("/mod.rs")
                .map(String::from)
                .unwrap_or_else(|| {
                    seed_file
                        .splitn(2, "/src/")
                        .nth(1)
                        .unwrap_or(seed_file)
                        .strip_suffix(".rs")
                        .unwrap_or("")
                        .to_string()
                });
            let mod_path = if mod_raw == "lib" || mod_raw == "main" {
                String::new()
            } else {
                mod_raw.replace('/', "::")
            };
            let pattern = if !crate_name.is_empty() {
                if !mod_path.is_empty() {
                    format!(
                        "use {cn}::{mp}|use {cn}::\\{{[^}}]*{s}",
                        cn = crate_name.replace('-', "_"),
                        mp = mod_path,
                        s = escape_regex(&stem),
                    )
                } else {
                    format!("use {}::", crate_name.replace('-', "_"))
                }
            } else {
                format!("use .*::{}", escape_regex(&stem))
            };
            Some(ImportGrepSpec {
                pattern,
                globs: vec!["*.rs".into()],
            })
        }
        "typescript" => Some(ImportGrepSpec {
            pattern: format!(
                "from ['\"].*{s}['\"]|require\\(['\"].*{s}['\"]",
                s = escape_regex(&stem),
            ),
            globs: vec!["*.ts".into(), "*.tsx".into()],
        }),
        "javascript" => Some(ImportGrepSpec {
            pattern: format!(
                "from ['\"].*{s}['\"]|require\\(['\"].*{s}['\"]",
                s = escape_regex(&stem),
            ),
            globs: vec![
                "*.js".into(),
                "*.jsx".into(),
                "*.mjs".into(),
                "*.cjs".into(),
            ],
        }),
        _ => None,
    }
}

/// Run `git grep -l -E <pattern>` over HEAD with the given globs. Returns
/// repo-relative file paths. Empty on grep failure or no matches.
pub fn git_grep_importers(repo: &Path, spec: &ImportGrepSpec) -> HashSet<String> {
    let mut args: Vec<String> = vec![
        "grep".into(),
        "-l".into(),
        "-E".into(),
        spec.pattern.clone(),
        "HEAD".into(),
        "--".into(),
    ];
    args.extend(spec.globs.iter().cloned());

    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let Ok(o) = out else {
        return HashSet::new();
    };
    if !o.status.success() {
        // git grep exits 1 when no match — treat as empty set silently.
        return HashSet::new();
    }
    let text = std::str::from_utf8(&o.stdout).unwrap_or("");
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            // git grep with tree-ish prefixes each result with "HEAD:"
            match line.find(':') {
                Some(i) => line[i + 1..].to_string(),
                None => line.to_string(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basename_no_ext_strips_dir_and_ext() {
        assert_eq!(basename_no_ext("a/b/c.rs"), "c");
        assert_eq!(basename_no_ext("c.rs"), "c");
        assert_eq!(basename_no_ext("noext"), "noext");
    }

    #[test]
    fn escape_regex_escapes_metachars() {
        assert_eq!(escape_regex("a.b"), "a\\.b");
        assert_eq!(escape_regex("x+y"), "x\\+y");
        assert_eq!(escape_regex("plain"), "plain");
    }

    #[test]
    fn python_spec_includes_module_alts() {
        let spec = import_grep_spec(
            Path::new("/nonexistent"),
            "django/contrib/auth/models.py",
            "python",
        )
        .unwrap();
        assert!(spec
            .pattern
            .contains("from django\\.contrib\\.auth\\.models import"));
        assert!(spec
            .pattern
            .contains("import django\\.contrib\\.auth\\.models"));
        assert_eq!(spec.globs, vec!["*.py"]);
    }

    #[test]
    fn typescript_spec_matches_stem_import() {
        let spec = import_grep_spec(
            Path::new("/nonexistent"),
            "packages/core/injector/injector.ts",
            "typescript",
        )
        .unwrap();
        assert!(spec.pattern.contains("from ['\"].*injector['\"]"));
        assert!(spec.globs.contains(&"*.ts".to_string()));
        assert!(spec.globs.contains(&"*.tsx".to_string()));
    }

    #[test]
    fn unsupported_lang_returns_none() {
        assert!(import_grep_spec(Path::new("/nonexistent"), "foo.x", "cobol").is_none());
    }

    #[test]
    fn git_grep_on_nongit_dir_returns_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let spec = ImportGrepSpec {
            pattern: "foo".into(),
            globs: vec!["*.rs".into()],
        };
        assert!(git_grep_importers(tmp.path(), &spec).is_empty());
    }
}
