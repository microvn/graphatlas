import { describe, expect, test } from "bun:test";
import { parseHashRoute, serializeRoute } from "../router";

describe("parseHashRoute", () => {
  test("empty hash → page=list", () => {
    expect(parseHashRoute("")).toEqual({ page: "list" });
    expect(parseHashRoute("#")).toEqual({ page: "list" });
  });

  test("project/:slug → page=detail, tab=graph default", () => {
    expect(parseHashRoute("#/project/abc123")).toEqual({
      page: "detail",
      slug: "abc123",
      tab: "graph",
    });
  });

  test("project/:slug/graph → graph tab explicit", () => {
    expect(parseHashRoute("#/project/abc/graph")).toEqual({
      page: "detail",
      slug: "abc",
      tab: "graph",
    });
  });

  test("project/:slug/index → Index Control tab", () => {
    expect(parseHashRoute("#/project/abc/index")).toEqual({
      page: "detail",
      slug: "abc",
      tab: "index",
    });
  });

  test("unknown tab → graceful fallback to default graph tab", () => {
    expect(parseHashRoute("#/project/abc/wat")).toEqual({
      page: "detail",
      slug: "abc",
      tab: "graph",
    });
  });

  test("slug accepts alphanumeric + hyphen + underscore", () => {
    expect(parseHashRoute("#/project/my_repo-v2")).toEqual({
      page: "detail",
      slug: "my_repo-v2",
      tab: "graph",
    });
  });

  test("trailing slash tolerated", () => {
    expect(parseHashRoute("#/project/abc/")).toEqual({
      page: "detail",
      slug: "abc",
      tab: "graph",
    });
  });

  test("garbage → list fallback", () => {
    expect(parseHashRoute("#/project/")).toEqual({ page: "list" });
    expect(parseHashRoute("#nope")).toEqual({ page: "list" });
    expect(parseHashRoute("#/project/abc/graph/extra")).toEqual({ page: "list" });
  });

  test("special chars in slug rejected (security guard)", () => {
    expect(parseHashRoute("#/project/abc!@#")).toEqual({ page: "list" });
    expect(parseHashRoute("#/project/abc%2Fdef")).toEqual({ page: "list" });
  });
});

describe("serializeRoute", () => {
  test("list → empty string", () => {
    expect(serializeRoute({ page: "list" })).toBe("");
  });

  test("detail default tab → omits tab", () => {
    expect(
      serializeRoute({ page: "detail", slug: "abc", tab: "graph" }),
    ).toBe("/project/abc");
  });

  test("detail non-default tab → included", () => {
    expect(
      serializeRoute({ page: "detail", slug: "abc", tab: "index" }),
    ).toBe("/project/abc/index");
  });

  test("round-trip stable", () => {
    const routes: { page: "detail"; slug: string; tab: "graph" | "index" }[] = [
      { page: "detail", slug: "abc", tab: "graph" },
      { page: "detail", slug: "xyz_repo-2", tab: "index" },
    ];
    for (const r of routes) {
      expect(parseHashRoute("#" + serializeRoute(r))).toEqual(r);
    }
  });
});
