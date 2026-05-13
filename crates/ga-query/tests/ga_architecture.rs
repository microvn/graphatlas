//! S-005 ga_architecture — module map integration tests.
//!
//! Spec contract (graphatlas-v1.1-tools.md S-005):
//!   AS-014: Happy path — modules + inter-module edges weighted by call/
//!     import counts.
//!   AS-015: Architecture with depth limit — max_modules cap + meta
//!     {truncated, total_modules}.
//!   Tools-C6: meta.convention_used names which convention was applied.
//!
//! Module discovery conventions:
//!   - Python: directory containing `__init__.py`
//!   - Rust:   directory containing `Cargo.toml`
//!   - TS/JS:  directory containing `package.json`

use ga_index::Store;
use ga_query::architecture::{architecture, ArchitectureRequest};
use ga_query::indexer::build_index;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn setup() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    (tmp, cache, repo)
}

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

// ─────────────────────────────────────────────────────────────────────────
// AS-014 — Module map happy path
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn returns_one_module_per_python_package() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("auth/__init__.py"), "");
    write(&repo.join("auth/login.py"), "def login():\n    return 1\n");
    write(&repo.join("billing/__init__.py"), "");
    write(
        &repo.join("billing/charge.py"),
        "def charge():\n    return 1\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = architecture(&store, &ArchitectureRequest::default()).expect("ok");
    let names: Vec<&str> = resp.modules.iter().map(|m| m.name.as_str()).collect();
    assert!(
        names.contains(&"auth"),
        "expected `auth` module; got {names:?}"
    );
    assert!(
        names.contains(&"billing"),
        "expected `billing` module; got {names:?}"
    );
    assert_eq!(
        resp.modules.len(),
        2,
        "exactly 2 Python packages → 2 modules"
    );
}

#[test]
fn module_carries_files_list_and_symbol_count() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("svc/__init__.py"), "");
    write(&repo.join("svc/a.py"), "def a():\n    return 0\n");
    write(&repo.join("svc/b.py"), "def b():\n    return 0\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = architecture(&store, &ArchitectureRequest::default()).expect("ok");
    let svc = resp
        .modules
        .iter()
        .find(|m| m.name == "svc")
        .expect("svc module");
    assert!(
        svc.files.iter().any(|f| f == "svc/a.py"),
        "files must list svc/a.py; got {:?}",
        svc.files
    );
    assert!(
        svc.files.iter().any(|f| f == "svc/b.py"),
        "files must list svc/b.py; got {:?}",
        svc.files
    );
    assert!(
        svc.symbol_count >= 2,
        "symbol_count counts a + b ≥ 2; got {}",
        svc.symbol_count
    );
}

#[test]
fn cross_module_call_emits_calls_edge_with_weight() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("auth/__init__.py"), "");
    write(
        &repo.join("auth/check.py"),
        "def check_pw(pw):\n    return pw == 'ok'\n",
    );
    write(&repo.join("api/__init__.py"), "");
    write(
        &repo.join("api/login.py"),
        "from auth.check import check_pw\n\ndef login(pw):\n    return check_pw(pw)\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = architecture(&store, &ArchitectureRequest::default()).expect("ok");
    let calls_edge = resp
        .edges
        .iter()
        .find(|e| e.from == "api" && e.to == "auth" && e.kind == "calls")
        .expect("api → auth `calls` edge present");
    assert!(
        calls_edge.weight >= 1,
        "calls edge weight ≥ 1; got {}",
        calls_edge.weight
    );
}

#[test]
fn cross_module_import_emits_imports_edge() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("util/__init__.py"), "");
    write(
        &repo.join("util/helpers.py"),
        "def helper():\n    return 1\n",
    );
    write(&repo.join("svc/__init__.py"), "");
    write(
        &repo.join("svc/main.py"),
        "from util.helpers import helper\n\ndef run():\n    return helper()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = architecture(&store, &ArchitectureRequest::default()).expect("ok");
    let import_edge = resp
        .edges
        .iter()
        .find(|e| e.from == "svc" && e.to == "util" && e.kind == "imports");
    assert!(
        import_edge.is_some(),
        "svc → util imports edge expected; got edges {:?}",
        resp.edges
    );
}

