//! S-005 AS-024 — parallel re-parse via rayon.
//!
//! Spec: "Parallel parsing via rayon. Progress streamed if >5s. Graph atomic
//! commit (all-or-nothing). Total ≤10s on M1 16GB for 500-file batch."
//!
//! This test file covers the unit layer: `parallel_reparse` fans out work
//! across threads and gathers per-file results in the same order as input.
//! The graph-atomic-commit part lives at integration layer and is exercised
//! when `ga_reindex` is wired in Tools phase.

use ga_core::{Lang, Result};
use ga_parser::parallel_reparse::{parallel_reparse, ReparseProgress, ReparseResult};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn paths(n: usize) -> Vec<PathBuf> {
    (0..n)
        .map(|i| PathBuf::from(format!("file-{i:04}.py")))
        .collect()
}

#[test]
fn reparse_empty_input_is_empty_output() {
    let parse_fn = |_: &Path| -> Result<ReparseResult> {
        Ok(ReparseResult {
            path: PathBuf::new(),
            symbols: Vec::new(),
            lang: Lang::Python,
            bytes: 0,
        })
    };
    let out = parallel_reparse(&[], parse_fn, None::<fn(ReparseProgress)>);
    assert!(out.is_empty());
}

#[test]
fn reparse_preserves_input_order_in_output() {
    let parse_fn = |p: &Path| -> Result<ReparseResult> {
        Ok(ReparseResult {
            path: p.to_path_buf(),
            symbols: Vec::new(),
            lang: Lang::Python,
            bytes: p.to_string_lossy().len() as u64,
        })
    };
    let input = paths(50);
    let out = parallel_reparse(&input, parse_fn, None::<fn(ReparseProgress)>);
    assert_eq!(out.len(), 50);
    for (i, result) in out.iter().enumerate() {
        let r = result.as_ref().expect("ok");
        assert_eq!(r.path, PathBuf::from(format!("file-{i:04}.py")));
    }
}

#[test]
fn reparse_parallelism_observed_via_concurrent_counter() {
    // If work really runs in parallel, the peak concurrent-execution count
    // must exceed 1 on any multi-core runner. This is a smoke of
    // parallelism, not a strict guarantee — rayon may serialize on a
    // single-core box. Use a tight busy-wait so threads overlap.
    let peak = Arc::new(AtomicU32::new(0));
    let in_flight = Arc::new(AtomicU32::new(0));
    let peak_c = peak.clone();
    let in_flight_c = in_flight.clone();
    let parse_fn = move |p: &Path| -> Result<ReparseResult> {
        let n = in_flight_c.fetch_add(1, Ordering::SeqCst) + 1;
        peak_c.fetch_max(n, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(20));
        in_flight_c.fetch_sub(1, Ordering::SeqCst);
        Ok(ReparseResult {
            path: p.to_path_buf(),
            symbols: Vec::new(),
            lang: Lang::Python,
            bytes: 0,
        })
    };
    let _ = parallel_reparse(&paths(24), parse_fn, None::<fn(ReparseProgress)>);
    let observed = peak.load(Ordering::SeqCst);
    if num_cpus() >= 2 {
        assert!(
            observed > 1,
            "expected parallelism on multi-core host, observed peak {observed}"
        );
    }
}

#[test]
fn reparse_per_file_errors_surface_in_output() {
    let parse_fn = |p: &Path| -> Result<ReparseResult> {
        if p.to_string_lossy().contains("0002") {
            Err(ga_core::Error::Other(anyhow::anyhow!("synthetic failure")))
        } else {
            Ok(ReparseResult {
                path: p.to_path_buf(),
                symbols: Vec::new(),
                lang: Lang::Python,
                bytes: 0,
            })
        }
    };
    let out = parallel_reparse(&paths(5), parse_fn, None::<fn(ReparseProgress)>);
    assert_eq!(out.len(), 5);
    assert!(out[2].is_err(), "0002 should fail");
    for i in [0, 1, 3, 4] {
        assert!(out[i].is_ok(), "{i} should succeed");
    }
}

#[test]
fn progress_callback_invoked() {
    // Progress callback fires at least once with completion count. Spec
    // says "stream if >5s" but the callback mechanism must work regardless
    // of duration — batch runner decides when to emit.
    let seen = Arc::new(std::sync::Mutex::new(Vec::<ReparseProgress>::new()));
    let seen_c = seen.clone();
    let progress = move |p: ReparseProgress| {
        seen_c.lock().unwrap().push(p);
    };
    let parse_fn = |p: &Path| -> Result<ReparseResult> {
        Ok(ReparseResult {
            path: p.to_path_buf(),
            symbols: Vec::new(),
            lang: Lang::Python,
            bytes: 0,
        })
    };
    let _ = parallel_reparse(&paths(10), parse_fn, Some(progress));
    let observed = seen.lock().unwrap();
    assert!(
        !observed.is_empty(),
        "expected at least one progress tick; got none"
    );
    // Progress callbacks fire from multiple threads in parallel; Vec::last
    // reflects push ORDER, not logical completion order. Assert the
    // invariants that actually matter: (a) every tick has total=10, and
    // (b) the max `completed` reached == 10.
    let max_completed = observed.iter().map(|p| p.completed).max().unwrap();
    assert_eq!(
        max_completed, 10,
        "max completed should be 10: {observed:?}"
    );
    for p in observed.iter() {
        assert_eq!(p.total, 10);
    }
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}
