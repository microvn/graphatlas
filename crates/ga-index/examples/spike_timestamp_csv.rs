//! PR6 follow-up: empirically verify lbug TIMESTAMP CSV format claims
//! before documenting them in `docs/guide/ladybug-engine-capabilities.md`
//! §4.13. Original PR6 only verified the space-separated format gets
//! ingested (via the production happy-path test) — never tested rejected
//! variants. This spike fills the gap.

use lbug::{Connection, Database, SystemConfig};
use std::io::Write;

fn try_variant(label: &str, csv_value: &str) {
    let dir = tempfile::TempDir::new().unwrap();
    let db = Database::new(dir.path().join("v.db"), SystemConfig::default()).unwrap();
    let conn = Connection::new(&db).unwrap();
    conn.query("CREATE NODE TABLE T(id STRING PRIMARY KEY, t TIMESTAMP)")
        .unwrap();
    let csv = dir.path().join("r.csv");
    let mut f = std::fs::File::create(&csv).unwrap();
    writeln!(f, "row1,{csv_value}").unwrap();
    drop(f);
    let copy_ok = match conn.query(&format!("COPY T FROM '{}' (header=false)", csv.display())) {
        Ok(_) => true,
        Err(e) => {
            let msg = format!("{e}");
            let line = msg.lines().next().unwrap_or("");
            println!("  {label}: ❌ COPY rejected: {line}");
            return;
        }
    };
    if !copy_ok {
        return;
    }
    let qres = conn.query("MATCH (t:T) RETURN t.t");
    let rs = match qres {
        Ok(r) => r,
        Err(e) => {
            println!("  {label}: query err {e}");
            return;
        }
    };
    let vals: Vec<String> = rs
        .map(|row| {
            row.into_iter()
                .next()
                .map(|v| format!("{v:?}"))
                .unwrap_or_default()
        })
        .collect();
    for v in vals {
        println!("  {label}: ✅ stored as {v}");
    }
}

fn main() {
    println!("=== PR6 follow-up: TIMESTAMP CSV format ===\n");

    // Variants based on common datetime serializations.
    try_variant("A — epoch seconds (1715000000)", "1715000000");
    try_variant("B — epoch ms (1715000000000)", "1715000000000");
    try_variant(
        "C — ISO 8601 with `T` + Z (2026-05-06T10:00:00Z)",
        "2026-05-06T10:00:00Z",
    );
    try_variant(
        "D — ISO 8601 with `T` no tz (2026-05-06T10:00:00)",
        "2026-05-06T10:00:00",
    );
    try_variant(
        "E — space-separated (2026-05-06 10:00:00)",
        "2026-05-06 10:00:00",
    );
    try_variant(
        "F — space + microseconds (2026-05-06 10:00:00.123456)",
        "2026-05-06 10:00:00.123456",
    );
    try_variant("G — date only (2026-05-06)", "2026-05-06");
    try_variant(
        "H — RFC 2822 (Mon, 06 May 2026 10:00:00 GMT)",
        "Mon, 06 May 2026 10:00:00 GMT",
    );

    println!("\nUpdate §4.13 to reflect empirical results.");
}