#[test]
fn no_self_loop_edges_within_module() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("only/__init__.py"), "");
    write(
        &repo.join("only/a.py"),
        "def a():\n    return 1\n\ndef b():\n    return a()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = architecture(&store, &ArchitectureRequest::default()).expect("ok");
    let self_loops: Vec<_> = resp.edges.iter().filter(|e| e.from == e.to).collect();
    assert!(
        self_loops.is_empty(),
        "intra-module edges must NOT appear in inter-module map; got {:?}",
        self_loops
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Tools-C6 — meta.convention_used names which convention applied
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn meta_convention_used_python_init_py() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("pkg/__init__.py"), "");
    write(&repo.join("pkg/m.py"), "def m():\n    return 1\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = architecture(&store, &ArchitectureRequest::default()).expect("ok");
    assert!(
        resp.meta.convention_used.contains("python-init-py"),
        "convention_used must name python-init-py; got `{}`",
        resp.meta.convention_used
    );
}

// ─────────────────────────────────────────────────────────────────────────
// AS-015 — max_modules cap + meta.truncated + meta.total_modules
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn max_modules_caps_response_and_marks_truncated() {
    let (_tmp, cache, repo) = setup();
    for i in 0..5 {
        write(&repo.join(format!("m{i}/__init__.py")), "");
        write(
            &repo.join(format!("m{i}/file.py")),
            &format!("def fn_{i}():\n    return {i}\n"),
        );
    }
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = ArchitectureRequest {
        max_modules: Some(2),
    };
    let resp = architecture(&store, &req).expect("ok");
    assert_eq!(resp.modules.len(), 2, "max_modules=2 caps response");
    assert!(resp.meta.truncated, "meta.truncated must be true");
    assert_eq!(
        resp.meta.total_modules, 5,
        "meta.total_modules surfaces pre-cap total"
    );
}

#[test]
fn max_modules_selects_largest_modules_first() {
    let (_tmp, cache, repo) = setup();
    // Big module: 4 symbols.
    write(&repo.join("big/__init__.py"), "");
    for i in 0..4 {
        write(
            &repo.join(format!("big/f{i}.py")),
            &format!("def big_{i}():\n    return {i}\n"),
        );
    }
    // Small modules: 1 symbol each.
    for i in 0..3 {
        write(&repo.join(format!("small{i}/__init__.py")), "");
        write(
            &repo.join(format!("small{i}/x.py")),
            "def x():\n    return 0\n",
        );
    }
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = ArchitectureRequest {
        max_modules: Some(1),
    };
    let resp = architecture(&store, &req).expect("ok");
    assert_eq!(resp.modules.len(), 1);
    assert_eq!(
        resp.modules[0].name, "big",
        "top-1 by symbol_count must be `big`; got {}",
        resp.modules[0].name
    );
}

#[test]
fn no_truncation_when_max_modules_above_total() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("a/__init__.py"), "");
    write(&repo.join("a/x.py"), "def x():\n    return 0\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = ArchitectureRequest {
        max_modules: Some(100),
    };
    let resp = architecture(&store, &req).expect("ok");
    assert!(
        !resp.meta.truncated,
        "max_modules>total must NOT mark truncated"
    );
    assert_eq!(resp.meta.total_modules, 1);
}

// ─────────────────────────────────────────────────────────────────────────
// Edge cases — empty index, no modules, invalid input
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn empty_index_returns_index_not_ready() {
    let (_tmp, cache, repo) = setup();
    let store = Store::open_with_root(&cache, &repo).unwrap();
    let res = architecture(&store, &ArchitectureRequest::default());
    use ga_core::Error;
    assert!(
        matches!(res, Err(Error::IndexNotReady { .. })),
        "empty graph must Err IndexNotReady; got {res:?}"
    );
}

#[test]
fn flat_repo_with_no_module_markers_returns_empty_modules() {
    // Source files exist, but no __init__.py / Cargo.toml / package.json.
    let (_tmp, cache, repo) = setup();
    write(&repo.join("loose.py"), "def loose():\n    return 0\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = architecture(&store, &ArchitectureRequest::default()).expect("ok");
    assert!(
        resp.modules.is_empty(),
        "no module markers → empty modules; got {:?}",
        resp.modules
    );
    // Convention field still present, naming the negative result.
    assert!(
        !resp.meta.convention_used.is_empty(),
        "convention_used must always be set; got empty"
    );
}

#[test]
fn max_modules_zero_returns_invalid_params() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("a/__init__.py"), "");
    write(&repo.join("a/x.py"), "def x():\n    return 0\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = ArchitectureRequest {
        max_modules: Some(0),
    };
    let res = architecture(&store, &req);
    use ga_core::Error;
    assert!(
        matches!(res, Err(Error::InvalidParams(_))),
        "max_modules=0 is meaningless; got {res:?}"
    );
}

#[test]
fn modules_returned_in_deterministic_order() {
    let (_tmp, cache, repo) = setup();
    for n in ["zeta", "alpha", "mid"] {
        write(&repo.join(format!("{n}/__init__.py")), "");
        write(&repo.join(format!("{n}/x.py")), "def x():\n    return 0\n");
    }
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let a = architecture(&store, &ArchitectureRequest::default()).expect("ok");
    let b = architecture(&store, &ArchitectureRequest::default()).expect("ok");
    let names_a: Vec<_> = a.modules.iter().map(|m| &m.name).collect();
    let names_b: Vec<_> = b.modules.iter().map(|m| &m.name).collect();
    assert_eq!(names_a, names_b, "module order must be deterministic");
}
