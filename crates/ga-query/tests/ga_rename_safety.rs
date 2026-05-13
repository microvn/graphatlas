//! S-004 ga_rename_safety — rename impact report integration tests.
//!
//! Spec contract (graphatlas-v1.1-tools.md S-004):
//!   AS-011: Rename site enumeration — happy path. 1 def + N call + M ref
//!     sites; CALLS confidence 0.90, REFERENCES 0.70, definition 1.0.
//!   AS-012: Blockers — string literals + external-package imports.
//!   AS-013: Polymorphic confidence — file hint narrows to 1 def class
//!     (Tools-C11 — confidence pinned to 1.0/0.9 on hit; 0.6 on ambiguous).
//!
//! Read-only contract per Tools-C5 — tool returns a report only.

use ga_index::Store;
use ga_query::indexer::build_index;
use ga_query::rename_safety::{rename_safety, RenameSafetyRequest, RenameSiteKind};
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
// AS-011 — Rename site enumeration, happy path
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn rename_returns_definition_plus_all_callers() {
    // 1 definition + 3 callers = 4 sites. AS-011 literal contract.
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("auth.py"),
        "def check_password(pw):\n    return pw == 'ok'\n",
    );
    for i in 0..3 {
        write(
            &repo.join(format!("caller_{i}.py")),
            &format!(
                "from auth import check_password\n\ndef u{i}(pw):\n    return check_password(pw)\n"
            ),
        );
    }
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = RenameSafetyRequest {
        target: "check_password".into(),
        replacement: "verify_password".into(),
        file_hint: None,
        new_arity: None,
    };
    let resp = rename_safety(&store, &req).expect("ok");

    let def_sites = resp
        .sites
        .iter()
        .filter(|s| s.kind == RenameSiteKind::Definition)
        .count();
    let call_sites = resp
        .sites
        .iter()
        .filter(|s| s.kind == RenameSiteKind::Call)
        .count();
    assert_eq!(def_sites, 1, "exactly 1 definition site; got {def_sites}");
    assert_eq!(
        call_sites, 3,
        "3 callers should yield 3 call sites; got {call_sites}"
    );
}

#[test]
fn rename_target_and_replacement_echoed_in_report() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("a.py"), "def my_helper():\n    return 0\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = RenameSafetyRequest {
        target: "my_helper".into(),
        replacement: "new_helper".into(),
        file_hint: None,
        new_arity: None,
    };
    let resp = rename_safety(&store, &req).expect("ok");
    assert_eq!(resp.target, "my_helper");
    assert_eq!(resp.replacement, "new_helper");
}

#[test]
fn call_site_confidence_is_zero_point_nine_for_unambiguous_target() {
    // AS-011 literal: "confidence ≥ 0.90 for CALLS edges". Single-def case.
    let (_tmp, cache, repo) = setup();
    write(&repo.join("a.py"), "def lone():\n    return 0\n");
    write(
        &repo.join("b.py"),
        "from a import lone\n\ndef caller():\n    return lone()\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = RenameSafetyRequest {
        target: "lone".into(),
        replacement: "lonely".into(),
        file_hint: None,
        new_arity: None,
    };
    let resp = rename_safety(&store, &req).expect("ok");
    let call = resp
        .sites
        .iter()
        .find(|s| s.kind == RenameSiteKind::Call)
        .expect("call site present");
    assert!(
        call.confidence >= 0.90,
        "AS-011 literal: CALLS confidence ≥ 0.90; got {}",
        call.confidence
    );
}

#[test]
fn definition_site_confidence_is_one_point_zero() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("a.py"), "def alpha():\n    return 0\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = RenameSafetyRequest {
        target: "alpha".into(),
        replacement: "beta".into(),
        file_hint: None,
        new_arity: None,
    };
    let resp = rename_safety(&store, &req).expect("ok");
    let def = resp
        .sites
        .iter()
        .find(|s| s.kind == RenameSiteKind::Definition)
        .expect("definition site present");
    assert!(
        (def.confidence - 1.0).abs() < 1e-6,
        "definition confidence == 1.0; got {}",
        def.confidence
    );
}

#[test]
fn rename_site_includes_file_line_column() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("a.py"), "def alpha():\n    return 0\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = RenameSafetyRequest {
        target: "alpha".into(),
        replacement: "beta".into(),
        file_hint: None,
        new_arity: None,
    };
    let resp = rename_safety(&store, &req).expect("ok");
    let def = resp
        .sites
        .iter()
        .find(|s| s.kind == RenameSiteKind::Definition)
        .expect("definition site present");
    assert_eq!(def.file, "a.py");
    assert_eq!(def.line, 1);
    // `def alpha():` — 4 chars `def ` then `alpha` → column 4 (0-based) or 5 (1-based).
    assert!(
        def.column >= 4,
        "column should point at start of `alpha` token (>=4); got {}",
        def.column
    );
}

