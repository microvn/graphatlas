//! Test 4 fix paths for the FLOAT-DEFAULT-omitted-from-COPY-column-list bug
//! (kuzu#5159 family, see docs/guide/ladybug-engine-capabilities.md §4.10).

use lbug::{Connection, Database, SystemConfig};
use std::io::Write;
use tempfile::TempDir;

fn try_path(name: &str, ddl: &str, csv: &str, copy_stmt: impl Fn(&str) -> String) {
    let dir = TempDir::new().unwrap();
    let db = Database::new(dir.path().join("t.db"), SystemConfig::default()).unwrap();
    let conn = Connection::new(&db).unwrap();
    if let Err(e) = conn.query(ddl) {
        println!("[{name}] DDL FAIL: {e}");
        return;
    }
    let csv_path = dir.path().join("r.csv");
    let mut f = std::fs::File::create(&csv_path).unwrap();
    f.write_all(csv.as_bytes()).unwrap();
    drop(f);
    let q = copy_stmt(&csv_path.to_string_lossy());
    match conn.query(&q) {
        Ok(_) => {
            // Verify default got applied
            let rs = conn.query("MATCH (t:T) RETURN t.id, t.confidence").unwrap();
            for row in rs {
                let cells: Vec<_> = row.into_iter().collect();
                println!("[{name}] OK — row: {cells:?}");
            }
        }
        Err(e) => println!("[{name}] COPY FAIL: {e}"),
    };
}

