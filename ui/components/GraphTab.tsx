import { useCallback, useEffect, useRef, useState } from "react";
import type { ApiClient } from "../api";
import {
  CacheCorruptError,
  type GraphEdge,
  type GraphNode,
  type GraphResponse,
  type ProjectRow,
  type RelationPage,
  type SymbolDetail,
} from "../types";
import { GraphSidebar } from "./GraphSidebar";

// Color map mirrors prototype legend. Backend emits `kind` lowercase
// (function/method/struct/class/...) — keys here are lowercase and
// lookups normalize, so a casing drift never silently greys the graph.
import { kindColor } from "../kind-colors";

const EDIT_SCHEMES = ["vscode://", "cursor://", "idea://", "file://"] as const;

/**
 * File-path heuristic — same patterns the (deferred) backend test-kind
 * detection would use. Centralised here so keeper-priority sort stays
 * stable across language conventions.
 */
function isTestPath(p: string): boolean {
  return (
    /\.(test|spec)\.[a-z]+$/i.test(p) ||
    /\/(__tests__|__test__|tests|test|spec)\//i.test(p) ||
    /_test\.[a-z]+$/i.test(p) // Go convention
  );
}

/**
 * Ego-mode collapse: same `(name, kind)` siblings appearing in N
 * different files (a textbook test-helper pattern) read as visual
 * noise — 8 dots saying `setupTenant`. Fold the dupes onto one
 * representative node, redirect edges, attach a count for the badge.
 * Skipped in full-graph mode (focus === null) where users want the
 * raw structure, and never touches a group that contains the focused
 * node itself.
 */
function collapseDuplicateSiblings(
  graph: GraphResponse,
  _focus: string | null,
): { graph: GraphResponse; fanCounts: Map<string, number> } {
  const fanCounts = new Map<string, number>();
  // Earlier rev gated this on `focus !== null` (ego mode only) — but
  // the dupe-fanout pattern (8 test files each defining `setupTenant`)
  // is just as noisy in full-graph view, so collapse always.
  const groups = new Map<string, GraphNode[]>();
  for (const n of graph.nodes) {
    const key = `${n.name}::${n.kind}`;
    let arr = groups.get(key);
    if (!arr) {
      arr = [];
      groups.set(key, arr);
    }
    arr.push(n);
  }
  const dropped = new Set<string>();
  const idRemap = new Map<string, string>();
  for (const arr of groups.values()) {
    if (arr.length < 2) continue;
    // (Earlier rev guarded "don't collapse group containing focus" by
    // name match — but when the user focuses `setupTenant` and the
    // index has 8 same-name defs, ALL members match focus and nothing
    // collapses. That's the exact case dupes need to fold. Collapse
    // unconditionally; keeper's file:line preserved on the fan node.)
    //
    // Keeper picked deterministically: prefer non-test file (so the
    // canonical `helpers.ts` becomes the surviving node, not one of
    // the `.test.ts` wrappers), then highest degree, then alphabetical
    // file path. arr.sort is in-place but `groups` map isn't reused.
    arr.sort((a, b) => {
      const aTest = isTestPath(a.file) ? 1 : 0;
      const bTest = isTestPath(b.file) ? 1 : 0;
      if (aTest !== bTest) return aTest - bTest;
      if (a.degree !== b.degree) return b.degree - a.degree;
      return a.file.localeCompare(b.file);
    });
    const keeper = arr[0]!;
    fanCounts.set(keeper.id, arr.length);
    for (let i = 1; i < arr.length; i++) {
      dropped.add(arr[i]!.id);
      idRemap.set(arr[i]!.id, keeper.id);
    }
  }
  if (dropped.size === 0) return { graph, fanCounts };
  const nodes = graph.nodes.filter((n) => !dropped.has(n.id));
  const seen = new Set<string>();
  const edges: GraphEdge[] = [];
  for (const e of graph.edges) {
    const from = idRemap.get(e.from) ?? e.from;
    const to = idRemap.get(e.to) ?? e.to;
    if (from === to) continue; // self-loop after redirect — noise
    const k = `${from}${to}${e.kind}`;
    if (seen.has(k)) continue;
    seen.add(k);
    edges.push({ ...e, from, to });
  }
  return {
    graph: { ...graph, nodes, edges },
    fanCounts,
  };
}

