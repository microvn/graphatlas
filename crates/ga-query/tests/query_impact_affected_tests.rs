//! Tools S-006 cluster C4 — `affected_tests` field: convention-matched test
//! files whose path hints at the seed symbol or its defining file.
//!
//! Edge-based surfacing (TESTED_BY) is exercised by the `_via_edge_` tests
//! below. EXP-M2-05 extends the edge path to a directed CALLS*1..3 transitive
//! chain before the TESTED_BY hop.

use ga_index::Store;
use ga_query::{impact, indexer::build_index, AffectedTestReason, ImpactRequest};
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

fn run(store: &Store, symbol: &str) -> ga_query::ImpactResponse {
    impact(
        store,
        &ImpactRequest {
            symbol: Some(symbol.into()),
            ..Default::default()
        },
    )
    .unwrap()
}

fn paths(resp: &ga_query::ImpactResponse) -> Vec<String> {
    resp.affected_tests.iter().map(|t| t.path.clone()).collect()
}

// v1.1-M4 — git-history-based co-change signal. RED: test files that
// have NO structural edge to the seed (no CALLS, no IMPORTS, no name
// match in path, not in same package) but co-change with the seed
// file in recent git history MUST surface. Mirrors mockito's GT
// shape: cross-package / cross-module test files that get touched
// alongside seed_file in fix commits.
#[test]
fn co_change_signal_surfaces_unrelated_named_test() {
    use std::process::Command;
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);

    // Seed file + a structurally-unrelated test file. Critically, the
    // test path does NOT contain the seed symbol name, the seed stem,
    // and is not in any Maven/co-package layout — only co-change can
    // surface it.
    write(&repo.join("alpha.py"), "def alpha():\n    pass\n");
    write(
        &repo.join("tests/test_unrelated_zzz.py"),
        "def test_zzz():\n    pass\n",
    );
    // Init repo + 3 commits where both files change together.
    Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(&repo)
        .status()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "t@t"])
        .current_dir(&repo)
        .status()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "t"])
        .current_dir(&repo)
        .status()
        .unwrap();
    for i in 0..3 {
        // mutate both
        std::fs::write(
            repo.join("alpha.py"),
            format!("# v{i}\ndef alpha():\n    pass\n"),
        )
        .unwrap();
        std::fs::write(
            repo.join("tests/test_unrelated_zzz.py"),
            format!("# v{i}\ndef test_zzz():\n    pass\n"),
        )
        .unwrap();
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(&repo)
            .status()
            .unwrap();
        Command::new("git")
            .args(["commit", "-q", "-m", &format!("c{i}")])
            .current_dir(&repo)
            .status()
            .unwrap();
    }

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();
    let resp = run(&store, "alpha");
    let p = paths(&resp);
    assert!(
        p.contains(&"tests/test_unrelated_zzz.py".to_string()),
        "co-change signal should surface unrelated test file co-changed in 3 commits: {p:?}"
    );
}

// v1.1-M4 — text-fallback signal. RED: when structural signals yield
// no test files (no CALLS chain, no IMPORTS edge, no convention match,
// no co-package mirror, no co-change), as a final layer of recall,
// any test file whose CONTENT mentions the seed symbol must surface.
// Steals ripgrep's lever for the test_recall dim without disturbing
// other dims (text-fallback only fires when by_path is otherwise empty).
#[test]
fn text_fallback_surfaces_test_mentioning_seed_in_content() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("alpha.py"), "def alpha():\n    pass\n");
    // Test path doesn't mention `alpha` (filename = test_qqq.py),
    // no co-package, no import, no calls — only text content matches.
    write(
        &repo.join("tests/test_qqq.py"),
        "# uses alpha indirectly via mocking framework\n\
         def test_q():\n    \"\"\"alpha behaviour\"\"\"\n    pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();
    let resp = run(&store, "alpha");
    let p = paths(&resp);
    assert!(
        p.contains(&"tests/test_qqq.py".to_string()),
        "text-fallback should surface test file whose content mentions `alpha`: {p:?}"
    );
}

// v1.1-M4 — Java test convention. The TDD-RED case for the
// is_test_path Java fix (commit 36cdb11): without Java arms in
// is_test_path, this test would have failed with `affected_tests`
// returning [] (no convention match because *Test.java didn't pass
// the test-path filter). Pins the contract at integration level so a
// future regression in is_test_path is caught here, not via M2 gate
// score drift.
#[test]
fn java_class_test_suffix_convention_matches() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("src/main/java/foo/UserRepository.java"),
        "package foo;\npublic class UserRepository {}\n",
    );
    write(
        &repo.join("src/test/java/foo/UserRepositoryTest.java"),
        "package foo;\nimport foo.UserRepository;\npublic class UserRepositoryTest {}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "UserRepository");
    let p = paths(&resp);
    assert!(
        p.contains(&"src/test/java/foo/UserRepositoryTest.java".to_string()),
        "Java *Test.java sibling must surface via convention match: {p:?}"
    );
}

