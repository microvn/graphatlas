/**
 * Spec B B-C6 — fetcher pre-configured with X-GA-Token header.
 *
 * Token bootstrap (Spec D AS-001 / AS-009): `ga ui` opens the browser
 * at `http://host:port/#token=<hex>`. On first load we read the hash,
 * stash it in sessionStorage (so reload survives), strip from URL via
 * history.replaceState, and inject it on every request.
 */

import {
  type ApiError,
  type AddProjectMode,
  type AddProjectResponse,
  type DeleteIntentResponse,
  type FileSummaryDetail,
  type GraphResponse,
  type LayerSymbolsResponse,
  type LayersResponse,
  type ProjectRow,
  type RelationPage,
  type ReindexStartResponse,
  type ReindexStatus,
  type SymbolDetail,
  type SymbolSearchResponse,
  type WatcherSnapshot,
  BadPatternError,
  CacheBuildingError,
  CacheCorruptError,
  NetworkError,
  SessionExpiredError,
} from "./types";

const TOKEN_STORAGE_KEY = "ga_ui_token";
const TOKEN_HASH_PREFIX = "#token=";

/**
 * Read token from URL hash → store → strip. Idempotent — calling
 * twice yields the same stored token.
 *
 * Returns the token string, or `null` if neither URL nor storage has one.
 *
 * Exported as a pure function so tests can drive it via a fake Location/Storage.
 */
export function bootstrapTokenFromLocation(
  location: { hash: string; pathname: string; search: string },
  storage: Pick<Storage, "getItem" | "setItem">,
  replaceUrl?: (url: string) => void,
): string | null {
  const hash = location.hash;
  if (hash.startsWith(TOKEN_HASH_PREFIX)) {
    const token = hash.slice(TOKEN_HASH_PREFIX.length);
    if (token.length > 0) {
      storage.setItem(TOKEN_STORAGE_KEY, token);
      if (replaceUrl) {
        replaceUrl(`${location.pathname}${location.search}`);
      }
      return token;
    }
  }
  return storage.getItem(TOKEN_STORAGE_KEY);
}

/**
 * Detect the `X-GA-Corrupt-Count` response header. Phase 1 needs the
 * count to render the top banner per AS-007.
 */
export function readCorruptCount(headers: Headers): number {
  const raw = headers.get("x-ga-corrupt-count");
  if (!raw) return 0;
  const n = Number.parseInt(raw, 10);
  return Number.isFinite(n) && n > 0 ? n : 0;
}

/** Mirror server `cache_state::CacheState` for client-side rendering. */
export interface FetchProjectsResult {
  rows: ProjectRow[];
  corruptCount: number;
}

export class ApiClient {
  constructor(
    private readonly baseUrl = "",
    private readonly fetchImpl: typeof fetch = fetch.bind(globalThis),
  ) {}

  private async request(input: string, init?: RequestInit): Promise<Response> {
    let resp: Response;
    try {
      resp = await this.fetchImpl(`${this.baseUrl}${input}`, init);
    } catch (e) {
      throw new NetworkError(e);
    }
    if (resp.status === 401 || resp.status === 403) {
      // Origin/Host gate rejected the request — treat the same way the
      // token path used to: bubble up a SessionExpiredError so the UI
      // can show "restart ga ui". Realistically only happens when the
      // user opens a stale Origin (e.g. wrong port).
      throw new SessionExpiredError();
    }
    return resp;
  }

  async fetchProjects(): Promise<FetchProjectsResult> {
    const resp = await this.request("/api/projects");
    if (!resp.ok) {
      throw await asApiError(resp);
    }
    const rows = (await resp.json()) as ProjectRow[];
    return { rows, corruptCount: readCorruptCount(resp.headers) };
  }

