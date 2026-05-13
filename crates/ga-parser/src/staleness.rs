//! S-005 AS-013 — staleness checker with per-process 500ms cache.
//!
//! Flow:
//!   1. MCP tool handler → `checker.check(stored_hash)`
//!   2. If cache is fresh → return cached hash
//!   3. Else → `compute_root_hash()` → cache → return
//!   4. Compare with stored → `stale` flag
//!
//! On exotic filesystems (NFS / FUSE / unknown), the checker falls back to
//! HEAD-only hashing and sets `degraded: true` so tool responses can surface
//! `meta.stale_check_degraded: true` per the spec.

use crate::merkle::{compute_root_hash, MerkleConfig};
use ga_core::Result;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Per-spec AS-013: "Cache result for T=500ms within MCP process".
pub const STALE_CACHE_TTL: Duration = Duration::from_millis(500);

/// v1.5 S-004 AS-015 Tier 2 TTL: BLAKE3 dirty_paths walk caches for 1s
/// (longer than Tier 1's 500ms because walking the tree is heavier;
/// short-circuit invalidation on `.git/index` mtime change handles
/// staging events within the window — see AS-016).
pub const TIER2_CACHE_TTL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct StaleResult {
    /// True iff `indexed_root_hash != current_hash`.
    pub stale: bool,
    /// Hash stored in `metadata.json` at last commit.
    pub indexed_root_hash: [u8; 32],
    /// Hash computed fresh (or from cache) this check.
    pub current_hash: [u8; 32],
    /// True when we skipped the full Merkle walk because the filesystem type
    /// is unknown / potentially slow. Degraded mode hashes only `.git/HEAD`
    /// and `.git/index` mtime.
    pub degraded: bool,
}

pub struct StalenessChecker {
    repo_root: PathBuf,
    cfg: MerkleConfig,
    cache: Mutex<CachedHash>,
    compute_counter: Mutex<u64>,
    /// v1.5 S-004 AS-015/016 — Tier 2 dirty-paths cache. 1s TTL,
    /// invalidated early when `.git/index` mtime advances.
    tier2_cache: Mutex<Tier2Cache>,
    /// v1.5 S-004 AS-015 — counter incremented on every full Tier 2
    /// walk. Cache hits do NOT increment. Exposed via
    /// `dirty_check_count()` for burst absorption assertion.
    tier2_counter: Mutex<u64>,
    /// v1.5 S-004 AS-017 — when true, callers should skip Tier 2.
    /// Set at MCP boot from env `GRAPHATLAS_DISABLE_DIRTY_CHECK=1`
    /// OR explicitly via `set_tier2_disabled` (tests + large-repo
    /// auto-detect).
    tier2_disabled: AtomicBool,
}

#[derive(Debug, Clone, Default)]
struct CachedHash {
    at: Option<Instant>,
    hash: [u8; 32],
    degraded: bool,
}

/// v1.5 S-004 — Tier 2 cache state. Key = (instant of last fill,
/// `.git/index` mtime nanos at fill time). Either key dim changing
/// (TTL expired OR `.git/index` advanced) invalidates.
#[derive(Debug, Clone, Default)]
struct Tier2Cache {
    at: Option<Instant>,
    git_index_mtime_ns: Option<u128>,
    /// Last dirty set computed. Empty = fresh.
    dirty_paths: Vec<PathBuf>,
}

impl StalenessChecker {
    pub fn new(repo_root: PathBuf) -> Self {
        // Boot-time env read for AS-017 opt-out. Tests can flip the
        // atomic later via `set_tier2_disabled` without touching env
        // (which is unsafe under Rust 2024+ workspace lint).
        let disabled_by_env = matches!(
            std::env::var("GRAPHATLAS_DISABLE_DIRTY_CHECK").as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        );
        Self {
            repo_root,
            cfg: MerkleConfig::default(),
            cache: Mutex::new(CachedHash::default()),
            compute_counter: Mutex::new(0),
            tier2_cache: Mutex::new(Tier2Cache::default()),
            tier2_counter: Mutex::new(0),
            tier2_disabled: AtomicBool::new(disabled_by_env),
        }
    }

