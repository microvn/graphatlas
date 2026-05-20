/**
 * Spec C S-001 — URL hash router for Page 2 navigation.
 *
 * URL format: `#/project/<slug>[/<tab>]`
 *   - tab ∈ {graph, index} (default = graph per AS-001)
 *   - bare `#` or non-matching hash → page=list (Page 1)
 *
 * Kept pure so unit tests don't need a DOM. The component layer reads
 * `parseHashRoute(location.hash)` on mount + subscribes to hashchange.
 */

export type Route =
  | { page: "list" }
  | { page: "detail"; slug: string; tab: ProjectTab };

export type ProjectTab = "graph" | "index";

const DEFAULT_TAB: ProjectTab = "graph";

/** Parse `location.hash` (with or without leading `#`). */
export function parseHashRoute(hash: string): Route {
  // Strip leading `#` (location.hash includes it; tests may pass either).
  const h = hash.startsWith("#") ? hash.slice(1) : hash;
  const path = h.startsWith("/") ? h : `/${h}`;

  // /project/<slug>[/<tab>]
  const m = /^\/project\/([A-Za-z0-9_-]+)(?:\/([a-z-]+))?\/?$/.exec(path);
  if (!m) return { page: "list" };
  const slug = m[1]!;
  const tabRaw = m[2];
  const tab = isProjectTab(tabRaw) ? tabRaw : DEFAULT_TAB;
  return { page: "detail", slug, tab };
}

/** Inverse — serialize a Route back to the hash string (without leading `#`). */
export function serializeRoute(route: Route): string {
  if (route.page === "list") return "";
  // Omit tab when it's the default — keeps URLs short.
  if (route.tab === DEFAULT_TAB) return `/project/${route.slug}`;
  return `/project/${route.slug}/${route.tab}`;
}

function isProjectTab(s: string | undefined): s is ProjectTab {
  return s === "graph" || s === "index";
}
