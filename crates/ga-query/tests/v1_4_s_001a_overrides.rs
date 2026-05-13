//! v1.4 S-001a — OVERRIDES Symbol→Symbol REL emission for Java/Kotlin/C#.
//!
//! Spec: spec, S-001a.
//!
//! Scope of this test file:
//! - AS-001: 3-lang (Java/Kotlin/C#) override-pair OVERRIDES emit + AT-011
//!   class-level witness verification.
//! - AS-002: external/unresolved parent → has_unresolved_override=true,
//!   no edge, counter increments. AT-014 invariant guard.
//! - AS-003: self-override skip + AT-009 zero self-edges.
//! - AS-006: 3-level inheritance chain → 2 single-step OVERRIDES rows
//!   (NOT flattened).

use ga_index::Store;
use ga_query::indexer::build_index;
use std::path::Path;
use tempfile::TempDir;

fn index_repo(repo: &Path) -> (TempDir, Store, ga_query::indexer::IndexStats) {
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let store = Store::open_with_root(&cache_root, repo).unwrap();
    let stats = build_index(&store, repo).unwrap();
    store.commit().unwrap();
    let store = Store::open_with_root(&cache_root, repo).unwrap();
    (tmp, store, stats)
}

fn write_file(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, body).unwrap();
}

fn count_rel(store: &Store, rel: &str) -> i64 {
    let conn = store.connection().unwrap();
    let q = format!("MATCH ()-[r:{rel}]->() RETURN count(r)");
    let rs = conn.query(&q).unwrap();
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            return n;
        }
    }
    0
}

fn has_overrides_pair(store: &Store, sub_name: &str, sub_file: &str, par_name: &str) -> bool {
    let conn = store.connection().unwrap();
    let q = format!(
        "MATCH (sub:Symbol {{name: '{sub_name}', file: '{sub_file}'}})-[:OVERRIDES]->(par:Symbol {{name: '{par_name}'}}) \
         RETURN count(*)"
    );
    let rs = conn.query(&q).unwrap();
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            return n >= 1;
        }
    }
    false
}

// ─────────────────────────────────────────────────────────────────────────
// AS-001 — 3-lang OVERRIDES emit
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn java_override_emits_overrides_edge() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "Animal.java",
        "package p;\npublic class Animal { public void speak() {} }\n",
    );
    write_file(
        repo.path(),
        "Dog.java",
        "package p;\npublic class Dog extends Animal { @Override public void speak() {} }\n",
    );
    let (_t, store, _) = index_repo(repo.path());
    assert!(
        has_overrides_pair(&store, "speak", "Dog.java", "speak"),
        "Java @Override must emit OVERRIDES(Dog.speak → Animal.speak)"
    );
}

#[test]
fn kotlin_override_emits_overrides_edge() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "Animal.kt",
        "package p\nopen class Animal { open fun speak() {} }\n",
    );
    write_file(
        repo.path(),
        "Dog.kt",
        "package p\nclass Dog : Animal() { override fun speak() {} }\n",
    );
    let (_t, store, _) = index_repo(repo.path());
    assert!(
        has_overrides_pair(&store, "speak", "Dog.kt", "speak"),
        "Kotlin override fun must emit OVERRIDES(Dog.speak → Animal.speak)"
    );
}

#[test]
fn csharp_override_emits_overrides_edge() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "Animal.cs",
        "namespace P { public class Animal { public virtual void Speak() {} } }\n",
    );
    write_file(
        repo.path(),
        "Dog.cs",
        "namespace P { public class Dog : Animal { public override void Speak() {} } }\n",
    );
    let (_t, store, _) = index_repo(repo.path());
    assert!(
        has_overrides_pair(&store, "Speak", "Dog.cs", "Speak"),
        "C# override must emit OVERRIDES(Dog.Speak → Animal.Speak)"
    );
}

