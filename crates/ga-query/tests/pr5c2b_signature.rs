//! v1.3 PR5c2b — per-lang `extract_modifiers` + `extract_params` for the
//! 7 remaining wired langs: TS / JS / Go / Java / Kotlin / C# / Ruby.
//!
//! Plus generics safety test: Rust `Vec<i32>` and TS `Map<K, V>` with `<` `>`
//! `,` inside STRUCT[] CSV — verify lbug COPY doesn't break.

use ga_index::Store;
use ga_query::indexer::build_index;
use std::path::Path;
use tempfile::TempDir;

fn index_repo(repo: &Path) -> (TempDir, Store) {
    let tmp = TempDir::new().unwrap();
    let cache_root = tmp.path().join(".graphatlas");
    let store = Store::open_with_root(&cache_root, repo).unwrap();
    build_index(&store, repo).unwrap();
    store.commit().unwrap();
    let store = Store::open_with_root(&cache_root, repo).unwrap();
    (tmp, store)
}

fn write_file(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, body).unwrap();
}

fn modifiers_of(store: &Store, name: &str) -> Vec<String> {
    let conn = store.connection().unwrap();
    let q = format!("MATCH (s:Symbol {{name: '{name}'}}) RETURN s.modifiers");
    let rs = conn.query(&q).unwrap();
    for row in rs {
        if let Some(v) = row.into_iter().next() {
            return match v {
                lbug::Value::List(_, items) => items
                    .into_iter()
                    .filter_map(|x| {
                        if let lbug::Value::String(s) = x {
                            Some(s)
                        } else {
                            None
                        }
                    })
                    .collect(),
                _ => Vec::new(),
            };
        }
    }
    Vec::new()
}

fn param_count_of(store: &Store, name: &str) -> Option<i64> {
    let conn = store.connection().unwrap();
    let q = format!("MATCH (s:Symbol {{name: '{name}'}}) RETURN size(s.params)");
    let rs = conn.query(&q).unwrap();
    for row in rs {
        if let Some(lbug::Value::Int64(n)) = row.into_iter().next() {
            return Some(n);
        }
    }
    None
}

fn param_names_of(store: &Store, name: &str) -> Vec<String> {
    let conn = store.connection().unwrap();
    let q = format!("MATCH (s:Symbol {{name: '{name}'}}) UNWIND s.params AS p RETURN p.name");
    let rs = conn.query(&q).unwrap();
    let mut out = Vec::new();
    for row in rs {
        if let Some(lbug::Value::String(s)) = row.into_iter().next() {
            out.push(s);
        }
    }
    out
}

#[test]
fn typescript_params_and_modifiers() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "lib.ts",
        "export async function fetch_data(url: string, retries: number = 3): Promise<string> { return ''; }\n\
         function plain(x: number) { return x; }\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(param_count_of(&store, "fetch_data"), Some(2));
    assert_eq!(
        param_names_of(&store, "fetch_data"),
        vec!["url".to_string(), "retries".to_string()]
    );
    let m = modifiers_of(&store, "fetch_data");
    assert!(
        m.contains(&"async".to_string()) || m.contains(&"export".to_string()),
        "TS expected async or export in {m:?}"
    );
    assert_eq!(param_count_of(&store, "plain"), Some(1));
}

#[test]
fn javascript_params_extracted() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "lib.js",
        "function add(a, b) { return a + b; }\n\
         async function fetch_url(url) { return url; }\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(param_count_of(&store, "add"), Some(2));
    assert_eq!(
        param_names_of(&store, "add"),
        vec!["a".to_string(), "b".to_string()]
    );
    let m = modifiers_of(&store, "fetch_url");
    assert!(
        m.contains(&"async".to_string()),
        "JS async modifier in {m:?}"
    );
}

#[test]
fn go_params_with_grouped_names() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "lib.go",
        "package lib\nfunc Add(a int, b int) int { return a + b }\n\
         func Grouped(p, q string) string { return p }\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(param_count_of(&store, "Add"), Some(2));
    assert_eq!(
        param_names_of(&store, "Add"),
        vec!["a".to_string(), "b".to_string()]
    );
    // Grouped: `p, q string` — 2 idents in single parameter_declaration
    assert_eq!(param_count_of(&store, "Grouped"), Some(2));
    assert_eq!(
        param_names_of(&store, "Grouped"),
        vec!["p".to_string(), "q".to_string()]
    );
}