  async addProject(
    path: string,
    mode: AddProjectMode,
  ): Promise<AddProjectResponse> {
    const resp = await this.request("/api/projects", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path, mode }),
    });
    if (!resp.ok) throw await asApiError(resp);
    return (await resp.json()) as AddProjectResponse;
  }

  async issueDeleteToken(slug: string): Promise<DeleteIntentResponse> {
    const resp = await this.request(
      `/api/projects/${encodeURIComponent(slug)}/delete-intent`,
      { method: "POST" },
    );
    if (!resp.ok) throw await asApiError(resp);
    return (await resp.json()) as DeleteIntentResponse;
  }

  async removeProject(slug: string, confirmToken: string): Promise<void> {
    const resp = await this.request(
      `/api/projects/${encodeURIComponent(slug)}?confirm=${encodeURIComponent(
        confirmToken,
      )}`,
      { method: "DELETE" },
    );
    if (resp.status === 204) return;
    if (!resp.ok) throw await asApiError(resp);
  }

  // ============== Spec C — Page 2 data + reindex + watcher ==============

  async graphDump(
    slug: string,
    focus?: string,
    hops: 1 | 2 = 2,
  ): Promise<{ graph: GraphResponse; stale: boolean }> {
    const params = new URLSearchParams();
    if (focus) params.set("focus", focus);
    params.set("hops", String(hops));
    const resp = await this.request(
      `/api/projects/${encodeURIComponent(slug)}/graph?${params}`,
    );
    await throwIfCacheBad(resp);
    if (!resp.ok) throw await asApiError(resp);
    const graph = (await resp.json()) as GraphResponse;
    return { graph, stale: readStale(resp.headers) };
  }

  async symbolDetail(
    slug: string,
    symbolId: string,
  ): Promise<{ detail: SymbolDetail; stale: boolean }> {
    const resp = await this.request(
      `/api/projects/${encodeURIComponent(slug)}/symbol/${encodeURIComponent(
        symbolId,
      )}`,
    );
    await throwIfCacheBad(resp);
    if (!resp.ok) throw await asApiError(resp);
    const detail = (await resp.json()) as SymbolDetail;
    return { detail, stale: readStale(resp.headers) };
  }

  async callers(
    slug: string,
    symbolId: string,
    offset = 0,
    limit = 50,
  ): Promise<RelationPage> {
    const resp = await this.request(
      `/api/projects/${encodeURIComponent(slug)}/symbol/${encodeURIComponent(
        symbolId,
      )}/callers?offset=${offset}&limit=${limit}`,
    );
    await throwIfCacheBad(resp);
    if (!resp.ok) throw await asApiError(resp);
    return (await resp.json()) as RelationPage;
  }

  async callees(
    slug: string,
    symbolId: string,
    offset = 0,
    limit = 50,
  ): Promise<RelationPage> {
    const resp = await this.request(
      `/api/projects/${encodeURIComponent(slug)}/symbol/${encodeURIComponent(
        symbolId,
      )}/callees?offset=${offset}&limit=${limit}`,
    );
    await throwIfCacheBad(resp);
    if (!resp.ok) throw await asApiError(resp);
    return (await resp.json()) as RelationPage;
  }

  async fileSummary(
    slug: string,
    path: string,
  ): Promise<FileSummaryDetail> {
    const resp = await this.request(
      `/api/projects/${encodeURIComponent(slug)}/file?path=${encodeURIComponent(
        path,
      )}`,
    );
    await throwIfCacheBad(resp);
    if (!resp.ok) throw await asApiError(resp);
    return (await resp.json()) as FileSummaryDetail;
  }

  async startReindex(slug: string): Promise<ReindexStartResponse> {
    const resp = await this.request(
      `/api/projects/${encodeURIComponent(slug)}/reindex`,
      { method: "POST" },
    );
    if (!resp.ok) throw await asApiError(resp);
    return (await resp.json()) as ReindexStartResponse;
  }

  async reindexStatus(slug: string, jobId: string): Promise<ReindexStatus> {
    const resp = await this.request(
      `/api/projects/${encodeURIComponent(slug)}/reindex/${encodeURIComponent(
        jobId,
      )}/status`,
    );
    if (!resp.ok) throw await asApiError(resp);
    return (await resp.json()) as ReindexStatus;
  }

  async cancelReindex(slug: string, jobId: string): Promise<void> {
    const resp = await this.request(
      `/api/projects/${encodeURIComponent(slug)}/reindex/${encodeURIComponent(
        jobId,
      )}`,
      { method: "DELETE" },
    );
    if (!resp.ok) throw await asApiError(resp);
  }

  async watcherStatus(slug: string): Promise<WatcherSnapshot> {
    const resp = await this.request(
      `/api/projects/${encodeURIComponent(slug)}/watcher`,
    );
    if (!resp.ok) throw await asApiError(resp);
    return (await resp.json()) as WatcherSnapshot;
  }

  // ============== Spec E — search / layers ==============

  async searchSymbols(
    slug: string,
    q: string,
    limit = 50,
  ): Promise<SymbolSearchResponse> {
    const resp = await this.request(
      `/api/projects/${encodeURIComponent(slug)}/symbols?q=${encodeURIComponent(
        q,
      )}&limit=${limit}`,
    );
    if (resp.status === 400) {
      // AS-003: pattern rejected by is_safe_ident.
      const body = (await resp.json().catch(() => ({}))) as ApiError;
      if (body.error === "bad_pattern") throw new BadPatternError();
    }
    await throwIfCacheBad(resp);
    if (!resp.ok) throw await asApiError(resp);
    return (await resp.json()) as SymbolSearchResponse;
  }

  async layers(slug: string): Promise<LayersResponse> {
    const resp = await this.request(
      `/api/projects/${encodeURIComponent(slug)}/layers`,
    );
    await throwIfCacheBad(resp);
    if (!resp.ok) throw await asApiError(resp);
    return (await resp.json()) as LayersResponse;
  }

  async layerSymbols(
    slug: string,
    layerName: string,
  ): Promise<LayerSymbolsResponse> {
    const resp = await this.request(
      `/api/projects/${encodeURIComponent(slug)}/layers/${encodeURIComponent(
        layerName,
      )}/symbols`,
    );
    await throwIfCacheBad(resp);
    if (!resp.ok) throw await asApiError(resp);
    return (await resp.json()) as LayerSymbolsResponse;
  }

  async watcherAction(
    slug: string,
    action: "start" | "stop",
  ): Promise<WatcherSnapshot> {
    const resp = await this.request(
      `/api/projects/${encodeURIComponent(slug)}/watcher`,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ action }),
      },
    );
    if (!resp.ok) throw await asApiError(resp);
    return (await resp.json()) as WatcherSnapshot;
  }
}