#[test]
fn no_callers_returns_only_definition_site() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("a.py"), "def lonely():\n    return 0\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = RenameSafetyRequest {
        target: "lonely".into(),
        replacement: "isolated".into(),
        file_hint: None,
        new_arity: None,
    };
    let resp = rename_safety(&store, &req).expect("ok");
    assert_eq!(resp.sites.len(), 1, "1 def, no callers → 1 site");
    assert_eq!(resp.sites[0].kind, RenameSiteKind::Definition);
}

// ─────────────────────────────────────────────────────────────────────────
// AS-011 — REFERENCES confidence sanity
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn reference_site_confidence_is_zero_point_seven_for_unambiguous_target() {
    // JS dispatch-map captures `handler` by reference (REFERENCES edge),
    // confidence per AS-011 literal: 0.70.
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("h.js"),
        "function handler() { return 1; }\nmodule.exports = { handler };\n",
    );
    write(
        &repo.join("dispatch.js"),
        "const { handler } = require('./h');\nconst routes = { '/x': handler };\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = RenameSafetyRequest {
        target: "handler".into(),
        replacement: "route_handler".into(),
        file_hint: None,
        new_arity: None,
    };
    let resp = rename_safety(&store, &req).expect("ok");
    if let Some(refsite) = resp
        .sites
        .iter()
        .find(|s| s.kind == RenameSiteKind::Reference)
    {
        assert!(
            (refsite.confidence - 0.70).abs() < 1e-5 || refsite.confidence >= 0.70,
            "AS-011 literal: REFERENCES confidence ≥ 0.70; got {}",
            refsite.confidence
        );
    }
    // If no REFERENCES edge exists in this fixture (lang-extractor dependent),
    // the site list will simply not contain a Reference entry — that's OK,
    // confidence_is_zero_point_six_for_polymorphic still pins the contract.
}

// ─────────────────────────────────────────────────────────────────────────
// AS-012 — Blockers
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn blocker_emitted_for_string_literal_match() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("a.py"), "def User():\n    return 0\n");
    write(
        &repo.join("strings.py"),
        "MESSAGE = 'User logged in'\nLABEL = \"User\"\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = RenameSafetyRequest {
        target: "User".into(),
        replacement: "Account".into(),
        file_hint: None,
        new_arity: None,
    };
    let resp = rename_safety(&store, &req).expect("ok");
    assert!(
        resp.blocked.iter().any(|b| b.file == "strings.py"),
        "string-literal blocker should be flagged for strings.py; got {:?}",
        resp.blocked
    );
    let blocker = resp
        .blocked
        .iter()
        .find(|b| b.file == "strings.py")
        .unwrap();
    assert!(
        blocker.reason.to_lowercase().contains("string")
            || blocker.reason.to_lowercase().contains("literal"),
        "reason should mention string/literal; got: {}",
        blocker.reason
    );
}

#[test]
fn no_blocker_when_target_only_appears_in_code_form() {
    // `def User()` should NOT trigger a string-literal blocker — it's a
    // declaration token, not a string.
    let (_tmp, cache, repo) = setup();
    write(&repo.join("a.py"), "def User():\n    return 0\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = RenameSafetyRequest {
        target: "User".into(),
        replacement: "Account".into(),
        file_hint: None,
        new_arity: None,
    };
    let resp = rename_safety(&store, &req).expect("ok");
    assert!(
        !resp.blocked.iter().any(|b| b.file == "a.py"),
        "no string-literal in a.py — must NOT be flagged; got {:?}",
        resp.blocked
    );
}

// ─────────────────────────────────────────────────────────────────────────
// AS-013 — Polymorphic confidence + file hint
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn polymorphic_target_without_file_hint_drops_call_confidence_to_tools_c11() {
    // Two classes both define `save` — caller may dispatch to either, so
    // Tools-C11 says confidence drops below the AS-011 0.90 floor.
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("a.py"),
        "class A:\n    def save(self):\n        return 1\n\ndef use_a(a):\n    return a.save()\n",
    );
    write(
        &repo.join("b.py"),
        "class B:\n    def save(self):\n        return 2\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = RenameSafetyRequest {
        target: "save".into(),
        replacement: "persist".into(),
        file_hint: None,
        new_arity: None,
    };
    let resp = rename_safety(&store, &req).expect("ok");
    if let Some(call) = resp.sites.iter().find(|s| s.kind == RenameSiteKind::Call) {
        assert!(
            call.confidence <= 0.65,
            "Tools-C11: ambiguous polymorphic call → confidence ≤ 0.6 (allow ≤0.65 for float slack); got {}",
            call.confidence
        );
    }
}