export function GraphTab({
  api,
  project,
}: {
  api: ApiClient;
  project: ProjectRow;
}) {
  const slug = project.slug;
  const containerRef = useRef<HTMLDivElement | null>(null);
  const sigmaRef = useRef<any>(null); // Sigma instance, untyped lazy import
  const graphRef = useRef<any>(null); // graphology Graph

  const [loading, setLoading] = useState(true);
  const [err, setErr] = useState<string | null>(null);
  const [graph, setGraph] = useState<GraphResponse | null>(null);
  const [focus, setFocus] = useState<string | null>(null);
  const [stale, setStale] = useState(false);

  const [detail, setDetail] = useState<SymbolDetail | null>(null);
  const [callers, setCallers] = useState<RelationPage | null>(null);
  const [callees, setCallees] = useState<RelationPage | null>(null);

  // Spec E S-002 AS-009 — chip-active layer + symbol membership set
  // for canvas highlight/dim.
  const [activeLayer, setActiveLayer] = useState<string | null>(null);
  const [layerMembers, setLayerMembers] = useState<Set<string> | null>(null);

  const onLayerChange = useCallback(
    (layer: string | null, members: Set<string> | null) => {
      setActiveLayer(layer);
      setLayerMembers(members);
    },
    [],
  );

  // Apply membership to Sigma nodes — dim non-members when active.
  useEffect(() => {
    const g = graphRef.current;
    const sigma = sigmaRef.current;
    if (!g || !sigma) return;
    g.forEachNode((id: string, attrs: any) => {
      if (!layerMembers) {
        // Restore original color (best-effort: kindColor of stored attr).
        const k = (attrs.kind as string) ?? "other";
        g.setNodeAttribute(id, "color", kindColor(k));
        return;
      }
      const inLayer = layerMembers.has(id);
      const k = (attrs.kind as string) ?? "other";
      g.setNodeAttribute(
        id,
        "color",
        inLayer ? kindColor(k) : "#262626",
      );
    });
    try {
      sigma.refresh();
    } catch {
      /* sigma not ready */
    }
  }, [layerMembers]);
  const [edgeTip, setEdgeTip] = useState<{
    x: number;
    y: number;
    from: string;
    to: string;
    kind: string;
  } | null>(null);

  // ============== Fetch graph ==============
  const fetchGraph = useCallback(
    async (f?: string | null) => {
      setLoading(true);
      setErr(null);
      try {
        const { graph, stale } = await api.graphDump(slug, f ?? undefined, 2);
        setGraph(graph);
        setStale(stale);
      } catch (e) {
        if (e instanceof CacheCorruptError) {
          setErr("cache_corrupt");
        } else {
          setErr((e as Error).message);
        }
      } finally {
        setLoading(false);
      }
    },
    [api, slug],
  );

  useEffect(() => {
    fetchGraph(focus);
  }, [fetchGraph, focus]);

  // ============== Sigma render ==============
  useEffect(() => {
    if (!graph || !containerRef.current) return;

    let cancelled = false;
    let renderer: any = null;
    let g: any = null;

    (async () => {
      // Lazy-load Sigma + graphology via CDN ESM. Bundled deps would
      // bloat the main bundle past 2 MB; Page 1 doesn't need them.
      const [{ default: Graph }, { Sigma }, fa2Mod] = await Promise.all([
        import("https://cdn.jsdelivr.net/npm/graphology@0.25.4/+esm" as any),
        import("https://cdn.jsdelivr.net/npm/sigma@3.0.0-beta.30/+esm" as any),
        import(
          "https://cdn.jsdelivr.net/npm/graphology-layout-forceatlas2@0.10.1/+esm" as any
        ),
      ]).catch((e) => {
        if (!cancelled) setErr(`Sigma.js load failed: ${e}`);
        return [null, null, null] as const;
      });
      if (cancelled || !Graph || !Sigma) return;
      const forceAtlas2 = (fa2Mod as any).default ?? fa2Mod;

      g = new Graph();
      // Collapse same-(name,kind) siblings in ego mode so duplicated
      // test-helper definitions read as one fan node, not 8 dots.
      const { graph: rendered, fanCounts } = collapseDuplicateSiblings(
        graph,
        focus,
      );
      // Add nodes with initial random positions; FA2 layout below
      // moves them to a stable graph-aware layout.
      const N = rendered.nodes.length;
      const radius = Math.sqrt(N) * 18;
      for (let i = 0; i < N; i++) {
        const node = rendered.nodes[i]!;
        const angle = (i / N) * Math.PI * 2;
        const isFocused = focus === node.name;
        const fanCount = fanCounts.get(node.id) ?? 0;
        const baseSize = isFocused ? 8 : Math.min(8, 2 + Math.log2(node.degree + 1));
        const fanBonus = fanCount > 1 ? Math.min(6, Math.log2(fanCount) * 2) : 0;
        g.addNode(node.id, {
          x: Math.cos(angle) * radius + (Math.random() - 0.5) * 4,
          y: Math.sin(angle) * radius + (Math.random() - 0.5) * 4,
          size: baseSize + fanBonus,
          color: isFocused ? "#7AA2F7" : kindColor(node.kind),
          label: fanCount > 1 ? `${node.name} ×${fanCount}` : node.name,
          name: node.name,
          file: node.file,
          line: node.line,
          kind: node.kind,
          fanCount,
        });
      }
      for (const e of rendered.edges) {
        if (!g.hasNode(e.from) || !g.hasNode(e.to)) continue;
        const key = `${e.from}->${e.to}`;
        if (g.hasEdge(key)) continue;
        try {
          g.addEdgeWithKey(key, e.from, e.to, {
            color: "#333",
            size: 0.5,
            kind: e.kind,
          });
        } catch {
          /* parallel-edge race — ignore */
        }
      }

      // FA2 — short pass for ego graphs (<200 nodes), longer for full.
      const iters = N <= 200 ? 100 : N <= 2000 ? 80 : 40;
      try {
        forceAtlas2.assign(g, {
          iterations: iters,
          settings: {
            gravity: 1,
            scalingRatio: 10,
            slowDown: 1,
            barnesHutOptimize: N > 1000,
          },
        });
      } catch {
        /* layout best-effort */
      }

      if (cancelled || !containerRef.current) return;
      renderer = new Sigma(g, containerRef.current, {
        renderLabels: N <= 500,
        labelSize: 11,
        labelColor: { color: "#FAFAFA" },
        defaultEdgeColor: "#333",
        labelDensity: 0.4,
        labelGridCellSize: 60,
        // Sigma's stock hover draws a light pill that clashes with the
        // dark theme. Repaint it with surface/border/text tokens.
        defaultDrawNodeHover: (
          ctx: CanvasRenderingContext2D,
          data: { x: number; y: number; size: number; label?: string | null },
          settings: { labelSize: number; labelFont: string; labelWeight: string },
        ) => {
          const label = data.label;
          if (!label) return;
          const sz = settings.labelSize;
          ctx.font = `${settings.labelWeight} ${sz}px ${settings.labelFont}`;
          const pad = 6;
          const tw = ctx.measureText(label).width;
          const x = Math.round(data.x);
          const y = Math.round(data.y);
          const boxX = x - data.size - pad;
          const boxY = y - sz / 2 - pad;
          const boxW = data.size * 2 + tw + pad * 3;
          const boxH = sz + pad * 2;
          const r = 4;
          ctx.beginPath();
          ctx.moveTo(boxX + r, boxY);
          ctx.arcTo(boxX + boxW, boxY, boxX + boxW, boxY + boxH, r);
          ctx.arcTo(boxX + boxW, boxY + boxH, boxX, boxY + boxH, r);
          ctx.arcTo(boxX, boxY + boxH, boxX, boxY, r);
          ctx.arcTo(boxX, boxY, boxX + boxW, boxY, r);
          ctx.closePath();
          ctx.fillStyle = "#141414";
          ctx.fill();
          ctx.lineWidth = 1;
          ctx.strokeStyle = "#262626";
          ctx.stroke();
          ctx.beginPath();
          ctx.arc(x, y, data.size, 0, Math.PI * 2);
          ctx.fillStyle = "#7AA2F7";
          ctx.fill();
          ctx.fillStyle = "#FAFAFA";
          ctx.textBaseline = "middle";
          ctx.fillText(label, x + data.size + pad, y);
        },
      });

      renderer.on("clickNode", (e: { node: string }) => {
        // Backend (Phase 1) resolves symbols by name, not the composite
        // file::name:line node key. Focus the name so detail + ego load.
        const name = (g.getNodeAttribute(e.node, "name") as string) ?? e.node;
        setFocus(name);
        setEdgeTip(null);
      });
      renderer.on("clickEdge", (e: { edge: string; event: { x: number; y: number } }) => {
        try {
          const [from, to] = e.edge.split("->");
          const kind = (g.getEdgeAttribute(e.edge, "kind") as string) ?? "edge";
          const fromName = (g.getNodeAttribute(from, "label") as string) ?? from;
          const toName = (g.getNodeAttribute(to, "label") as string) ?? to;
          setEdgeTip({
            x: e.event.x,
            y: e.event.y,
            from: fromName,
            to: toName,
            kind,
          });
        } catch {
          /* edge meta missing — ignore */
        }
      });
      renderer.on("clickStage", () => setEdgeTip(null));

      sigmaRef.current = renderer;
      graphRef.current = g;
    })();

    return () => {
      cancelled = true;
      if (renderer) renderer.kill();
      if (g) g.clear();
      sigmaRef.current = null;
      graphRef.current = null;
    };
  }, [graph, focus]);

  // ============== Fetch detail when focus set ==============
  useEffect(() => {
    if (!focus) {
      setDetail(null);
      setCallers(null);
      setCallees(null);
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const [d, c1, c2] = await Promise.all([
          api.symbolDetail(slug, focus).then((r) => r.detail),
          api.callers(slug, focus, 0, 50).catch(() => null),
          api.callees(slug, focus, 0, 50).catch(() => null),
        ]);
        if (cancelled) return;
        setDetail(d);
        setCallers(c1);
        setCallees(c2);
      } catch {
        if (cancelled) return;
        setDetail(null);
        setCallers(null);
        setCallees(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [api, slug, focus]);

  const resetView = useCallback(() => {
    setFocus(null);
  }, []);

  const editorLink = useCallback((file: string, line: number): string | null => {
    // AS-020 — sanitize. Reject controls/newlines that could break URL.
    if (/[\x00-\x1F]/.test(file)) return null;
    return `vscode://file/${encodeURIComponent(file)}:${line}`;
  }, []);

  if (err === "cache_corrupt") {
    return (
      <section className="tab-panel graph-tab active">
        <div style={{ padding: 32, textAlign: "center", color: "var(--err)" }}>
          <h3 style={{ marginBottom: 8 }}>Cache corrupt</h3>
          <p style={{ marginBottom: 16 }}>Reindex required to render graph.</p>
          <button className="btn btn-primary" onClick={() => location.hash = `#/project/${slug}/index`}>
            Open Index Control
          </button>
        </div>
      </section>
    );
  }

  return (
    <section className="tab-panel graph-tab active">
      <GraphSidebar
        api={api}
        slug={slug}
        activeLayer={activeLayer}
        onActiveLayerChange={onLayerChange}
        onSelectSymbol={(name) => setFocus(name)}
      />
      <div className="canvas-wrap">
        <div className="canvas-toolbar">
          <button
            className={`pill${focus === null ? " active" : ""}`}
            onClick={resetView}
          >
            Full
          </button>
          {focus && (
            <>
              <button className="pill active">Ego × 2</button>
              <button className="pill" onClick={resetView}>
                Reset
              </button>
            </>
          )}
        </div>
        <div className="canvas-meta">
          {graph ? (
            <>
              {graph.truncated && `top ${graph.nodes.length} of `}
              {graph.total_node_count.toLocaleString()} nodes
              {stale && (
                <span style={{ marginLeft: 8, color: "var(--warn)" }}>
                  · reindex in progress
                </span>
              )}
            </>
          ) : (
            "loading…"
          )}
        </div>

        {loading && !graph && (
          <div
            style={{
              position: "absolute",
              inset: 0,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              color: "var(--muted)",
            }}
          >
            loading graph…
          </div>
        )}

        {graph && graph.nodes.length === 0 && !loading && (
          <div
            style={{
              position: "absolute",
              inset: 0,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              color: "var(--muted)",
              textAlign: "center",
            }}
          >
            <div>
              <div style={{ fontFamily: "var(--font-display)", fontSize: 18 }}>
                Repo không có symbol
              </div>
              <div style={{ marginTop: 6, fontSize: 12 }}>
                Check <code>ga reindex</code> log — có thể parser không bắt được
                file
              </div>
            </div>
          </div>
        )}

        <div
          ref={containerRef}
          className="canvas-svg"
          style={{ width: "100%", height: "100%" }}
        />

        {graph && graph.truncated && (
          <div
            style={{
              position: "absolute",
              bottom: 32,
              left: "50%",
              transform: "translateX(-50%)",
              background: "var(--surface)",
              border: "1px solid var(--warn)",
              padding: "6px 12px",
              borderRadius: 4,
              fontSize: 11,
              color: "var(--warn)",
              fontFamily: "var(--font-mono)",
            }}
          >
            Showing top {graph.nodes.length} by degree — focus a symbol to
            expand
          </div>
        )}

        {edgeTip && (
          <div
            data-testid="edge-tooltip"
            style={{
              position: "absolute",
              left: edgeTip.x + 8,
              top: edgeTip.y + 8,
              background: "var(--surface)",
              border: "1px solid var(--border)",
              padding: "6px 10px",
              borderRadius: 4,
              fontSize: 11,
              fontFamily: "var(--font-mono)",
              color: "var(--text)",
              pointerEvents: "none",
              zIndex: 5,
              maxWidth: 280,
            }}
          >
            <div style={{ color: "var(--accent)", marginBottom: 2 }}>
              {edgeTip.kind.toLowerCase()}
            </div>
            <div>{edgeTip.from}</div>
            <div style={{ color: "var(--muted)" }}>→ {edgeTip.to}</div>
          </div>
        )}

        <div className="canvas-legend">
          {Object.entries(KIND_COLOR).slice(0, 6).map(([k, c]) => (
            <span key={k}>
              <span className="legend-swatch" style={{ background: c }} />
              {k.toLowerCase()}
            </span>
          ))}
          <span style={{ marginLeft: 12 }}>
            <span className="legend-edge" />
            calls
          </span>
        </div>
      </div>

      <aside className="detail-panel">
        {!focus && (
          <div style={{ color: "var(--muted)", fontSize: 12 }}>
            <h3>Detail panel</h3>
            <p style={{ marginTop: 8 }}>
              Click một node trên canvas để xem signature, callers, callees,
              file summary.
            </p>
          </div>
        )}
        {focus && !detail && (
          <div style={{ color: "var(--muted)", fontSize: 12 }}>
            loading detail for <code>{focus}</code>…
          </div>
        )}
        {focus && detail && (
          <SymbolDetailPanel
            detail={detail}
            callers={callers}
            callees={callees}
            editorLink={editorLink}
            onSelectSymbol={(id) => setFocus(id)}
          />
        )}
      </aside>
    </section>
  );
}

// Spec E S-003 — redesigned 4-section panel.
function SymbolDetailPanel({
  detail,
  callers,
  callees,
  editorLink,
  onSelectSymbol,
}: {
  detail: SymbolDetail;
  callers: RelationPage | null;
  callees: RelationPage | null;
  editorLink: (file: string, line: number) => string | null;
  onSelectSymbol: (id: string) => void;
}) {
  const editor = editorLink(detail.file, detail.line);
  const badges: string[] = [];
  if (detail.is_async) badges.push("async");
  if (detail.is_abstract) badges.push("abstract");
  if (detail.is_static) badges.push("static");
  if (detail.is_override) badges.push("override");

  return (
    <>
      {/* AS-011 IDENTITY */}
      <div className="dp-section">
        <h3 className="dp-section-label">IDENTITY</h3>
        <div className="dp-row"><span className="dp-key">Name</span>{detail.qualified_name ?? detail.name}</div>
        <div className="dp-row"><span className="dp-key">Type</span>{detail.kind.toLowerCase()}</div>
        {detail.layer && (
          <div className="dp-row"><span className="dp-key">Layer</span>{detail.layer}</div>
        )}
        <div className="dp-row">
          <span className="dp-key">Location</span>
          {editor ? (
            <a href={editor}>{detail.file}:{detail.line}</a>
          ) : (
            <>{detail.file}:{detail.line}</>
          )}
        </div>
        {badges.length > 0 && (
          <div className="dp-badges">
            {badges.map((b) => (
              <span key={b} className="dp-badge">{b}</span>
            ))}
          </div>
        )}
      </div>

      {/* SIGNATURE */}
      <div className="dp-section">
        <h3 className="dp-section-label">SIGNATURE</h3>
        <div className="dp-sig">{detail.rendered_signature}</div>
        {detail.params === null || detail.params === undefined ? (
          <div className="dp-row"><span className="dp-key">Params</span><span className="dp-muted">—</span></div>
        ) : detail.params.length === 0 ? (
          <div className="dp-row"><span className="dp-key">Params</span><span className="dp-muted">none</span></div>
        ) : (
          <div className="dp-params">
            {detail.params.map((p, i) => (
              <div key={i} className="dp-param">
                <span className="dp-param-name">{p.name}</span>
                {p.type && <span className="dp-param-type">: {p.type}</span>}
                {p.default_value && <span className="dp-param-default"> = {p.default_value}</span>}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* AS-012 RELATIONSHIPS */}
      <div className="dp-section">
        <h3 className="dp-section-label">RELATIONSHIPS</h3>
        <div className="dp-row"><span className="dp-key">Calls</span>{detail.callee_count ?? 0}</div>
        <div className="dp-row"><span className="dp-key">Called by</span>{detail.caller_count ?? 0}</div>
        <div className="dp-row"><span className="dp-key">Importers</span>{detail.importer_count ?? 0}</div>
        <div className="dp-row"><span className="dp-key">Impact</span>{detail.impact_edge_count ?? 0} edges</div>
        <RelationSection title="Callers" page={callers} onSelect={onSelectSymbol} />
        <RelationSection title="Callees" page={callees} onSelect={onSelectSymbol} />
      </div>

      {/* AS-013 QUALITY */}
      <div className="dp-section">
        <h3 className="dp-section-label">QUALITY</h3>
        <div className="dp-row">
          <span className="dp-key">Coverage</span>
          <span className={`dp-badge ${detail.tested ? "ok" : "muted"}`}>
            {detail.tested ? "tested" : "chưa test"}
          </span>
        </div>
        {detail.loc != null && (
          <div className="dp-row"><span className="dp-key">LOC</span>{detail.loc}</div>
        )}
        <div className="dp-row">
          <span className="dp-key">Docstring</span>
          {detail.doc_summary ? (
            <span className="dp-doc">{detail.doc_summary}</span>
          ) : (
            <span className="dp-muted">none</span>
          )}
        </div>
        {detail.confidence != null && (
          <div className="dp-row"><span className="dp-key">Confidence</span>{detail.confidence.toFixed(2)}</div>
        )}
        {detail.is_dead_code && (
          <div className="dp-row"><span className="dp-badge warn">dead code</span></div>
        )}
        {detail.is_hub && (
          <div className="dp-row"><span className="dp-badge">hub</span></div>
        )}
      </div>
    </>
  );
}

function RelationSection({
  title,
  page,
  onSelect,
}: {
  title: string;
  page: RelationPage | null;
  onSelect: (id: string) => void;
}) {
  if (!page) return null;
  return (
    <div className="dp-section">
      <h3>
        {title}{" "}
        <span className="rel-count">
          ({page.total}
          {page.has_more ? `, showing ${page.entries.length}` : ""})
        </span>
      </h3>
      {page.entries.length === 0 ? (
        <div className="empty-dash">—</div>
      ) : (
        <div className="rel-list">
          {page.entries.map((e, idx) => {
            const isExternal = e.kind === "external" || e.file === "<external>";
            return (
              <div
                key={`${e.id}-${idx}`}
                className="rel-item"
                onClick={() => !isExternal && onSelect(e.name)}
                style={isExternal ? { cursor: "default", opacity: 0.7 } : undefined}
                title={
                  isExternal
                    ? "External call — target not defined in this project (library/builtin)"
                    : undefined
                }
              >
                <span>{e.name}</span>
                <span className="file">
                  {isExternal ? "library" : `${e.file}:${e.line}`}
                </span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
