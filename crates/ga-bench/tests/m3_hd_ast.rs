//! S-005 cycle A — Hd-ast rule (`ga_dead_code` GT).
//!
//! Per spec:
//! - AS-014.T1: GT uses extract_calls + extract_references (raw AST), with
//!   (name, file) identity matching the S-003 fix on the def side.
//! - AS-015.T1: entry-point set excludes name-based (main/__main__) +
//!   per-file (__all__) + manifest-based (pyproject scripts, Cargo bins).
//!   Route handlers ship in S-005 cycle B (single-source refactor).
//! - AS-015.T2: policy_bias() lists known gaps (clap/Cobra/pub use/route
//!   handlers) so retrievers fail honestly per C4 honest-fail.

use ga_bench::gt_gen::hd_ast::HdAst;
use ga_bench::gt_gen::GtRule;
use ga_index::Store;
use serde_json::Value;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn empty_store(repo: &Path) -> (Store, TempDir) {
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let store = Store::open_with_root(&cache, repo).unwrap();
    (store, tmp)
}

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

/// Helper — assert a generated task has the given (name, file) and
/// `expected_dead` boolean, returning the task for further inspection.
fn task_for(tasks: &[ga_bench::gt_gen::GeneratedTask], name: &str, file: &str) -> Value {
    let task = tasks
        .iter()
        .find(|t| {
            t.query.get("name").and_then(|v| v.as_str()) == Some(name)
                && t.query.get("file").and_then(|v| v.as_str()) == Some(file)
        })
        .unwrap_or_else(|| {
            panic!(
                "no task for ({name}, {file}); tasks: {:?}",
                tasks
                    .iter()
                    .map(|t| (
                        t.query.get("name").and_then(|v| v.as_str()),
                        t.query.get("file").and_then(|v| v.as_str())
                    ))
                    .collect::<Vec<_>>()
            )
        });
    task.query.clone()
}

#[test]
fn as_014_t1_def_with_zero_callers_marked_dead() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    write(
        &repo.join("util.py"),
        "def dead_helper(x):\n    return x + 1\n\ndef live_helper(x):\n    return x - 1\n",
    );
    write(
        &repo.join("driver.py"),
        "from util import live_helper\n\ndef driver():\n    return live_helper(1)\n",
    );

    let (store, _t) = empty_store(repo);
    let tasks = HdAst.scan(&store, repo).expect("hd_ast scan must succeed");

    let dead_task = task_for(&tasks, "dead_helper", "util.py");
    assert_eq!(
        dead_task.get("expected_dead").and_then(|v| v.as_bool()),
        Some(true),
        "dead_helper has zero callers → expected_dead=true; got: {dead_task}"
    );

    let live_task = task_for(&tasks, "live_helper", "util.py");
    assert_eq!(
        live_task.get("expected_dead").and_then(|v| v.as_bool()),
        Some(false),
        "live_helper has a caller in driver.py → expected_dead=false"
    );
}

#[test]
fn as_014_t1_references_count_as_targeting_too() {
    // A reference (e.g. assigning a function as a value) counts the same
    // as a call site for liveness — extract_references covers map values,
    // shorthand, etc.
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    write(&repo.join("a.py"), "def passed_as_arg():\n    return 1\n");
    write(
        &repo.join("b.py"),
        "from a import passed_as_arg\n\ndef driver():\n    return [passed_as_arg]\n",
    );

    let (store, _t) = empty_store(repo);
    let tasks = HdAst.scan(&store, repo).unwrap();
    let t = task_for(&tasks, "passed_as_arg", "a.py");
    assert_eq!(
        t.get("expected_dead").and_then(|v| v.as_bool()),
        Some(false),
        "function used as array element should be live; got: {t}"
    );
}

#[test]
fn as_014_t2_identity_is_name_file_tuple() {
    // Two homonyms: a.py::foo (no callers) + b.py::foo (called locally).
    // Per S-003, GT must list a.py::foo as expected_dead=true and
    // b.py::foo as expected_dead=false — they are distinct entries.
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    write(&repo.join("a.py"), "def foo():\n    return 1\n");
    write(
        &repo.join("b.py"),
        "def foo():\n    return 2\n\ndef driver():\n    return foo()\n",
    );

    let (store, _t) = empty_store(repo);
    let tasks = HdAst.scan(&store, repo).unwrap();

    // S-005 cycle B': per-file resolution matches production S-003.
    // a.py::foo has zero callers (no intra-file call, no import from
    // b.py to a.py::foo) → expected_dead=true. b.py::foo is called
    // intra-file → expected_dead=false. Two distinct GT entries.
    let a_foo = task_for(&tasks, "foo", "a.py");
    let b_foo = task_for(&tasks, "foo", "b.py");
    assert_eq!(
        a_foo.get("expected_dead").and_then(|v| v.as_bool()),
        Some(true),
        "cycle B': a.py::foo has zero resolved callers → dead; got: {a_foo}"
    );
    assert_eq!(
        b_foo.get("expected_dead").and_then(|v| v.as_bool()),
        Some(false),
        "b.py::foo is intra-file called → live; got: {b_foo}"
    );
    assert_ne!(
        a_foo, b_foo,
        "(name,file) identity must yield 2 distinct tasks"
    );
}

