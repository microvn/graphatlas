//! PR6 spike: lbug BLOB CSV format. Test variants to find what produces a
//! 32-byte raw Blob value (vs the 64-byte ASCII-hex we got).

use lbug::{Connection, Database, SystemConfig};
use std::io::Write;

fn try_variant(label: &str, csv_value: &str) {
    let dir = tempfile::TempDir::new().unwrap();
    let db = Database::new(dir.path().join("v.db"), SystemConfig::default()).unwrap();
    let conn = Connection::new(&db).unwrap();
    conn.query("CREATE NODE TABLE T(id STRING PRIMARY KEY, h BLOB)")
        .unwrap();
    let csv = dir.path().join("r.csv");
    let mut f = std::fs::File::create(&csv).unwrap();
    writeln!(f, "row1,{csv_value}").unwrap();
    drop(f);
    let copy_ok = conn
        .query(&format!("COPY T FROM '{}' (header=false)", csv.display()))
        .is_ok();
    if !copy_ok {
        println!("  {label}: COPY failed");
        return;
    }
    let qres = conn.query("MATCH (t:T) RETURN t.h");
    let rs = match qres {
        Ok(r) => r,
        Err(e) => {
            println!("  {label}: query err {e}");
            return;
        }
    };
    let blobs: Vec<(usize, Vec<u8>)> = rs
        .map(|row| {
            let v = row.into_iter().next();
            if let Some(lbug::Value::Blob(b)) = v {
                (b.len(), b.iter().take(8).copied().collect())
            } else {
                (0, vec![])
            }
        })
        .collect();
    for (len, first) in blobs {
        println!("  {label}: blob len={len} first={first:02x?}");
    }
}

fn main() {
    println!("=== PR6 spike: lbug BLOB CSV format ===\n");
    let hex32 = "deadbeefcafebabe1122334455667788aabbccdd00112233445566778899aabb";
    try_variant("Variant A — bare hex (deadbeef...)", hex32);
    try_variant("Variant B — `\\x` prefix", &format!("\\x{hex32}"));
    try_variant("Variant C — `0x` prefix", &format!("0x{hex32}"));
    try_variant("Variant D — quoted hex", &format!("\"{hex32}\""));
    try_variant("Variant E — quoted \\x prefix", &format!("\"\\x{hex32}\""));

    // F: per-byte \xHH escape (Postgres-style raw hex)
    let mut per_byte = String::new();
    let raw = hex::decode_safe(hex32);
    for b in &raw {
        per_byte.push_str(&format!("\\x{b:02x}"));
    }
    try_variant("Variant F — per-byte \\xHH (Postgres bytea)", &per_byte);

    // G: same but quoted
    try_variant(
        "Variant G — quoted per-byte \\xHH",
        &format!("\"{per_byte}\""),
    );
}

mod hex {
    pub fn decode_safe(s: &str) -> Vec<u8> {
        let mut out = Vec::with_capacity(s.len() / 2);
        let bytes = s.as_bytes();
        let mut i = 0;
        while i + 1 < bytes.len() {
            let hi = (bytes[i] as char).to_digit(16).unwrap() as u8;
            let lo = (bytes[i + 1] as char).to_digit(16).unwrap() as u8;
            out.push(hi * 16 + lo);
            i += 2;
        }
        out
    }
}
