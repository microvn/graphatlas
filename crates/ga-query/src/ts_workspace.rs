//! TS/JS workspace resolver — maps a bare import specifier (`@scope/pkg`,
//! `pkg/sub`) to an in-repo file, so monorepo cross-package imports become
//! IMPORTS edges instead of being dropped as external `node_modules`.
//!
//! Layered authority (project-declared, most explicit first — mirrors how
//! `enhanced-resolve` / `rev-dep` resolve, NOT a name-convention guess):
//!   L2 `tsconfig.json` `compilerOptions.paths` — the project's own alias map
//!      (`@nestjs/common` → `./packages/common`).
//!   L3 `package.json` `name` → its directory (preact: `preact-compat` →
//!      `compat`), for repos that declare no tsconfig paths.
//!
//! The specifier's PACKAGE portion is matched (longest prefix wins); the
//! sub-path is ignored for module attribution — landing anywhere in the target
//! package's directory is enough for an architecture module edge. The package
//! is resolved to a representative entry file (`main`/`index.*`/`src/index.*`).

use serde::Deserialize;
use std::collections::HashSet;
use std::path::Path;

#[derive(Deserialize, Default)]
struct PkgJson {
    name: Option<String>,
    main: Option<String>,
    module: Option<String>,
    types: Option<String>,
}

#[derive(Deserialize, Default)]
struct TsConfig {
    #[serde(default, rename = "compilerOptions")]
    compiler_options: CompilerOptions,
}

#[derive(Deserialize, Default)]
struct CompilerOptions {
    #[serde(default, rename = "paths")]
    paths: std::collections::BTreeMap<String, Vec<String>>,
}

/// One workspace package: the specifier it is imported by, its dir, and an
/// optional declared entry (from package.json) relative to that dir.
struct Pkg {
    specifier: String,
    dir: String,
    entry: Option<String>,
    /// Authority rank for a specifier collision (lower wins). tsconfig `paths`
    /// is the project's explicit alias (0); a non-root package.json `name` (1)
    /// beats the root's (2) — monorepos often name the dev-root after the main
    /// package (nest root is `@nestjs/core`, same as `packages/core`), so the
    /// root must lose to the real package dir.
    priority: u8,
}

/// Per-repo import-resolution context for languages whose cross-package
/// imports name a package/module rather than a path: JS/TS workspaces
/// (package.json + tsconfig paths) and Go (go.mod module prefix). Empty for
/// repos of neither kind.
#[derive(Default)]
pub struct TsWorkspace {
    /// Longest-specifier-first, so `@scope/a/b` wins over `@scope/a`.
    pkgs: Vec<Pkg>,
    /// Go module path from `go.mod` (`github.com/gin-gonic/gin`). A Go import
    /// `<prefix>/<pkg-path>` maps to the in-repo dir `<pkg-path>`.
    go_module_prefix: Option<String>,
}

impl TsWorkspace {
    pub fn is_empty(&self) -> bool {
        self.pkgs.is_empty() && self.go_module_prefix.is_none()
    }

    /// Scan `repo_root` for package.json (`name` → dir, + entry), tsconfig
    /// `paths` (alias → dir), and `go.mod` (module prefix). Empty when none
    /// present (non-JS, non-Go repos).
    pub fn load(repo_root: &Path) -> Self {
        let mut pkgs: Vec<Pkg> = Vec::new();
        scan(repo_root, repo_root, &mut pkgs);
        // Longest specifier first (prefix matching); within equal specifiers,
        // lowest priority number wins the dedup (tsconfig paths > non-root pkg
        // name > root pkg name) so a name collision resolves to the real dir.
        pkgs.sort_by(|a, b| {
            b.specifier
                .len()
                .cmp(&a.specifier.len())
                .then(a.specifier.cmp(&b.specifier))
                .then(a.priority.cmp(&b.priority))
        });
        pkgs.dedup_by(|a, b| a.specifier == b.specifier);
        TsWorkspace {
            pkgs,
            go_module_prefix: go_module_prefix(repo_root),
        }
    }

    /// Resolve a Go import path to an in-repo `.go` file. Strips the go.mod
    /// module prefix → the remaining path is the package directory; returns any
    /// indexed `.go` file in that dir (module attribution only needs the dir).
    /// `None` for stdlib / third-party imports outside the module.
    pub fn resolve_go(&self, raw: &str, file_paths: &HashSet<String>) -> Option<String> {
        let prefix = self.go_module_prefix.as_deref()?;
        let pkg_dir = if raw == prefix {
            ""
        } else {
            raw.strip_prefix(&format!("{prefix}/"))?
        };
        file_paths.iter().find_map(|f| {
            if !f.ends_with(".go") {
                return None;
            }
            let dir = f.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            (dir == pkg_dir).then(|| f.clone())
        })
    }

