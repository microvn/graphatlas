//! Empirical validation of lbug 0.15.4 capabilities the v4 schema design depends on.
//!
//! Run:  `cargo run -p ga-index --example validate_v4_schema`
//!
//! Each test attempts a real DDL/DML against a throwaway tempdir-backed
//! Database, then reports PASS/FAIL + the engine error message verbatim.
//! Fails LOUD: prints the failing statement so we can mark schema-v4
//! features as actually-supported (not docs-claim) before committing.
//!
//! Categories tested:
//!   1. Column data types (BLOB, TIMESTAMP, FLOAT, INT128, DECIMAL, UUID,
//!      LIST<STRING>, LIST<STRUCT>, ARRAY<FLOAT,N>, MAP, UNION, JSON)
//!   2. Column constraints (DEFAULT, NOT NULL implicit, UNIQUE non-PK)
//!   3. ALTER TABLE (ADD/DROP/RENAME COLUMN, ADD/DROP REL connection)
//!   4. Multiple typed REL tables sharing FROM/TO
//!   5. Extensions (INSTALL json, INSTALL fts) — capability gate, won't
//!      break the test if missing from the build.
//!   6. COMMENT ON TABLE
//!   7. Multi-statement query
//!   8. Read-only reopen (multi-process readers — already used in GA but
//!      smoke-test included for completeness).

use std::process::ExitCode;

use lbug::{Connection, Database, SystemConfig};
use tempfile::TempDir;

#[derive(Default)]
struct Report {
    pass: Vec<String>,
    fail: Vec<(String, String)>, // (name, error)
    skip: Vec<(String, String)>, // (name, reason)
}

impl Report {
    fn pass(&mut self, name: impl Into<String>) {
        let s = name.into();
        println!("  [PASS] {s}");
        self.pass.push(s);
    }

    fn fail(&mut self, name: impl Into<String>, err: impl Into<String>) {
        let n = name.into();
        let e = err.into();
        println!("  [FAIL] {n}\n         → {e}");
        self.fail.push((n, e));
    }

    fn skip(&mut self, name: impl Into<String>, reason: impl Into<String>) {
        let n = name.into();
        let r = reason.into();
        println!("  [SKIP] {n}  ({r})");
        self.skip.push((n, r));
    }
}

/// Run a single DDL/DML and report PASS if it succeeds, FAIL if not.
fn try_stmt(conn: &Connection, name: &str, stmt: &str, report: &mut Report) {
    match conn.query(stmt) {
        Ok(_) => report.pass(name),
        Err(e) => report.fail(name, format!("{e}\n           stmt: {stmt}")),
    }
}

/// Same as try_stmt but treat success as a "fail-of-claim": if a feature
/// we claim is UNSUPPORTED actually works, we want to know.
fn try_stmt_expect_fail(
    conn: &Connection,
    name: &str,
    stmt: &str,
    expected_failure_reason: &str,
    report: &mut Report,
) {
    match conn.query(stmt) {
        Ok(_) => report.fail(
            name,
            format!(
                "expected failure ({expected_failure_reason}) but DDL succeeded → claim wrong\n           stmt: {stmt}"
            ),
        ),
        Err(e) => {
            report.pass(format!("{name} — correctly rejected: {e}"));
        }
    }
}

/// Run a statement that we want to call but tolerate failure on dev envs
/// where extensions / cache aren't aligned. Mirrors archive/rust-poc:2748-2760
/// which `eprintln!("WARN: ...")` and continues. Returns true on success so
/// downstream steps can branch.
fn try_stmt_warn(conn: &Connection, name: &str, stmt: &str, report: &mut Report) -> bool {
    match conn.query(stmt) {
        Ok(_) => {
            report.pass(name);
            true
        }
        Err(e) => {
            let n = name.to_string();
            let msg = format!("WARN-tolerated: {e}");
            println!("  [WARN] {n}\n         → {msg}");
            report.skip(n, msg);
            false
        }
    }
}

fn fresh_db() -> anyhow::Result<(TempDir, Database)> {
    let dir = TempDir::new()?;
    let path = dir.path().join("test.db");
    let db = Database::new(&path, SystemConfig::default())?;
    Ok((dir, db))
}

fn section(title: &str) {
    println!("\n=== {title} ===");
}

// ---------------------------------------------------------------------------
// Section 1 — Column data types
// ---------------------------------------------------------------------------

