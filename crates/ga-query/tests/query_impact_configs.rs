//! Tools S-006 cluster C6 — `affected_configs` field: env / yaml / toml / json
//! files where the seed symbol (or its file stem) is mentioned.

use ga_index::Store;
use ga_query::{impact, indexer::build_index, ImpactRequest};
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

#[test]
fn env_file_case_sensitive_symbol_match() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("auth.py"), "def check_password():\n    pass\n");
    write(
        &repo.join(".env"),
        "DATABASE_URL=postgres://x\n# documentation for check_password helper\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "check_password");
    let hits: Vec<_> = resp
        .affected_configs
        .iter()
        .filter(|c| c.path == ".env")
        .collect();
    assert_eq!(hits.len(), 1, "{:?}", resp.affected_configs);
    assert_eq!(hits[0].line, 2, "match on line 2 (1-indexed): {hits:?}");
}

#[test]
fn yaml_file_mention_surfaced() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("users.py"), "def handle_users():\n    pass\n");
    write(
        &repo.join("config.yaml"),
        "database:\n  host: localhost\nhandlers:\n  - handle_users\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "handle_users");
    let hits: Vec<_> = resp
        .affected_configs
        .iter()
        .filter(|c| c.path == "config.yaml")
        .collect();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].line, 4);
}

#[test]
fn toml_file_mention_surfaced() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("migrate.py"), "def run_migrate():\n    pass\n");
    write(
        &repo.join("pyproject.toml"),
        "[tool.pytest]\nruns = [\"run_migrate\"]\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "run_migrate");
    let hits: Vec<_> = resp
        .affected_configs
        .iter()
        .filter(|c| c.path == "pyproject.toml")
        .collect();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].line, 2);
}

#[test]
fn json_file_mention_surfaced() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("build.js"), "function buildApp(){}\n");
    write(
        &repo.join("package.json"),
        "{\n  \"scripts\": {\n    \"build\": \"buildApp\"\n  }\n}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "buildApp");
    let hits: Vec<_> = resp
        .affected_configs
        .iter()
        .filter(|c| c.path == "package.json")
        .collect();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].line, 3);
}

#[test]
fn file_stem_mention_surfaced_when_symbol_absent() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("auth.py"), "def check_password():\n    pass\n");
    // Only the file stem "auth" (not the symbol) appears in config.
    write(&repo.join("config.yaml"), "modules:\n  - auth\n  - users\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "check_password");
    let hits: Vec<_> = resp
        .affected_configs
        .iter()
        .filter(|c| c.path == "config.yaml")
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "stem 'auth' on line 2: {:?}",
        resp.affected_configs
    );
    assert_eq!(hits[0].line, 2);
}

#[test]
fn multiple_mentions_surface_each_line() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("worker.py"), "def do_work():\n    pass\n");
    write(
        &repo.join("config.yaml"),
        "primary: do_work\nbackup: do_work\nfallback: other\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "do_work");
    let hits: Vec<_> = resp
        .affected_configs
        .iter()
        .filter(|c| c.path == "config.yaml")
        .collect();
    assert_eq!(hits.len(), 2);
    let mut lines: Vec<u32> = hits.iter().map(|c| c.line).collect();
    lines.sort();
    assert_eq!(lines, vec![1, 2]);
}

#[test]
fn non_config_file_is_ignored() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("app.py"), "def my_func():\n    pass\n");
    // README.md mentions symbol but is NOT a config file.
    write(&repo.join("README.md"), "# Docs\nCall `my_func()`.\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "my_func");
    assert!(
        resp.affected_configs.is_empty(),
        "README.md must not be scanned: {:?}",
        resp.affected_configs
    );
}

#[test]
fn no_match_returns_empty_configs() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("app.py"), "def my_func():\n    pass\n");
    write(&repo.join(".env"), "UNRELATED=value\nDB=pg\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "my_func");
    assert!(resp.affected_configs.is_empty());
}

#[test]
fn heavy_dirs_are_skipped() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("app.py"), "def my_func():\n    pass\n");
    // Write matching config under a skip-listed dir — must be ignored.
    write(
        &repo.join("node_modules/pkg/config.yaml"),
        "handler: my_func\n",
    );
    write(&repo.join(".git/config"), "my_func\n");
    write(
        &repo.join("target/debug/meta.json"),
        "{\"k\": \"my_func\"}\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "my_func");
    assert!(
        resp.affected_configs.is_empty(),
        "vendored / build dirs must be skipped: {:?}",
        resp.affected_configs
    );
}

#[test]
fn non_ident_seed_returns_empty_configs() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("app.py"), "def foo():\n    pass\n");
    write(&repo.join(".env"), "FOO=foo\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "fo'o");
    assert!(resp.affected_configs.is_empty());
}

// =============================================================================
// CORE-1 regression tests (2026-05-22): non-app-config dirs and filenames
// must NOT contribute to affected_configs. See
// docs/investigate/ga-vs-codegraph-head-to-head-2026-05-21.md.
// =============================================================================

