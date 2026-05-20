import { describe, expect, mock, test } from "bun:test";
import { ApiClient } from "../api";
import { CacheBuildingError, CacheCorruptError } from "../types";

function mockFetch(
  responder: (input: string, init?: RequestInit) => Response | Promise<Response>,
): typeof fetch {
  return mock(((input: string, init?: RequestInit) =>
    Promise.resolve(responder(input, init))) as typeof fetch);
}

describe("ApiClient.graphDump", () => {
  test("happy path with focus + hops", async () => {
    let capturedUrl = "";
    const fake = mockFetch((url) => {
      capturedUrl = url;
      return new Response(
        JSON.stringify({
          nodes: [
            { id: "s1", name: "alpha", kind: "Function", file: "x", line: 1, line_end: 10, degree: 1 },
          ],
          edges: [],
          truncated: false,
          total_node_count: 1,
        }),
        { status: 200 },
      );
    });
    const c = new ApiClient("", fake);
    const { graph, stale } = await c.graphDump("slug-1", "sym_42", 2);
    expect(graph.nodes.length).toBe(1);
    expect(stale).toBe(false);
    expect(capturedUrl).toContain("focus=sym_42");
    expect(capturedUrl).toContain("hops=2");
  });

  test("X-GA-Stale header surfaces", async () => {
    const fake = mockFetch(() =>
      new Response(
        JSON.stringify({ nodes: [], edges: [], truncated: false, total_node_count: 0 }),
        { status: 200, headers: { "X-GA-Stale": "reindex-in-progress" } },
      ),
    );
    const c = new ApiClient("", fake);
    const { stale } = await c.graphDump("s");
    expect(stale).toBe(true);
  });

  test("503 cache_corrupt → CacheCorruptError", async () => {
    const fake = mockFetch(() =>
      new Response(JSON.stringify({ error: "cache_corrupt", message: "" }), {
        status: 503,
      }),
    );
    const c = new ApiClient("", fake);
    await expect(c.graphDump("s")).rejects.toBeInstanceOf(CacheCorruptError);
  });

  test("503 cache_building → CacheBuildingError", async () => {
    const fake = mockFetch(() =>
      new Response(JSON.stringify({ error: "cache_building", message: "" }), {
        status: 503,
      }),
    );
    const c = new ApiClient("", fake);
    await expect(c.graphDump("s")).rejects.toBeInstanceOf(CacheBuildingError);
  });
});

describe("ApiClient.symbolDetail + callers + callees + fileSummary", () => {
  test("symbolDetail returns rendered_signature", async () => {
    const fake = mockFetch(() =>
      new Response(
        JSON.stringify({
          id: "s1",
          name: "alpha",
          kind: "Function",
          file: "x",
          line: 1,
          rendered_signature: "alpha() -> &str",
        }),
        { status: 200 },
      ),
    );
    const c = new ApiClient("", fake);
    const { detail } = await c.symbolDetail("slug", "alpha");
    expect(detail.rendered_signature).toBe("alpha() -> &str");
  });

  test("callers pagination URL", async () => {
    let capturedUrl = "";
    const fake = mockFetch((url) => {
      capturedUrl = url;
      return new Response(
        JSON.stringify({ entries: [], total: 0, has_more: false, offset: 50, limit: 50 }),
        { status: 200 },
      );
    });
    const c = new ApiClient("", fake);
    await c.callers("slug", "sym", 50, 50);
    expect(capturedUrl).toContain("offset=50");
    expect(capturedUrl).toContain("limit=50");
  });

  test("fileSummary 404 → FileNotFound error code", async () => {
    const fake = mockFetch(() =>
      new Response(JSON.stringify({ error: "file_not_found", message: "" }), {
        status: 404,
      }),
    );
    const c = new ApiClient("", fake);
    try {
      await c.fileSummary("slug", "ghost.rs");
      throw new Error("should throw");
    } catch (e) {
      expect((e as { error?: string }).error).toBe("file_not_found");
    }
  });
});

describe("ApiClient.reindex* + watcher*", () => {
  test("startReindex POST returns job_id", async () => {
    let captured: { method?: string } = {};
    const fake = mockFetch((_url, init) => {
      captured.method = init?.method;
      return new Response(JSON.stringify({ slug: "s", job_id: "j1" }), {
        status: 202,
      });
    });
    const c = new ApiClient("", fake);
    const r = await c.startReindex("s");
    expect(captured.method).toBe("POST");
    expect(r.job_id).toBe("j1");
  });

  test("reindexStatus shape", async () => {
    const fake = mockFetch(() =>
      new Response(
        JSON.stringify({
          job_id: "j1",
          slug: "s",
          state: "Running",
          percent: 42.5,
          current_file: "src/x.rs",
          files_done: 21,
          files_total: 50,
          duration_ms: 1234,
          error: null,
          log_tail: ["[ok] start"],
        }),
        { status: 200 },
      ),
    );
    const c = new ApiClient("", fake);
    const s = await c.reindexStatus("s", "j1");
    expect(s.state).toBe("Running");
    expect(s.percent).toBe(42.5);
    expect(s.log_tail).toEqual(["[ok] start"]);
  });

  test("startReindex 409 propagates job_id on thrown error", async () => {
    // Regression: asApiError previously stripped extra fields, so the
    // component's "reindex_in_progress" branch silently no-op'd because
    // it couldn't resume polling the existing job_id.
    const fake = mockFetch(() =>
      new Response(
        JSON.stringify({
          error: "reindex_in_progress",
          slug: "s",
          job_id: "existing-job",
        }),
        { status: 409 },
      ),
    );
    const c = new ApiClient("", fake);
    let caught: any = null;
    try {
      await c.startReindex("s");
    } catch (e) {
      caught = e;
    }
    expect(caught?.error).toBe("reindex_in_progress");
    expect(caught?.job_id).toBe("existing-job");
  });

  test("cancelReindex DELETE", async () => {
    let captured: { method?: string } = {};
    const fake = mockFetch((_url, init) => {
      captured.method = init?.method;
      return new Response(
        JSON.stringify({ job_id: "j1", state: "Cancelled" }),
        { status: 202 },
      );
    });
    const c = new ApiClient("", fake);
    await c.cancelReindex("s", "j1");
    expect(captured.method).toBe("DELETE");
  });

  test("watcherAction POST start", async () => {
    let body: string | undefined;
    const fake = mockFetch((_url, init) => {
      body = init?.body as string;
      return new Response(
        JSON.stringify({
          slug: "s",
          status: "Running",
          mode: "inotify",
          queue_pending: 0,
          dirty_flag: false,
          last_event_unix: null,
          error: null,
        }),
        { status: 200 },
      );
    });
    const c = new ApiClient("", fake);
    const r = await c.watcherAction("s", "start");
    expect(JSON.parse(body!)).toEqual({ action: "start" });
    expect(r.status).toBe("Running");
  });
});
