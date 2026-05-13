//! v1.1-M4 S-001c — Java EXTENDS extraction (AS-002).
//!
//! Spec contract (graphatlas-v1.1-languages.md AS-002):
//!   Given: class Admin extends User implements Printable
//!   When: indexed
//!   Then: EXTENDS edges (Admin → User) AND (Admin → Printable).
//!         Interfaces treated as EXTENDS for dispatch resolution.

use ga_core::Lang;
use ga_parser::extract_extends;

#[test]
fn class_extends_single_superclass() {
    let src = b"\
public class Admin extends User {\n\
    public Admin() {}\n\
}\n";
    let edges = extract_extends(Lang::Java, src).expect("extract_extends Ok");
    let bases: Vec<&str> = edges
        .iter()
        .filter(|e| e.class_name == "Admin")
        .map(|e| e.base_name.as_str())
        .collect();
    assert!(
        bases.contains(&"User"),
        "Admin extends User → User base expected; got {bases:?}"
    );
}

#[test]
fn class_implements_single_interface() {
    let src = b"\
public class Job implements Runnable {\n\
    public void run() {}\n\
}\n";
    let edges = extract_extends(Lang::Java, src).expect("extract_extends Ok");
    let bases: Vec<&str> = edges
        .iter()
        .filter(|e| e.class_name == "Job")
        .map(|e| e.base_name.as_str())
        .collect();
    assert!(
        bases.contains(&"Runnable"),
        "Job implements Runnable → Runnable base expected; got {bases:?}"
    );
}

#[test]
fn class_extends_and_implements_emits_per_base() {
    // AS-002 canonical example: extends User + implements Printable.
    let src = b"\
public class Admin extends User implements Printable {\n\
    public void print() {}\n\
}\n";
    let edges = extract_extends(Lang::Java, src).expect("extract_extends Ok");
    let bases: Vec<&str> = edges
        .iter()
        .filter(|e| e.class_name == "Admin")
        .map(|e| e.base_name.as_str())
        .collect();
    assert!(bases.contains(&"User"), "must include User: {bases:?}");
    assert!(
        bases.contains(&"Printable"),
        "must include Printable: {bases:?}"
    );
}

#[test]
fn class_implements_multiple_interfaces() {
    let src = b"\
public class Combo implements Runnable, AutoCloseable, Printable {\n\
    public void run() {}\n\
    public void close() {}\n\
    public void print() {}\n\
}\n";
    let edges = extract_extends(Lang::Java, src).expect("extract_extends Ok");
    let bases: Vec<&str> = edges
        .iter()
        .filter(|e| e.class_name == "Combo")
        .map(|e| e.base_name.as_str())
        .collect();
    for required in &["Runnable", "AutoCloseable", "Printable"] {
        assert!(
            bases.contains(required),
            "Combo must include {required}: {bases:?}"
        );
    }
}

#[test]
fn extends_strips_module_prefix_on_qualified_base() {
    // `extends pkg.Base` → base_name = "Base" (last segment only, matches
    // the indexer's symbol-name layer).
    let src = b"\
public class Admin extends com.example.User {\n\
    public Admin() {}\n\
}\n";
    let edges = extract_extends(Lang::Java, src).expect("extract_extends Ok");
    let bases: Vec<&str> = edges
        .iter()
        .filter(|e| e.class_name == "Admin")
        .map(|e| e.base_name.as_str())
        .collect();
    assert!(
        bases.contains(&"User"),
        "qualified extends `com.example.User` must surface trailing `User`: {bases:?}"
    );
}

#[test]
fn interface_extends_other_interfaces() {
    // Java interfaces use `extends` (not `implements`) to inherit from
    // other interfaces, possibly multiple.
    let src = b"\
public interface Printable extends Serializable, Cloneable {\n\
    void print();\n\
}\n";
    let edges = extract_extends(Lang::Java, src).expect("extract_extends Ok");
    let bases: Vec<&str> = edges
        .iter()
        .filter(|e| e.class_name == "Printable")
        .map(|e| e.base_name.as_str())
        .collect();
    for required in &["Serializable", "Cloneable"] {
        assert!(
            bases.contains(required),
            "Printable interface must extend {required}: {bases:?}"
        );
    }
}

#[test]
fn class_without_extends_emits_no_edges() {
    let src = b"\
public class Plain {\n\
    public Plain() {}\n\
}\n";
    let edges = extract_extends(Lang::Java, src).expect("extract_extends Ok");
    assert!(
        edges.iter().all(|e| e.class_name != "Plain"),
        "Plain class should not emit any EXTENDS edge: {edges:?}"
    );
}

#[test]
fn empty_java_source_yields_no_extends() {
    let edges = extract_extends(Lang::Java, b"").expect("Ok");
    assert!(edges.is_empty());
}

// ─── Edge-Case Compliance (skill Phase 1 — added during S-001c cleanup) ───
//
// Phase 1 of /mf-build mandates the 8-row Edge-Case table per story.
// AS-002 happy-path tests above cover Empty + (degenerate) Boundary.
// The three tests below close Error/Boundary-large/Special-chars rows.

#[test]
fn malformed_extends_does_not_panic() {
    // Error path (R12 contract): tree-sitter is permissive, but the
    // extends extractor must never panic on broken syntax.
    let garbage: &[u8] = b"public class Broken extends }}}{ implements ;;; \x00\x01 void void";
    let result = std::panic::catch_unwind(|| extract_extends(Lang::Java, garbage));
    assert!(result.is_ok(), "extract_extends panicked on malformed Java");
}

#[test]
fn class_with_many_interfaces_extracts_each() {
    // Boundary / large-data: a class implementing 50 interfaces must
    // emit one EXTENDS edge per interface (no truncation, no overflow).
    let mut src = String::from("public class Wide implements ");
    let names: Vec<String> = (0..50).map(|i| format!("Iface{i}")).collect();
    src.push_str(&names.join(", "));
    src.push_str(" {}\n");

    let edges = extract_extends(Lang::Java, src.as_bytes()).expect("Ok");
    let bases: Vec<&str> = edges
        .iter()
        .filter(|e| e.class_name == "Wide")
        .map(|e| e.base_name.as_str())
        .collect();

    for required in &names {
        assert!(
            bases.contains(&required.as_str()),
            "missing interface {required} from Wide's bases (count={}): {bases:?}",
            bases.len()
        );
    }
}

#[test]
fn unicode_in_source_does_not_corrupt_extraction() {
    // Special chars: source containing Unicode (comments, string literals,
    // identifiers) must not panic the extractor, and ASCII base classes
    // declared alongside Unicode content must still surface cleanly.
    // (Tree-sitter-java's identifier acceptance for non-ASCII letters
    // varies by grammar version; this test pins only "Unicode source
    // doesn't break ASCII paths" — the conservative cross-version
    // contract.)
    let src = "// 日本語コメント\npublic class Sub extends Base {}\n".as_bytes();
    let edges = extract_extends(Lang::Java, src).expect("Ok");
    assert!(
        edges
            .iter()
            .any(|e| e.class_name == "Sub" && e.base_name == "Base"),
        "ASCII base must extract cleanly even with Unicode comment in file: {edges:?}"
    );
}
