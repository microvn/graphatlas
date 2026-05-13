//! KG-1 — TESTED_BY edge emission.
//!
//! Regression: ga-query/src/indexer.rs — schema + query were wired since
//! M2 landed, but edges were never emitted. `affected_tests.rs:10` self-
//! documented the miss: "currently empty because the indexer doesn't
//! emit them". Measured impact pre-fix: django/nest/ts-eslint
//! test_recall 0.10–0.44 on the 24-task uc-impact corpus.
//!
//! Emission rule: for each resolved CALLS edge (caller → callee), emit
//! `TESTED_BY (callee → caller)` when caller.file matches a test-path
//! convention AND callee.file does NOT match (prod callee only) AND
//! callee is non-external. See graphatlas-tools.md Known Shipped Gaps
//! KG-1 and `affected_tests.rs::is_test_path` for the path rules.
//!
//! Direction reminder: TESTED_BY points FROM production symbol TO the
//! test symbol that exercises it — matches the query in
//! `impact/affected_tests.rs:40` (`prod -[TESTED_BY]-> test`).

use ga_index::Store;
use ga_query::indexer::build_index;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn setup(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    (cache, repo)
}

fn write(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn tested_by_pairs(store: &Store, prod_name: &str) -> Vec<(String, String)> {
    let conn = store.connection().unwrap();
    let cypher = format!(
        "MATCH (p:Symbol {{name: '{prod_name}'}})-[:TESTED_BY]->(t:Symbol) \
         RETURN t.name, t.file"
    );
    let rs = conn.query(&cypher).unwrap();
    rs.into_iter()
        .filter_map(|r| {
            let cols: Vec<lbug::Value> = r.into_iter().collect();
            match (cols.first(), cols.get(1)) {
                (Some(lbug::Value::String(n)), Some(lbug::Value::String(f))) => {
                    Some((n.clone(), f.clone()))
                }
                _ => None,
            }
        })
        .collect()
}

#[test]
fn tested_by_emitted_for_python_test_calling_prod() {
    // test_mod.py::test_process_user calls process_user in mod.py.
    // Expected: TESTED_BY(process_user → test_process_user).
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("mod.py"), "def process_user(u):\n    return u\n");
    write(
        &repo.join("test_mod.py"),
        "from mod import process_user\n\n\
         def test_process_user():\n    assert process_user(1) == 1\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let pairs = tested_by_pairs(&store, "process_user");
    assert_eq!(
        pairs.len(),
        1,
        "expected exactly one TESTED_BY edge, got {pairs:?}"
    );
    assert_eq!(pairs[0].0, "test_process_user");
    assert_eq!(pairs[0].1, "test_mod.py");
}

#[test]
fn tested_by_not_emitted_when_caller_is_prod_code() {
    // production→production call must NOT create TESTED_BY.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("a.py"),
        "def inner():\n    return 1\n\ndef outer():\n    return inner()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let pairs = tested_by_pairs(&store, "inner");
    assert!(
        pairs.is_empty(),
        "production caller must not emit TESTED_BY, got {pairs:?}"
    );
}

#[test]
fn tested_by_not_emitted_when_callee_is_also_test_helper() {
    // test file calling a test helper (both test files) must NOT emit.
    // Spec semantic: TESTED_BY surfaces "tests that cover production
    // code", not test-to-test helper relationships.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("test_helpers.py"),
        "def make_fixture():\n    return {}\n",
    );
    write(
        &repo.join("test_consumer.py"),
        "from test_helpers import make_fixture\n\n\
         def test_with_fixture():\n    assert make_fixture() == {}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let pairs = tested_by_pairs(&store, "make_fixture");
    assert!(
        pairs.is_empty(),
        "test-helper callee must not be TESTED_BY target, got {pairs:?}"
    );
}

#[test]
fn tested_by_covers_java_test_class_suffix_convention() {
    // v1.1-M4 — Java *Test.java + Maven src/test/ layout.
    // Pre-fix (before commit 36cdb11) the indexer rejected every Java
    // test path → zero TESTED_BY edges in the graph for any Java
    // fixture. This test pins the post-fix contract.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("src/main/java/foo/UserRepository.java"),
        "package foo;\npublic class UserRepository {\n  public void findById(long id) {}\n}\n",
    );
    write(
        &repo.join("src/test/java/foo/UserRepositoryTest.java"),
        "package foo;\nimport foo.UserRepository;\n\
         public class UserRepositoryTest {\n  void run() { new UserRepository().findById(1L); }\n}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    // findById is the prod callee; UserRepositoryTest::run calls it
    // → TESTED_BY(findById → run) expected.
    let pairs = tested_by_pairs(&store, "findById");
    assert!(
        !pairs.is_empty(),
        "expected ≥1 TESTED_BY edge for Java convention, got empty"
    );
    let test_files: Vec<&str> = pairs.iter().map(|p| p.1.as_str()).collect();
    assert!(
        test_files
            .iter()
            .any(|f| f.ends_with("UserRepositoryTest.java")),
        "TESTED_BY target file must be the Java *Test.java sibling: {pairs:?}"
    );
}

#[test]
fn tested_by_covers_csharp_test_class_suffix_convention() {
    // v1.1-M4 S-003c — C# *Tests.cs / *Test.cs convention (xUnit/NUnit/MSTest).
    // is_test_path canonical (post §4.2.6 refactor) recognizes both suffixes.
    // This test pins the integration contract so a future regression in the
    // canonical is_test_path is caught here, not via M2 score drift on a
    // future C# fixture.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("src/App/UserRepository.cs"),
        "namespace App;\npublic class UserRepository {\n  public void FindById(long id) {}\n}\n",
    );
    write(
        &repo.join("test/App.Tests/UserRepositoryTests.cs"),
        "namespace App.Tests;\nusing App;\n\
         public class UserRepositoryTests {\n  void Run() { new UserRepository().FindById(1L); }\n}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let pairs = tested_by_pairs(&store, "FindById");
    assert!(
        !pairs.is_empty(),
        "expected ≥1 TESTED_BY edge for C# convention, got empty"
    );
    let test_files: Vec<&str> = pairs.iter().map(|p| p.1.as_str()).collect();
    assert!(
        test_files
            .iter()
            .any(|f| f.ends_with("UserRepositoryTests.cs")),
        "TESTED_BY target must be the C# *Tests.cs sibling: {pairs:?}"
    );
}

#[test]
fn tested_by_covers_ruby_spec_suffix_convention() {
    // v1.1-M4 S-004c — Ruby `_spec.rb` (RSpec) + `_test.rb` (Minitest)
    // conventions. is_test_path canonical (post §4.2.6 refactor) recognizes
    // both suffixes plus the generic `spec/` / `test/` path segment. This
    // test pins the integration contract so a future regression in the
    // canonical is_test_path is caught here, not via M2 score drift on a
    // future Ruby fixture.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("app/models/user_repository.rb"),
        "module App\n  class UserRepository\n    def find_by_id(id)\n      id\n    end\n  end\nend\n",
    );
    write(
        &repo.join("spec/models/user_repository_spec.rb"),
        "require_relative '../../app/models/user_repository'\n\
         module App\n  class UserRepositorySpec\n    def run\n      \
         UserRepository.new.find_by_id(1)\n    end\n  end\nend\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let pairs = tested_by_pairs(&store, "find_by_id");
    assert!(
        !pairs.is_empty(),
        "expected ≥1 TESTED_BY edge for Ruby `_spec.rb` convention, got empty"
    );
    let test_files: Vec<&str> = pairs.iter().map(|p| p.1.as_str()).collect();
    assert!(
        test_files
            .iter()
            .any(|f| f.ends_with("user_repository_spec.rb")),
        "TESTED_BY target file must be the Ruby `_spec.rb` sibling: {pairs:?}"
    );
}

#[test]
fn tested_by_covers_kotlin_test_class_suffix_convention() {
    // v1.1-M4 S-002c — Kotlin *Test.kt + Gradle src/test/ layout.
    // Mirror of `tested_by_covers_java_test_class_suffix_convention`.
    // is_test_path Kotlin coverage was added in commit 36cdb11 (lock-step
    // sweep alongside Java); this test pins the integration contract so a
    // future regression is caught here, not via M2 gate score drift on a
    // Kotlin fixture.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("src/main/kotlin/foo/UserRepository.kt"),
        "package foo\nclass UserRepository {\n  fun findById(id: Long) {}\n}\n",
    );
    write(
        &repo.join("src/test/kotlin/foo/UserRepositoryTest.kt"),
        "package foo\nimport foo.UserRepository\n\
         class UserRepositoryTest {\n  fun run() { UserRepository().findById(1L) }\n}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    // findById is the prod callee; UserRepositoryTest::run calls it
    // → TESTED_BY(findById → run) expected.
    let pairs = tested_by_pairs(&store, "findById");
    assert!(
        !pairs.is_empty(),
        "expected ≥1 TESTED_BY edge for Kotlin convention, got empty"
    );
    let test_files: Vec<&str> = pairs.iter().map(|p| p.1.as_str()).collect();
    assert!(
        test_files
            .iter()
            .any(|f| f.ends_with("UserRepositoryTest.kt")),
        "TESTED_BY target file must be the Kotlin *Test.kt sibling: {pairs:?}"
    );
}

#[test]
fn tested_by_covers_go_convention_test_files() {
    // Go uses *_test.go convention; ensure the test-path matcher
    // covers it the same way python test_*.py does.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("svc.go"),
        "package svc\n\nfunc Process(x int) int { return x }\n",
    );
    write(
        &repo.join("svc_test.go"),
        "package svc\n\nfunc TestProcess(t *testing.T) { Process(1) }\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let pairs = tested_by_pairs(&store, "Process");
    assert_eq!(
        pairs.len(),
        1,
        "expected one TESTED_BY edge for Go convention, got {pairs:?}"
    );
    assert_eq!(pairs[0].0, "TestProcess");
    assert_eq!(pairs[0].1, "svc_test.go");
}