#[test]
fn as_015_t1_main_function_excluded_as_entry_point() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    write(&repo.join("app.py"), "def main():\n    return 0\n");

    let (store, _t) = empty_store(repo);
    let tasks = HdAst.scan(&store, repo).unwrap();
    let t = task_for(&tasks, "main", "app.py");
    assert_eq!(
        t.get("expected_dead").and_then(|v| v.as_bool()),
        Some(false),
        "`main` must be excluded as entry-point even with zero callers"
    );
    assert_eq!(
        t.get("entry_point_kind").and_then(|v| v.as_str()),
        Some("main"),
        "entry_point_kind must record why this def is exempted; got: {t}"
    );
}

#[test]
fn as_015_t1_dunder_all_export_excluded_as_entry_point() {
    // Production semantic (ga_query::entry_points::collect_dunder_all):
    // only `__init__.py` files are scanned. The export covers any sibling
    // file in the same package via `same_package`.
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    write(&repo.join("api/__init__.py"), "__all__ = ['public_api']\n");
    write(
        &repo.join("api/core.py"),
        "def public_api():\n    return 1\n",
    );

    let (store, _t) = empty_store(repo);
    let tasks = HdAst.scan(&store, repo).unwrap();
    let t = task_for(&tasks, "public_api", "api/core.py");
    assert_eq!(
        t.get("expected_dead").and_then(|v| v.as_bool()),
        Some(false),
        "__all__-listed symbols are public API → exempt; got: {t}"
    );
    assert_eq!(
        t.get("entry_point_kind").and_then(|v| v.as_str()),
        Some("dunder_all"),
    );
}

#[test]
fn as_015_t1_pyproject_scripts_entry_excluded_as_entry_point() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    write(
        &repo.join("pyproject.toml"),
        "[project.scripts]\nmycli = \"mypkg.cli:run_cli\"\n",
    );
    write(&repo.join("mypkg/cli.py"), "def run_cli():\n    return 0\n");

    let (store, _t) = empty_store(repo);
    let tasks = HdAst.scan(&store, repo).unwrap();
    let t = task_for(&tasks, "run_cli", "mypkg/cli.py");
    assert_eq!(
        t.get("expected_dead").and_then(|v| v.as_bool()),
        Some(false),
        "pyproject [project.scripts] target → exempt; got: {t}"
    );
    assert_eq!(
        t.get("entry_point_kind").and_then(|v| v.as_str()),
        Some("project_scripts"),
    );
}

#[test]
fn as_015_t1_cargo_bin_entry_excluded_as_entry_point() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    // Cargo.toml [[bin]] declares an entry point; convention is that the
    // bin's main()-like function is named after the bin or `main` in the
    // bin file. Spec's ga_dead_code shipped behaviour treats the bin
    // file's `main` (and any `pub fn` reachable from it) as live; we
    // mirror that with name-based "main" + the bin's declared name.
    write(
        &repo.join("Cargo.toml"),
        "[package]\nname=\"x\"\nversion=\"0\"\n\n[[bin]]\nname = \"mytool\"\npath = \"src/bin/mytool.rs\"\n",
    );
    write(
        &repo.join("src/bin/mytool.rs"),
        "fn main() {}\nfn helper() {}\n",
    );

    let (store, _t) = empty_store(repo);
    let tasks = HdAst.scan(&store, repo).unwrap();
    let main_task = task_for(&tasks, "main", "src/bin/mytool.rs");
    assert_eq!(
        main_task.get("expected_dead").and_then(|v| v.as_bool()),
        Some(false),
        "main() in a Cargo bin is an entry point; got: {main_task}"
    );
    assert!(
        matches!(
            main_task.get("entry_point_kind").and_then(|v| v.as_str()),
            Some("main") | Some("cargo_bin"),
        ),
        "main() in src/bin/* should record entry_point_kind=main or cargo_bin; got: {main_task}"
    );
}

#[test]
fn as_015_t2_policy_bias_lists_known_gaps() {
    let rule = HdAst;
    let bias = rule.policy_bias();
    let lower = bias.to_lowercase();
    // Post-cycle-B: route handlers are SHIPPED (gin/django/rails/axum/nest);
    // remaining honest gaps are in derive macros / re-exports per AS-015.T2.
    for gap in ["clap", "cobra", "pub use"] {
        assert!(
            lower.contains(&gap.to_lowercase()),
            "policy_bias must list `{gap}` as a known gap; got: {bias}"
        );
    }
    // Verify cycle B coverage is named so reviewers can confirm what's done.
    for shipped in ["gin", "django", "rails", "axum", "nest"] {
        assert!(
            lower.contains(shipped),
            "policy_bias must name cycle-B-shipped framework `{shipped}`; got: {bias}"
        );
    }
}

#[test]
fn as_014_t1_class_definitions_emitted_with_kind() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path();
    write(
        &repo.join("mod.py"),
        "class MyClass:\n    def method(self):\n        return 1\n",
    );

    let (store, _t) = empty_store(repo);
    let tasks = HdAst.scan(&store, repo).unwrap();
    let cls = task_for(&tasks, "MyClass", "mod.py");
    let kind = cls.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    assert_eq!(
        kind, "class",
        "class def must record kind=class; got: {cls}"
    );
}

#[test]
fn as_014_t1_uc_and_id_match_spec() {
    let r = HdAst;
    assert_eq!(r.uc(), "dead_code");
    assert_eq!(r.id(), "Hd-ast");
}
