import { useCallback, useEffect, useState } from "react";
import type { ApiClient } from "../api";
import type { ProjectRow } from "../types";
import { parseHashRoute, serializeRoute, type ProjectTab } from "../router";
import { formatCount, formatRelativeTime } from "../formatters";
import { IndexControlTab } from "./IndexControlTab";
import { GraphTab } from "./GraphTab";

export function ProjectDetailPage({
  api,
  slug,
  initialTab,
}: {
  api: ApiClient;
  slug: string;
  initialTab: ProjectTab;
}) {
  const [tab, setTab] = useState<ProjectTab>(initialTab);
  const [project, setProject] = useState<ProjectRow | null>(null);
  const [loadErr, setLoadErr] = useState<string | null>(null);
  const [reloadTick, setReloadTick] = useState(0);
  const [autoStartTick, setAutoStartTick] = useState(0);

  // Resolve project by re-using /api/projects (Page 2 doesn't have a
  // dedicated single-project endpoint Phase 1; this is fine because
  // the list is bounded and cached client-side).
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const { rows } = await api.fetchProjects();
        if (cancelled) return;
        const found = rows.find((r) => r.slug === slug);
        if (!found) {
          setLoadErr("not_found");
        } else {
          setProject(found);
          setLoadErr(null);
        }
      } catch (e) {
        if (cancelled) return;
        setLoadErr((e as Error).message);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [api, slug, reloadTick]);

  const switchTab = useCallback(
    (next: ProjectTab) => {
      setTab(next);
      window.history.replaceState(
        null,
        "",
        "#" + serializeRoute({ page: "detail", slug, tab: next }),
      );
    },
    [slug],
  );

  // React to manual hashchange (e.g. browser back/forward).
  useEffect(() => {
    const onHash = () => {
      const r = parseHashRoute(window.location.hash);
      if (r.page === "detail" && r.slug === slug) {
        setTab(r.tab);
      }
    };
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, [slug]);

  if (loadErr === "not_found") {
    return <NotFoundPage slug={slug} />;
  }
  if (loadErr) {
    return (
      <div style={{ padding: 48, textAlign: "center" }}>
        <div style={{ color: "var(--err)", marginBottom: 8 }}>
          Lỗi load project
        </div>
        <div style={{ color: "var(--muted)", fontSize: 12 }}>{loadErr}</div>
      </div>
    );
  }
  if (!project) {
    return <div style={{ padding: 32, color: "var(--muted)" }}>loading…</div>;
  }

  const isOrphan = project.index_state === "Orphan";
  const isCorrupt = project.index_state === "Corrupt";
  const now = Math.floor(Date.now() / 1000);

  return (
    <>
      <div className="detail-header">
        <div style={{ flex: 1 }}>
          <div className="crumbs">
            <a
              href="#"
              onClick={(ev) => {
                ev.preventDefault();
                window.location.hash = "";
              }}
            >
              projects
            </a>{" "}
            / {project.name}
          </div>
          <h1>{project.name}</h1>
          <div className="meta">
            slug <b style={{ color: "var(--text)" }}>{project.slug}</b> · indexed{" "}
            {formatRelativeTime(project.last_indexed_unix, now)} ·{" "}
            {formatCount(project.index_counts?.node_count)} nodes ·{" "}
            {formatCount(project.index_counts?.edge_count)} edges
          </div>
        </div>
        <button
          className="btn btn-ghost"
          onClick={() =>
            project.repo_root &&
            navigator.clipboard?.writeText(project.repo_root)
          }
          title="Copy repo path"
        >
          Copy path
        </button>
        <button
          className="btn btn-primary"
          onClick={() => {
            switchTab("index");
            setAutoStartTick((n) => n + 1);
          }}
          disabled={isOrphan}
        >
          Reindex
        </button>
      </div>

      {isOrphan && (
        <div
          style={{
            padding: "8px 24px",
            background: "#EF44441A",
            borderBottom: "1px solid var(--err)",
            color: "var(--err)",
            fontSize: 12,
            fontFamily: "var(--font-mono)",
          }}
        >
          Source path missing — read-only mode
        </div>
      )}
      {isCorrupt && (
        <div
          style={{
            padding: "8px 24px",
            background: "#EF44441A",
            borderBottom: "1px solid var(--err)",
            color: "var(--err)",
            fontSize: 12,
            fontFamily: "var(--font-mono)",
          }}
        >
          Cache corrupt — reindex required.{" "}
          <a
            href="#"
            onClick={(ev) => {
              ev.preventDefault();
              switchTab("index");
            }}
            style={{ color: "var(--accent)", marginLeft: 4 }}
          >
            Open Index Control →
          </a>
        </div>
      )}

      <nav className="tabs">
        <button
          id="tab-graph"
          className={`tab${tab === "graph" ? " active" : ""}`}
          onClick={() => switchTab("graph")}
        >
          Graph
          {project.index_counts && (
            <span className="badge-mono">
              {formatCount(project.index_counts.node_count)}
            </span>
          )}
        </button>
        <button
          id="tab-index"
          className={`tab${tab === "index" ? " active" : ""}`}
          onClick={() => switchTab("index")}
        >
          Index control
        </button>
      </nav>

      {/* Render both tabs but only show active — preserves state on switch (AS-003). */}
      <div style={{ display: tab === "graph" ? "block" : "none" }}>
        <GraphTab api={api} project={project} />
      </div>
      <div style={{ display: tab === "index" ? "block" : "none" }}>
        <IndexControlTab
          api={api}
          project={project}
          autoStartTick={autoStartTick}
          onChanged={() => setReloadTick((n) => n + 1)}
          onDeleted={() => {
            window.location.hash = "";
          }}
        />
      </div>
    </>
  );
}

function NotFoundPage({ slug }: { slug: string }) {
  return (
    <div style={{ padding: 64, textAlign: "center" }}>
      <h1 style={{ marginBottom: 12 }}>Project không tồn tại</h1>
      <div style={{ color: "var(--muted)", marginBottom: 16 }}>
        Slug <code>{slug}</code> không có trong registry.
      </div>
      <button
        className="btn btn-primary"
        onClick={() => (window.location.hash = "")}
      >
        ← Về Projects list
      </button>
    </div>
  );
}
