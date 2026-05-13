use anyhow::Result;
use ga_index::Store;
use ga_query::indexer::build_index;
use std::path::PathBuf;
use std::time::Instant;

fn main() -> Result<()> {
    let fixture = std::env::args()
        .nth(1)
        .expect("usage: time_index <fixture-dir>");
    let fixture = PathBuf::from(fixture).canonicalize()?;

    let cache_root = tempfile::tempdir()?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(cache_root.path(), std::fs::Permissions::from_mode(0o700))?;
    }
    let store = Store::open_with_root(cache_root.path(), &fixture)?;

    let t0 = Instant::now();
    let stats = build_index(&store, &fixture)?;
    let elapsed = t0.elapsed();

    println!(
        "fixture={} files={} symbols={} elapsed_ms={}",
        fixture.display(),
        stats.files,
        stats.symbols,
        elapsed.as_millis()
    );
    Ok(())
}
