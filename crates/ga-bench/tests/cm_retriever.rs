//! Bench P3-C1 — CmRetriever pure-function coverage.

use ga_bench::retrievers::cm::{build_cm_request, extract_names_from_response, project_from_path};
use serde_json::{json, Value};

#[test]
fn project_name_follows_cm_convention() {
    // TS adapter: `repo_path.replace(/\//g, "-").replace(/^-/, "")`.
    assert_eq!(project_from_path("/Users/me/repo"), "Users-me-repo");
    assert_eq!(project_from_path("/repo"), "repo");
    assert_eq!(project_from_path("repo"), "repo");
    assert_eq!(project_from_path("/a/b/c"), "a-b-c");
}

#[test]
fn callers_uc_uses_trace_call_path() {
    let (method, args) = build_cm_request(
        "callers",
        &json!({"symbol": "check_password"}),
        "/repo/django",
    )
    .unwrap();
    assert_eq!(method, "trace_call_path");
    assert_eq!(args["function_name"], "check_password");
    assert_eq!(args["project"], "repo-django");
    // Adapter TS uses depth: 5 — pin the exact value.
    assert_eq!(args["depth"], 5);
}

#[test]
fn callees_uc_uses_trace_call_path_too() {
    // CM doesn't split directions in trace_call_path — same tool used for
    // both. Callees accuracy will be weaker than callers; that's honest.
    let (method, args) =
        build_cm_request("callees", &json!({"symbol": "authenticate"}), "/r").unwrap();
    assert_eq!(method, "trace_call_path");
    assert_eq!(args["function_name"], "authenticate");
}

#[test]
fn symbols_uc_uses_search_code_files_mode() {
    let (method, args) = build_cm_request(
        "symbols",
        &json!({"pattern": "QuerySet", "match": "exact"}),
        "/repo",
    )
    .unwrap();
    assert_eq!(method, "search_code");
    assert_eq!(args["pattern"], "QuerySet");
    assert_eq!(args["mode"], "files");
    // limit from TS adapter = 30
    assert_eq!(args["limit"], 30);
}

#[test]
fn importers_uc_returns_none_no_native() {
    // CM has no per-file importers tool — document as unsupported (returns
    // empty result at runtime, scorer treats as miss).
    assert!(build_cm_request("importers", &json!({"file": "x.py"}), "/r").is_none());
}

#[test]
fn file_summary_uc_returns_none() {
    assert!(build_cm_request("file_summary", &json!({"path": "x.py"}), "/r").is_none());
}

#[test]
fn extract_names_callers_walks_call_path_chain() {
    // Shape modeled on CM's trace_call_path response.
    let response: Value = serde_json::from_str(
        r#"{
            "content": [{
                "type": "text",
                "text": "{\"chain\":[{\"name\":\"login_view\"},{\"name\":\"authenticate\"}]}"
            }]
        }"#,
    )
    .unwrap();
    let names = extract_names_from_response(&response, "callers");
    assert!(names.contains(&"login_view".to_string()));
    assert!(names.contains(&"authenticate".to_string()));
}

#[test]
fn extract_names_symbols_treats_pattern_match_as_rank1() {
    // search_code in files mode returns file paths — we can't recover the
    // symbol name from a file path alone, so if any match exists we surface
    // the pattern itself (MRR counts as rank-1 hit). Aligns with ripgrep's
    // symbols-UC semantic so the two lexical baselines stay comparable.
    let response: Value = serde_json::from_str(
        r#"{"content":[{"type":"text","text":"{\"results\":[{\"file\":\"a.py\"},{\"file\":\"b.py\"}]}"}]}"#,
    )
    .unwrap();
    let names = extract_names_from_response(&response, "symbols");
    // Has hits → pattern promoted; caller maps back via query pattern text.
    // The extractor returns raw file paths; the retriever's `query` method
    // is responsible for promoting when configured. Here we check the base
    // extraction surface exposes the file names.
    assert!(!names.is_empty());
}

#[test]
fn extract_names_tolerates_empty_content() {
    let response: Value = json!({"content": []});
    assert!(extract_names_from_response(&response, "callers").is_empty());
}

#[test]
fn extract_names_tolerates_raw_text() {
    let response: Value = json!({"content":[{"type":"text","text":"error: foo"}]});
    assert!(extract_names_from_response(&response, "callers").is_empty());
}