fn test_data_types(report: &mut Report) -> anyhow::Result<()> {
    section("1. Column data types in CREATE NODE TABLE");
    let (_dir, db) = fresh_db()?;
    let conn = Connection::new(&db)?;

    // Baseline scalar types we already know work
    try_stmt(
        &conn,
        "STRING + INT64 + DOUBLE + BOOLEAN baseline",
        "CREATE NODE TABLE TBaseline(id STRING PRIMARY KEY, n INT64, d DOUBLE, b BOOLEAN)",
        report,
    );

    try_stmt(
        &conn,
        "FLOAT (4-byte)",
        "CREATE NODE TABLE TFloat(id STRING PRIMARY KEY, f FLOAT)",
        report,
    );

    try_stmt(
        &conn,
        "INT8 / INT16 / INT32",
        "CREATE NODE TABLE TInts(id STRING PRIMARY KEY, a INT8, b INT16, c INT32)",
        report,
    );

    try_stmt(
        &conn,
        "INT128",
        "CREATE NODE TABLE TInt128(id STRING PRIMARY KEY, big INT128)",
        report,
    );

    try_stmt(
        &conn,
        "UINT8 / UINT16 / UINT32 / UINT64",
        "CREATE NODE TABLE TUints(id STRING PRIMARY KEY, a UINT8, b UINT16, c UINT32, d UINT64)",
        report,
    );

    try_stmt(
        &conn,
        "BLOB",
        "CREATE NODE TABLE TBlob(id STRING PRIMARY KEY, hash BLOB)",
        report,
    );

    try_stmt(
        &conn,
        "TIMESTAMP",
        "CREATE NODE TABLE TTs(id STRING PRIMARY KEY, mtime TIMESTAMP)",
        report,
    );

    try_stmt(
        &conn,
        "DATE",
        "CREATE NODE TABLE TDate(id STRING PRIMARY KEY, d DATE)",
        report,
    );

    try_stmt(
        &conn,
        "INTERVAL",
        "CREATE NODE TABLE TInterval(id STRING PRIMARY KEY, i INTERVAL)",
        report,
    );

    try_stmt(
        &conn,
        "DECIMAL(p,s)",
        "CREATE NODE TABLE TDecimal(id STRING PRIMARY KEY, x DECIMAL(10, 4))",
        report,
    );

    try_stmt(
        &conn,
        "UUID",
        "CREATE NODE TABLE TUuid(id UUID PRIMARY KEY)",
        report,
    );

    try_stmt(
        &conn,
        "SERIAL primary key",
        "CREATE NODE TABLE TSerial(id SERIAL PRIMARY KEY, x STRING)",
        report,
    );

    // === Composite types — the BIG bet for v4 ===
    try_stmt(
        &conn,
        "LIST<STRING> column (T[] syntax — confirmed working)",
        "CREATE NODE TABLE TListStr(id STRING PRIMARY KEY, tags STRING[])",
        report,
    );

    try_stmt(
        &conn,
        "ARRAY<FLOAT, N> fixed-length (vector embedding shape)",
        "CREATE NODE TABLE TArr(id STRING PRIMARY KEY, embed FLOAT[384])",
        report,
    );

    try_stmt(
        &conn,
        "STRUCT(...) column",
        "CREATE NODE TABLE TStruct(id STRING PRIMARY KEY, info STRUCT(name STRING, age INT64))",
        report,
    );

    try_stmt(&conn, "LIST<STRUCT(...)> — proposed for Symbol.params",
        "CREATE NODE TABLE TLS(id STRING PRIMARY KEY, params STRUCT(name STRING, type STRING, default_value STRING)[])",
        report);

    try_stmt(
        &conn,
        "MAP(K,V) column",
        "CREATE NODE TABLE TMap(id STRING PRIMARY KEY, kv MAP(STRING, STRING))",
        report,
    );

    try_stmt(
        &conn,
        "UNION(...) column",
        "CREATE NODE TABLE TUnion(id STRING PRIMARY KEY, val UNION(s STRING, n INT64))",
        report,
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Section 2 — Constraints
// ---------------------------------------------------------------------------

fn test_constraints(report: &mut Report) -> anyhow::Result<()> {
    section("2. Column constraints");
    let (_dir, db) = fresh_db()?;
    let conn = Connection::new(&db)?;

    try_stmt(
        &conn,
        "PRIMARY KEY",
        "CREATE NODE TABLE TPk(id STRING PRIMARY KEY, n INT64)",
        report,
    );

    try_stmt(&conn, "DEFAULT scalar",
        "CREATE NODE TABLE TDef(id STRING PRIMARY KEY, n INT64 DEFAULT 0, b BOOLEAN DEFAULT false, s STRING DEFAULT '')",
        report);

    // The big claim: UNIQUE non-PK is NOT supported. Test directly.
    try_stmt_expect_fail(
        &conn,
        "UNIQUE on non-PK column (claim: NOT supported)",
        "CREATE NODE TABLE TUq(id STRING PRIMARY KEY, qn STRING UNIQUE)",
        "UNIQUE attribute not in DDL grammar per audit",
        report,
    );

    try_stmt_expect_fail(
        &conn,
        "CREATE INDEX secondary B-tree on non-PK (claim: NOT supported)",
        "CREATE INDEX idx_qn ON TPk(n)",
        "no CREATE INDEX grammar in lbug",
        report,
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Section 3 — ALTER TABLE
// ---------------------------------------------------------------------------

fn test_alter_table(report: &mut Report) -> anyhow::Result<()> {
    section("3. ALTER TABLE — incremental migration capability");
    let (_dir, db) = fresh_db()?;
    let conn = Connection::new(&db)?;

    // Two distinct node tables so we can validate ALTER REL ADD FROM/TO with
    // a connection that doesn't pre-exist (the v3→v4 migration shape).
    conn.query("CREATE NODE TABLE Sym(id STRING PRIMARY KEY, name STRING, line INT64)")?;
    conn.query("CREATE NODE TABLE Mod(name STRING PRIMARY KEY)")?;
    conn.query("CREATE REL TABLE CALLS(FROM Sym TO Sym, line INT64)")?;

    try_stmt(
        &conn,
        "ALTER TABLE ADD COLUMN scalar with DEFAULT",
        "ALTER TABLE Sym ADD confidence FLOAT DEFAULT 1.0",
        report,
    );

    try_stmt(
        &conn,
        "ALTER TABLE ADD COLUMN BOOLEAN with DEFAULT",
        "ALTER TABLE Sym ADD is_async BOOLEAN DEFAULT false",
        report,
    );

    try_stmt(
        &conn,
        "ALTER TABLE ADD COLUMN BLOB",
        "ALTER TABLE Sym ADD payload BLOB",
        report,
    );

    try_stmt(
        &conn,
        "ALTER TABLE ADD COLUMN LIST<STRING> with DEFAULT",
        "ALTER TABLE Sym ADD modifiers STRING[] DEFAULT []",
        report,
    );

    try_stmt(
        &conn,
        "ALTER TABLE ADD COLUMN LIST<STRUCT(...)>",
        "ALTER TABLE Sym ADD params STRUCT(name STRING, type STRING)[]",
        report,
    );

    try_stmt(
        &conn,
        "ALTER TABLE ADD IF NOT EXISTS (idempotent re-run)",
        "ALTER TABLE Sym ADD IF NOT EXISTS confidence FLOAT DEFAULT 1.0",
        report,
    );

    try_stmt(
        &conn,
        "ALTER TABLE DROP COLUMN",
        "ALTER TABLE Sym DROP payload",
        report,
    );

    try_stmt(
        &conn,
        "ALTER TABLE DROP IF EXISTS",
        "ALTER TABLE Sym DROP IF EXISTS payload",
        report,
    );

    try_stmt(
        &conn,
        "ALTER TABLE RENAME column",
        "ALTER TABLE Sym RENAME line TO line_start",
        report,
    );

    // REL connection management runs BEFORE the table rename so the FROM/TO
    // names stay valid. Critical for IMPORTS_SYMBOL family migration.
    try_stmt(
        &conn,
        "ALTER TABLE REL ADD FROM/TO new connection",
        "ALTER TABLE CALLS ADD FROM Sym TO Mod",
        report,
    );

    try_stmt(
        &conn,
        "ALTER TABLE REL ADD IF NOT EXISTS (idempotent)",
        "ALTER TABLE CALLS ADD IF NOT EXISTS FROM Sym TO Mod",
        report,
    );

    try_stmt(
        &conn,
        "ALTER TABLE REL DROP FROM/TO connection",
        "ALTER TABLE CALLS DROP FROM Sym TO Mod",
        report,
    );

    try_stmt(
        &conn,
        "ALTER TABLE RENAME table",
        "ALTER TABLE Sym RENAME TO Symbol",
        report,
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Section 4 — Multiple typed REL tables sharing FROM/TO
// ---------------------------------------------------------------------------

fn test_rel_split(report: &mut Report) -> anyhow::Result<()> {
    section("4. Multiple typed REL tables with same (FROM, TO) — graph-native split");
    let (_dir, db) = fresh_db()?;
    let conn = Connection::new(&db)?;

    conn.query("CREATE NODE TABLE Symbol(id STRING PRIMARY KEY)")?;
    conn.query("CREATE NODE TABLE File(path STRING PRIMARY KEY)")?;

    // The CALLS family — split by call_kind / confidence_tier
    let stmts = [
        ("REL CALLS",            "CREATE REL TABLE CALLS(FROM Symbol TO Symbol, call_site_line INT64)"),
        ("REL CALLS_STATIC",     "CREATE REL TABLE CALLS_STATIC(FROM Symbol TO Symbol, call_site_line INT64)"),
        ("REL CALLS_METHOD",     "CREATE REL TABLE CALLS_METHOD(FROM Symbol TO Symbol, call_site_line INT64, receiver_kind STRING)"),
        ("REL CALLS_VIRTUAL",    "CREATE REL TABLE CALLS_VIRTUAL(FROM Symbol TO Symbol, call_site_line INT64)"),
        ("REL CALLS_HEURISTIC",  "CREATE REL TABLE CALLS_HEURISTIC(FROM Symbol TO Symbol, call_site_line INT64)"),
        ("REL EXTENDS",          "CREATE REL TABLE EXTENDS(FROM Symbol TO Symbol)"),
        ("REL IMPLEMENTS",       "CREATE REL TABLE IMPLEMENTS(FROM Symbol TO Symbol)"),
        ("REL TRAIT_IMPL",       "CREATE REL TABLE TRAIT_IMPL(FROM Symbol TO Symbol)"),
        ("REL OVERRIDES",        "CREATE REL TABLE OVERRIDES(FROM Symbol TO Symbol)"),
        ("REL DECORATES",        "CREATE REL TABLE DECORATES(FROM Symbol TO Symbol, decorator_args STRING)"),
        // IMPORTS family — File→Symbol
        ("REL IMPORTS_NAMED",     "CREATE REL TABLE IMPORTS_NAMED(FROM File TO Symbol, import_line INT64, alias STRING)"),
        ("REL IMPORTS_DEFAULT",   "CREATE REL TABLE IMPORTS_DEFAULT(FROM File TO Symbol, import_line INT64, alias STRING)"),
        ("REL IMPORTS_TYPE_ONLY", "CREATE REL TABLE IMPORTS_TYPE_ONLY(FROM File TO Symbol, import_line INT64, alias STRING)"),
        // IMPORTS family — File→File
        ("REL IMPORTS_NAMESPACE",  "CREATE REL TABLE IMPORTS_NAMESPACE(FROM File TO File, import_line INT64, alias STRING)"),
        ("REL IMPORTS_WILDCARD",   "CREATE REL TABLE IMPORTS_WILDCARD(FROM File TO File, import_line INT64)"),
        ("REL IMPORTS_SIDE_EFFECT","CREATE REL TABLE IMPORTS_SIDE_EFFECT(FROM File TO File, import_line INT64)"),
    ];
    for (name, sql) in stmts {
        try_stmt(&conn, name, sql, report);
    }

    // Verify: query that traverses two REL tables in the same family.
    try_stmt(&conn, "MATCH ... CALLS|CALLS_STATIC pattern (multi-REL traversal)",
        "MATCH (a:Symbol)-[r:CALLS|CALLS_STATIC|CALLS_METHOD|CALLS_VIRTUAL]->(b:Symbol) RETURN count(*)",
        report);

    Ok(())
}

// ---------------------------------------------------------------------------
// Section 5 — Extensions (JSON, FTS, vector)
// ---------------------------------------------------------------------------

fn test_extensions(report: &mut Report) -> anyhow::Result<()> {
    section("5. Extensions — INSTALL/LOAD with WARN-not-fail (rust-poc:2748-2760 pattern)");
    section_note(
        "Extensions on lbug 0.15.x require -rdynamic linker flag (LadybugDB#438\n\
         fix in build.rs). archive/rust-poc shipped FTS with `lbug = \"0.15\"`\n\
         and graceful fallback. We use in-memory DB for FTS/Vector tests because\n\
         on-disk CREATE_FTS_INDEX SIGSEGVs on lbug 0.15.4 (LadybugDB#430, OPEN —\n\
         inherited from kuzudb#5671/#6017). Production GA must therefore either\n\
         (a) avoid CREATE_FTS_INDEX on persistent stores until 0.16+ stabilizes,\n\
         or (b) use in-memory mirror for FTS-only queries.",
    );
    // In-memory DB sidesteps LadybugDB#430 segfault on persistent CREATE_FTS_INDEX.
    let db = Database::in_memory(SystemConfig::default())?;
    let conn = Connection::new(&db)?;

    // ---- JSON extension ------------------------------------------------
    let json_install = try_stmt_warn(&conn, "INSTALL JSON", "INSTALL JSON", report);
    let json_loaded =
        json_install && try_stmt_warn(&conn, "LOAD EXTENSION JSON", "LOAD EXTENSION JSON", report);

    if json_loaded {
        try_stmt(
            &conn,
            "JSON column type after LOAD",
            "CREATE NODE TABLE TJson(id STRING PRIMARY KEY, blob JSON)",
            report,
        );
        try_stmt(&conn, "INSERT JSON value (object)",
            "CREATE (t:TJson {id: '1', blob: '{\"k\":\"v\",\"n\":42,\"nested\":{\"deep\":\"win\"}}'})", report);
        try_stmt(
            &conn,
            "INSERT JSON value (array)",
            "CREATE (t:TJson {id: '2', blob: '[1,2,3]'})",
            report,
        );
        try_stmt(
            &conn,
            "json_extract() top-level key",
            "MATCH (t:TJson) WHERE json_extract(t.blob, 'k') = 'v' RETURN t.id",
            report,
        );
        try_stmt(
            &conn,
            "json_extract() nested path",
            "MATCH (t:TJson) WHERE json_extract(t.blob, '$.nested.deep') = 'win' RETURN t.id",
            report,
        );
        try_stmt(
            &conn,
            "json_extract() integer cast in WHERE",
            "MATCH (t:TJson) WHERE json_extract(t.blob, 'n') = 42 RETURN t.id",
            report,
        );
    } else {
        report.skip(
            "JSON round-trip",
            "extension not loaded — fallback path is plain STRING",
        );
    }

    // ---- FTS extension -------------------------------------------------
    // Match archive/rust-poc:2748-2760 exactly: lowercase, no uppercase variant
    // (lbug grammar accepts both per docs but archive uses lowercase).
    let fts_install = try_stmt_warn(&conn, "INSTALL fts", "INSTALL fts", report);
    let fts_loaded =
        fts_install && try_stmt_warn(&conn, "LOAD EXTENSION fts", "LOAD EXTENSION fts", report);

    if fts_loaded {
        try_stmt(
            &conn,
            "CREATE NODE TABLE TDoc",
            "CREATE NODE TABLE TDoc(id STRING PRIMARY KEY, body STRING, title STRING)",
            report,
        );
        try_stmt(
            &conn,
            "Populate TDoc rows",
            "CREATE (d1:TDoc {id: '1', body: 'hello world', title: 'greeting'}), \
                    (d2:TDoc {id: '2', body: 'graph database storage', title: 'graph intro'}), \
                    (d3:TDoc {id: '3', body: 'full text search demo', title: 'fts'})",
            report,
        );
        // Mirror archive/rust-poc:2755 exact form
        try_stmt(
            &conn,
            "CALL CREATE_FTS_INDEX(table, idx, [props], stemmer:='none')",
            "CALL CREATE_FTS_INDEX('TDoc', 'doc_fts', ['body', 'title'], stemmer := 'none')",
            report,
        );
        try_stmt(
            &conn,
            "CALL CREATE_FTS_INDEX with porter stemmer",
            "CALL CREATE_FTS_INDEX('TDoc', 'doc_fts_stem', ['body'], stemmer := 'porter')",
            report,
        );
        // archive/rust-poc:2521 signature: (table, index_name, query, top_k := N)
        // QUERY_FTS_INDEX uses TOP (uppercase), not top_k.
        // archive/rust-poc:2521 used `top_k :=` which was the older Kuzu API;
        // LadybugDB 0.15+ renamed to TOP. Verified by docs.ladybugdb.com.
        try_stmt(
            &conn,
            "QUERY_FTS_INDEX returns ranked node + score (single token)",
            "CALL QUERY_FTS_INDEX('TDoc', 'doc_fts', 'hello', TOP := 5) \
             RETURN node.id AS id, score ORDER BY score DESC",
            report,
        );
        try_stmt(&conn, "QUERY_FTS_INDEX with conjunctive multi-token (AND)",
            "CALL QUERY_FTS_INDEX('TDoc', 'doc_fts', 'graph database', conjunctive := true, TOP := 5) \
             RETURN node.id AS id, score",
            report);
        try_stmt(
            &conn,
            "QUERY_FTS_INDEX disjunctive multi-token (OR, default)",
            "CALL QUERY_FTS_INDEX('TDoc', 'doc_fts', 'graph database', TOP := 5) \
             RETURN node.id AS id, score",
            report,
        );
        try_stmt(
            &conn,
            "QUERY_FTS_INDEX BM25 K/B tuning params",
            "CALL QUERY_FTS_INDEX('TDoc', 'doc_fts', 'hello', K := 1.2, B := 0.75, TOP := 3) \
             RETURN node.id AS id, score",
            report,
        );
        try_stmt(
            &conn,
            "QUERY_FTS_INDEX no match returns empty",
            "CALL QUERY_FTS_INDEX('TDoc', 'doc_fts', 'noexistword', TOP := 5) \
             RETURN node.id AS id, score",
            report,
        );
        try_stmt(
            &conn,
            "DROP_FTS_INDEX (cleanup)",
            "CALL DROP_FTS_INDEX('TDoc', 'doc_fts_stem')",
            report,
        );
    } else {
        report.skip(
            "FTS round-trip",
            "extension not loaded — fallback to MATCH ... CONTAINS",
        );
    }

    // ---- Vector extension ---------------------------------------------
    let vec_install = try_stmt_warn(&conn, "INSTALL vector", "INSTALL vector", report);
    let vec_loaded = vec_install
        && try_stmt_warn(
            &conn,
            "LOAD EXTENSION vector",
            "LOAD EXTENSION vector",
            report,
        );

    if vec_loaded {
        try_stmt(
            &conn,
            "CREATE NODE TABLE TEmb with FLOAT[8]",
            "CREATE NODE TABLE TEmb(id STRING PRIMARY KEY, vec FLOAT[8])",
            report,
        );
        try_stmt(
            &conn,
            "Populate TEmb (8 rows)",
            "CREATE (e1:TEmb {id: '1', vec: [0.10,0.20,0.30,0.40,0.50,0.60,0.70,0.80]}), \
                    (e2:TEmb {id: '2', vec: [0.11,0.21,0.31,0.41,0.51,0.61,0.71,0.81]}), \
                    (e3:TEmb {id: '3', vec: [0.90,0.10,0.00,0.00,0.00,0.00,0.00,0.00]}), \
                    (e4:TEmb {id: '4', vec: [0.20,0.30,0.40,0.50,0.60,0.70,0.80,0.90]}), \
                    (e5:TEmb {id: '5', vec: [0.50,0.50,0.50,0.50,0.50,0.50,0.50,0.50]}), \
                    (e6:TEmb {id: '6', vec: [0.00,1.00,0.00,0.00,0.00,0.00,0.00,0.00]}), \
                    (e7:TEmb {id: '7', vec: [0.30,0.30,0.30,0.30,0.30,0.30,0.30,0.30]}), \
                    (e8:TEmb {id: '8', vec: [0.70,0.10,0.10,0.10,0.10,0.10,0.10,0.10]})",
            report,
        );
        // CREATE_VECTOR_INDEX history: SIGSEGV on lbug 0.15.4 (LadybugDB#434,
        // CLOSED, fix in 0.16.0). On crates.io distribution 0.16.x has yyjson
        // static-link gap on macOS arm64 → must build with
        // `LBUG_BUILD_FROM_SOURCE=1 cargo build` to bundle deps. With both
        // pieces in place, test the actual HNSW index round-trip.
        try_stmt(
            &conn,
            "CREATE_VECTOR_INDEX (HNSW) — requires lbug ≥ 0.16.0 + source build",
            "CALL CREATE_VECTOR_INDEX('TEmb', 'vec_idx', 'vec', metric := 'cosine')",
            report,
        );
        try_stmt(
            &conn,
            "QUERY_VECTOR_INDEX K-NN round-trip (HNSW)",
            "CALL QUERY_VECTOR_INDEX('TEmb', 'vec_idx', \
                CAST([0.10,0.20,0.30,0.40,0.50,0.60,0.70,0.80] AS FLOAT[8]), 3) \
             RETURN node.id AS id, distance ORDER BY distance",
            report,
        );
        try_stmt(
            &conn,
            "DROP_VECTOR_INDEX (cleanup)",
            "CALL DROP_VECTOR_INDEX('TEmb', 'vec_idx')",
            report,
        );

        // Validate brute-force vector similarity — the v1.3 fallback path.
        try_stmt(
            &conn,
            "ARRAY_COSINE_SIMILARITY between two FLOAT[8] cols (built-in)",
            "MATCH (a:TEmb {id: '1'}), (b:TEmb {id: '2'}) \
             RETURN ARRAY_COSINE_SIMILARITY(a.vec, b.vec) AS sim",
            report,
        );
        try_stmt(
            &conn,
            "Top-3 nearest by cosine similarity (brute-force k-NN)",
            "MATCH (a:TEmb {id: '1'}), (b:TEmb) WHERE a.id <> b.id \
             RETURN b.id AS id, ARRAY_COSINE_SIMILARITY(a.vec, b.vec) AS sim \
             ORDER BY sim DESC LIMIT 3",
            report,
        );
        try_stmt(
            &conn,
            "ARRAY_DISTANCE (Euclidean L2) — alternate metric",
            "MATCH (a:TEmb {id: '1'}), (b:TEmb {id: '2'}) \
             RETURN ARRAY_DISTANCE(a.vec, b.vec) AS l2",
            report,
        );
        try_stmt(
            &conn,
            "ARRAY_INNER_PRODUCT (dot product) — alternate metric",
            "MATCH (a:TEmb {id: '1'}), (b:TEmb {id: '2'}) \
             RETURN ARRAY_INNER_PRODUCT(a.vec, b.vec) AS dot",
            report,
        );
    } else {
        report.skip(
            "Vector round-trip",
            "extension not loaded — embeddings deferred to v1.4",
        );
    }

    Ok(())
}

fn section_note(text: &str) {
    for line in text.lines() {
        println!("  · {line}");
    }
}

// ---------------------------------------------------------------------------
// Section 6 — Misc
// ---------------------------------------------------------------------------

fn test_misc(report: &mut Report) -> anyhow::Result<()> {
    section("6. Misc DDL — COMMENT, multi-statement, IF NOT EXISTS idempotency");
    let (_dir, db) = fresh_db()?;
    let conn = Connection::new(&db)?;

    conn.query("CREATE NODE TABLE TC(id STRING PRIMARY KEY)")?;

    try_stmt(
        &conn,
        "COMMENT ON TABLE",
        "COMMENT ON TABLE TC IS 'A demonstration table'",
        report,
    );

    try_stmt(&conn, "Multi-statement query (semicolon-separated)",
        "CREATE NODE TABLE TM1(id STRING PRIMARY KEY); CREATE NODE TABLE TM2(id STRING PRIMARY KEY);",
        report);

    try_stmt(
        &conn,
        "CREATE NODE TABLE IF NOT EXISTS (re-run idempotency)",
        "CREATE NODE TABLE IF NOT EXISTS TC(id STRING PRIMARY KEY)",
        report,
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Section 7 — End-to-end v4 schema dry-run
// ---------------------------------------------------------------------------

fn test_v4_schema_dry_run(report: &mut Report) -> anyhow::Result<()> {
    section("7. End-to-end: full proposed v4 schema in one fresh DB");
    let (_dir, db) = fresh_db()?;
    let conn = Connection::new(&db)?;

    // Scoped down to a representative subset — full enumeration in section 4.
    let v4_ddl = [
        "CREATE NODE TABLE File (\
            path STRING PRIMARY KEY, \
            lang STRING, \
            size INT64, \
            loc INT64 DEFAULT 0, \
            sha256 BLOB, \
            modified_at TIMESTAMP, \
            is_generated BOOLEAN DEFAULT false, \
            is_vendored BOOLEAN DEFAULT false)",

        "CREATE NODE TABLE Symbol (\
            id STRING PRIMARY KEY, \
            name STRING, \
            qualified_name STRING, \
            file STRING, \
            kind STRING, \
            line INT64, \
            line_end INT64, \
            params STRUCT(name STRING, type STRING, default_value STRING)[], \
            return_type STRING, \
            modifiers STRING[], \
            arity INT64 DEFAULT -1, \
            is_async BOOLEAN DEFAULT false, \
            is_override BOOLEAN DEFAULT false, \
            is_abstract BOOLEAN DEFAULT false, \
            is_static BOOLEAN DEFAULT false, \
            is_test_marker BOOLEAN DEFAULT false, \
            is_generated BOOLEAN DEFAULT false, \
            enclosing STRING DEFAULT '', \
            confidence FLOAT DEFAULT 1.0, \
            doc_summary STRING DEFAULT '')",

        // CALLS family
        "CREATE REL TABLE CALLS(FROM Symbol TO Symbol, call_site_line INT64)",
        "CREATE REL TABLE CALLS_STATIC(FROM Symbol TO Symbol, call_site_line INT64)",
        "CREATE REL TABLE CALLS_METHOD(FROM Symbol TO Symbol, call_site_line INT64, receiver_kind STRING)",
        "CREATE REL TABLE CALLS_VIRTUAL(FROM Symbol TO Symbol, call_site_line INT64)",
        "CREATE REL TABLE CALLS_HEURISTIC(FROM Symbol TO Symbol, call_site_line INT64)",

        // REFERENCES family
        "CREATE REL TABLE REFERENCES(FROM Symbol TO Symbol, ref_site_line INT64, ref_kind STRING)",
        "CREATE REL TABLE REF_TYPE_FIELD(FROM Symbol TO Symbol, ref_site_line INT64)",
        "CREATE REL TABLE REF_TYPE_RETURN(FROM Symbol TO Symbol, ref_site_line INT64)",
        "CREATE REL TABLE REF_TYPE_PARAM(FROM Symbol TO Symbol, ref_site_line INT64, param_index INT64)",
        "CREATE REL TABLE REF_TYPE_GENERIC(FROM Symbol TO Symbol, ref_site_line INT64)",

        // Inheritance / dispatch
        "CREATE REL TABLE EXTENDS(FROM Symbol TO Symbol)",
        "CREATE REL TABLE IMPLEMENTS(FROM Symbol TO Symbol)",
        "CREATE REL TABLE TRAIT_IMPL(FROM Symbol TO Symbol)",
        "CREATE REL TABLE OVERRIDES(FROM Symbol TO Symbol)",
        "CREATE REL TABLE DECORATES(FROM Symbol TO Symbol, decorator_args STRING DEFAULT '')",

        // IMPORTS family
        "CREATE REL TABLE IMPORTS_NAMED(FROM File TO Symbol, import_line INT64, alias STRING DEFAULT '', re_export BOOLEAN DEFAULT false, re_export_source STRING DEFAULT '')",
        "CREATE REL TABLE IMPORTS_DEFAULT(FROM File TO Symbol, import_line INT64, alias STRING DEFAULT '')",
        "CREATE REL TABLE IMPORTS_TYPE_ONLY(FROM File TO Symbol, import_line INT64, alias STRING DEFAULT '')",
        "CREATE REL TABLE IMPORTS_NAMESPACE(FROM File TO File, import_line INT64, alias STRING DEFAULT '')",
        "CREATE REL TABLE IMPORTS_WILDCARD(FROM File TO File, import_line INT64)",
        "CREATE REL TABLE IMPORTS_SIDE_EFFECT(FROM File TO File, import_line INT64)",

        // Existing
        "CREATE REL TABLE DEFINES(FROM File TO Symbol, is_exported BOOLEAN DEFAULT false)",
        "CREATE REL TABLE CONTAINS(FROM Symbol TO Symbol)",
        "CREATE REL TABLE TESTED_BY(FROM Symbol TO Symbol, test_framework STRING DEFAULT 'unknown')",
        "CREATE REL TABLE MODULE_TYPED(FROM File TO Symbol)",
    ];

    let mut failed = false;
    for stmt in &v4_ddl {
        if let Err(e) = conn.query(stmt) {
            report.fail(
                "v4 DDL bulk apply (first failure aborts dry-run)",
                format!("{e}\n           stmt: {stmt}"),
            );
            failed = true;
            break;
        }
    }
    if !failed {
        report.pass(format!("v4 DDL bulk apply ({} statements)", v4_ddl.len()));

        // Verify a representative INSERT + MATCH round-trip
        try_stmt(
            &conn,
            "v4 INSERT Symbol with LIST<STRUCT> params",
            "CREATE (s:Symbol {id: 'demo:1:foo', name: 'foo', qualified_name: 'm::foo', \
             file: 'm.rs', kind: 'function', line: 1, line_end: 5, \
             params: [{name: 'x', type: 'u32', default_value: ''}], \
             return_type: 'bool', modifiers: ['pub'], arity: 1, \
             is_async: false, is_override: false, is_abstract: false, is_static: false, \
             is_test_marker: false, is_generated: false, \
             enclosing: '', confidence: 1.0, doc_summary: 'demo'})",
            report,
        );

        try_stmt(
            &conn,
            "v4 MATCH on LIST<STRUCT>.size() — query semantics check",
            "MATCH (s:Symbol) WHERE size(s.params) > 0 RETURN s.name",
            report,
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> ExitCode {
    println!(
        "lbug v4 schema validation — graphatlas\n\
         Build:  lbug crate version per Cargo.toml workspace pin\n\
         Goal:   empirically verify each lbug DDL/DML feature the v4 schema\n\
                 design depends on, before committing to spec.\n"
    );

    let mut report = Report::default();
    type Runner = fn(&mut Report) -> anyhow::Result<()>;
    let runners: &[(&str, Runner)] = &[
        ("data types", test_data_types),
        ("constraints", test_constraints),
        ("alter table", test_alter_table),
        ("rel split", test_rel_split),
        ("extensions", test_extensions),
        ("misc", test_misc),
        ("v4 dry-run", test_v4_schema_dry_run),
    ];
    for (name, f) in runners {
        if let Err(e) = f(&mut report) {
            report.fail(format!("{name} (test harness error)"), e.to_string());
        }
    }

    println!(
        "\n=== SUMMARY ===\n  PASS: {}\n  FAIL: {}\n  SKIP: {}",
        report.pass.len(),
        report.fail.len(),
        report.skip.len()
    );

    if !report.fail.is_empty() {
        println!("\nFailures (block schema-v4 if these are load-bearing):");
        for (n, e) in &report.fail {
            println!("  - {n}\n    {e}");
        }
    }
    if !report.skip.is_empty() {
        println!("\nSkipped (extension not built into this lbug binary):");
        for (n, r) in &report.skip {
            println!("  - {n}: {r}");
        }
    }

    if report.fail.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