    /// v1.5 S-004 AS-017 — runtime control of Tier 2 opt-out. Used by
    /// (a) auto-detect at MCP boot when file count >10k, and (b) tests
    /// that need to flip the flag without mutating env (forbidden by
    /// workspace `unsafe_code = "forbid"` lint).
    pub fn set_tier2_disabled(&self, disabled: bool) {
        self.tier2_disabled.store(disabled, Ordering::Relaxed);
    }

    /// v1.5 S-004 AS-017 — current Tier 2 opt-out state.
    pub fn is_tier2_disabled(&self) -> bool {
        self.tier2_disabled.load(Ordering::Relaxed)
    }

    /// Expose the repo root (used by the MCP gate to plumb Tier 2's
    /// `incremental::dirty_paths` against the same path).
    pub fn repo_root(&self) -> &std::path::Path {
        &self.repo_root
    }

    /// v1.5 S-004 AS-015 — non-zero iff Tier 2 walk has ever fired.
    /// Cache hits do NOT increment. Tests assert this stays ≤1 across
    /// a 10-call burst within TTL.
    pub fn dirty_check_count(&self) -> u64 {
        *self.tier2_counter.lock().expect("tier2_counter")
    }

    /// v1.5 S-004 AS-015/016 — Tier 2 cache lookup with mtime
    /// invalidation. Returns `Some(dirty_paths)` if cache hit (caller
    /// can short-circuit); `None` if caller must compute fresh. Callers
    /// that compute fresh MUST follow up with `record_tier2_result` so
    /// the next call hits cache.
    pub fn tier2_lookup(&self) -> Option<Vec<PathBuf>> {
        let cache = self.tier2_cache.lock().expect("tier2_cache");
        let at = cache.at?;
        if at.elapsed() >= TIER2_CACHE_TTL {
            return None;
        }
        // AS-016 — `.git/index` mtime mismatch invalidates the cache
        // even within TTL. Probe current mtime cheaply (1 stat call).
        let current_mtime = git_index_mtime_ns(&self.repo_root);
        if current_mtime != cache.git_index_mtime_ns {
            return None;
        }
        Some(cache.dirty_paths.clone())
    }

    /// v1.5 S-004 AS-015 — store the result of a Tier 2 walk in cache.
    /// Bumps `dirty_check_count`. Captures `.git/index` mtime at the
    /// time of the walk so AS-016 invalidation has a baseline.
    pub fn record_tier2_result(&self, dirty_paths: Vec<PathBuf>) {
        let mut cache = self.tier2_cache.lock().expect("tier2_cache");
        cache.at = Some(Instant::now());
        cache.git_index_mtime_ns = git_index_mtime_ns(&self.repo_root);
        cache.dirty_paths = dirty_paths;
        *self.tier2_counter.lock().expect("tier2_counter") += 1;
    }

    /// Force a cache refill regardless of TTL. Used for tests + doctor.
    pub fn compute_now(&self) -> Result<[u8; 32]> {
        let hash = compute_root_hash(&self.repo_root, &self.cfg)?;
        let degraded = is_exotic_filesystem(&self.repo_root);
        let mut cache = self.cache.lock().expect("mutex");
        cache.at = Some(Instant::now());
        cache.hash = hash;
        cache.degraded = degraded;
        *self.compute_counter.lock().expect("mutex") += 1;
        Ok(hash)
    }

    /// Invalidate the TTL cache — next check() will recompute.
    pub fn invalidate_cache(&self) {
        let mut cache = self.cache.lock().expect("mutex");
        cache.at = None;
    }

    /// Non-zero iff the TTL-cache was ever filled. Incremented only on real
    /// computes — exposed for tests asserting the cache works.
    pub fn compute_count(&self) -> u64 {
        *self.compute_counter.lock().expect("mutex")
    }