    /// Resolve a bare specifier to an in-repo file. `None` for relative paths
    /// (handled elsewhere) and external packages not in the workspace.
    pub fn resolve(&self, raw: &str, file_paths: &HashSet<String>) -> Option<String> {
        if raw.starts_with('.') {
            return None;
        }
        let pkg = self
            .pkgs
            .iter()
            .find(|p| raw == p.specifier || raw.starts_with(&format!("{}/", p.specifier)))?;
        entry_file(&pkg.dir, pkg.entry.as_deref(), file_paths)
    }
}

/// Module path declared in `go.mod` (`module github.com/gin-gonic/gin`).
/// `None` when there is no `go.mod` or no `module` directive.
fn go_module_prefix(repo_root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(repo_root.join("go.mod")).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("module ") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

/// Workspace package globs from the root `package.json` `workspaces` field
/// (npm/yarn array form `["packages/*"]` or pnpm-ish object form
/// `{"packages": [...]}`). Empty when not declared — the caller then treats
/// every package.json dir as a module (convention monorepos like preact).
pub fn workspace_globs(repo_root: &Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(repo_root.join("package.json")) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Vec::new();
    };
    let ws = match v.get("workspaces") {
        Some(serde_json::Value::Array(a)) => a.clone(),
        Some(serde_json::Value::Object(o)) => o
            .get("packages")
            .and_then(|p| p.as_array())
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    ws.iter()
        .filter_map(|x| x.as_str().map(String::from))
        .collect()
}

/// Does repo-relative dir `rel` match a `workspaces` glob? Supports the common
/// `prefix/*` (one level) and exact-path forms; `**` is treated as `*`.
pub fn glob_matches(globs: &[String], rel: &str) -> bool {
    for g in globs {
        let g = g.trim_end_matches('/');
        if let Some(prefix) = g.strip_suffix("/*").or_else(|| g.strip_suffix("/**")) {
            if let Some(rest) = rel.strip_prefix(&format!("{prefix}/")) {
                if !rest.is_empty() {
                    return true;
                }
            }
        } else if g == rel {
            return true;
        }
    }
    false
}

fn scan(repo_root: &Path, dir: &Path, out: &mut Vec<Pkg>) {
    let rel = dir
        .strip_prefix(repo_root)
        .unwrap_or(dir)
        .to_string_lossy()
        .replace('\\', "/");

    if let Ok(bytes) = std::fs::read(dir.join("package.json")) {
        if let Ok(p) = serde_json::from_slice::<PkgJson>(&bytes) {
            if let Some(name) = p.name {
                let entry = p
                    .types
                    .or(p.module)
                    .or(p.main)
                    .map(|e| e.trim_start_matches("./").to_string());
                out.push(Pkg {
                    specifier: name,
                    dir: rel.clone(),
                    entry,
                    priority: if rel.is_empty() { 2 } else { 1 },
                });
            }
        }
    }

    // tsconfig `paths` alias → dir (`@nestjs/common` → packages/common).
    for ts in ["tsconfig.json", "tsconfig.base.json", "tsconfig.build.json"] {
        if let Ok(s) = std::fs::read_to_string(dir.join(ts)) {
            if let Ok(cfg) = serde_json::from_str::<TsConfig>(&strip_jsonc(&s)) {
                for (pattern, targets) in &cfg.compiler_options.paths {
                    let Some(first) = targets.first() else {
                        continue;
                    };
                    let spec = pattern.trim_end_matches("/*").trim_end_matches('/');
                    let target = first.trim_start_matches("./").trim_end_matches("/*");
                    if spec.is_empty() || target.is_empty() {
                        continue;
                    }
                    // tsconfig paths are relative to the tsconfig's dir.
                    let target_dir = if rel.is_empty() {
                        target.to_string()
                    } else {
                        format!("{rel}/{target}")
                    };
                    out.push(Pkg {
                        specifier: spec.to_string(),
                        dir: target_dir,
                        entry: None,
                        priority: 0,
                    });
                }
            }
        }
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            let base = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(
                base,
                "node_modules" | ".git" | "dist" | "build" | "target" | ".tox" | "__pycache__"
            ) {
                continue;
            }
            scan(repo_root, &path, out);
        }
    }
}