fn main() {
    println!("\n=== Path A1: full-column CSV, FLOAT, explicit default literal in row ===");
    try_path(
        "A1-float-explicit",
        "CREATE NODE TABLE T(id STRING PRIMARY KEY, confidence FLOAT DEFAULT 1.0)",
        "a,1.0\n",
        |p| format!("COPY T FROM '{p}' (header=false)"),
    );

    println!("\n=== Path A2: full-column CSV, DOUBLE, explicit default literal ===");
    try_path(
        "A2-double-explicit",
        "CREATE NODE TABLE T(id STRING PRIMARY KEY, confidence DOUBLE DEFAULT 1.0)",
        "a,1.0\n",
        |p| format!("COPY T FROM '{p}' (header=false)"),
    );

    println!("\n=== Path B: NO DEFAULT on column, full-column CSV with NULL placeholder ===");
    try_path(
        "B-no-default-null",
        "CREATE NODE TABLE T(id STRING PRIMARY KEY, confidence FLOAT)",
        "a,\n", // empty cell = NULL
        |p| format!("COPY T FROM '{p}' (header=false)"),
    );

    println!("\n=== Path C1: CREATE without DEFAULT col, then ALTER ADD COLUMN with DEFAULT ===");
    {
        let dir = TempDir::new().unwrap();
        let db = Database::new(dir.path().join("t.db"), SystemConfig::default()).unwrap();
        let conn = Connection::new(&db).unwrap();
        conn.query("CREATE NODE TABLE T(id STRING PRIMARY KEY)")
            .unwrap();
        let csv = dir.path().join("r.csv");
        std::fs::write(&csv, "a\n").unwrap();
        if let Err(e) = conn.query(&format!("COPY T FROM '{}' (header=false)", csv.display())) {
            println!("[C1-pre-alter-copy] FAIL: {e}");
        }
        match conn.query("ALTER TABLE T ADD confidence FLOAT DEFAULT 1.0") {
            Ok(_) => {
                let rs = conn.query("MATCH (t:T) RETURN t.id, t.confidence").unwrap();
                for row in rs {
                    let cells: Vec<_> = row.into_iter().collect();
                    println!("[C1-alter-after-copy-FLOAT] OK — row: {cells:?}");
                }
            }
            Err(e) => println!("[C1-alter-after-copy-FLOAT] FAIL: {e}"),
        };
    }

    println!("\n=== Path C2: same but ALTER ADD STRUCT[] DEFAULT ===");
    {
        let dir = TempDir::new().unwrap();
        let db = Database::new(dir.path().join("t.db"), SystemConfig::default()).unwrap();
        let conn = Connection::new(&db).unwrap();
        conn.query("CREATE NODE TABLE T(id STRING PRIMARY KEY)")
            .unwrap();
        let csv = dir.path().join("r.csv");
        std::fs::write(&csv, "a\n").unwrap();
        let _ = conn.query(&format!("COPY T FROM '{}' (header=false)", csv.display()));
        match conn.query(
            "ALTER TABLE T ADD params STRUCT(name STRING, type STRING)[] DEFAULT CAST([] AS STRUCT(name STRING, type STRING)[])",
        ) {
            Ok(_) => {
                let rs = conn.query("MATCH (t:T) RETURN t.id, t.params").unwrap();
                for row in rs {
                    let cells: Vec<_> = row.into_iter().collect();
                    println!("[C2-alter-struct-after-copy] OK — row: {cells:?}");
                }
            }
            Err(e) => println!("[C2-alter-struct-after-copy] FAIL: {e}"),
        };
    }

    println!(
        "\n=== Path D: full-column CSV with FLOAT default literal in CSV (the simple fix) ==="
    );
    try_path(
        "D-float-with-cell",
        "CREATE NODE TABLE T(id STRING PRIMARY KEY, x INT64, confidence FLOAT DEFAULT 1.0)",
        "a,42,1.0\n",
        |p| format!("COPY T FROM '{p}' (header=false)"),
    );

    println!("\n=== Path E: SKIPPED (verification helper hardcodes 'confidence' col) ===");

    println!("\n=== Path F: ALTER ADD COLUMN FIRST, then COPY (graphatlas v1.3 lifecycle) ===");
    {
        let dir = TempDir::new().unwrap();
        let db = Database::new(dir.path().join("t.db"), SystemConfig::default()).unwrap();
        let conn = Connection::new(&db).unwrap();
        // Step 1: CREATE v3 shape
        conn.query("CREATE NODE TABLE T(id STRING PRIMARY KEY, name STRING)")
            .unwrap();
        // Step 2: ALTER ADD v4 cols (Path C migration phase)
        conn.query("ALTER TABLE T ADD confidence FLOAT DEFAULT 1.0")
            .unwrap();
        conn.query(
            "ALTER TABLE T ADD params STRUCT(name STRING, type STRING)[] \
                    DEFAULT CAST([] AS STRUCT(name STRING, type STRING)[])",
        )
        .unwrap();
        // Step 3: COPY with column-list omitting the ALTERed v4 cols
        let csv = dir.path().join("r.csv");
        std::fs::write(&csv, "a,foo\n").unwrap();
        match conn.query(&format!(
            "COPY T (id, name) FROM '{}' (header=false)",
            csv.display()
        )) {
            Ok(_) => {
                let rs = conn
                    .query("MATCH (t:T) RETURN t.id, t.name, t.confidence, size(t.params)")
                    .unwrap();
                for row in rs {
                    let cells: Vec<_> = row.into_iter().collect();
                    println!("[F-alter-then-copy-omit] OK — row: {cells:?}");
                }
            }
            Err(e) => println!("[F-alter-then-copy-omit] COPY FAIL: {e}"),
        };
    }

    println!("\n=== Path G: same as F but COPY without column-list (full row, all cols) ===");
    {
        let dir = TempDir::new().unwrap();
        let db = Database::new(dir.path().join("t.db"), SystemConfig::default()).unwrap();
        let conn = Connection::new(&db).unwrap();
        conn.query("CREATE NODE TABLE T(id STRING PRIMARY KEY, name STRING)")
            .unwrap();
        conn.query("ALTER TABLE T ADD confidence FLOAT DEFAULT 1.0")
            .unwrap();
        let csv = dir.path().join("r.csv");
        // Write CSV with all 3 cols (id, name, confidence)
        std::fs::write(&csv, "a,foo,0.7\n").unwrap();
        match conn.query(&format!("COPY T FROM '{}' (header=false)", csv.display())) {
            Ok(_) => {
                let rs = conn
                    .query("MATCH (t:T) RETURN t.id, t.name, t.confidence")
                    .unwrap();
                for row in rs {
                    let cells: Vec<_> = row.into_iter().collect();
                    println!("[G-alter-then-fullcopy] OK — row: {cells:?}");
                }
            }
            Err(e) => println!("[G-alter-then-fullcopy] COPY FAIL: {e}"),
        };
    }
}