#[test]
fn at_011_class_level_witness_holds() {
    // Tools-C18: every OVERRIDES(sub_method, par_method) row must have a
    // matching (sub_class, par_class) class-level witness in EXTENDS or
    // IMPLEMENTS. Audit query: zero rows where the witness is missing.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "Animal.java",
        "package p;\npublic class Animal { public void speak() {} }\n",
    );
    write_file(
        repo.path(),
        "Dog.java",
        "package p;\npublic class Dog extends Animal { @Override public void speak() {} }\n",
    );
    let (_t, store, _) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    let rs = conn
        .query(
            "MATCH (sm:Symbol)-[:OVERRIDES]->(pm:Symbol) \
             MATCH (sc:Symbol)-[:CONTAINS]->(sm) \
             MATCH (pc:Symbol)-[:CONTAINS]->(pm) \
             WHERE NOT EXISTS { MATCH (sc)-[:EXTENDS]->(pc) } \
               AND NOT EXISTS { MATCH (sc)-[:IMPLEMENTS]->(pc) } \
             RETURN count(*)",
        )
        .unwrap();
    let mut violations = 0i64;
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            violations = n;
        }
    }
    assert_eq!(
        violations, 0,
        "AT-011: every OVERRIDES row must have class-level EXTENDS-or-IMPLEMENTS witness"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// AS-002 — external / unresolved parent → has_unresolved_override flag
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn external_parent_sets_has_unresolved_override_and_increments_counter() {
    // Subclass extends a class NOT in the indexed repo (vendored/external).
    // Per Tools-C12 no synthetic edge: no OVERRIDES row emitted. Per H1
    // fix: child Symbol's has_unresolved_override flag is true. Counter
    // unresolved_overrides_count increments.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "MyService.java",
        "package my;\nimport com.vendor.lib.AbstractService;\n\
         public class MyService extends AbstractService { \
             @Override public void handle() {} \
         }\n",
    );
    let (_t, store, stats) = index_repo(repo.path());

    // No OVERRIDES emitted
    let n = count_rel(&store, "OVERRIDES");
    assert_eq!(
        n, 0,
        "external parent must NOT emit OVERRIDES (Tools-C12 no synthetic)"
    );

    // has_unresolved_override = true on child
    let conn = store.connection().unwrap();
    let rs = conn
        .query(
            "MATCH (s:Symbol {name: 'handle', file: 'MyService.java'}) \
             RETURN s.has_unresolved_override, s.is_override",
        )
        .unwrap();
    let mut found = false;
    for row in rs {
        let mut it = row.into_iter();
        let huo = matches!(it.next(), Some(lbug::Value::Bool(true)));
        let iov = matches!(it.next(), Some(lbug::Value::Bool(true)));
        assert!(
            huo,
            "has_unresolved_override must be true for unresolved override"
        );
        assert!(iov, "is_override must be true (parser sets via @Override)");
        found = true;
    }
    assert!(found, "MyService.handle Symbol not found");

    // Counter incremented
    assert!(
        stats.unresolved_overrides_count >= 1,
        "unresolved_overrides_count must increment for external parent (got {})",
        stats.unresolved_overrides_count
    );
}

#[test]
fn at_014_has_unresolved_override_implies_is_override() {
    // AT-014: every Symbol with has_unresolved_override=true MUST also have
    // is_override=true (the flag means "tried to resolve parent and failed",
    // not "was never an override"). Audit: count of violators must be 0.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "MyService.java",
        "package my;\nimport com.vendor.lib.AbstractService;\n\
         public class MyService extends AbstractService { \
             @Override public void handle() {} \
         }\n",
    );
    let (_t, store, _) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    let rs = conn
        .query(
            "MATCH (s:Symbol) \
             WHERE s.has_unresolved_override = true AND s.is_override = false \
             RETURN count(*)",
        )
        .unwrap();
    let mut violators = 0i64;
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            violators = n;
        }
    }
    assert_eq!(
        violators, 0,
        "AT-014: has_unresolved_override=true implies is_override=true"
    );
}

// ─────────────────────────────────────────────────────────────────────────
// AS-003 — self-override guard
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn at_009_zero_self_overrides_edges() {
    // Audit AT-009: MATCH (s)-[:OVERRIDES]->(s) RETURN count(*) = 0
    // across all fixtures. Use the regular Java fixture from AS-001.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "Animal.java",
        "package p;\npublic class Animal { public void speak() {} }\n",
    );
    write_file(
        repo.path(),
        "Dog.java",
        "package p;\npublic class Dog extends Animal { @Override public void speak() {} }\n",
    );
    let (_t, store, _) = index_repo(repo.path());
    let conn = store.connection().unwrap();
    let rs = conn
        .query("MATCH (s:Symbol)-[:OVERRIDES]->(s) RETURN count(*)")
        .unwrap();
    let mut self_loops = 0i64;
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            self_loops = n;
        }
    }
    assert_eq!(self_loops, 0, "AT-009: OVERRIDES must have zero self-edges");
}