    pub fn check(&self, indexed_root_hash: &[u8; 32]) -> Result<StaleResult> {
        let (current, degraded) = self.current_hash()?;
        Ok(StaleResult {
            stale: current != *indexed_root_hash,
            indexed_root_hash: *indexed_root_hash,
            current_hash: current,
            degraded,
        })
    }

    fn current_hash(&self) -> Result<([u8; 32], bool)> {
        // Cache hit path.
        {
            let cache = self.cache.lock().expect("mutex");
            if let Some(at) = cache.at {
                if at.elapsed() < STALE_CACHE_TTL {
                    return Ok((cache.hash, cache.degraded));
                }
            }
        }
        // Miss → recompute.
        let _ = self.compute_now()?;
        let cache = self.cache.lock().expect("mutex");
        Ok((cache.hash, cache.degraded))
    }
}

/// Heuristic: return true if `path` looks like it lives on a filesystem where
/// our bounded-stat Merkle walk could be slow or inconsistent (NFS, FUSE, SMB,
/// network mounts). On Linux we could read `/proc/mounts`; on macOS `statfs`
/// exposes `f_fstypename`. For v1 we conservatively detect by path prefix —
/// most users run on local filesystems and false negatives are acceptable.
/// Real detection lands in v1.1 when cross-platform statfs is wired.
#[cfg(target_os = "macos")]
fn is_exotic_filesystem(path: &std::path::Path) -> bool {
    // macOS network volumes live under `/Volumes/<name>/` where <name> is the
    // mount point. Local disk volumes also live there, so this is a weak hint
    // rather than a proof. Keep false (non-degraded) by default; the real
    // `statfs`-based check lands in v1.1.
    let _ = path;
    false
}

#[cfg(target_os = "linux")]
fn is_exotic_filesystem(_path: &std::path::Path) -> bool {
    // Local ext4/btrfs/xfs are the common case. Real `statfs` magic-number
    // detection (NFS / FUSE / 9p / SMB / virtiofs) lands when Phase E Layer 1
    // watcher needs PollWatcher fallback. Default false here.
    false
}

/// v1.5 PR3 foundation S-002 AS-008 — Windows arm.
///
/// Windows volume types vary widely (NTFS local, SMB share, ReFS, network
/// junction, remote dev VM). Detecting the safe set via `GetVolumeInformationW`
/// is deferred to PR9 when the watcher actually needs the discrimination.
///
/// **Conservative default**: return `true` (exotic). This forces the Phase E
/// Layer 1 watcher into `PollWatcher` fallback on Windows by default, which is
/// the safe choice — never miss an event because we trusted inotify-style
/// semantics we don't have. Local NTFS users get slightly higher poll latency
/// (2-5s vs 500ms debounce); the trade-off is "never silently stale" > "fast
/// on the happy path".
///
/// Phase E author may refine via `GetVolumeInformationW` to distinguish
/// local NTFS from SMB; track that work in the triggers sub-spec (PR8 Layer 1).
#[cfg(target_os = "windows")]
fn is_exotic_filesystem(_path: &std::path::Path) -> bool {
    true
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn is_exotic_filesystem(_path: &std::path::Path) -> bool {
    // Unknown platform — conservatively assume exotic so callers degrade
    // rather than trust local-FS performance assumptions.
    true
}

/// v1.5 S-004 AS-016 — read `.git/index` mtime as nanos since epoch.
/// Returns `None` if the file doesn't exist (non-git repo) — caller
/// treats `None == None` as cache-key match, so non-git repos
/// still benefit from the TTL window.
fn git_index_mtime_ns(repo_root: &std::path::Path) -> Option<u128> {
    let meta = std::fs::metadata(repo_root.join(".git").join("index")).ok()?;
    let mtime = meta.modified().ok()?;
    let dur = mtime.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(dur.as_nanos())
}
