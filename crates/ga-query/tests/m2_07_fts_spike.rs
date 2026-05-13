//! M2-07 FTS approach spike — read-only benchmark (B: trigram, C: cached Levenshtein).
//!
//! Run with:
//!   GA_DJANGO_DB=$PWD/.graphatlas-bench-cache/refresh-gt-callers-django/django-9126cd/graph.db \
//!     cargo test --release -p ga-query --test m2_07_fts_spike -- --ignored --nocapture
//!
//! Not part of the regular test suite (`#[ignore]`) — results feed
//! `docs/spikes/fts-approach-decision.md`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

fn sample_seeds_from(names: &[String]) -> Vec<String> {
    // Deterministic but spread: stride through by a prime coprime with len(names),
    // pick names with 5..=15 chars.
    let n = names.len();
    if n == 0 {
        return Vec::new();
    }
    let stride: usize = 1013; // prime, likely coprime with n
    let mut out = Vec::new();
    let mut idx = 7usize; // arbitrary offset
    let mut tries = 0;
    while out.len() < 20 && tries < n * 2 {
        let name = &names[idx % n];
        if name.len() >= 5 && name.len() <= 15 {
            out.push(name.clone());
        }
        idx = idx.wrapping_add(stride);
        tries += 1;
    }
    out.truncate(20);
    out
}

fn percentile(mut xs: Vec<u128>, p: f64) -> u128 {
    xs.sort();
    if xs.is_empty() {
        return 0;
    }
    let idx = ((xs.len() as f64 - 1.0) * p).round() as usize;
    xs[idx]
}

fn trigrams(s: &str) -> Vec<[u8; 3]> {
    let b = s.as_bytes();
    if b.len() < 3 {
        return Vec::new();
    }
    let mut v = Vec::with_capacity(b.len() - 2);
    for w in b.windows(3) {
        v.push([
            w[0].to_ascii_lowercase(),
            w[1].to_ascii_lowercase(),
            w[2].to_ascii_lowercase(),
        ]);
    }
    v
}

