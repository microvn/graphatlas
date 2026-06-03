//! C1 — independent PHP PSR-4 autoload reader for the `architecture` GT.
//!
//! ## Anti-tautology policy (§C1)
//! Does NOT import `ga_query::*` (not even the engine's `ts_workspace`
//! PSR-4 resolver) — the GT authority must be derived independently of the
//! engine, or the comparison is tautological. This module re-parses every
//! `composer.json` from raw JSON and resolves `use Ns\Class` → repo-relative
//! `.php` path itself, mirroring the PSR-4 spec (longest namespace prefix
//! wins) rather than calling the tool's resolver.

use serde_json::Value;
use std::path::Path;

/// PSR-4 autoload map: `(namespace_prefix, repo-relative dir)`, longest-prefix
/// first. Built by walking every `composer.json` under the fixture.
pub fn psr4_map(repo_root: &Path) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    walk(repo_root, repo_root, &mut out);
    out.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

/// Resolve a PHP `use Ns\Class` namespace to a repo-relative `.php` file path
/// via the PSR-4 map (longest matching prefix). `None` for namespaces outside
/// every PSR-4 root. Does NOT check existence — the caller maps the path to a
/// module and the module filter rejects unknown targets.
pub fn resolve(psr4: &[(String, String)], raw: &str) -> Option<String> {
    let ns = raw.trim_start_matches('\\');
    for (prefix, dir) in psr4 {
        let Some(remainder) = ns.strip_prefix(prefix.as_str()) else {
            continue;
        };
        if remainder.is_empty() {
            continue;
        }
        let rel = remainder.replace('\\', "/");
        return Some(if dir.is_empty() {
            format!("{rel}.php")
        } else {
            format!("{dir}/{rel}.php")
        });
    }
    None
}

fn walk(repo_root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
    let rel = dir
        .strip_prefix(repo_root)
        .unwrap_or(dir)
        .to_string_lossy()
        .replace('\\', "/");
    if let Ok(bytes) = std::fs::read(dir.join("composer.json")) {
        if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
            for key in ["autoload", "autoload-dev"] {
                let Some(map) = v
                    .get(key)
                    .and_then(|a| a.get("psr-4"))
                    .and_then(|p| p.as_object())
                else {
                    continue;
                };
                for (ns, target) in map {
                    let raw_dir = match target {
                        Value::String(s) => Some(s.clone()),
                        Value::Array(a) => a.first().and_then(|x| x.as_str()).map(String::from),
                        _ => None,
                    };
                    let Some(raw_dir) = raw_dir else { continue };
                    let sub = raw_dir.trim_start_matches("./").trim_end_matches('/');
                    let target_dir = match (rel.is_empty(), sub.is_empty()) {
                        (true, true) => String::new(),
                        (true, false) => sub.to_string(),
                        (false, true) => rel.clone(),
                        (false, false) => format!("{rel}/{sub}"),
                    };
                    out.push((ns.clone(), target_dir));
                }
            }
        }
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            let base = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(
                base,
                ".git" | "node_modules" | "target" | "dist" | "build" | "__pycache__"
            ) {
                continue;
            }
            walk(repo_root, &path, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(p: &Path, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }

    #[test]
    fn longest_prefix_wins() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(
            &root.join("composer.json"),
            r#"{"autoload":{"psr-4":{"League\\Flysystem\\":"src"}}}"#,
        );
        write(
            &root.join("src/Ftp/composer.json"),
            r#"{"autoload":{"psr-4":{"League\\Flysystem\\Ftp\\":""}}}"#,
        );
        let map = psr4_map(root);
        // Core class → root prefix.
        assert_eq!(
            resolve(&map, "League\\Flysystem\\Config").as_deref(),
            Some("src/Config.php")
        );
        // Ftp class → the longer Ftp prefix (dir src/Ftp), not the core src.
        assert_eq!(
            resolve(&map, "League\\Flysystem\\Ftp\\FtpAdapter").as_deref(),
            Some("src/Ftp/FtpAdapter.php")
        );
        // Nested namespace → nested path.
        assert_eq!(
            resolve(&map, "League\\Flysystem\\UnableTo\\Thing").as_deref(),
            Some("src/UnableTo/Thing.php")
        );
    }

    #[test]
    fn leading_backslash_tolerated() {
        let map = vec![("App\\".to_string(), "src".to_string())];
        assert_eq!(resolve(&map, "\\App\\Foo").as_deref(), Some("src/Foo.php"));
    }

    #[test]
    fn unmapped_namespace_is_none() {
        let map = vec![("App\\".to_string(), "src".to_string())];
        assert_eq!(resolve(&map, "Psr\\Log\\LoggerInterface"), None);
    }

    #[test]
    fn empty_dir_root_namespace() {
        let map = vec![("App\\".to_string(), String::new())];
        assert_eq!(
            resolve(&map, "App\\Foo\\Bar").as_deref(),
            Some("Foo/Bar.php")
        );
    }
}
