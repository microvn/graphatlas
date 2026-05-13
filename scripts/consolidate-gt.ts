#!/usr/bin/env bun
// Phase A3 — Consolidate 100 tasks → benches/uc-impact/ground-truth.json
//                 + SHA256 manifest + METHODOLOGY.md
//
// Takes first 20 tasks per repo (5 repos × 20 = 100), adds stratified
// 30/70 dev/test split (6/14 per repo), emits git-committable artifacts.

import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const RAW_DIR = "benches/uc-impact/raw";
const OUT_DIR = "benches/uc-impact";
// v1.1-M4 — mockito added per Lang-C1 Java contract closure.
// S-002-bench — kotlinx-coroutines + kotlinx-serialization added per Lang-C1
// Kotlin contract closure (AS-007 + AS-008 + Lang-C7 split across 2 fixtures
// per docs/guide/dataset-for-new-language.md §0.6 paradigm matrix coverage).
// S-003-bench — MQTTnet added per Lang-C1 C# contract closure (single
// fixture covers all C# atomic UC gates; Lang-C7 PARTIAL exercised lightly
// per language idiom — most C# OSS uses constructor-injection over
// field-attribute DI).
const REPOS = [
  "django", "preact", "nest", "gin", "axum", "regex", "tokio", "mockito",
  "kotlinx-coroutines", "kotlinx-serialization",
  "MQTTnet", "Polly",
  "jekyll", "faraday",
] as const;
const TASKS_PER_REPO = 20;
const DEV_PER_REPO = 6; // 30% → 6/20
// test = 14/20 (70%)

interface RawTask {
  task_id: string;
  repo: string;
  base_commit: string;
  fix_commit: string;
  subject: string;
  seed_file: string;
  seed_symbol: string;
  source_files: string[];
  expected_files: string[];
  expected_tests: string[];
  should_touch_files: string[]; // structural blast radius (schema v3)
  max_expected_depth: number;
}

interface GtTask extends RawTask {
  lang: string;
  split: "dev" | "test";
}

const LANG_BY_REPO: Record<string, string> = {
  django: "python",
  preact: "javascript",
  nest: "typescript",
  gin: "go",
  axum: "rust",
  regex: "rust",
  tokio: "rust",
  mockito: "java",
  "kotlinx-coroutines": "kotlin",
  "kotlinx-serialization": "kotlin",
  MQTTnet: "csharp",
  Polly: "csharp",
  jekyll: "ruby",
  faraday: "ruby",
};

// Deterministic stratified split — use task_id lex sort to pick dev vs test.
// 2026-04-28 (Story C v3): when a repo has fewer than TASKS_PER_REPO tasks
// (Story C tightened mining filter dropped some repos below 20), use a
// 30% ratio split instead of fixed 6 dev slots. Otherwise small repos
// (e.g. axum with 6 fix-subject commits after Story C v3) had ALL tasks
// land in dev and zero in test → bench couldn't measure them.
function assignSplits(tasks: RawTask[]): GtTask[] {
  const sorted = [...tasks].sort((a, b) => a.task_id.localeCompare(b.task_id));
  const devCount =
    sorted.length >= TASKS_PER_REPO
      ? DEV_PER_REPO
      : Math.max(1, Math.floor(sorted.length * 0.3));
  return sorted.map((t, i) => ({
    ...t,
    lang: LANG_BY_REPO[t.repo],
    split: i < devCount ? "dev" : "test",
  }));
}

/** Normalize subject for cherry-pick detection: strip branch tags ([4.2.x])
 * and trailing PR numbers (#4535). Same bug across maintenance branches will
 * then collapse to one group. */
function normalizeSubject(subject: string): string {
  return subject
    .replace(/^\[[\d.x\s]+\]\s*/i, "")
    .replace(/\s*\(#\d+\)\s*$/, "")
    .trim()
    .toLowerCase();
}

/** Dedupe cherry-picks by normalized subject. Keeps the most recent commit
 * (highest SHA lex — proxy for newer since git SHAs are random but recent
 * commits tend to reach first in our miner). Returns deduped + reject count. */
function dedupeCherryPicks(tasks: RawTask[]): { kept: RawTask[]; dropped: number } {
  const bySubj = new Map<string, RawTask>();
  for (const t of tasks) {
    const key = `${t.repo}:${normalizeSubject(t.subject)}`;
    const existing = bySubj.get(key);
    if (!existing || t.fix_commit > existing.fix_commit) {
      bySubj.set(key, t);
    }
  }
  return { kept: [...bySubj.values()], dropped: tasks.length - bySubj.size };
}

const allTasks: GtTask[] = [];
for (const repo of REPOS) {
  const raw = JSON.parse(
    readFileSync(join(RAW_DIR, `${repo}-tasks.json`), "utf8"),
  ) as { tasks: RawTask[] };
  const { kept, dropped } = dedupeCherryPicks(raw.tasks);
  if (dropped > 0) {
    console.log(`  [${repo}] deduped ${dropped} cherry-pick(s) — ${kept.length} unique`);
  }
  const trimmed = kept.slice(0, TASKS_PER_REPO);
  if (trimmed.length < TASKS_PER_REPO) {
    console.warn(
      `  WARN: ${repo} only has ${trimmed.length}/${TASKS_PER_REPO} unique tasks after dedupe`,
    );
  }
  allTasks.push(...assignSplits(trimmed));
}

const manifest = {
  schema_version: 3,
  source: `git-mining-${new Date().toISOString().slice(0, 10)}`,
  uc: "impact",
  spec: "S-004 AS-009",
  mining_tool: "scripts/mine-fix-commits.ts + scripts/extract-seeds.ts + should_touch_files-v3",
  total_tasks: allTasks.length,
  per_repo: Object.fromEntries(
    REPOS.map((r) => [r, allTasks.filter((t) => t.repo === r).length]),
  ),
  per_lang: Object.fromEntries(
    [...new Set(allTasks.map((t) => t.lang))].map((l) => [
      l,
      allTasks.filter((t) => t.lang === l).length,
    ]),
  ),
  splits: {
    dev: allTasks.filter((t) => t.split === "dev").length,
    test: allTasks.filter((t) => t.split === "test").length,
  },
  tasks: allTasks,
};

const outPath = join(OUT_DIR, "ground-truth.json");
const json = JSON.stringify(manifest, null, 2);
writeFileSync(outPath, json);

// SHA256 manifest
const hash = createHash("sha256").update(json).digest("hex");
const shaPath = join(OUT_DIR, "ground-truth.sha256");
writeFileSync(shaPath, `${hash}  ground-truth.json\n`);

console.log(`Wrote ${outPath}`);
console.log(`Wrote ${shaPath}: ${hash.slice(0, 16)}...`);
console.log(`\nSummary:`);
console.log(`  Total: ${manifest.total_tasks}`);
console.log(`  Per repo:`, manifest.per_repo);
console.log(`  Per lang:`, manifest.per_lang);
console.log(`  Splits:`, manifest.splits);
