/**
 * Pure formatting helpers — DESIGN.md typography + mono columns.
 * All functions deterministic per inputs; no Date.now() except inside
 * `formatRelativeTime` where the caller supplies `now_unix`.
 */

import type { ProjectIndexState, WatcherStatus } from "./types";

/** "2h ago" / "5m ago" / "3d ago" / "62d ago". Coarse buckets. */
export function formatRelativeTime(unix_seconds: number, now_unix: number): string {
  if (unix_seconds <= 0) return "—";
  const delta = Math.max(0, now_unix - unix_seconds);
  if (delta < 60) return `${delta}s ago`;
  if (delta < 60 * 60) return `${Math.floor(delta / 60)}m ago`;
  if (delta < 60 * 60 * 24) return `${Math.floor(delta / 3600)}h ago`;
  return `${Math.floor(delta / 86400)}d ago`;
}

/** Human-readable size, matches prototype: 12 MB / 4.1 MB / 1.2 GB. */
export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return "—";
  const KB = 1024;
  const MB = KB * 1024;
  const GB = MB * 1024;
  if (n >= GB) return `${(n / GB).toFixed(1)} GB`;
  if (n >= MB) return `${Math.round(n / MB)} MB`;
  if (n >= KB) return `${Math.round(n / KB)} KB`;
  return `${n} B`;
}

/** Compact integer with thousands separator. */
export function formatCount(n: number | null | undefined): string {
  if (n === null || n === undefined || !Number.isFinite(n)) return "—";
  return n.toLocaleString("en-US");
}

/** Map `ProjectIndexState` → CSS class on the badge span. */
export function indexStateBadgeClass(state: ProjectIndexState): string {
  switch (state) {
    case "Fresh":
      return "badge badge-fresh";
    case "Building":
      return "badge badge-build";
    case "Corrupt":
      return "badge badge-corrupt";
    case "Orphan":
      return "badge badge-orphan";
    case "Stale":
      return "badge badge-stale";
  }
}

export function indexStateLabel(state: ProjectIndexState): string {
  return state.toLowerCase();
}

/** Watcher → dot CSS class. */
export function watcherDotClass(status: WatcherStatus): string {
  switch (status) {
    case "Running":
      return "dot dot-on";
    case "Stopped":
      return "dot dot-off";
    case "Errored":
      return "dot dot-err";
  }
}

/** Project name from repo_root basename (frontend mirror of server slug). */
export function projectBasename(repoRoot: string): string {
  const parts = repoRoot.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? repoRoot;
}

/** Client-side filter: name OR path substring (case-insensitive). */
export function filterProjects<T extends { name: string; repo_root: string }>(
  rows: T[],
  query: string,
): T[] {
  const q = query.trim().toLowerCase();
  if (!q) return rows;
  return rows.filter(
    (r) =>
      r.name.toLowerCase().includes(q) ||
      r.repo_root.toLowerCase().includes(q),
  );
}

/** Sort by `last_indexed_unix` desc (default). */
export function sortByLastIndexedDesc<
  T extends { last_indexed_unix: number },
>(rows: T[]): T[] {
  return [...rows].sort((a, b) => b.last_indexed_unix - a.last_indexed_unix);
}
