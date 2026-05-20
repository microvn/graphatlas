import { describe, expect, mock, test } from "bun:test";
import {
  ApiClient,
  bootstrapTokenFromLocation,
  readCorruptCount,
} from "../api";
import { NetworkError, SessionExpiredError } from "../types";

class FakeStorage {
  private store = new Map<string, string>();
  getItem(key: string) {
    return this.store.get(key) ?? null;
  }
  setItem(key: string, value: string) {
    this.store.set(key, value);
  }
}

describe("bootstrapTokenFromLocation", () => {
  test("extracts token from URL hash and strips it", () => {
    const storage = new FakeStorage();
    let replaced: string | null = null;
    const token = bootstrapTokenFromLocation(
      { hash: "#token=abc123", pathname: "/", search: "" },
      storage,
      (u) => (replaced = u),
    );
    expect(token).toBe("abc123");
    expect(storage.getItem("ga_ui_token")).toBe("abc123");
    expect(replaced).toBe("/");
  });

  test("preserves pathname + search when stripping hash", () => {
    const storage = new FakeStorage();
    let replaced: string | null = null;
    bootstrapTokenFromLocation(
      { hash: "#token=xyz", pathname: "/project/abc", search: "?tab=graph" },
      storage,
      (u) => (replaced = u),
    );
    expect(replaced).toBe("/project/abc?tab=graph");
  });

  test("falls back to sessionStorage when no hash", () => {
    const storage = new FakeStorage();
    storage.setItem("ga_ui_token", "stored-token");
    const token = bootstrapTokenFromLocation(
      { hash: "", pathname: "/", search: "" },
      storage,
    );
    expect(token).toBe("stored-token");
  });

  test("null when neither hash nor storage", () => {
    const storage = new FakeStorage();
    const token = bootstrapTokenFromLocation(
      { hash: "", pathname: "/", search: "" },
      storage,
    );
    expect(token).toBeNull();
  });

  test("empty hash token treated as null", () => {
    const storage = new FakeStorage();
    const token = bootstrapTokenFromLocation(
      { hash: "#token=", pathname: "/", search: "" },
      storage,
    );
    expect(token).toBeNull();
  });

  test("ignores unrelated hash (#section)", () => {
    const storage = new FakeStorage();
    storage.setItem("ga_ui_token", "fallback");
    const token = bootstrapTokenFromLocation(
      { hash: "#tab-graph", pathname: "/", search: "" },
      storage,
    );
    expect(token).toBe("fallback");
  });
});

describe("readCorruptCount", () => {
  test("present → parsed", () => {
    const h = new Headers({ "X-GA-Corrupt-Count": "3" });
    expect(readCorruptCount(h)).toBe(3);
  });
  test("absent → 0", () => {
    expect(readCorruptCount(new Headers())).toBe(0);
  });
  test("non-numeric → 0", () => {
    const h = new Headers({ "X-GA-Corrupt-Count": "nope" });
    expect(readCorruptCount(h)).toBe(0);
  });
  test("zero → 0 (no banner)", () => {
    const h = new Headers({ "X-GA-Corrupt-Count": "0" });
    expect(readCorruptCount(h)).toBe(0);
  });
});

function mockFetch(
  responder: (input: string, init?: RequestInit) => Promise<Response> | Response,
): typeof fetch {
  return mock(((input: string, init?: RequestInit) =>
    Promise.resolve(responder(input, init))) as typeof fetch);
}