/// Find an indexed entry file for a package dir: declared `main`/etc first,
/// then conventional `index.*` / `src/index.*`.
fn entry_file(dir: &str, declared: Option<&str>, file_paths: &HashSet<String>) -> Option<String> {
    let join = |rel: &str| -> String {
        if dir.is_empty() {
            rel.to_string()
        } else {
            format!("{dir}/{rel}")
        }
    };
    if let Some(d) = declared {
        // package.json `main` may point at built `dist/*.js`; also try the
        // source twin (`.ts`) since fixtures index source, not build output.
        let cand = join(d);
        if file_paths.contains(&cand) {
            return Some(cand);
        }
        if let Some(stem) = cand.strip_suffix(".js") {
            for ext in [".ts", ".tsx"] {
                let c = format!("{stem}{ext}");
                if file_paths.contains(&c) {
                    return Some(c);
                }
            }
        }
    }
    for rel in [
        "index.ts",
        "index.tsx",
        "index.js",
        "index.mjs",
        "src/index.ts",
        "src/index.tsx",
        "src/index.js",
        "index.d.ts",
    ] {
        let c = join(rel);
        if file_paths.contains(&c) {
            return Some(c);
        }
    }
    None
}

/// Strip `//` line and `/* */` block comments so JSONC tsconfig parses.
fn strip_jsonc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let (mut i, mut in_str) = (0, false);
    while i < b.len() {
        let c = b[i];
        if in_str {
            out.push(c as char);
            if c == b'\\' && i + 1 < b.len() {
                out.push(b[i + 1] as char);
                i += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
            i += 1;
        } else if c == b'"' {
            in_str = true;
            out.push('"');
            i += 1;
        } else if c == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
        } else {
            out.push(c as char);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn w(p: &Path, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }

    #[test]
    fn tsconfig_paths_resolve_bare_specifier_to_entry() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        w(
            &root.join("tsconfig.json"),
            r#"{"compilerOptions":{"paths":{"@scope/common":["./packages/common"],"@scope/common/*":["./packages/common/*"]}}}"#,
        );
        w(
            &root.join("packages/common/index.ts"),
            "export const x = 1;",
        );

        let ws = TsWorkspace::load(root);
        let files: HashSet<String> = ["packages/common/index.ts".to_string()].into();
        // bare + subpath both attribute to the package dir's entry.
        assert_eq!(
            ws.resolve("@scope/common", &files).as_deref(),
            Some("packages/common/index.ts")
        );
        assert_eq!(
            ws.resolve("@scope/common/internal", &files).as_deref(),
            Some("packages/common/index.ts")
        );
        assert!(ws.resolve("lodash", &files).is_none());
    }

    #[test]
    fn tsconfig_path_wins_over_colliding_root_package_name() {
        // nest shape: root package.json is ALSO named @scope/core (dev-root),
        // colliding with packages/core. tsconfig paths explicitly map
        // @scope/core → ./packages/core, which must win — else imports of
        // @scope/core resolve to the root and every X→core edge is lost.
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        w(&root.join("package.json"), r#"{"name":"@scope/core"}"#);
        w(
            &root.join("tsconfig.json"),
            r#"{"compilerOptions":{"paths":{"@scope/core":["./packages/core"]}}}"#,
        );
        w(
            &root.join("packages/core/package.json"),
            r#"{"name":"@scope/core"}"#,
        );
        w(&root.join("packages/core/index.ts"), "export const c = 1;");

        let ws = TsWorkspace::load(root);
        let files: HashSet<String> = ["packages/core/index.ts".to_string()].into();
        assert_eq!(
            ws.resolve("@scope/core", &files).as_deref(),
            Some("packages/core/index.ts"),
            "tsconfig paths must resolve @scope/core to packages/core, not the colliding root"
        );
    }

    #[test]
    fn package_name_resolves_when_no_tsconfig_paths() {
        // preact shape: name differs from dir, no tsconfig paths.
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        w(
            &root.join("compat/package.json"),
            r#"{"name":"preact-compat","main":"src/index.js"}"#,
        );
        w(&root.join("compat/src/index.js"), "export default 1;");

        let ws = TsWorkspace::load(root);
        let files: HashSet<String> = ["compat/src/index.js".to_string()].into();
        assert_eq!(
            ws.resolve("preact-compat", &files).as_deref(),
            Some("compat/src/index.js"),
            "name→dir works even when basename (compat) != package name"
        );
    }
}
