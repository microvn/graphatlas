import { useEffect, useState, useCallback } from "react";
import { createRoot } from "react-dom/client";
import { ApiClient } from "./api";
import { ProjectsTable } from "./components/ProjectsTable";
import { AddProjectModal } from "./components/AddProjectModal";
import { RemoveProjectDialog } from "./components/RemoveProjectDialog";
import { ProjectDetailPage } from "./components/ProjectDetailPage";
import {
  filterProjects,
  sortByLastIndexedDesc,
} from "./formatters";
import { parseHashRoute, type Route } from "./router";
import { type ProjectRow, SessionExpiredError, NetworkError } from "./types";

import "./styles.css";

const POLL_INTERVAL_MS = 10_000; // Spec B AS-011 / B-C4

function App() {
  // Token auth removed 2026-05-17 — Origin + Host validation already
  // gates the loopback server. No bootstrap needed; URL is just the
  // root path.
  const [api] = useState(() => new ApiClient());
  const [rows, setRows] = useState<ProjectRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<"net" | "session" | "other" | null>(null);
  const [errMsg, setErrMsg] = useState("");
  const [corruptCount, setCorruptCount] = useState(0);
  const [filterQ, setFilterQ] = useState("");
  const [showAdd, setShowAdd] = useState(false);
  const [removeTarget, setRemoveTarget] = useState<ProjectRow | null>(null);
  const [nowUnix, setNowUnix] = useState(Math.floor(Date.now() / 1000));

  const refresh = useCallback(async () => {
    if (!api) return;
    try {
      const { rows, corruptCount } = await api.fetchProjects();
      setRows(rows);
      setCorruptCount(corruptCount);
      setErr(null);
      setLoading(false);
    } catch (e) {
      setLoading(false);
      if (e instanceof SessionExpiredError) {
        setErr("session");
      } else if (e instanceof NetworkError) {
        setErr("net");
        setErrMsg((e as Error).message);
      } else {
        setErr("other");
        setErrMsg((e as Error).message);
      }
    }
  }, [api]);

  useEffect(() => {
    if (!api) return;
    refresh();
    const id = setInterval(() => {
      refresh();
      setNowUnix(Math.floor(Date.now() / 1000));
    }, POLL_INTERVAL_MS);
    return () => clearInterval(id);
  }, [api, refresh]);

  const handleSelect = useCallback((slug: string) => {
    window.location.hash = `#/project/${slug}`;
  }, []);

  // Spec C — top-level hash routing.
  const [route, setRoute] = useState<Route>(() =>
    parseHashRoute(window.location.hash),
  );
  useEffect(() => {
    const onHash = () => setRoute(parseHashRoute(window.location.hash));
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, []);

  // Page 2 — Project detail. Topbar still rendered for context.
  if (route.page === "detail") {
    return (
      <>
        <Topbar />
        <ProjectDetailPage
          api={api}
          slug={route.slug}
          initialTab={route.tab}
        />
      </>
    );
  }

  const filtered = sortByLastIndexedDesc(filterProjects(rows, filterQ));

  return (
    <>
      <Topbar />

      {/* Banners */}
      {corruptCount > 0 && (
        <Banner kind="warn">
          {corruptCount} cache bị hỏng — chạy <code>ga doctor</code>
        </Banner>
      )}
      {err === "session" && (
        <Banner kind="err">
          Origin/Host rejected — kiểm tra <code>ga ui</code> đang chạy đúng port
        </Banner>
      )}

      <main className="page active">
        <div className="list-header">
          <h1>
            Projects
            <span className="count">
              {loading ? "…" : filtered.length}
            </span>
          </h1>
          <div className="grow" />
          <input
            className="search"
            placeholder="filter name or path…"
            value={filterQ}
            onChange={(e) => setFilterQ(e.target.value)}
          />
          <button className="btn btn-ghost" onClick={refresh} disabled={loading}>
            Refresh
          </button>
          <button
            className="btn btn-primary"
            onClick={() => setShowAdd(true)}
          >
            + Add project
          </button>
        </div>

        {loading && <LoadingSkeleton />}
        {!loading && err === "net" && (
          <ErrorState
            title="Không kết nối được ga-server"
            detail={errMsg}
            onRetry={refresh}
          />
        )}
        {!loading && !err && filtered.length === 0 && rows.length === 0 && (
          <EmptyState onAdd={() => setShowAdd(true)} />
        )}
        {!loading && !err && filtered.length === 0 && rows.length > 0 && (
          <div style={{ padding: 32, color: "var(--muted)" }}>
            Không có project nào khớp filter "{filterQ}"
          </div>
        )}
        {!loading && !err && filtered.length > 0 && (
          <ProjectsTable rows={filtered} nowUnix={nowUnix} onSelect={handleSelect} />
        )}
      </main>

      {api && showAdd && (
        <AddProjectModal
          api={api}
          onClose={() => setShowAdd(false)}
          onAdded={refresh}
        />
      )}
      {api && removeTarget && (
        <RemoveProjectDialog
          api={api}
          slug={removeTarget.slug}
          name={removeTarget.name}
          onClose={() => setRemoveTarget(null)}
          onRemoved={refresh}
        />
      )}
    </>
  );
}

function Topbar() {
  return (
    <header className="topbar">
      <div className="brand">
        <span className="brand-mark" />
        GraphAtlas
        <span className="brand-version">ui v0.1</span>
      </div>
      <div className="topbar-spacer" />
      <div className="topbar-meta">
        backend <b>{window.location.host}</b>
      </div>
    </header>
  );
}

function Banner({ kind, children }: { kind: "warn" | "err"; children: React.ReactNode }) {
  const bg = kind === "err" ? "#EF44441A" : "#F59E0B1A";
  const color = kind === "err" ? "var(--err)" : "var(--warn)";
  return (
    <div
      style={{
        padding: "8px 24px",
        background: bg,
        color,
        fontSize: 12,
        borderBottom: "1px solid var(--border)",
        fontFamily: "var(--font-mono)",
      }}
    >
      {children}
    </div>
  );
}

function LoadingSkeleton() {
  return (
    <div className="tbl-wrap">
      <div style={{ display: "flex", flexDirection: "column", gap: 8, marginTop: 16 }}>
        {[0, 1, 2].map((i) => (
          <div
            key={i}
            style={{
              height: 32,
              background:
                "linear-gradient(90deg, var(--surface) 0%, var(--surface-2) 50%, var(--surface) 100%)",
              borderRadius: 4,
              animation: `shimmer 1.4s linear ${i * 0.15}s infinite`,
              backgroundSize: "200% 100%",
            }}
          />
        ))}
      </div>
      <style>{`@keyframes shimmer { 0%{background-position:0% 50%;} 100%{background-position:200% 50%;} }`}</style>
    </div>
  );
}

function EmptyState({ onAdd }: { onAdd: () => void }) {
  return (
    <div
      style={{
        padding: "64px 24px",
        textAlign: "center",
        color: "var(--muted)",
      }}
    >
      <div style={{ fontFamily: "var(--font-display)", fontSize: 20, color: "var(--text)", marginBottom: 8 }}>
        Chưa có project nào
      </div>
      <div style={{ marginBottom: 16, fontSize: 13 }}>
        Bấm "Add project" hoặc chạy <code>ga index &lt;repo&gt;</code> ở terminal.
      </div>
      <button className="btn btn-primary" onClick={onAdd}>
        + Add project
      </button>
    </div>
  );
}

function ErrorState({
  title,
  detail,
  onRetry,
}: {
  title: string;
  detail: string;
  onRetry: () => void;
}) {
  return (
    <div style={{ padding: "48px 24px", textAlign: "center" }}>
      <div style={{ fontSize: 16, marginBottom: 8, color: "var(--err)" }}>{title}</div>
      <div style={{ marginBottom: 16, color: "var(--muted)", fontSize: 12, fontFamily: "var(--font-mono)" }}>
        {detail}
      </div>
      <button className="btn" onClick={onRetry}>
        Retry
      </button>
    </div>
  );
}

const root = createRoot(document.getElementById("root")!);
root.render(<App />);
