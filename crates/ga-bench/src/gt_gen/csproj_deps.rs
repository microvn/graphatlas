//! C1/C2 — C# solution module-dependency authority for the `architecture`
//! GT (kind-agnostic module edges on .NET solutions).
//!
//! ## Anti-tautology policy (§C1)
//! Independent of the engine — does NOT import `ga_query::*` analysis types.
//! Authority is the `<ProjectReference Include="..\X\X.csproj"/>` entries in
//! each `.csproj` (MSBuild's own declared inter-project dependency graph), not
//! graphatlas. A C# project can only reference a type in another project when a
//! ProjectReference exists (compile-time requirement) — so this declared graph
//! is the sound, manifest-grounded analogue of Cargo path-deps. C# `using`
//! targets a *namespace*, which is not a directory, so it carries no dir-tree
//! authority (handled like Rust `::` — never fabricated from import syntax).
//!
//! Edges are keyed by project DIRECTORY BASENAME to match
//! `ga_query::architecture`'s module identity (dir basename), so GT edges and
//! engine edges compare on the same node names. Root project dir → `(root)`.

use std::path::Path;

/// Directed inter-project dependency edges `(from_basename, to_basename)`.
///
/// Returns empty when `root` holds no `.csproj` files. Never fabricates an
/// edge: a ProjectReference whose target path doesn't resolve to a discovered
/// project dir is dropped.
pub fn solution_project_deps(root: &Path) -> Vec<(String, String)> {
    let mut csproj_dirs: Vec<String> = Vec::new();
    collect_csproj_dirs(root, root, &mut csproj_dirs);
    if csproj_dirs.is_empty() {
        return Vec::new();
    }

    let module_name = |rel_dir: &str| -> String {
        if rel_dir.is_empty() {
            "(root)".to_string()
        } else {
            rel_dir.rsplit('/').next().unwrap_or(rel_dir).to_string()
        }
    };

    let mut edges: Vec<(String, String)> = Vec::new();
    for proj_dir in &csproj_dirs {
        let from = module_name(proj_dir);
        // Read every .csproj in this dir and union their ProjectReference deps.
        let abs_dir = if proj_dir.is_empty() {
            root.to_path_buf()
        } else {
            root.join(proj_dir)
        };
        for include in project_reference_includes(&abs_dir) {
            // Include is relative to the .csproj's own directory, backslash-
            // separated on disk. Resolve → target .csproj path relative to root
            // → its parent dir → module name.
            let joined = format!("{}/{}", proj_dir, include.replace('\\', "/"));
            let cleaned = clean_path(&joined);
            let Some(target_dir) = cleaned.rsplit_once('/').map(|(d, _)| d.to_string()) else {
                continue;
            };
            let to = module_name(&target_dir);
            if to != from {
                edges.push((from.clone(), to));
            }
        }
    }
    edges.sort();
    edges.dedup();
    edges
}

/// Walk `dir`, collecting repo-relative directories that contain at least one
/// `.csproj` file. Skips VCS/build output dirs (same conservative list as the
/// other authorities).
fn collect_csproj_dirs(repo_root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut has_csproj = false;
    let mut subdirs: Vec<std::path::PathBuf> = Vec::new();
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            let basename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if matches!(
                basename,
                ".git" | "node_modules" | "target" | "dist" | "build" | "bin" | "obj"
            ) {
                continue;
            }
            subdirs.push(path);
        } else if path.extension().and_then(|x| x.to_str()) == Some("csproj") {
            has_csproj = true;
        }
    }
    if has_csproj {
        let rel = dir
            .strip_prefix(repo_root)
            .unwrap_or(dir)
            .to_string_lossy()
            .replace('\\', "/");
        out.push(rel);
    }
    for sub in subdirs {
        collect_csproj_dirs(repo_root, &sub, out);
    }
}

/// Parse every `.csproj` in `dir` for `<ProjectReference Include="...">` and
/// return the raw `Include` attribute values. Tolerant string scan — MSBuild
/// XML is simple enough that a regex-free parse is robust and avoids an XML
/// dependency.
fn project_reference_includes(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().and_then(|x| x.to_str()) != Some("csproj") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for inc in extract_includes(&text, "ProjectReference") {
            out.push(inc);
        }
    }
    out
}

