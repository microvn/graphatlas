import { useCallback, useEffect, useState } from "react";
import type { ApiClient } from "../api";
import {
  CacheCorruptError,
  type JobState,
  type ProjectRow,
  type ReindexStatus,
  type WatcherSnapshot,
} from "../types";
import { formatBytes, formatCount, formatRelativeTime } from "../formatters";
import { RemoveProjectDialog } from "./RemoveProjectDialog";

const POLL_INTERVAL_MS = 1000;

const JOB_KEY_PREFIX = "ga_ui_job_";

function rememberJob(slug: string, jobId: string | null) {
  if (jobId) localStorage.setItem(`${JOB_KEY_PREFIX}${slug}`, jobId);
  else localStorage.removeItem(`${JOB_KEY_PREFIX}${slug}`);
}

function recallJob(slug: string): string | null {
  return localStorage.getItem(`${JOB_KEY_PREFIX}${slug}`);
}

export function IndexControlTab({
  api,
  project,
  onChanged,
  onDeleted,
  autoStartTick = 0,
}: {
  api: ApiClient;
  project: ProjectRow;
  onChanged: () => void;
  onDeleted?: () => void;
  autoStartTick?: number;
}) {
  const slug = project.slug;
  const [job, setJob] = useState<ReindexStatus | null>(null);
  const [jobId, setJobId] = useState<string | null>(() => recallJob(slug));
  const [watcher, setWatcher] = useState<WatcherSnapshot | null>(null);
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [logOpen, setLogOpen] = useState(false);
  const [showRemove, setShowRemove] = useState(false);
  const isOrphan = project.index_state === "Orphan";

  // Poll active job.
  useEffect(() => {
    if (!jobId) {
      setJob(null);
      return;
    }
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;

    const tick = async () => {
      try {
        const s = await api.reindexStatus(slug, jobId);
        if (cancelled) return;
        setJob(s);
        if (s.state === "Running") {
          timer = setTimeout(tick, POLL_INTERVAL_MS);
        } else {
          // Terminal — Done / Error / Cancelled. Clear remembered job
          // so a future Reindex starts fresh. Trigger parent refresh.
          rememberJob(slug, null);
          setJobId(null);
          onChanged();
        }
      } catch (e) {
        if (cancelled) return;
        if ((e as { error?: string }).error === "job_not_found") {
          rememberJob(slug, null);
          setJobId(null);
        } else {
          setActionError((e as Error).message);
        }
      }
    };
    tick();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [api, slug, jobId, onChanged]);

  // Watcher snapshot — poll only while the watcher is Running. When
  // Stopped/Errored there's nothing to refresh; polling would just
  // spam fetches with no payload change. After a toggle the
  // `toggleWatcher` handler sets fresh state, which retriggers this
  // effect via the deps and re-decides whether to poll.
  const watcherRunning = watcher?.status === "Running";
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const tick = async () => {
      try {
        const w = await api.watcherStatus(slug);
        if (cancelled) return;
        setWatcher(w);
        // If the server flipped to Stopped/Errored mid-poll, stop here.
        if (w.status !== "Running") return;
      } catch {
        /* network blip — keep last good snapshot, try again next tick */
      }
      if (!cancelled) timer = setTimeout(tick, POLL_INTERVAL_MS);
    };
    if (watcherRunning) {
      tick();
    } else if (!watcher) {
      // One-shot fetch on first mount so we know the initial status.
      api
        .watcherStatus(slug)
        .then((w) => !cancelled && setWatcher(w))
        .catch(() => {});
    }
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api, slug, watcherRunning]);

  const startReindex = useCallback(async () => {
    if (busy || isOrphan) return;
    setBusy(true);
    setActionError(null);
    try {
      const r = await api.startReindex(slug);
      rememberJob(slug, r.job_id);
      setJobId(r.job_id);
    } catch (e) {
      const code = (e as { error?: string }).error;
      if (code === "reindex_in_progress") {
        // Resume polling that existing job_id from response. If the
        // server returned 409 without a job_id, surface the message
        // instead of swallowing the click silently.
        const id = (e as { job_id?: string }).job_id;
        if (id) {
          rememberJob(slug, id);
          setJobId(id);
        } else {
          setActionError(
            (e as Error).message ||
              "Reindex bị chặn — state đang Building. Thử reload trang.",
          );
        }
      } else {
        setActionError((e as Error).message);
      }
    } finally {
      setBusy(false);
    }
  }, [api, slug, busy, isOrphan]);

  // Header "Reindex" button bumps autoStartTick to fire the same flow
  // a tab-local Reindex-now click would. Skip the initial mount (tick 0).
  useEffect(() => {
    if (autoStartTick > 0) startReindex();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autoStartTick]);

  const toggleWatcher = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    setActionError(null);
    try {
      const action = watcher?.status === "Running" ? "stop" : "start";
      const w = await api.watcherAction(slug, action);
      setWatcher(w);
    } catch (e) {
      setActionError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }, [api, slug, busy, watcher]);

  const now = Math.floor(Date.now() / 1000);
  const isReindexing = job?.state === "Running";
  const isPostCancel = job?.state === "Cancelled" || project.index_state === "Corrupt";
  const isStale = project.index_state === "Stale";

  return (
    <>
    {showRemove && (
      <RemoveProjectDialog
        api={api}
        slug={slug}
        name={project.name}
        onClose={() => setShowRemove(false)}
        onRemoved={() => {
          setShowRemove(false);
          onDeleted?.();
        }}
      />
    )}
    <section className="tab-panel index-tab active">
      {/* Status card */}
      <div className="card">
        <h3>Status</h3>
        <CardRow label="State" value={<StateBadge state={project.index_state} />} />
        <CardRow
          label="Last indexed"
          value={formatRelativeTime(project.last_indexed_unix, now)}
        />
        <CardRow
          label="Schema"
          value="v5 (current)"
        />
        {isPostCancel && (
          <div
            style={{
              marginTop: 12,
              padding: 8,
              background: "#EF44441A",
              border: "1px solid var(--err)",
              borderRadius: 4,
              fontSize: 11,
              color: "var(--err)",
            }}
          >
            Cache có thể inconsistent — chạy Reindex để rebuild
          </div>
        )}
        {isStale && !isPostCancel && !isReindexing && (
          <div
            data-testid="stale-suggest"
            style={{
              marginTop: 12,
              padding: 8,
              background: "#F59E0B1A",
              border: "1px solid var(--warn)",
              borderRadius: 4,
              fontSize: 11,
              color: "var(--warn)",
              display: "flex",
              alignItems: "center",
              gap: 8,
            }}
          >
            <span style={{ flex: 1 }}>
              Source đã thay đổi từ lần index cuối — nên Reindex để cập nhật.
            </span>
            <button
              className="btn btn-primary"
              style={{ fontSize: 11, padding: "2px 10px" }}
              onClick={startReindex}
              disabled={busy || isOrphan}
            >
              Reindex now
            </button>
          </div>
        )}
      </div>

      {/* Cache card */}
      <div className="card">
        <h3>Cache</h3>
        <CardRow
          label="Path"
          value={
            <span style={{ fontSize: 11, textAlign: "right" }}>
              ~/.graphatlas/{project.name}-{project.slug}
            </span>
          }
        />
        <CardRow
          label="Size on disk"
          value={formatBytes(project.index_counts?.db_size_bytes ?? 0)}
        />
        <CardRow
          label="Files indexed"
          value={formatCount(project.index_counts?.file_count)}
        />
        <CardRow label="Slug" value={<code>{project.slug}</code>} />
      </div>

      {/* Reindex card — span 2 */}
      <div className="card" style={{ gridColumn: "span 2" }}>
        <h3>Reindex</h3>
        <div
          style={{
            display: "flex",
            gap: 12,
            alignItems: "center",
            marginBottom: 8,
          }}
        >
          <button
            className="btn btn-primary"
            onClick={startReindex}
            disabled={busy || isReindexing || isOrphan}
            title={isOrphan ? "Source path không tồn tại" : undefined}
          >
            {isReindexing ? "Reindexing…" : "Reindex now"}
          </button>
          <button
            className="btn btn-ghost"
            onClick={() => setShowRemove(true)}
            disabled={busy || isReindexing}
            style={{ color: "var(--err)", borderColor: "var(--err)" }}
            title="Xoá index cache (source code không bị ảnh hưởng)"
          >
            Delete
          </button>
          <div style={{ flex: 1 }} />
          {job && (
            <div className="kv">
              <span className="k">job</span>{" "}
              <span className="v">{job.job_id.slice(0, 8)}</span>
              {" · "}
              <span className="k">state</span>{" "}
              <span className="v" style={{ color: stateColor(job.state) }}>
                {job.state}
              </span>
            </div>
          )}
        </div>

        {isReindexing && (
          <>
            <div className="progress">
              <div
                className="progress-fill"
                style={{ width: `${job!.percent}%` }}
              />
            </div>
            <div className="progress-meta">
              <span>{job!.phase ?? job!.current_file ?? "preparing…"}</span>
              <span className="pct">
                {job!.percent.toFixed(0)}% · {formatCount(job!.files_done)}/
                {formatCount(job!.files_total)}
              </span>
            </div>
          </>
        )}

        {job?.error && (
          <div
            style={{
              marginTop: 8,
              padding: 8,
              background: "#EF44441A",
              borderRadius: 4,
              fontSize: 12,
              color: "var(--err)",
            }}
          >
            <b>Error:</b> {job.error}
          </div>
        )}

        {actionError && (
          <div style={{ marginTop: 8, color: "var(--err)", fontSize: 12 }}>
            {actionError}
          </div>
        )}

        {job?.log_tail && job.log_tail.length > 0 && (
          <>
            <div
              className="show-more"
              onClick={() => setLogOpen((v) => !v)}
              style={{ marginTop: 12 }}
            >
              {logOpen ? "Hide log ↑" : `View log (${job.log_tail.length} lines) →`}
            </div>
            {logOpen && (
              <div className="log">
                {job.log_tail.map((line, i) => (
                  <div key={i}>{line}</div>
                ))}
              </div>
            )}
          </>
        )}
      </div>

      {/* Watcher card */}
      <div className="card">
        <h3>Watcher</h3>
        <CardRow
          label="Enabled"
          value={
            <span
              className={`toggle${watcher?.status === "Running" ? " on" : ""}`}
              onClick={toggleWatcher}
              style={{ cursor: busy ? "wait" : "pointer" }}
            >
              <span className="toggle-knob" />
              <span className="kv">
                <span className="v">{watcher?.status?.toLowerCase() ?? "—"}</span>
              </span>
            </span>
          }
        />
        <CardRow label="Mode" value={watcher?.mode ?? "—"} />
        <CardRow label="Queue pending" value={formatCount(watcher?.queue_pending ?? 0)} />
        <CardRow
          label="Last event"
          value={
            watcher?.last_event_unix
              ? formatRelativeTime(watcher.last_event_unix, now)
              : "—"
          }
        />
        {watcher?.status === "Errored" && watcher.error && (
          <div
            style={{
              marginTop: 8,
              padding: 6,
              background: "#F59E0B1A",
              fontSize: 11,
              color: "var(--warn)",
              borderRadius: 4,
            }}
          >
            {watcher.error}
            <button
              className="btn"
              style={{ marginTop: 6, fontSize: 11 }}
              onClick={toggleWatcher}
              disabled={busy}
            >
              Restart watcher
            </button>
          </div>
        )}
      </div>

      {/* Last run summary */}
      <div className="card">
        <h3>Last run</h3>
        {project.index_counts ? (
          <>
            <CardRow
              label="Duration"
              value={`${(project.index_counts.last_index_duration_ms / 1000).toFixed(1)}s`}
            />
            <CardRow label="Files" value={formatCount(project.index_counts.file_count)} />
            <CardRow label="Nodes" value={formatCount(project.index_counts.node_count)} />
            <CardRow label="Edges" value={formatCount(project.index_counts.edge_count)} />
          </>
        ) : (
          <div style={{ color: "var(--muted)", fontSize: 12, padding: "8px 0" }}>
            — chưa có metric. Reindex để cập nhật.
          </div>
        )}
      </div>
    </section>
    </>
  );
}

function CardRow({
  label,
  value,
}: {
  label: string;
  value: React.ReactNode;
}) {
  return (
    <div className="card-row">
      <span className="lbl">{label}</span>
      <span className="val">{value}</span>
    </div>
  );
}

function StateBadge({ state }: { state: ProjectRow["index_state"] }) {
  const cls: Record<typeof state, string> = {
    Fresh: "badge badge-fresh",
    Building: "badge badge-build",
    Corrupt: "badge badge-corrupt",
    Orphan: "badge badge-orphan",
    Stale: "badge badge-stale",
  };
  return <span className={cls[state]}>{state.toLowerCase()}</span>;
}

function stateColor(state: JobState): string {
  switch (state) {
    case "Done":
      return "var(--ok)";
    case "Error":
      return "var(--err)";
    case "Cancelled":
      return "var(--warn)";
    case "Running":
      return "var(--accent-cool)";
  }
}
