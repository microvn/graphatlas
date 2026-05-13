//! Audit: 3 sample FPs from dead_code engine vs Hd-ast GT.
//!
//! Engine flags symbols as dead. GT (Hd-ast) scans raw text and finds
//! callers via name-match. Engine FPs = engine says dead, GT says alive.
//! For each sample: show symbol + file + raw text caller line + check
//! engine indexer's caller count to determine whether the gap is engine
//! resolver weakness or GT noise.
//!
//! Run:
//!   cargo test -p ga-bench --test _audit_dead_code_fp -- --ignored --nocapture

use ga_bench::gt_gen::hd_ast::HdAst;
use ga_bench::gt_gen::GtRule;
use ga_index::Store;
use std::collections::BTreeSet;
use std::path::PathBuf;

fn open_store(label: &str, fixture: &PathBuf) -> Store {
    use std::os::unix::fs::PermissionsExt;
    let cache = std::env::temp_dir().join(format!("ga-audit-{label}"));
    let _ = std::fs::remove_dir_all(&cache);
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::set_permissions(&cache, std::fs::Permissions::from_mode(0o700)).unwrap();
    Store::open_with_root(&cache, fixture).unwrap()
}

fn audit_repo(repo: &str) {
    println!("\n════════════════════════════════════════════════════════════════════");
    println!("DEAD_CODE FP AUDIT — {repo}");
    println!("════════════════════════════════════════════════════════════════════");
    let fixture = PathBuf::from("/Volumes/Data/projects/me/graphatlas/benches/fixtures").join(repo);
    if !fixture.is_dir() {
        eprintln!("[SKIP] {repo} not init'd");
        return;
    }
    let store = open_store(&format!("dead-{repo}"), &fixture);
    ga_query::indexer::build_index(&store, &fixture).unwrap();

    // 1. Engine dead set.
    let engine_resp =
        ga_query::dead_code::dead_code(&store, &ga_query::dead_code::DeadCodeRequest::default())
            .unwrap();
    let engine_dead: BTreeSet<(String, String)> = engine_resp
        .dead
        .iter()
        .map(|d| (d.symbol.clone(), d.file.clone()))
        .collect();

    // 2. GT (Hd-ast) dead set — filter to expected_dead=true only, matching
    //    the bench's aligned_expected logic (m3_dead_code.rs::score_dead_code).
    let gt_store = open_store(&format!("dead-gt-{repo}"), &fixture);
    let rule = HdAst;
    let tasks = rule.scan(&gt_store, &fixture).unwrap();
    let gt_dead: BTreeSet<(String, String)> = tasks
        .iter()
        .filter(|t| {
            t.query
                .get("expected_dead")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        .filter_map(|t| {
            let name = t.query.get("name")?.as_str()?.to_string();
            let file = t.query.get("file")?.as_str()?.to_string();
            Some((name, file))
        })
        .collect();

    // 3. FP = engine says dead, GT says alive (NOT in gt_dead).
    let fp: Vec<&(String, String)> = engine_dead.difference(&gt_dead).collect();
    println!(
        "engine_dead={} | gt_dead={} | FP (engine says dead, GT says alive)={}",
        engine_dead.len(),
        gt_dead.len(),
        fp.len()
    );

    // 4. Pick 3 deterministic samples (lex first/middle/last).
    let n = fp.len();
    if n == 0 {
        println!("No FPs — perfect agreement.");
        return;
    }
    let picks = if n >= 3 {
        vec![fp[0], fp[n / 2], fp[n - 1]]
    } else {
        fp.clone()
    };

    // For caller diagnosis: query indexer for callers of each pick.
    let conn = store.connection().unwrap();

    for (name, file) in picks {
        println!("\n─── {name} @ {file} ───");

        // Raw text scan: count occurrences in the fixture (word boundary).
        let abs_fixture = fixture.clone();
        let raw_hits = scan_raw_callers(&abs_fixture, name, file);
        println!(
            "  raw-text callers (excluding def file): {} files, e.g. {:?}",
            raw_hits.len(),
            raw_hits.iter().take(3).collect::<Vec<_>>()
        );

        // Indexer's callers: query CALLS edges into this symbol.
        let cypher = format!(
            "MATCH (caller:Symbol)-[:CALLS]->(s:Symbol) \
             WHERE s.name = '{}' AND s.file = '{}' \
             RETURN caller.name, caller.file LIMIT 10",
            name.replace('\'', "''"),
            file.replace('\'', "''")
        );
        let calls_count = match conn.query(&cypher) {
            Ok(rs) => rs.into_iter().count(),
            Err(_) => 0,
        };
        let cypher_refs = format!(
            "MATCH (ref:Symbol)-[:REFERENCES]->(s:Symbol) \
             WHERE s.name = '{}' AND s.file = '{}' \
             RETURN ref.name, ref.file LIMIT 10",
            name.replace('\'', "''"),
            file.replace('\'', "''")
        );
        let refs_count = match conn.query(&cypher_refs) {
            Ok(rs) => rs.into_iter().count(),
            Err(_) => 0,
        };
        println!("  indexer CALLS edges = {calls_count}, REFERENCES edges = {refs_count}",);

        // Verdict.
        if !raw_hits.is_empty() && calls_count == 0 && refs_count == 0 {
            println!("  → VERDICT: engine resolver MISSED caller (raw text has it). Engine FP.");
        } else if raw_hits.is_empty() {
            println!("  → VERDICT: GT noise (no raw caller either). Both saying dead but rule logic differs?");
        } else {
            println!("  → VERDICT: edges present in indexer — investigate engine query logic.");
        }
    }
}

/// Count files in fixture (excluding def_file itself) where `name` appears
/// at a word boundary. Cap at 100 file scans to keep test fast.
fn scan_raw_callers(fixture: &std::path::Path, name: &str, def_file: &str) -> Vec<String> {
    // Use std::fs::read_dir recursively (no `ignore` dep needed for audit).
    let mut hits = Vec::new();
    let needle = name.as_bytes();
    let mut scanned = 0usize;
    let mut stack: Vec<PathBuf> = vec![fixture.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if scanned >= 500 {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for ent in entries.flatten() {
            let path = ent.path();
            if path.is_dir() {
                let dn = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !dn.starts_with('.') && dn != "node_modules" && dn != "target" {
                    stack.push(path);
                }
                continue;
            }
            if !path.is_file() {
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !matches!(ext, "rs" | "go" | "py" | "ts" | "tsx" | "js" | "jsx") {
                continue;
            }
            let rel = path
                .strip_prefix(fixture)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            if rel == def_file {
                continue;
            }
            let Ok(body) = std::fs::read(&path) else {
                continue;
            };
            scanned += 1;
            if has_word_boundary_match(&body, needle) {
                hits.push(rel);
                if hits.len() >= 10 {
                    return hits;
                }
            }
        }
    }
    hits
}

fn has_word_boundary_match(body: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut i = 0;
    while i + needle.len() <= body.len() {
        if &body[i..i + needle.len()] == needle {
            let before_ok = i == 0 || !is_ident_byte(body[i - 1]);
            let after_ok = i + needle.len() == body.len() || !is_ident_byte(body[i + needle.len()]);
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[test]
#[ignore]
fn audit_dead_regex() {
    audit_repo("regex");
}

#[test]
#[ignore]
fn audit_dead_gin() {
    audit_repo("gin");
}

#[test]
#[ignore]
fn audit_dead_axum() {
    audit_repo("axum");
}

#[test]
#[ignore]
fn audit_dead_nest() {
    audit_repo("nest");
}