// v1.1-M4 S-003c — C# test convention. Mirror of Java/Kotlin tests.
// Pins integration contract for C# *Tests.cs / xUnit-style layout. Canonical
// is_test_path (post §4.2.6 refactor) covers both suffix.
#[test]
fn csharp_class_test_suffix_convention_matches() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("src/App/UserRepository.cs"),
        "namespace App;\npublic class UserRepository {}\n",
    );
    write(
        &repo.join("test/App.Tests/UserRepositoryTests.cs"),
        "namespace App.Tests;\nusing App;\npublic class UserRepositoryTests {}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "UserRepository");
    let p = paths(&resp);
    assert!(
        p.contains(&"test/App.Tests/UserRepositoryTests.cs".to_string()),
        "C# *Tests.cs sibling must surface via convention match: {p:?}"
    );
}

// v1.1-M4 S-002c — Kotlin test convention. Mirror of
// `java_class_test_suffix_convention_matches`. Pins the integration
// contract for Kotlin *Test.kt + Gradle src/test/ layout. is_test_path
// Kotlin row landed in commit 36cdb11; this test guarantees a future
// regression in is_test_path is caught here, not via M2 score drift.
#[test]
fn kotlin_class_test_suffix_convention_matches() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("src/main/kotlin/foo/UserRepository.kt"),
        "package foo\nclass UserRepository\n",
    );
    write(
        &repo.join("src/test/kotlin/foo/UserRepositoryTest.kt"),
        "package foo\nimport foo.UserRepository\nclass UserRepositoryTest\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "UserRepository");
    let p = paths(&resp);
    assert!(
        p.contains(&"src/test/kotlin/foo/UserRepositoryTest.kt".to_string()),
        "Kotlin *Test.kt sibling must surface via convention match: {p:?}"
    );
}

// v1.1-M4 S-004c — Ruby Rails convention. AS-013 spec contract:
// `app/models/user.rb` ↔ `spec/models/user_spec.rb` must match. Canonical
// is_test_path (post §4.2.6 refactor) recognizes `_spec.rb` / `_test.rb`
// suffixes and the `spec/` / `test/` path segment. Pins the Ruby row at
// integration level so a future regression in is_test_path surfaces here.
#[test]
fn ruby_spec_suffix_convention_matches() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("app/models/user.rb"),
        "module App\n  class User\n  end\nend\n",
    );
    write(
        &repo.join("spec/models/user_spec.rb"),
        "require_relative '../../app/models/user'\n\
         module App\n  class UserSpec\n  end\nend\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "User");
    let p = paths(&resp);
    assert!(
        p.contains(&"spec/models/user_spec.rb".to_string()),
        "Ruby `_spec.rb` sibling must surface via convention match (Rails idiom): {p:?}"
    );
}

#[test]
fn python_test_prefix_convention_matches() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("foo.py"), "def foo():\n    pass\n");
    write(&repo.join("test_foo.py"), "def test_foo():\n    pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "foo");
    assert_eq!(paths(&resp), vec!["test_foo.py".to_string()]);
    assert_eq!(
        resp.affected_tests[0].reason,
        AffectedTestReason::Convention
    );
}

#[test]
fn python_underscore_test_suffix_and_tests_dir() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("auth.py"), "def check_password():\n    pass\n");
    write(
        &repo.join("auth_test.py"),
        "def test_check_password():\n    pass\n",
    );
    write(
        &repo.join("tests/test_auth.py"),
        "def test_check():\n    pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "check_password");
    let mut p = paths(&resp);
    p.sort();
    assert_eq!(
        p,
        vec!["auth_test.py".to_string(), "tests/test_auth.py".to_string()]
    );
}

#[test]
fn typescript_dot_test_and_tests_dir_convention() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("users.ts"), "export function handleUsers() {}\n");
    write(
        &repo.join("users.test.ts"),
        "import { handleUsers } from './users';\ntest('x', () => { handleUsers(); });\n",
    );
    write(
        &repo.join("__tests__/users.ts"),
        "import { handleUsers } from '../users';\ntest('y', () => { handleUsers(); });\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "handleUsers");
    let mut p = paths(&resp);
    p.sort();
    assert_eq!(
        p,
        vec![
            "__tests__/users.ts".to_string(),
            "users.test.ts".to_string()
        ]
    );
}

#[test]
fn go_underscore_test_suffix_convention() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("handler.go"),
        "package main\n\nfunc CreateUser() {}\n",
    );
    write(
        &repo.join("handler_test.go"),
        "package main\n\nfunc TestCreateUser(t *testing.T) { CreateUser() }\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "CreateUser");
    assert_eq!(paths(&resp), vec!["handler_test.go".to_string()]);
}

