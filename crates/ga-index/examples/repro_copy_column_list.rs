//! Minimal repro: COPY with column-list against a table whose other columns
//! have DEFAULTs of various scalar types. Pinpoints which DEFAULT type trips
//! the column_chunk_data.cpp:295 physical-type assertion.

use lbug::{Connection, Database, SystemConfig};
use std::io::Write;
use tempfile::TempDir;

fn try_case(name: &str, ddl: &str, copy_cols: &str, csv_line: &str) {
    let dir = TempDir::new().unwrap();
    let db = Database::new(dir.path().join("t.db"), SystemConfig::default()).unwrap();
    let conn = Connection::new(&db).unwrap();
    if let Err(e) = conn.query(ddl) {
        println!("[{name}] DDL FAIL: {e}");
        return;
    }
    let csv = dir.path().join("r.csv");
    let mut f = std::fs::File::create(&csv).unwrap();
    writeln!(f, "{csv_line}").unwrap();
    drop(f);
    let q = format!(
        "COPY T ({copy_cols}) FROM '{}' (header=false)",
        csv.display()
    );
    match conn.query(&q) {
        Ok(_) => println!("[{name}] OK"),
        Err(e) => println!("[{name}] COPY FAIL: {e}"),
    };
}

fn main() {
    try_case(
        "baseline-no-defaults",
        "CREATE NODE TABLE T(id STRING PRIMARY KEY, n INT64)",
        "id, n",
        "a,1",
    );
    try_case(
        "string-default-omitted",
        "CREATE NODE TABLE T(id STRING PRIMARY KEY, x STRING DEFAULT '')",
        "id",
        "a",
    );
    try_case(
        "int-default-omitted",
        "CREATE NODE TABLE T(id STRING PRIMARY KEY, x INT64 DEFAULT -1)",
        "id",
        "a",
    );
    try_case(
        "bool-default-omitted",
        "CREATE NODE TABLE T(id STRING PRIMARY KEY, x BOOLEAN DEFAULT false)",
        "id",
        "a",
    );
    try_case(
        "float-default-omitted",
        "CREATE NODE TABLE T(id STRING PRIMARY KEY, x FLOAT DEFAULT 1.0)",
        "id",
        "a",
    );
    try_case(
        "double-default-omitted",
        "CREATE NODE TABLE T(id STRING PRIMARY KEY, x DOUBLE DEFAULT 1.0)",
        "id",
        "a",
    );
    try_case(
        "blob-no-default-omitted",
        "CREATE NODE TABLE T(id STRING PRIMARY KEY, x BLOB)",
        "id",
        "a",
    );
    try_case(
        "timestamp-no-default-omitted",
        "CREATE NODE TABLE T(id STRING PRIMARY KEY, x TIMESTAMP)",
        "id",
        "a",
    );
}