// ─────────────────────────────────────────────────────────────────────────
// AS-006 — multi-level (3-level chain) emits single-step OVERRIDES
// ─────────────────────────────────────────────────────────────────────────

#[test]
fn three_level_inheritance_emits_two_single_step_overrides() {
    // C extends B extends A; all 3 override foo(). Per Q1 clarification:
    // OVERRIDES emits (C.foo, B.foo) and (B.foo, A.foo) only — NOT
    // (C.foo, A.foo). Transitive recovered via OVERRIDES* (or
    // app-layer walk). Tools-C19 single-step.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "A.java",
        "package p;\npublic class A { public void foo() {} }\n",
    );
    write_file(
        repo.path(),
        "B.java",
        "package p;\npublic class B extends A { @Override public void foo() {} }\n",
    );
    write_file(
        repo.path(),
        "C.java",
        "package p;\npublic class C extends B { @Override public void foo() {} }\n",
    );
    let (_t, store, _) = index_repo(repo.path());

    // Two OVERRIDES rows: C.foo→B.foo, B.foo→A.foo
    assert!(
        has_overrides_pair(&store, "foo", "C.java", "foo"),
        "C.foo must override B.foo (single-step)"
    );
    assert!(
        has_overrides_pair(&store, "foo", "B.java", "foo"),
        "B.foo must override A.foo (single-step)"
    );

    // NO direct C.foo → A.foo edge (single-step invariant)
    let conn = store.connection().unwrap();
    let rs = conn
        .query(
            "MATCH (c:Symbol {name: 'foo', file: 'C.java'})-[:OVERRIDES]->(par:Symbol) \
             RETURN par.file ORDER BY par.file",
        )
        .unwrap();
    let mut targets: Vec<String> = Vec::new();
    for row in rs {
        if let Some(lbug::Value::String(s)) = row.into_iter().next() {
            targets.push(s);
        }
    }
    assert_eq!(
        targets,
        vec!["B.java".to_string()],
        "C.foo must override only B.foo (single-step), not A.foo"
    );

    // Total OVERRIDES rows = 2 (B→A and C→B; no flattened C→A)
    assert_eq!(
        count_rel(&store, "OVERRIDES"),
        2,
        "3-level chain must emit exactly 2 OVERRIDES rows (single-step)"
    );
}

#[test]
fn three_level_chain_transitive_traversal_finds_root() {
    // Q1 clarification: transitive closure recoverable via app-layer walk.
    // For this test, just verify multi-hop pattern matching works. lbug
    // 0.16.1 may or may not support OVERRIDES* Kleene; if not, the test
    // can be skipped via a separate eval.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "A.java",
        "package p;\npublic class A { public void foo() {} }\n",
    );
    write_file(
        repo.path(),
        "B.java",
        "package p;\npublic class B extends A { @Override public void foo() {} }\n",
    );
    write_file(
        repo.path(),
        "C.java",
        "package p;\npublic class C extends B { @Override public void foo() {} }\n",
    );
    let (_t, store, _) = index_repo(repo.path());

    // Manual 2-hop join (works in any Cypher dialect; doesn't depend on
    // Kleene path support).
    let conn = store.connection().unwrap();
    let rs = conn
        .query(
            "MATCH (c:Symbol {name: 'foo', file: 'C.java'})-[:OVERRIDES]->(b:Symbol)-[:OVERRIDES]->(a:Symbol) \
             WHERE a.file = 'A.java' \
             RETURN count(*)",
        )
        .unwrap();
    let mut chain = 0i64;
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            chain = n;
        }
    }
    assert_eq!(
        chain, 1,
        "transitive 2-hop OVERRIDES from C.foo must reach A.foo via B.foo"
    );
}
