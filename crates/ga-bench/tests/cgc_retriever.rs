//! Bench P2-C2 — CgcRetriever pure-function coverage. End-to-end against
//! real `cgc` requires the binary installed; those tests gate themselves
//! on `which cgc`. What matters for CI hermeticity is the argument shape
//! and response parsing — both are pure functions below.

use ga_bench::retrievers::cgc::{build_cgc_request, extract_names_from_response};
use serde_json::{json, Value};

#[test]
fn callers_uc_uses_find_callers_query_type() {
    let (method, args) = build_cgc_request(
        "callers",
        &json!({"symbol": "check_password"}),
        "/fixture/path",
    )
    .expect("callers → Some");
    assert_eq!(method, "analyze_code_relationships");
    assert_eq!(args["query_type"], "find_callers");
    assert_eq!(args["target"], "check_password");
    assert_eq!(args["repo_path"], "/fixture/path");
}

#[test]
fn callees_uc_uses_find_callees_query_type() {
    let (method, args) =
        build_cgc_request("callees", &json!({"symbol": "authenticate"}), "/p").unwrap();
    assert_eq!(method, "analyze_code_relationships");
    assert_eq!(args["query_type"], "find_callees");
}

#[test]
fn importers_uc_converts_file_to_module_name() {
    // TS adapter: `const moduleName = task.target.file.split("/").pop()?.replace(/\.\w+$/, "")`
    // Our importers UC query is {file: "django/http/request.py"} → module "request".
    let (_, args) = build_cgc_request(
        "importers",
        &json!({"file": "django/http/request.py"}),
        "/repo",
    )
    .unwrap();
    assert_eq!(args["query_type"], "find_importers");
    assert_eq!(args["target"], "request");
}

#[test]
fn symbols_uc_uses_find_code() {
    let (method, args) = build_cgc_request(
        "symbols",
        &json!({"pattern": "QuerySet", "match": "exact"}),
        "/r",
    )
    .unwrap();
    assert_eq!(method, "find_code");
    assert_eq!(args["query"], "QuerySet");
    assert_eq!(args["repo_path"], "/r");
}

#[test]
fn file_summary_uc_returns_none_unsupported() {
    // CGC has no native file outline — retriever must return None so caller
    // writes an empty result (honest "not supported" path).
    assert!(build_cgc_request("file_summary", &json!({"path": "x.py"}), "/r",).is_none());
}

#[test]
fn unknown_uc_returns_none() {
    assert!(build_cgc_request("weird_uc", &json!({}), "/r").is_none());
}

#[test]
fn extract_names_walks_nested_callers_field() {
    // Shape modeled on CGC's analyze_code_relationships response for find_callers.
    let response: Value = serde_json::from_str(
        r#"{
            "content": [{
                "type": "text",
                "text": "{\"callers\":[{\"caller_name\":\"authenticate\",\"caller_file\":\"auth.py\"},{\"caller_name\":\"login_view\",\"caller_file\":\"views.py\"}]}"
            }]
        }"#,
    )
    .unwrap();
    let names = extract_names_from_response(&response, "callers");
    assert!(names.contains(&"authenticate".to_string()));
    assert!(names.contains(&"login_view".to_string()));
}

#[test]
fn extract_names_callees_uses_callee_name_field() {
    let response: Value = serde_json::from_str(
        r#"{
            "content": [{
                "type": "text",
                "text": "{\"callees\":[{\"callee_name\":\"check_password\"},{\"callee_name\":\"log_attempt\"}]}"
            }]
        }"#,
    )
    .unwrap();
    let names = extract_names_from_response(&response, "callees");
    assert!(names.contains(&"check_password".to_string()));
    assert!(names.contains(&"log_attempt".to_string()));
}

#[test]
fn extract_names_importers_extracts_file_paths() {
    let response: Value = serde_json::from_str(
        r#"{
            "content": [{
                "type": "text",
                "text": "{\"importers\":[{\"file_path\":\"main.py\"},{\"file_path\":\"app.py\"}]}"
            }]
        }"#,
    )
    .unwrap();
    let names = extract_names_from_response(&response, "importers");
    assert!(names.contains(&"main.py".to_string()));
    assert!(names.contains(&"app.py".to_string()));
}

#[test]
fn extract_names_tolerates_empty_content() {
    let response: Value = json!({"content": []});
    let names = extract_names_from_response(&response, "callers");
    assert!(names.is_empty());
}

#[test]
fn extract_names_tolerates_raw_text_without_json() {
    // When CGC returns plain-text results (degraded mode), extractor must not
    // panic — just returns empty so scorer treats the task as a miss.
    let response: Value = json!({
        "content": [{"type": "text", "text": "something went wrong, no data"}]
    });
    let names = extract_names_from_response(&response, "callers");
    assert!(names.is_empty());
}