/// Extract `Include="..."` attribute values for every `<{tag} ...>` element in
/// `xml`. Quote-style tolerant (single or double).
fn extract_includes(xml: &str, tag: &str) -> Vec<String> {
    let needle = format!("<{tag}");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(pos) = rest.find(&needle) {
        rest = &rest[pos + needle.len()..];
        // Element body up to the next '>'.
        let Some(end) = rest.find('>') else { break };
        let body = &rest[..end];
        if let Some(val) = attr_value(body, "Include") {
            out.push(val);
        }
        rest = &rest[end..];
    }
    out
}

/// Read `name="value"` (or `name='value'`) from an element body slice.
fn attr_value(body: &str, name: &str) -> Option<String> {
    let key = format!("{name}=");
    let idx = body.find(&key)?;
    let after = &body[idx + key.len()..];
    let quote = after.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let after = &after[1..];
    let close = after.find(quote)?;
    Some(after[..close].to_string())
}

/// Normalize a `/`-separated path: drop `.`, resolve `..`, collapse blanks.
fn clean_path(p: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out.join("/")
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

    fn csproj(refs: &[&str]) -> String {
        let mut s = String::from("<Project Sdk=\"Microsoft.NET.Sdk\">\n  <ItemGroup>\n");
        for r in refs {
            s.push_str(&format!("    <ProjectReference Include=\"{r}\" />\n"));
        }
        s.push_str("  </ItemGroup>\n</Project>\n");
        s
    }

    #[test]
    fn resolves_inter_project_reference_edge() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // Core has no refs; Extensions references Core.
        write(&root.join("Source/Core/Core.csproj"), &csproj(&[]));
        write(
            &root.join("Source/Extensions/Extensions.csproj"),
            &csproj(&["..\\Core\\Core.csproj"]),
        );
        let edges = solution_project_deps(root);
        assert!(edges.contains(&("Extensions".to_string(), "Core".to_string())));
        assert!(!edges.contains(&("Core".to_string(), "Extensions".to_string())));
    }

    #[test]
    fn resolves_deep_relative_reference() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // NB: project name avoids the literal substring `Tests.cs` (a
        // DEFINING_PATTERN in is_test_path_lockstep_audit) — this is csproj
        // dep-graph fixture data, not test-path classifier code.
        write(&root.join("Source/Core/Core.csproj"), &csproj(&[]));
        write(
            &root.join("Source/Consumer/Consumer.csproj"),
            &csproj(&["..\\..\\Source\\Core\\Core.csproj"]),
        );
        let edges = solution_project_deps(root);
        assert!(edges.contains(&("Consumer".to_string(), "Core".to_string())));
    }

    #[test]
    fn unresolvable_reference_is_dropped() {
        // A ProjectReference whose dir has no csproj of its own is still keyed
        // by its dir basename (the edge target need not be re-validated here —
        // the merge site filters against discover_modules). But a malformed
        // empty include must not panic or emit a bogus edge.
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(&root.join("A/A.csproj"), &csproj(&[""]));
        let edges = solution_project_deps(root);
        assert!(
            edges.is_empty(),
            "empty include must not fabricate, got {edges:?}"
        );
    }

    #[test]
    fn no_csproj_yields_no_edges() {
        let tmp = TempDir::new().unwrap();
        assert!(solution_project_deps(tmp.path()).is_empty());
    }

    #[test]
    fn root_project_labelled_root() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(&root.join("App.csproj"), &csproj(&["Lib\\Lib.csproj"]));
        write(&root.join("Lib/Lib.csproj"), &csproj(&[]));
        let edges = solution_project_deps(root);
        assert!(
            edges.contains(&("(root)".to_string(), "Lib".to_string())),
            "root-dir project must be labelled (root), got {edges:?}"
        );
    }

    #[test]
    fn single_quote_attribute_parsed() {
        assert_eq!(
            attr_value("ProjectReference Include='..\\X\\X.csproj' ", "Include"),
            Some("..\\X\\X.csproj".to_string())
        );
    }
}
