// Spec E S-001 + S-002 — left sidebar: global symbol search,
// layer chip filter, expand-on-demand symbol tree grouped by layer.

import { useCallback, useEffect, useRef, useState } from "react";
import type { ApiClient } from "../api";
import {
  BadPatternError,
  type LayerEntry,
  type SymbolHit,
} from "../types";
import { kindColor } from "../kind-colors";

type LayerExpansion = {
  symbols: SymbolHit[];
  symbol_ids: string[];
};

export function GraphSidebar({
  api,
  slug,
  activeLayer,
  onActiveLayerChange,
  onSelectSymbol,
}: {
  api: ApiClient;
  slug: string;
  activeLayer: string | null;
  onActiveLayerChange: (layer: string | null, memberIds: Set<string> | null) => void;
  onSelectSymbol: (name: string) => void;
}) {
  // ============== Search ==============
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SymbolHit[] | null>(null);
  const [searchErr, setSearchErr] = useState<string | null>(null);
  const [searchTruncated, setSearchTruncated] = useState(false);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    if (!query) {
      setHits(null);
      setSearchErr(null);
      setSearchTruncated(false);
      return;
    }
    // C-2: 250ms debounce.
    debounceRef.current = setTimeout(() => {
      api
        .searchSymbols(slug, query, 50)
        .then((r) => {
          setHits(r.hits);
          setSearchTruncated(r.truncated);
          setSearchErr(null);
        })
        .catch((e) => {
          setHits([]);
          setSearchTruncated(false);
          if (e instanceof BadPatternError) {
            setSearchErr("Ký tự không hợp lệ trong tìm kiếm");
          } else {
            setSearchErr((e as Error).message);
          }
        });
    }, 250);
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
    };
  }, [api, slug, query]);

  // ============== Layers ==============
  const [layers, setLayers] = useState<LayerEntry[] | null>(null);
  const [layersDegraded, setLayersDegraded] = useState(false);
  const [expanded, setExpanded] = useState<Map<string, LayerExpansion>>(
    new Map(),
  );

  useEffect(() => {
    api
      .layers(slug)
      .then((r) => {
        setLayers(r.layers);
        setLayersDegraded(r.degraded);
      })
      .catch(() => {
        setLayers([]);
        setLayersDegraded(true);
      });
  }, [api, slug]);

  const ensureLayerLoaded = useCallback(
    async (name: string): Promise<LayerExpansion | null> => {
      const cached = expanded.get(name);
      if (cached) return cached;
      try {
        const r = await api.layerSymbols(slug, name);
        setExpanded((prev) => {
          const next = new Map(prev);
          next.set(name, { symbols: r.symbols, symbol_ids: r.symbol_ids });
          return next;
        });
        return { symbols: r.symbols, symbol_ids: r.symbol_ids };
      } catch {
        return null;
      }
    },
    [api, slug, expanded],
  );

  // AS-008 expand-on-demand tree.
  const [open, setOpen] = useState<Set<string>>(new Set());
  const toggleOpen = useCallback(
    async (name: string) => {
      if (open.has(name)) {
        const next = new Set(open);
        next.delete(name);
        setOpen(next);
        return;
      }
      await ensureLayerLoaded(name);
      const next = new Set(open);
      next.add(name);
      setOpen(next);
    },
    [open, ensureLayerLoaded],
  );

  // AS-009 chip click → load + bubble membership.
  const toggleChip = useCallback(
    async (name: string) => {
      if (activeLayer === name) {
        onActiveLayerChange(null, null);
        return;
      }
      const data = await ensureLayerLoaded(name);
      onActiveLayerChange(name, data ? new Set(data.symbol_ids) : null);
    },
    [activeLayer, ensureLayerLoaded, onActiveLayerChange],
  );

  // ============== Render ==============
  return (
    <aside className="graph-sidebar">
      <div className="gs-search">
        <input
          className="gs-search-input"
          placeholder="Tìm symbol…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          spellCheck={false}
        />
        {searchErr && <div className="gs-search-err">{searchErr}</div>}
        {hits !== null && (
          <div className="gs-search-dropdown">
            {hits.length === 0 ? (
              <div className="gs-empty">Không tìm thấy symbol khớp</div>
            ) : (
              <>
                {hits.map((h) => (
                  <div
                    key={h.id}
                    className="gs-hit"
                    onClick={() => {
                      onSelectSymbol(h.name);
                      setQuery("");
                      setHits(null);
                    }}
                  >
                    <span className="gs-hit-name">{h.name}</span>
                    <span className="gs-hit-meta">
                      {h.kind} · {h.file}:{h.line}
                    </span>
                  </div>
                ))}
                {searchTruncated && (
                  <div className="gs-truncated">
                    + thêm nữa — gõ thêm để lọc
                  </div>
                )}
              </>
            )}
          </div>
        )}
      </div>

      {!layersDegraded && layers && layers.length > 0 && (
        <div className="gs-chips">
          {layers.map((l) => (
            <button
              key={l.name}
              className={`gs-chip${activeLayer === l.name ? " active" : ""}`}
              onClick={() => toggleChip(l.name)}
              title={`${l.symbol_count} symbols`}
            >
              {l.name} <span className="gs-chip-count">{l.symbol_count}</span>
            </button>
          ))}
        </div>
      )}

      <div className="gs-tree">
        {layersDegraded ? (
          <div className="gs-empty">— architecture không khả dụng</div>
        ) : (
          (layers ?? []).map((l) => {
            const isOpen = open.has(l.name);
            const exp = expanded.get(l.name);
            return (
              <div key={l.name} className="gs-tree-group">
                <div
                  className="gs-tree-header"
                  onClick={() => toggleOpen(l.name)}
                >
                  <span className="gs-tree-arrow">{isOpen ? "▾" : "▸"}</span>
                  <span className="gs-tree-name">{l.name}</span>
                  <span className="gs-tree-count">{l.symbol_count}</span>
                </div>
                {isOpen && exp && (
                  <div className="gs-tree-children">
                    {exp.symbols.slice(0, 200).map((s) => (
                      <div
                        key={s.id}
                        className="gs-tree-item"
                        onClick={() => onSelectSymbol(s.name)}
                        title={s.kind}
                      >
                        <span
                          className="gs-tree-item-dot"
                          style={{ background: kindColor(s.kind) }}
                          aria-label={s.kind}
                        />
                        <span className="gs-tree-item-name">{s.name}</span>
                      </div>
                    ))}
                    {exp.symbols.length > 200 && (
                      <div className="gs-tree-more">
                        + {exp.symbols.length - 200} more — narrow with search
                      </div>
                    )}
                  </div>
                )}
              </div>
            );
          })
        )}
      </div>
    </aside>
  );
}