async function throwIfCacheBad(resp: Response): Promise<void> {
  if (resp.status !== 503) return;
  try {
    const body = (await resp.clone().json()) as ApiError;
    if (body.error === "cache_corrupt") throw new CacheCorruptError();
    if (body.error === "cache_building") throw new CacheBuildingError();
  } catch (e) {
    if (e instanceof CacheCorruptError || e instanceof CacheBuildingError) {
      throw e;
    }
    // fall-through: body wasn't JSON → asApiError handles
  }
}

function readStale(headers: Headers): boolean {
  return headers.get("x-ga-stale") === "reindex-in-progress";
}

async function asApiError(
  resp: Response,
): Promise<ApiError & Error & Record<string, unknown>> {
  let body: Record<string, unknown> = { error: "unknown", message: "" };
  try {
    body = (await resp.json()) as Record<string, unknown>;
  } catch {
    /* non-JSON; keep defaults */
  }
  const message = typeof body.message === "string" ? body.message : "";
  const code = typeof body.error === "string" ? body.error : "unknown";
  const err = new Error(message || code) as Error &
    ApiError &
    Record<string, unknown>;
  err.error = code;
  err.message = message;
  // Allowlist forward — only fields known callers consume.
  // Avoids blind `Object.assign` of every server-sent property onto an
  // Error object (lower prototype-pollution surface, even if modern JS
  // makes own-prop assignment safe).
  for (const key of ["job_id", "slug", "confirm_token", "expires_in_secs"]) {
    if (key in body) err[key] = body[key];
  }
  return err;
}