#[test]
fn java_params_and_modifiers() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "Foo.java",
        "class Foo {\n  public static int add(int a, int b) { return a + b; }\n  \
         private final void hidden() {}\n}\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(param_count_of(&store, "add"), Some(2));
    assert_eq!(
        param_names_of(&store, "add"),
        vec!["a".to_string(), "b".to_string()]
    );
    let m = modifiers_of(&store, "add");
    assert!(m.contains(&"public".to_string()), "Java public in {m:?}");
    assert!(m.contains(&"static".to_string()), "Java static in {m:?}");
    let m2 = modifiers_of(&store, "hidden");
    assert!(
        m2.contains(&"private".to_string()),
        "Java private in {m2:?}"
    );
    assert!(m2.contains(&"final".to_string()), "Java final in {m2:?}");
}

#[test]
fn kotlin_params_and_modifiers() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "Foo.kt",
        "package x\n\
         suspend fun do_work(input: Int): Int { return input }\n\
         private fun helper(name: String): String { return name }\n",
    );
    let (_t, store) = index_repo(repo.path());
    // do_work param_count
    assert_eq!(param_count_of(&store, "do_work"), Some(1));
    // suspend modifier
    let m = modifiers_of(&store, "do_work");
    assert!(
        m.contains(&"suspend".to_string()),
        "Kotlin suspend in {m:?}"
    );
}

#[test]
fn csharp_params_and_modifiers() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "Foo.cs",
        "class Foo {\n  public static int Add(int a, int b) { return a + b; }\n  \
         private async Task<int> FetchAsync(string url) { return 1; }\n}\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(param_count_of(&store, "Add"), Some(2));
    let m = modifiers_of(&store, "Add");
    assert!(m.contains(&"public".to_string()), "C# public in {m:?}");
    assert!(m.contains(&"static".to_string()), "C# static in {m:?}");
    let m2 = modifiers_of(&store, "FetchAsync");
    assert!(m2.contains(&"private".to_string()), "C# private in {m2:?}");
    assert!(m2.contains(&"async".to_string()), "C# async in {m2:?}");
}

#[test]
fn ruby_params_extracted() {
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "lib.rb",
        "class C\n  def add(a, b)\n    a + b\n  end\n  \
         def with_default(x, y = 10)\n    x + y\n  end\n  \
         def nullary\n  end\nend\n",
    );
    let (_t, store) = index_repo(repo.path());
    assert_eq!(param_count_of(&store, "add"), Some(2));
    assert_eq!(
        param_names_of(&store, "add"),
        vec!["a".to_string(), "b".to_string()]
    );
    assert_eq!(param_count_of(&store, "with_default"), Some(2));
    assert_eq!(param_count_of(&store, "nullary"), Some(0));
}

#[test]
fn generics_in_param_types_survive_csv() {
    // Stress test: Rust `Vec<i32>` and `HashMap<String, i32>` contain `<`, `>`,
    // `,` — the exact chars that lbug STRUCT[] CSV format `[{...}]` uses as
    // delimiters. Verify COPY doesn't break + types round-trip.
    let repo = TempDir::new().unwrap();
    write_file(
        repo.path(),
        "src/lib.rs",
        "fn vec_fn(items: Vec<i32>) -> usize { items.len() }\n\
         fn map_fn(m: HashMap<String, i32>) -> i32 { 1 }\n",
    );
    let (_t, store) = index_repo(repo.path());
    // If COPY broke on `<i32>` or `<String, i32>`, these queries return None
    // because no Symbol row exists.
    assert_eq!(
        param_count_of(&store, "vec_fn"),
        Some(1),
        "Vec<i32> param must survive STRUCT[] CSV"
    );
    assert_eq!(
        param_count_of(&store, "map_fn"),
        Some(1),
        "HashMap<String, i32> param must survive STRUCT[] CSV"
    );
}
