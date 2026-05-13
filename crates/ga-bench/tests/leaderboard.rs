//! Bench S-001 cluster A — leaderboard markdown shape (AS-001).

use ga_bench::{write_leaderboard, LeaderEntry, Leaderboard};
use tempfile::TempDir;

fn sample() -> Leaderboard {
    Leaderboard {
        uc: "callers".to_string(),
        fixture: "benches/fixtures/mini".to_string(),
        hardware: "M1 Mac 16GB".to_string(),
        entries: vec![
            LeaderEntry {
                retriever: "ga".to_string(),
                f1: 0.92,
                recall: 0.95,
                precision: 0.90,
                mrr: 0.0,
                p95_latency_ms: 12,
                pass_rate: 1.0,
            },
            LeaderEntry {
                retriever: "ripgrep".to_string(),
                f1: 0.45,
                recall: 0.80,
                precision: 0.30,
                mrr: 0.0,
                p95_latency_ms: 5,
                pass_rate: 0.8,
            },
        ],
    }
}

#[test]
fn leaderboard_shape_matches_spec() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("callers-leaderboard.md");
    write_leaderboard(&sample(), &path).unwrap();
    let md = std::fs::read_to_string(&path).unwrap();

    assert!(md.contains("# Leaderboard: UC `callers`"));
    assert!(md.contains("**Hardware:** M1 Mac 16GB"));
    assert!(md.contains("| Retriever | F1 | Recall | Precision | MRR | p95 latency | pass rate |"));
    assert!(md.contains("| ga |"));
    assert!(md.contains("| ripgrep |"));
}

#[test]
fn leaderboard_sorts_by_f1_desc() {
    // ripgrep (f1 0.45) authored first but should render second after sort.
    let mut lb = sample();
    lb.entries.reverse();

    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("callers-leaderboard.md");
    write_leaderboard(&lb, &path).unwrap();
    let md = std::fs::read_to_string(&path).unwrap();

    let ga_pos = md.find("| ga |").unwrap();
    let rg_pos = md.find("| ripgrep |").unwrap();
    assert!(ga_pos < rg_pos, "ga must render above ripgrep (higher F1)");
}