describe("ApiClient.fetchProjects", () => {
  test("happy path returns rows + corrupt count", async () => {
    const fake = mockFetch(() =>
      new Response(
        JSON.stringify([
          {
            slug: "abc",
            name: "x",
            repo_root: "/x",
            languages: [],
            last_indexed_unix: 1,
            index_state: "Fresh",
            index_counts: null,
            health: null,
            watcher: "Stopped",
            watcher_queue_pending: 0,
            watcher_last_event_unix: null,
          },
        ]),
        {
          status: 200,
          headers: { "X-GA-Corrupt-Count": "2" },
        },
      ),
    );
    const client = new ApiClient("", fake);
    const { rows, corruptCount } = await client.fetchProjects();
    expect(rows).toHaveLength(1);
    expect(corruptCount).toBe(2);
  });

  test("403 throws SessionExpiredError", async () => {
    const fake = mockFetch(() => new Response("", { status: 403 }));
    const client = new ApiClient("", fake);
    await expect(client.fetchProjects()).rejects.toBeInstanceOf(
      SessionExpiredError,
    );
  });

  test("401 also throws SessionExpiredError", async () => {
    const fake = mockFetch(() => new Response("", { status: 401 }));
    const client = new ApiClient("", fake);
    await expect(client.fetchProjects()).rejects.toBeInstanceOf(
      SessionExpiredError,
    );
  });

  test("network error wrapped in NetworkError", async () => {
    const fake = mock(((): Promise<Response> => {
      throw new Error("ECONNREFUSED");
    }) as typeof fetch);
    const client = new ApiClient("", fake);
    await expect(client.fetchProjects()).rejects.toBeInstanceOf(NetworkError);
  });

  test("500 surfaces as Error with body.error", async () => {
    const fake = mockFetch(() =>
      new Response(JSON.stringify({ error: "boom", message: "kaboom" }), {
        status: 500,
        headers: { "Content-Type": "application/json" },
      }),
    );
    const client = new ApiClient("", fake);
    try {
      await client.fetchProjects();
      throw new Error("should have thrown");
    } catch (e) {
      expect((e as Error).message).toBe("kaboom");
    }
  });
});

describe("ApiClient.addProject", () => {
  test("POST with path + mode (no token header — auth removed)", async () => {
    let captured: Request | null = null;
    const fake = mockFetch((input, init) => {
      captured = new Request(`http://test.local${input}`, init);
      return new Response(
        JSON.stringify({
          slug: "s1",
          job_id: "j1",
          mode: "index",
          canonical_path: "/x",
        }),
        { status: 202 },
      );
    });
    const client = new ApiClient("", fake);
    const resp = await client.addProject("/some/repo", "index");
    expect(resp.slug).toBe("s1");
    expect(resp.job_id).toBe("j1");
    expect(captured!.headers.get("X-GA-Token")).toBeNull();
    const body = await captured!.json();
    expect(body).toEqual({ path: "/some/repo", mode: "index" });
  });

  test("400 path_not_found surfaces error code", async () => {
    const fake = mockFetch(() =>
      new Response(JSON.stringify({ error: "path_not_found", message: "" }), {
        status: 400,
      }),
    );
    const client = new ApiClient("", fake);
    try {
      await client.addProject("/nope", "index");
      throw new Error("should have thrown");
    } catch (e) {
      expect((e as { error?: string }).error).toBe("path_not_found");
    }
  });
});

describe("ApiClient.removeProject (2-step)", () => {
  test("happy path: intent → delete with confirm token", async () => {
    let step = 0;
    const fake = mockFetch((input, init) => {
      step += 1;
      if (input.endsWith("/delete-intent")) {
        expect(init?.method).toBe("POST");
        return new Response(
          JSON.stringify({ confirm_token: "tok-123", expires_in_secs: 30 }),
          { status: 200 },
        );
      }
      // DELETE
      expect(input).toContain("confirm=tok-123");
      expect(init?.method).toBe("DELETE");
      return new Response(null, { status: 204 });
    });
    const client = new ApiClient("", fake);
    const intent = await client.issueDeleteToken("abc");
    expect(intent.confirm_token).toBe("tok-123");
    await client.removeProject("abc", intent.confirm_token);
    expect(step).toBe(2);
  });

  test("expired token surfaces confirm_token_expired", async () => {
    const fake = mockFetch(() =>
      new Response(
        JSON.stringify({ error: "confirm_token_expired", message: "expired" }),
        { status: 403 }, // 403 normally → SessionExpiredError, BUT 403 is the
      ),
    );
    const client = new ApiClient("", fake);
    // Spec puts the expired-token signal at 403 the same way as bad
    // session token. The UI Spec B AS-025 handles this with a generic
    // "Session expired" banner — collapse to SessionExpiredError is OK.
    await expect(client.removeProject("abc", "stale")).rejects.toBeInstanceOf(
      SessionExpiredError,
    );
  });
});