#[test]
fn non_test_file_is_not_surfaced() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    // `foo_helper.py` looks Python but is not a test file.
    write(&repo.join("foo.py"), "def foo():\n    pass\n");
    write(
        &repo.join("foo_helper.py"),
        "def foo_utility():\n    pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "foo");
    assert!(
        paths(&resp).is_empty(),
        "non-test file must not appear: {:?}",
        resp.affected_tests
    );
}

#[test]
fn unrelated_test_file_does_not_match() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("foo.py"), "def foo():\n    pass\n");
    // Test file exists but covers something else — path doesn't contain "foo".
    write(&repo.join("test_bar.py"), "def test_bar():\n    pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "foo");
    assert!(
        paths(&resp).is_empty(),
        "unrelated test must be filtered out: {:?}",
        resp.affected_tests
    );
}

#[test]
fn results_are_deduped_and_sorted() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("foo.py"), "def foo():\n    pass\n");
    write(&repo.join("test_foo.py"), "def test_foo():\n    pass\n");
    write(&repo.join("tests/test_foo.py"), "def test_a():\n    pass\n");
    write(&repo.join("foo_test.py"), "def test_b():\n    pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "foo");
    let p = paths(&resp);
    let mut sorted = p.clone();
    sorted.sort();
    assert_eq!(p, sorted, "must be sorted by path");
    let mut unique = p.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(p.len(), unique.len(), "no duplicate paths");
    assert!(p.contains(&"test_foo.py".to_string()));
}

#[test]
fn non_ident_seed_symbol_returns_empty_tests() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("foo.py"), "def foo():\n    pass\n");
    write(&repo.join("test_foo.py"), "def test_foo():\n    pass\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "fo'o"); // Tools-C9-d allowlist rejects quote
    assert!(resp.affected_tests.is_empty());
}

#[test]
fn affected_tests_via_edge_transitive_chain_3_hops() {
    // EXP-M2-05: a non-test seed that transitively calls a production symbol
    // with a TESTED_BY edge should surface the test file via the Edge reason.
    // Pre-fix: returns empty (seed_is_test=false skips KG-10 path, direct
    // TESTED_BY WHERE prod=seed finds nothing, convention path doesn't match
    // because test_compute.py doesn't contain "orchestrate" or its stem).
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("orchestrate.py"),
        "from helper import helper\n\ndef orchestrate():\n    helper()\n",
    );
    write(
        &repo.join("helper.py"),
        "from compute import compute\n\ndef helper():\n    compute()\n",
    );
    write(&repo.join("compute.py"), "def compute():\n    pass\n");
    write(
        &repo.join("test_compute.py"),
        "from compute import compute\n\ndef test_compute():\n    compute()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "orchestrate");
    let paths = paths(&resp);
    assert!(
        paths.contains(&"test_compute.py".to_string()),
        "expected transitive chain to surface test_compute.py, got: {:?}",
        paths,
    );
    let edge_test = resp
        .affected_tests
        .iter()
        .find(|t| t.path == "test_compute.py")
        .unwrap();
    assert_eq!(
        edge_test.reason,
        AffectedTestReason::Edge,
        "transitive chain hit must use Edge reason, not Convention"
    );
}

#[test]
fn affected_tests_via_edge_direct_one_hop() {
    // Sanity: single-hop CALLS before TESTED_BY should also surface via Edge,
    // even when seed is not a test file and not identical to the production
    // symbol with the TESTED_BY edge.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(
        &repo.join("caller.py"),
        "from target import target\n\ndef caller():\n    target()\n",
    );
    write(&repo.join("target.py"), "def target():\n    pass\n");
    write(
        &repo.join("test_target.py"),
        "from target import target\n\ndef test_target():\n    target()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "caller");
    let paths = paths(&resp);
    assert!(
        paths.contains(&"test_target.py".to_string()),
        "expected 1-hop chain caller->target-[TESTED_BY]->test_target.py, got: {:?}",
        paths,
    );
    let edge_test = resp
        .affected_tests
        .iter()
        .find(|t| t.path == "test_target.py")
        .unwrap();
    assert_eq!(edge_test.reason, AffectedTestReason::Edge);
}

#[test]
fn matches_by_seed_file_stem_not_just_symbol_name() {
    // Test path contains seed file stem ("auth") even though it doesn't
    // contain the symbol name ("check_password") verbatim.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("auth.py"), "def check_password():\n    pass\n");
    // Test file named after the MODULE, not the function — common pattern.
    write(
        &repo.join("test_auth.py"),
        "def test_something():\n    pass\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "check_password");
    assert_eq!(paths(&resp), vec!["test_auth.py".to_string()]);
}
