//! Auto-GT generation on a controlled fixture.

use ga_bench::gt_gen::h1_polymorphism::H1Polymorphism;
use ga_bench::gt_gen::h5_reexport::H5ReExport;
use ga_bench::gt_gen::{default_rules, generate_gt, GtRule};
use ga_index::Store;
use ga_query::indexer::build_index;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn fixture_with_polymorphism() -> (TempDir, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    // Base + two subclasses, all overriding `speak`.
    write(
        &repo.join("base.py"),
        "class Animal:\n    def speak(self): return 'quiet'\n",
    );
    write(
        &repo.join("dog.py"),
        "from base import Animal\n\nclass Dog(Animal):\n    def speak(self): return 'woof'\n\ndef call_dog():\n    d = Dog()\n    return d.speak()\n",
    );
    write(
        &repo.join("cat.py"),
        "from base import Animal\n\nclass Cat(Animal):\n    def speak(self): return 'meow'\n\ndef call_cat():\n    c = Cat()\n    return c.speak()\n",
    );
    (tmp, repo)
}

fn fixture_with_reexport() -> (TempDir, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    // leaf.ts ← barrel.ts re-exports ← two consumers import from barrel.
    write(
        &repo.join("leaf.ts"),
        "export function leafy() { return 1; }\n",
    );
    write(&repo.join("barrel.ts"), "export * from './leaf';\n");
    write(
        &repo.join("a.ts"),
        "import { leafy } from './barrel';\nexport const a = leafy();\n",
    );
    write(
        &repo.join("b.ts"),
        "import { leafy } from './barrel';\nexport const b = leafy();\n",
    );
    (tmp, repo)
}

#[test]
fn h1_polymorphism_rule_finds_override_sites() {
    let (tmp, repo) = fixture_with_polymorphism();
    let store = Store::open_with_root(&tmp.path().join(".graphatlas"), &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let tasks = H1Polymorphism.scan(&store, &repo).unwrap();
    assert!(
        !tasks.is_empty(),
        "H1 should emit ≥1 task for Animal.speak with Dog/Cat overrides"
    );
    // Expected tasks have `speak` as the symbol.
    let speak_task = tasks
        .iter()
        .find(|t| t.query["symbol"] == "speak")
        .expect("speak task must exist");
    assert_eq!(speak_task.rule, "H1-polymorphism");
    // Expected callers: call_dog + call_cat (the two sites invoking `.speak()`).
    assert!(speak_task.expected.contains(&"call_dog".to_string()));
    assert!(speak_task.expected.contains(&"call_cat".to_string()));
}

#[test]
fn h5_reexport_rule_finds_chain_importers() {
    let (tmp, repo) = fixture_with_reexport();
    let store = Store::open_with_root(&tmp.path().join(".graphatlas"), &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let tasks = H5ReExport.scan(&store, &repo).unwrap();
    // At least one task should target leaf.ts (both a.ts + b.ts reach it via
    // barrel.ts re-export chain).
    let leaf_task = tasks
        .iter()
        .find(|t| t.query["file"] == "leaf.ts")
        .expect("leaf.ts should be flagged as re-export chain target");
    assert_eq!(leaf_task.rule, "H5-reexport");
    assert!(leaf_task.expected.contains(&"a.ts".to_string()));
    assert!(leaf_task.expected.contains(&"b.ts".to_string()));
}

#[test]
fn generate_gt_combines_rules_and_emits_valid_ground_truth() {
    // Fixture has polymorphism only; H5 rule won't match → 0 importers tasks.
    let (tmp, repo) = fixture_with_polymorphism();
    let store = Store::open_with_root(&tmp.path().join(".graphatlas"), &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let rules = default_rules();
    let gt = generate_gt("callers", "poly-fixture", &store, &repo, &rules).unwrap();
    assert_eq!(gt.uc, "callers");
    assert_eq!(gt.fixture, "poly-fixture");
    assert_eq!(gt.schema_version, 1);
    assert!(!gt.tasks.is_empty(), "callers UC should get H1 tasks");
}

#[test]
fn rules_with_no_matches_emit_empty_task_list() {
    // Empty fixture — no classes, no imports → both rules emit nothing.
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    write(&repo.join("m.py"), "def f(): pass\n");
    let store = Store::open_with_root(&tmp.path().join(".graphatlas"), &repo).unwrap();
    build_index(&store, &repo).unwrap();

    assert!(H1Polymorphism.scan(&store, &repo).unwrap().is_empty());
    assert!(H5ReExport.scan(&store, &repo).unwrap().is_empty());
}

#[test]
fn generated_tasks_carry_rationale_for_human_verification() {
    let (tmp, repo) = fixture_with_polymorphism();
    let store = Store::open_with_root(&tmp.path().join(".graphatlas"), &repo).unwrap();
    build_index(&store, &repo).unwrap();
    let tasks = H1Polymorphism.scan(&store, &repo).unwrap();
    assert!(!tasks.is_empty());
    assert!(
        !tasks[0].rationale.is_empty(),
        "each generated task must carry a rationale explaining why the rule matched"
    );
}
