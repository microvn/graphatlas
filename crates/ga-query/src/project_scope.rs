//! C# project-reference scope for name resolution (.NET solutions).
//!
//! Analogue of [`crate::crate_scope::CrateScope`] for C#. The repo-wide name
//! fallback (tier 3) resolves a bare call/reference to a same-named symbol
//! anywhere in the repo. On a .NET solution that lets a project's call resolve
//! into an unrelated project that merely reuses a type/method name — a
//! cross-project over-link the caller could never make, since C# requires a
//! `<ProjectReference>` to use another project's types (compile-time rule).
//!
//! This reads each `.csproj`'s `ProjectReference Include="..."` entries (the
//! same MSBuild authority `ga_bench::gt_gen::csproj_deps` uses for GT — by
//! design, mirroring the Rust `crate_scope`/`cargo_deps` pairing the gate
//! already accepts, with the TAUTOLOGY-SUSPECT guard catching convergence) so
//! the fallback only resolves within the caller's project or a project it
//! DIRECTLY references. Direct-only (not transitive) so the scope aligns with
//! the direct-edge ProjectReference graph: a transitively-reachable type in a
//! non-directly-referenced project is not an architecture edge.
//!
//! Empty (⇒ no filtering) for non-C# repos or solutions with no `.csproj`.

use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Project-membership + direct-reference closure for a .NET solution, keyed by
/// repo-relative project root directory.
#[derive(Default)]
pub struct ProjectScope {
    /// Project root dirs (repo-relative), longest-first for prefix matching.
    roots: Vec<String>,
    /// project-root-dir → allowed target project-root-dirs (itself + direct refs).
    allowed: HashMap<String, HashSet<String>>,
}

impl ProjectScope {
    /// Build by scanning every `.csproj` under `repo_root`. Returns an empty
    /// scope (no filtering) when there are no `.csproj` files.
    pub fn load(repo_root: &Path) -> Self {
        let mut proj_dirs: Vec<String> = Vec::new();
        collect_csproj_dirs(repo_root, repo_root, &mut proj_dirs);
        if proj_dirs.is_empty() {
            return Self::default();
        }
        let dir_set: HashSet<String> = proj_dirs.iter().cloned().collect();

        let mut allowed: HashMap<String, HashSet<String>> = HashMap::new();
        for proj_dir in &proj_dirs {
            let set = allowed.entry(proj_dir.clone()).or_default();
            set.insert(proj_dir.clone());
            let abs_dir = if proj_dir.is_empty() {
                repo_root.to_path_buf()
            } else {
                repo_root.join(proj_dir)
            };
            for include in project_reference_includes(&abs_dir) {
                let joined = format!("{}/{}", proj_dir, include.replace('\\', "/"));
                let cleaned = clean_path(&joined);
                // Include points at a .csproj; its parent dir is the target project.
                if let Some((target_dir, _)) = cleaned.rsplit_once('/') {
                    if dir_set.contains(target_dir) {
                        set.insert(target_dir.to_string());
                    }
                } else if dir_set.contains("") {
                    // root-dir target (Include="X.csproj" with X at repo root)
                    set.insert(String::new());
                }
            }
        }

        let mut roots: Vec<String> = proj_dirs;
        roots.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
        roots.dedup();
        ProjectScope { roots, allowed }
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    fn project_of<'a>(&'a self, file: &str) -> Option<&'a str> {
        for r in &self.roots {
            if r.is_empty() {
                continue;
            }
            if file == r.as_str() || file.starts_with(&format!("{r}/")) {
                return Some(r);
            }
        }
        if self.roots.iter().any(String::is_empty) {
            Some("")
        } else {
            None
        }
    }

    /// May a caller in `caller_file` resolve a name to a definition in
    /// `candidate_file`? True when there is no project info (don't filter),
    /// either file is outside any project, both are the same project, or the
    /// caller's project directly references the candidate's project.
    pub fn allows(&self, caller_file: &str, candidate_file: &str) -> bool {
        if self.roots.is_empty() {
            return true;
        }
        let (Some(cp), Some(tp)) = (
            self.project_of(caller_file),
            self.project_of(candidate_file),
        ) else {
            return true;
        };
        if cp == tp {
            return true;
        }
        self.allowed.get(cp).map(|s| s.contains(tp)).unwrap_or(true)
    }
}

/// Walk `dir`, collecting repo-relative directories that contain a `.csproj`.
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
        out.push(
            dir.strip_prefix(repo_root)
                .unwrap_or(dir)
                .to_string_lossy()
                .replace('\\', "/"),
        );
    }
    for sub in subdirs {
        collect_csproj_dirs(repo_root, &sub, out);
    }
}

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
        let needle = "<ProjectReference";
        let mut rest = text.as_str();
        while let Some(pos) = rest.find(needle) {
            rest = &rest[pos + needle.len()..];
            let Some(end) = rest.find('>') else { break };
            let body = &rest[..end];
            if let Some(val) = attr_value(body, "Include") {
                out.push(val);
            }
            rest = &rest[end..];
        }
    }
    out
}

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
        let mut s = String::from("<Project Sdk=\"Microsoft.NET.Sdk\"><ItemGroup>");
        for r in refs {
            s.push_str(&format!("<ProjectReference Include=\"{r}\" />"));
        }
        s.push_str("</ItemGroup></Project>");
        s
    }

    #[test]
    fn allows_referenced_blocks_unreferenced() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        // Extensions references Core; Other references nothing.
        write(&root.join("Source/Core/Core.csproj"), &csproj(&[]));
        write(
            &root.join("Source/Extensions/Extensions.csproj"),
            &csproj(&["..\\Core\\Core.csproj"]),
        );
        write(&root.join("Source/Other/Other.csproj"), &csproj(&[]));

        let sc = ProjectScope::load(root);
        assert!(!sc.is_empty());
        // Extensions → Core: directly referenced, allowed.
        assert!(sc.allows("Source/Extensions/X.cs", "Source/Core/Y.cs"));
        // Extensions → Other: NOT referenced, blocked (the over-link case).
        assert!(!sc.allows("Source/Extensions/X.cs", "Source/Other/Z.cs"));
        // Same project always allowed.
        assert!(sc.allows("Source/Core/A.cs", "Source/Core/B.cs"));
        // Reverse (Core → Extensions): not referenced, blocked.
        assert!(!sc.allows("Source/Core/A.cs", "Source/Extensions/X.cs"));
    }

    #[test]
    fn transitive_reference_is_blocked() {
        // A → B → C (direct refs). A → C must be blocked: not a DIRECT
        // reference, so not an architecture edge (matches the direct-edge GT).
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        write(&root.join("C/C.csproj"), &csproj(&[]));
        write(&root.join("B/B.csproj"), &csproj(&["..\\C\\C.csproj"]));
        write(&root.join("A/A.csproj"), &csproj(&["..\\B\\B.csproj"]));
        let sc = ProjectScope::load(root);
        assert!(sc.allows("A/x.cs", "B/y.cs"), "direct A→B allowed");
        assert!(sc.allows("B/y.cs", "C/z.cs"), "direct B→C allowed");
        assert!(!sc.allows("A/x.cs", "C/z.cs"), "transitive A→C blocked");
    }

    #[test]
    fn empty_scope_allows_everything() {
        let tmp = TempDir::new().unwrap();
        let sc = ProjectScope::load(tmp.path());
        assert!(sc.is_empty());
        assert!(sc.allows("x/a.cs", "y/b.cs"));
    }
}