#[test]
fn github_ci_dir_is_skipped() {
    // Regression: CORE-1 — .github/ISSUE_TEMPLATE/*.yaml and workflows leaked
    // into affected_configs on gin + tokio. They're repo metadata, not app config.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("app.py"), "def my_func():\n    pass\n");
    write(
        &repo.join(".github/ISSUE_TEMPLATE/bug-report.yaml"),
        "name: Bug\nfields:\n  - my_func\n",
    );
    write(
        &repo.join(".github/workflows/codeql.yml"),
        "jobs:\n  test:\n    runs-on: my_func\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "my_func");
    assert!(
        resp.affected_configs.is_empty(),
        ".github/ must be skipped: {:?}",
        resp.affected_configs
    );
}

#[test]
fn gitlab_circleci_travis_dirs_are_skipped() {
    // Regression: CORE-1 — other CI vendor dirs should also be skipped.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("app.py"), "def my_func():\n    pass\n");
    write(&repo.join(".gitlab/ci/build.yml"), "job: my_func\n");
    write(&repo.join(".circleci/config.yml"), "ref: my_func\n");
    write(&repo.join(".gitea/issue_template.yaml"), "label: my_func\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "my_func");
    assert!(
        resp.affected_configs.is_empty(),
        "CI vendor dirs must be skipped: {:?}",
        resp.affected_configs
    );
}

#[test]
fn tool_artifact_dirs_are_skipped() {
    // Regression: CORE-1 — other graph/code-review tools drop indexes inside
    // user repos. Treat them as user-installed tool caches, not config.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("app.py"), "def my_func():\n    pass\n");
    write(&repo.join(".arbor/graph.json"), "{\"node\":\"my_func\"}\n");
    write(
        &repo.join(".playwright-mcp/page-2026.yml"),
        "ref: my_func\n",
    );
    write(
        &repo.join(".codegraph/config.json"),
        "{\"sym\":\"my_func\"}\n",
    );
    write(
        &repo.join(".code-review-graph/cache.json"),
        "[\"my_func\"]\n",
    );
    write(&repo.join(".axon/index.json"), "{\"id\":\"my_func\"}\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "my_func");
    assert!(
        resp.affected_configs.is_empty(),
        "tool artifact dirs must be skipped: {:?}",
        resp.affected_configs
    );
}

#[test]
fn ide_config_dirs_are_skipped() {
    // Regression: CORE-1 — IDE workspace settings aren't app runtime config.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("app.py"), "def my_func():\n    pass\n");
    write(
        &repo.join(".idea/workspace.xml.json"),
        "{\"k\":\"my_func\"}\n",
    );
    write(&repo.join(".vscode/settings.json"), "{\"k\":\"my_func\"}\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "my_func");
    assert!(
        resp.affected_configs.is_empty(),
        "IDE dirs must be skipped: {:?}",
        resp.affected_configs
    );
}

#[test]
fn linter_and_tooling_configs_are_ignored() {
    // Regression: CORE-1 — repo-root linter/tooling configs mention symbols
    // (e.g., golangci excludes by name) but they don't represent app runtime
    // dependencies on the symbol.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("app.go"), "package main\nfunc my_func() {}\n");
    write(
        &repo.join(".golangci.yml"),
        "linters:\n  enable:\n    - my_func\n",
    );
    write(
        &repo.join(".goreleaser.yaml"),
        "builds:\n  - name: my_func\n",
    );
    write(
        &repo.join(".eslintrc.json"),
        "{\"rules\":{\"my_func\":\"off\"}}\n",
    );
    write(&repo.join(".prettierrc.json"), "{\"my_func\":\"format\"}\n");
    write(&repo.join(".stylelintrc.json"), "{\"my_func\":\"error\"}\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "my_func");
    assert!(
        resp.affected_configs.is_empty(),
        "linter configs must not surface in affected_configs: {:?}",
        resp.affected_configs
    );
}

#[test]
fn legit_app_config_at_root_is_still_surfaced() {
    // After CORE-1 fix, package.json / Cargo.toml / pyproject.toml / app yaml
    // at repo root MUST still surface — this is the happy path of the tool.
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("app.py"), "def my_func():\n    pass\n");
    write(
        &repo.join("pyproject.toml"),
        "[tool.app]\nentry = \"my_func\"\n",
    );
    write(&repo.join("config/prod.yaml"), "handler: my_func\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "my_func");
    let paths: Vec<&str> = resp
        .affected_configs
        .iter()
        .map(|c| c.path.as_str())
        .collect();
    assert!(
        paths.contains(&"pyproject.toml"),
        "pyproject.toml must still surface: {:?}",
        paths
    );
    assert!(
        paths.contains(&"config/prod.yaml"),
        "app yaml under config/ must still surface: {:?}",
        paths
    );
}

#[test]
fn configs_deterministic_sort() {
    let tmp = TempDir::new().unwrap();
    let (cache, repo) = setup(&tmp);
    write(&repo.join("app.py"), "def my_func():\n    pass\n");
    write(&repo.join("z.yaml"), "handler: my_func\n");
    write(&repo.join("a.yaml"), "handler: my_func\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let resp = run(&store, "my_func");
    let paths: Vec<String> = resp
        .affected_configs
        .iter()
        .map(|c| c.path.clone())
        .collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted);
}
