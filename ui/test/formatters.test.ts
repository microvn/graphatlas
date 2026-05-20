import { describe, expect, test } from "bun:test";
import {
  filterProjects,
  formatBytes,
  formatCount,
  formatRelativeTime,
  indexStateBadgeClass,
  indexStateLabel,
  projectBasename,
  sortByLastIndexedDesc,
  watcherDotClass,
} from "../formatters";

describe("formatRelativeTime", () => {
  // 1715712000 = 2024-05-14 — well past 62-day deltas tested below.
  const NOW = 1_715_712_000;

  test("seconds bucket", () => {
    expect(formatRelativeTime(NOW - 45, NOW)).toBe("45s ago");
  });

  test("minutes bucket", () => {
    expect(formatRelativeTime(NOW - 5 * 60, NOW)).toBe("5m ago");
  });

  test("hours bucket", () => {
    expect(formatRelativeTime(NOW - 2 * 3600, NOW)).toBe("2h ago");
  });

  test("days bucket", () => {
    expect(formatRelativeTime(NOW - 62 * 86400, NOW)).toBe("62d ago");
  });

  test("zero / negative timestamp → em-dash", () => {
    expect(formatRelativeTime(0, NOW)).toBe("—");
    expect(formatRelativeTime(-1, NOW)).toBe("—");
  });

  test("future timestamp → 0s", () => {
    expect(formatRelativeTime(NOW + 100, NOW)).toBe("0s ago");
  });
});

describe("formatBytes", () => {
  test("GB", () => expect(formatBytes(1.5 * 1024 ** 3)).toBe("1.5 GB"));
  test("MB", () => expect(formatBytes(12 * 1024 ** 2)).toBe("12 MB"));
  test("KB", () => expect(formatBytes(4 * 1024)).toBe("4 KB"));
  test("bytes", () => expect(formatBytes(512)).toBe("512 B"));
  test("zero → em-dash", () => expect(formatBytes(0)).toBe("—"));
  test("NaN → em-dash", () => expect(formatBytes(Number.NaN)).toBe("—"));
});

describe("formatCount", () => {
  test("thousands separator", () => expect(formatCount(9876)).toBe("9,876"));
  test("null → em-dash", () => expect(formatCount(null)).toBe("—"));
  test("undefined → em-dash", () => expect(formatCount(undefined)).toBe("—"));
});

describe("indexStateBadgeClass + label", () => {
  test("fresh", () => {
    expect(indexStateBadgeClass("Fresh")).toBe("badge badge-fresh");
    expect(indexStateLabel("Fresh")).toBe("fresh");
  });
  test("orphan", () => {
    expect(indexStateBadgeClass("Orphan")).toBe("badge badge-orphan");
  });
  test("corrupt", () => {
    expect(indexStateBadgeClass("Corrupt")).toBe("badge badge-corrupt");
  });
});

describe("watcherDotClass", () => {
  test("running → on", () =>
    expect(watcherDotClass("Running")).toBe("dot dot-on"));
  test("stopped → off", () =>
    expect(watcherDotClass("Stopped")).toBe("dot dot-off"));
  test("errored → err", () =>
    expect(watcherDotClass("Errored")).toBe("dot dot-err"));
});

describe("projectBasename", () => {
  test("posix", () => expect(projectBasename("/Users/x/work/axum")).toBe("axum"));
  test("trailing slash", () =>
    expect(projectBasename("/Users/x/work/axum/")).toBe("axum"));
  test("single dir", () => expect(projectBasename("axum")).toBe("axum"));
});

describe("filterProjects", () => {
  const rows = [
    { name: "axum", repo_root: "/Users/h/work/axum" },
    { name: "django", repo_root: "/Users/h/work/django" },
    { name: "gin", repo_root: "/Users/h/work/gin" },
  ];

  test("empty query → all", () => {
    expect(filterProjects(rows, "")).toHaveLength(3);
    expect(filterProjects(rows, "   ")).toHaveLength(3);
  });

  test("matches name", () => {
    expect(filterProjects(rows, "axu")).toEqual([rows[0]]);
  });

  test("matches path", () => {
    expect(filterProjects(rows, "django")).toHaveLength(1);
  });

  test("case-insensitive", () => {
    expect(filterProjects(rows, "AXUM")).toHaveLength(1);
  });

  test("no match → empty", () => {
    expect(filterProjects(rows, "zzz")).toEqual([]);
  });
});

describe("sortByLastIndexedDesc", () => {
  test("most recent first", () => {
    const rows = [
      { last_indexed_unix: 100 },
      { last_indexed_unix: 300 },
      { last_indexed_unix: 200 },
    ];
    const sorted = sortByLastIndexedDesc(rows);
    expect(sorted.map((r) => r.last_indexed_unix)).toEqual([300, 200, 100]);
  });

  test("does not mutate input", () => {
    const rows = [{ last_indexed_unix: 100 }, { last_indexed_unix: 200 }];
    sortByLastIndexedDesc(rows);
    expect(rows[0]!.last_indexed_unix).toBe(100);
  });

  test("empty array", () => {
    expect(sortByLastIndexedDesc([])).toEqual([]);
  });
});