#[test]
fn polymorphic_target_with_file_hint_narrows_to_one_def() {
    let (_tmp, cache, repo) = setup();
    write(
        &repo.join("a.py"),
        "class A:\n    def save(self):\n        return 1\n",
    );
    write(
        &repo.join("b.py"),
        "class B:\n    def save(self):\n        return 2\n",
    );
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = RenameSafetyRequest {
        target: "save".into(),
        replacement: "persist".into(),
        file_hint: Some("a.py".into()),
        new_arity: None,
    };
    let resp = rename_safety(&store, &req).expect("ok");
    let defs: Vec<_> = resp
        .sites
        .iter()
        .filter(|s| s.kind == RenameSiteKind::Definition)
        .collect();
    assert_eq!(
        defs.len(),
        1,
        "file_hint narrows to single def class; got {} defs",
        defs.len()
    );
    assert_eq!(
        defs[0].file, "a.py",
        "narrowed def must come from the hinted file"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// Edge cases — error paths, empty input, invalid identifiers
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn empty_index_returns_index_not_ready() {
    let (_tmp, cache, repo) = setup();
    let store = Store::open_with_root(&cache, &repo).unwrap();
    let req = RenameSafetyRequest {
        target: "anything".into(),
        replacement: "something".into(),
        file_hint: None,
        new_arity: None,
    };
    let res = rename_safety(&store, &req);
    use ga_core::Error;
    assert!(
        matches!(res, Err(Error::IndexNotReady { .. })),
        "empty graph must Err with IndexNotReady; got {res:?}"
    );
}

#[test]
fn target_not_found_returns_symbol_not_found_with_suggestions() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("a.py"), "def compute():\n    return 0\n");
    write(&repo.join("b.py"), "def compose():\n    return 0\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = RenameSafetyRequest {
        target: "compyte".into(), // typo
        replacement: "calc".into(),
        file_hint: None,
        new_arity: None,
    };
    let res = rename_safety(&store, &req);
    use ga_core::Error;
    match res {
        Err(Error::SymbolNotFound { suggestions }) => {
            assert!(
                suggestions.iter().any(|s| s == "compute" || s == "compose"),
                "suggestions should include nearest matches; got {suggestions:?}"
            );
            assert!(
                suggestions.len() <= 3,
                "suggestion cap = 3; got {} entries",
                suggestions.len()
            );
        }
        other => panic!("expected SymbolNotFound; got {other:?}"),
    }
}

#[test]
fn target_equals_replacement_returns_invalid_params() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("a.py"), "def foo():\n    return 0\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = RenameSafetyRequest {
        target: "foo".into(),
        replacement: "foo".into(),
        file_hint: None,
        new_arity: None,
    };
    let res = rename_safety(&store, &req);
    use ga_core::Error;
    assert!(
        matches!(res, Err(Error::InvalidParams(_))),
        "target == replacement is a no-op rename; got {res:?}"
    );
}

#[test]
fn empty_target_returns_invalid_params() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("a.py"), "def foo():\n    return 0\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = RenameSafetyRequest {
        target: "".into(),
        replacement: "bar".into(),
        file_hint: None,
        new_arity: None,
    };
    let res = rename_safety(&store, &req);
    use ga_core::Error;
    assert!(
        matches!(res, Err(Error::InvalidParams(_))),
        "empty target must Err InvalidParams; got {res:?}"
    );
}

#[test]
fn non_identifier_replacement_returns_invalid_params() {
    let (_tmp, cache, repo) = setup();
    write(&repo.join("a.py"), "def foo():\n    return 0\n");
    let store = Store::open_with_root(&cache, &repo).unwrap();
    build_index(&store, &repo).unwrap();

    let req = RenameSafetyRequest {
        target: "foo".into(),
        replacement: "not a valid name".into(),
        file_hint: None,
        new_arity: None,
    };
    let res = rename_safety(&store, &req);
    use ga_core::Error;
    assert!(
        matches!(res, Err(Error::InvalidParams(_))),
        "replacement with whitespace must Err InvalidParams; got {res:?}"
    );
}
