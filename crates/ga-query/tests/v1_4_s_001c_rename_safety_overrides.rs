//! v1.4 S-001c — `ga_rename_safety` `subclass_overrides_count` field.
//!
//! Spec: graphatlas-v1.4-data-model.md S-001c, AS-013. Wiring-only —
//! response surfaces a count of subclass methods that would silently
//! break under the rename. Existing fields (existing_arity,
//! param_count_changed) unchanged.

use ga_index::Store;
use ga_query::indexer::build_index;
use ga_query::rename_safety::{rename_safety, RenameSafetyRequest};
use std::fs;
use tempfile::TempDir;

fn write(p: &std::path::Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

#[test]
fn rename_safety_surfaces_subclass_overrides_count() {
    // Service.handle has 3 subclasses each with @Override handle().
    // rename_safety on Service.handle must report subclass_overrides_count = 3.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    write(
        &repo.join("Service.java"),
        "package p;\npublic class Service { public void handle() {} }\n",
    );
    for sub in ["ServiceA", "ServiceB", "ServiceC"] {
        write(
            &repo.join(format!("{sub}.java")),
            &format!(
                "package p;\npublic class {sub} extends Service {{ @Override public void handle() {{}} }}\n"
            ),
        );
    }

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = RenameSafetyRequest {
        target: "handle".to_string(),
        replacement: "process".to_string(),
        file_hint: Some("Service.java".to_string()),
        new_arity: None,
    };
    let report = rename_safety(&store, &req).expect("rename_safety ok");

    assert_eq!(
        report.subclass_overrides_count, 3,
        "Service.handle has 3 subclass overrides; expected count=3, got {}",
        report.subclass_overrides_count
    );
}

#[test]
fn rename_safety_zero_overrides_when_no_subclasses() {
    // Method with no override pairs at all. subclass_overrides_count == 0.
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join(".graphatlas");
    let repo = tmp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();

    write(
        &repo.join("Service.java"),
        "package p;\npublic class Service { public void handle() {} }\n",
    );
    // Caller (so target Symbol exists in graph and rename has something to
    // report) — but no subclass.
    write(
        &repo.join("Caller.java"),
        "package p;\npublic class Caller { public void call(Service s) { s.handle(); } }\n",
    );

    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = RenameSafetyRequest {
        target: "handle".to_string(),
        replacement: "process".to_string(),
        file_hint: Some("Service.java".to_string()),
        new_arity: None,
    };
    let report = rename_safety(&store, &req).expect("rename_safety ok");

    assert_eq!(
        report.subclass_overrides_count, 0,
        "no subclass overrides → count must be 0, got {}",
        report.subclass_overrides_count
    );
}