fn levenshtein(a: &str, b: &str) -> u32 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        return m as u32;
    }
    if m == 0 {
        return n as u32;
    }
    let mut prev: Vec<u32> = (0..=m as u32).collect();
    let mut curr: Vec<u32> = vec![0; m + 1];
    for i in 1..=n {
        curr[0] = i as u32;
        for j in 1..=m {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (curr[j - 1] + 1).min(prev[j] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

/// RSS in bytes on macOS via `ps -o rss= -p <pid>` (in KB).
fn rss_bytes() -> u64 {
    let pid = std::process::id();
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output();
    if let Ok(o) = out {
        if let Ok(s) = String::from_utf8(o.stdout) {
            if let Ok(kb) = s.trim().parse::<u64>() {
                return kb * 1024;
            }
        }
    }
    0
}

fn load_names(db_path: &str) -> Vec<String> {
    let db = lbug::Database::new(db_path, lbug::SystemConfig::default()).expect("open graph.db");
    let conn = lbug::Connection::new(&db).expect("conn");
    let rs = conn
        .query("MATCH (s:Symbol) WHERE s.kind <> 'external' RETURN s.name")
        .expect("query names");
    let mut names = Vec::new();
    for row in rs {
        if let Some(lbug::Value::String(n)) = row.into_iter().next() {
            names.push(n);
        }
    }
    names
}

#[test]
#[ignore]
fn spike_fts_bench() {
    let db_path = std::env::var("GA_DJANGO_DB").unwrap_or_else(|_| {
        let here = std::env::current_dir().unwrap();
        // tests run from crate dir; walk up to repo root
        let mut p = here;
        loop {
            let candidate =
                p.join(".graphatlas-bench-cache/refresh-gt-callers-django/django-9126cd/graph.db");
            if candidate.exists() {
                return candidate.to_string_lossy().to_string();
            }
            if !p.pop() {
                panic!("cannot find django graph.db; set GA_DJANGO_DB");
            }
        }
    });
    let db_path = PathBuf::from(db_path);
    assert!(db_path.exists(), "graph.db missing: {:?}", db_path);
    let db_path_str = db_path.to_string_lossy().to_string();

    println!("\n=== M2-07 FTS Spike (db: {}) ===\n", db_path_str);

    let rss0 = rss_bytes();

    // ---- Phase 1: load all symbol names ----
    let t = Instant::now();
    let names = load_names(&db_path_str);
    let load_ms = t.elapsed().as_millis();
    println!("loaded {} symbol names in {} ms", names.len(), load_ms);

    let rss_after_load = rss_bytes();
    println!(
        "RSS after load: {} MB (delta {} MB)",
        rss_after_load / 1_048_576,
        rss_after_load.saturating_sub(rss0) / 1_048_576
    );

    // Dedup to simulate index keying
    let mut dedup: Vec<String> = names.clone();
    dedup.sort();
    dedup.dedup();
    println!("unique names: {}", dedup.len());

    let seeds = sample_seeds_from(&dedup);
    println!("sample seeds ({}): {:?}", seeds.len(), seeds);

    // ---- Approach C: cached-Vec Levenshtein scan ----
    println!("\n-- Approach C: cached Vec + Levenshtein --");
    let t = Instant::now();
    let cache_c: Vec<String> = dedup.clone();
    let build_c_ms = t.elapsed().as_millis();
    let approx_mem_c: usize = cache_c.iter().map(|s| s.len() + 24).sum::<usize>();
    println!(
        "C build: {} ms; approx mem: {} KB",
        build_c_ms,
        approx_mem_c / 1024
    );

    let mut c_times: Vec<u128> = Vec::new();
    for seed in &seeds {
        let t = Instant::now();
        let mut best: Vec<(u32, &str)> = Vec::with_capacity(cache_c.len());
        for name in &cache_c {
            let d = levenshtein(seed, name);
            best.push((d, name));
        }
        best.sort_by_key(|(d, _)| *d);
        let _top10: Vec<&str> = best.into_iter().take(10).map(|(_, n)| n).collect();
        c_times.push(t.elapsed().as_micros());
    }
    let c_p50 = percentile(c_times.clone(), 0.50);
    let c_p95 = percentile(c_times.clone(), 0.95);
    let c_max = *c_times.iter().max().unwrap_or(&0);
    println!(
        "C query: p50={} µs ({} ms), p95={} µs ({} ms), max={} µs",
        c_p50,
        c_p50 / 1000,
        c_p95,
        c_p95 / 1000,
        c_max
    );

    // ---- Approach B: trigram BTreeMap ----
    println!("\n-- Approach B: trigram BTreeMap --");
    let t = Instant::now();
    let mut trig_idx: BTreeMap<[u8; 3], Vec<u32>> = BTreeMap::new();
    // We use u32 SymbolId = index into cache_c.
    for (i, name) in cache_c.iter().enumerate() {
        for g in trigrams(name) {
            trig_idx.entry(g).or_default().push(i as u32);
        }
    }
    // Optional: dedup posting lists (each name can emit same trigram twice)
    for v in trig_idx.values_mut() {
        v.sort_unstable();
        v.dedup();
    }
    let build_b_ms = t.elapsed().as_millis();
    // Rough memory: each entry = 3 bytes key + 24-byte Vec header + 4*len postings
    let approx_mem_b: usize = trig_idx
        .values()
        .map(|v| 3 + 24 + v.len() * 4)
        .sum::<usize>()
        + approx_mem_c; // still need to hold names for candidate→name resolution
    println!(
        "B build: {} ms; {} trigrams; approx mem: {} KB",
        build_b_ms,
        trig_idx.len(),
        approx_mem_b / 1024
    );

    let mut b_times: Vec<u128> = Vec::new();
    for seed in &seeds {
        let t = Instant::now();
        // 1. Collect candidate SymbolIds: union of posting lists for seed trigrams,
        //    with a per-id hit-count filter ≥ ceil(len(seed_trigrams)/2).
        let seed_grams = trigrams(seed);
        if seed_grams.is_empty() {
            b_times.push(t.elapsed().as_micros());
            continue;
        }
        let mut counts: std::collections::HashMap<u32, u16> = std::collections::HashMap::new();
        for g in &seed_grams {
            if let Some(postings) = trig_idx.get(g) {
                for id in postings {
                    *counts.entry(*id).or_insert(0) += 1;
                }
            }
        }
        let threshold = seed_grams.len().div_ceil(2) as u16;
        // 2. Filter to candidates with enough trigram overlap, then Levenshtein-rank.
        let mut cands: Vec<(u32, &str)> = counts
            .iter()
            .filter(|(_, c)| **c >= threshold)
            .map(|(id, _)| {
                let name = cache_c[*id as usize].as_str();
                (levenshtein(seed, name), name)
            })
            .collect();
        cands.sort_by_key(|(d, _)| *d);
        let _top10: Vec<&str> = cands.into_iter().take(10).map(|(_, n)| n).collect();
        b_times.push(t.elapsed().as_micros());
    }
    let b_p50 = percentile(b_times.clone(), 0.50);
    let b_p95 = percentile(b_times.clone(), 0.95);
    let b_max = *b_times.iter().max().unwrap_or(&0);
    println!(
        "B query: p50={} µs ({} ms), p95={} µs ({} ms), max={} µs",
        b_p50,
        b_p50 / 1000,
        b_p95,
        b_p95 / 1000,
        b_max
    );

    let rss_final = rss_bytes();
    println!(
        "\nRSS final: {} MB (delta vs start {} MB)",
        rss_final / 1_048_576,
        rss_final.saturating_sub(rss0) / 1_048_576
    );

    println!("\n=== SPIKE SUMMARY ===");
    println!("names                = {}", names.len());
    println!("unique_names         = {}", dedup.len());
    println!("Approach C build_ms  = {}", build_c_ms);
    println!("Approach C query_p95 = {} µs", c_p95);
    println!("Approach C mem_kb    = {}", approx_mem_c / 1024);
    println!("Approach B build_ms  = {}", build_b_ms);
    println!("Approach B query_p95 = {} µs", b_p95);
    println!("Approach B mem_kb    = {}", approx_mem_b / 1024);
}

// ---------------------------------------------------------------------------
// Approach A — LadybugDB native FTS via runtime INSTALL/LOAD EXTENSION
// ---------------------------------------------------------------------------
//
// The rust-poc main.rs uses this pattern in production (see main.rs:2749-2759).
// We run it against the same django graph.db cache, use the same 20 seeds, and
// compare against B / C numbers in the companion spike doc.
//
// Run with:
//   GA_DJANGO_DB=$PWD/.graphatlas-bench-cache/refresh-gt-callers-django/django-9126cd/graph.db \
//     cargo test --release -p ga-query --test m2_07_fts_spike -- --ignored fts_approach_a_native_kuzu --nocapture
#[test]
#[ignore]
fn fts_approach_a_native_kuzu() {
    let db_path = std::env::var("GA_DJANGO_DB").unwrap_or_else(|_| {
        let here = std::env::current_dir().unwrap();
        let mut p = here;
        loop {
            let candidate =
                p.join(".graphatlas-bench-cache/refresh-gt-callers-django/django-9126cd/graph.db");
            if candidate.exists() {
                return candidate.to_string_lossy().to_string();
            }
            if !p.pop() {
                panic!("cannot find django graph.db; set GA_DJANGO_DB");
            }
        }
    });
    let db_path = PathBuf::from(db_path);
    assert!(db_path.exists(), "graph.db missing: {:?}", db_path);
    let db_path_str = db_path.to_string_lossy().to_string();

    println!(
        "\n=== M2-07 FTS Spike — Approach A (native Kuzu FTS) ===\n(db: {})\n",
        db_path_str
    );

    let rss0 = rss_bytes();

    // Open the django cache directly — same bypass pattern Approach B/C use
    // because index_state is mid-build.
    let db =
        lbug::Database::new(&db_path_str, lbug::SystemConfig::default()).expect("open graph.db");
    let conn = lbug::Connection::new(&db).expect("conn");
    let rss_after_open = rss_bytes();
    println!(
        "RSS after DB open: {} MB (delta {} MB)",
        rss_after_open / 1_048_576,
        rss_after_open.saturating_sub(rss0) / 1_048_576
    );

    // Load names (for seed reuse and sanity)
    let t = Instant::now();
    let names = load_names(&db_path_str);
    let load_ms = t.elapsed().as_millis();
    println!("loaded {} symbol names in {} ms", names.len(), load_ms);

    let mut dedup: Vec<String> = names.clone();
    dedup.sort();
    dedup.dedup();
    println!("unique names: {}", dedup.len());
    let seeds = sample_seeds_from(&dedup);
    println!("sample seeds ({}): {:?}", seeds.len(), seeds);

    // ---- INSTALL / LOAD EXTENSION ----
    println!("\n-- INSTALL fts --");
    let t = Instant::now();
    let install_res = conn.query("INSTALL fts");
    let install_ms = t.elapsed().as_millis();
    match &install_res {
        Ok(_) => println!("INSTALL fts: OK ({} ms)", install_ms),
        Err(e) => {
            println!("INSTALL fts: ERR ({} ms): {}", install_ms, e);
            println!("\n>>> Approach A UNAVAILABLE: INSTALL fts failed.");
            println!(">>> This is the empirical finding for the spike.");
            return;
        }
    }

    println!("-- LOAD EXTENSION fts --");
    let t = Instant::now();
    let load_res = conn.query("LOAD EXTENSION fts");
    let load_ext_ms = t.elapsed().as_millis();
    match &load_res {
        Ok(_) => println!("LOAD EXTENSION fts: OK ({} ms)", load_ext_ms),
        Err(e) => {
            println!("LOAD EXTENSION fts: ERR ({} ms): {}", load_ext_ms, e);
            println!("\n>>> Approach A UNAVAILABLE: LOAD EXTENSION failed.");
            return;
        }
    }

    // ---- CREATE_FTS_INDEX ----
    println!("\n-- CREATE_FTS_INDEX (stemmer='none') --");
    let t = Instant::now();
    let create_res =
        conn.query("CALL CREATE_FTS_INDEX('Symbol', 'symbol_fts', ['name'], stemmer := 'none')");
    let build_ms = t.elapsed().as_millis();
    match &create_res {
        Ok(_) => println!("CREATE_FTS_INDEX: OK (build {} ms)", build_ms),
        Err(e) => {
            println!("CREATE_FTS_INDEX: ERR ({} ms): {}", build_ms, e);
            println!("\n>>> Approach A UNAVAILABLE: index creation failed.");
            return;
        }
    }

    let rss_after_build = rss_bytes();
    println!(
        "RSS after index build: {} MB (delta vs start {} MB, delta vs open {} MB)",
        rss_after_build / 1_048_576,
        rss_after_build.saturating_sub(rss0) / 1_048_576,
        rss_after_build.saturating_sub(rss_after_open) / 1_048_576
    );

    // ---- QUERY ----
    println!("\n-- QUERY_FTS_INDEX per seed --");
    let mut a_times: Vec<u128> = Vec::new();
    let mut hit_counts: Vec<usize> = Vec::new();
    for seed in &seeds {
        // Escape single quotes in seed (symbol identifiers shouldn't contain them, but be safe).
        let esc = seed.replace('\'', "''");
        let q = format!(
            "CALL QUERY_FTS_INDEX('symbol_fts', '{}', top_k := 20) RETURN node.name, node.file",
            esc
        );
        let t = Instant::now();
        let rs = conn.query(&q);
        let dt = t.elapsed().as_micros();
        match rs {
            Ok(rows) => {
                let n = rows.into_iter().count();
                hit_counts.push(n);
                a_times.push(dt);
            }
            Err(e) => {
                println!("  seed={:?} ERR: {}", seed, e);
                hit_counts.push(0);
                a_times.push(dt);
            }
        }
    }
    let a_p50 = percentile(a_times.clone(), 0.50);
    let a_p95 = percentile(a_times.clone(), 0.95);
    let a_max = *a_times.iter().max().unwrap_or(&0);
    let mean_hits: f64 =
        hit_counts.iter().map(|&x| x as f64).sum::<f64>() / hit_counts.len().max(1) as f64;
    println!(
        "A query: p50={} µs ({} ms), p95={} µs ({} ms), max={} µs, mean_hits={:.1}",
        a_p50,
        a_p50 / 1000,
        a_p95,
        a_p95 / 1000,
        a_max,
        mean_hits
    );

    let rss_after_query = rss_bytes();
    println!(
        "RSS after queries: {} MB (delta vs start {} MB)",
        rss_after_query / 1_048_576,
        rss_after_query.saturating_sub(rss0) / 1_048_576
    );

    // ---- DROP_FTS_INDEX to keep cache clean ----
    println!("\n-- DROP_FTS_INDEX --");
    match conn.query("CALL DROP_FTS_INDEX('Symbol', 'symbol_fts')") {
        Ok(_) => println!("DROP_FTS_INDEX: OK"),
        Err(e) => println!("DROP_FTS_INDEX: ERR: {}", e),
    }

    // ---- Gate + summary ----
    let gate_query = a_p95 < 50_000; // µs
    let gate_build = build_ms < 30_000;
    let mem_delta_bytes = rss_after_build.saturating_sub(rss0);
    let gate_mem = mem_delta_bytes < 500 * 1_048_576;

    println!("\n=== APPROACH A SUMMARY ===");
    println!("install_ms        = {}", install_ms);
    println!("load_ext_ms       = {}", load_ext_ms);
    println!("build_ms          = {}", build_ms);
    println!("query_p50_us      = {}", a_p50);
    println!("query_p95_us      = {}", a_p95);
    println!("query_max_us      = {}", a_max);
    println!("mem_delta_MB      = {}", mem_delta_bytes / 1_048_576);
    println!("mean_hits_top20   = {:.1}", mean_hits);
    println!(
        "GATE query<50ms   = {}  (measured {} µs)",
        gate_query, a_p95
    );
    println!(
        "GATE build<30s    = {}  (measured {} ms)",
        gate_build, build_ms
    );
    println!(
        "GATE mem<500MB    = {}  (measured {} MB)",
        gate_mem,
        mem_delta_bytes / 1_048_576
    );
    println!(
        "GATE OVERALL      = {}",
        gate_query && gate_build && gate_mem
    );
}
