import { describe, expect, mock, test } from "bun:test";
import { ApiClient } from "../api";
import { BadPatternError } from "../types";

function mockFetch(
  responder: (input: string, init?: RequestInit) => Promise<Response> | Response,
): typeof fetch {
  return mock(((input: string, init?: RequestInit) =>
    Promise.resolve(responder(input, init))) as typeof fetch);
}

describe("ApiClient.searchSymbols (Spec E S-001)", () => {
  test("AS-001 returns hits + truncated", async () => {
    const fake = mockFetch(() =>
      new Response(
        JSON.stringify({
          hits: [
            {
              id: "f.py::OnConnect:1",
              name: "OnConnect",
              kind: "function",
              file: "f.py",
              line: 1,
              layer: "core",
            },
          ],
          truncated: false,
        }),
        { status: 200 },
      ),
    );
    const c = new ApiClient("", fake);
    const r = await c.searchSymbols("slug", "connect");
    expect(r.hits).toHaveLength(1);
    expect(r.hits[0]!.name).toBe("OnConnect");
    expect(r.truncated).toBe(false);
  });

  test("AS-003 400 bad_pattern throws BadPatternError", async () => {
    const fake = mockFetch(() =>
      new Response(
        JSON.stringify({ error: "bad_pattern", message: "nope" }),
        { status: 400, headers: { "Content-Type": "application/json" } },
      ),
    );
    const c = new ApiClient("", fake);
    await expect(c.searchSymbols("slug", "foo()")).rejects.toBeInstanceOf(
      BadPatternError,
    );
  });

  test("URL encodes pattern and slug", async () => {
    let captured = "";
    const fake = mockFetch((url) => {
      captured = url;
      return new Response(
        JSON.stringify({ hits: [], truncated: false }),
        { status: 200 },
      );
    });
    const c = new ApiClient("", fake);
    await c.searchSymbols("my slug", "foo_bar");
    expect(captured).toContain("/api/projects/my%20slug/symbols");
    expect(captured).toContain("q=foo_bar");
    expect(captured).toContain("limit=50");
  });

  test("respects custom limit param", async () => {
    let captured = "";
    const fake = mockFetch((url) => {
      captured = url;
      return new Response(
        JSON.stringify({ hits: [], truncated: false }),
        { status: 200 },
      );
    });
    const c = new ApiClient("", fake);
    await c.searchSymbols("s", "x", 25);
    expect(captured).toContain("limit=25");
  });
});

describe("ApiClient.layers (Spec E S-002 AS-006/AS-007)", () => {
  test("AS-006 happy path returns sorted layers", async () => {
    const fake = mockFetch(() =>
      new Response(
        JSON.stringify({
          layers: [
            { name: "ga-query", symbol_count: 800 },
            { name: "ga-index", symbol_count: 600 },
          ],
          degraded: false,
        }),
        { status: 200 },
      ),
    );
    const c = new ApiClient("", fake);
    const r = await c.layers("slug");
    expect(r.layers).toHaveLength(2);
    expect(r.layers[0]!.symbol_count).toBe(800);
    expect(r.degraded).toBe(false);
  });

  test("AS-007 degrade returns empty + degraded:true", async () => {
    const fake = mockFetch(() =>
      new Response(
        JSON.stringify({ layers: [], degraded: true }),
        { status: 200 },
      ),
    );
    const c = new ApiClient("", fake);
    const r = await c.layers("slug");
    expect(r.layers).toHaveLength(0);
    expect(r.degraded).toBe(true);
  });
});

describe("ApiClient.layerSymbols (Spec E S-002 AS-008)", () => {
  test("returns symbols + symbol_ids", async () => {
    const fake = mockFetch(() =>
      new Response(
        JSON.stringify({
          symbols: [
            {
              id: "f.rs::symbols:21",
              name: "symbols",
              kind: "function",
              file: "f.rs",
              line: 21,
              layer: "ga-query",
            },
          ],
          symbol_ids: ["f.rs::symbols:21"],
        }),
        { status: 200 },
      ),
    );
    const c = new ApiClient("", fake);
    const r = await c.layerSymbols("slug", "ga-query");
    expect(r.symbols).toHaveLength(1);
    expect(r.symbol_ids).toEqual(["f.rs::symbols:21"]);
  });

  test("URL encodes layer name with hyphen", async () => {
    let captured = "";
    const fake = mockFetch((url) => {
      captured = url;
      return new Response(
        JSON.stringify({ symbols: [], symbol_ids: [] }),
        { status: 200 },
      );
    });
    const c = new ApiClient("", fake);
    await c.layerSymbols("slug", "ga-query");
    expect(captured).toContain("/layers/ga-query/symbols");
  });

  test("404 unknown layer surfaces as Error", async () => {
    const fake = mockFetch(() =>
      new Response(
        JSON.stringify({ error: "layer_not_found", message: "nope" }),
        { status: 404, headers: { "Content-Type": "application/json" } },
      ),
    );
    const c = new ApiClient("", fake);
    await expect(c.layerSymbols("slug", "ghost")).rejects.toThrow();
  });
});
