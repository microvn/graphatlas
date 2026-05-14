#!/usr/bin/env bun
// Phase A1 — Mine fix commits for M2 gate (S-004 AS-009)
//
// Output: benches/uc-impact/raw/<repo>-candidates.json
//   Array of validated fix commits per repo, each containing:
//     sha, parent_sha, subject, source_files, test_files
//
// Filter rules:
//   - Commit message matches /fix|fixes|bug/i (case-insensitive keyword)
//   - Not a merge commit
//   - 1 <= |source files| <= 15
//   - |test files| >= 1 (must have at least one test file touched)
//   - No vendored paths (node_modules, vendor, third_party, dist, build)
//
// Git-only: no gh CLI, no GitHub API. Uses `git -C <repo> log|show`.
// Adaptive paginated mining: stop when target reached, hard cap at MAX_CANDIDATES.
//
// Run: bun run scripts/mine-fix-commits.ts [--repo <name>] [--target <N>]

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { $ } from "bun";

const FIXTURES_DIR = "benches/fixtures";
const OUT_DIR = "benches/uc-impact/raw";
const TARGET_PER_REPO = 20;
const BATCH_SIZE = 50;
const MAX_CANDIDATES = 1000;

// Repos in scope. v1.1-M4 adds Java fixture (mockito) per Lang-C1 Java contract.
// S-002-bench adds 2 Kotlin fixtures (kotlinx-coroutines for AS-007 + AS-008
// saturation; kotlinx-serialization for Lang-C7 @Serializable AnnotatedFieldType).
const REPOS: { name: string; lang: string }[] = [
  { name: "django", lang: "python" },
  { name: "preact", lang: "javascript" },
  { name: "nest", lang: "typescript" },
  { name: "gin", lang: "go" },
  { name: "axum", lang: "rust" },
  { name: "regex", lang: "rust" },
  { name: "tokio", lang: "rust" },
  { name: "mockito", lang: "java" },
  { name: "kotlinx-coroutines", lang: "kotlin" },
  { name: "kotlinx-serialization", lang: "kotlin" },
  { name: "MQTTnet", lang: "csharp" },
  { name: "Polly", lang: "csharp" },
  { name: "jekyll", lang: "ruby" },
  { name: "faraday", lang: "ruby" },
  // v1.2-php S-002 AS-008 — PHP fixtures pinned per preflight log
  // docs/explore/php-fixture-preflight-2026-05-14.md
  { name: "php-symfony-console", lang: "php" },
  { name: "php-monolog", lang: "php" },
];

// Test-file detection. v1.1-M4 covers Java/Kotlin/C#/Ruby suffixes.
// S-002-bench additions: *IT.kt (Kotlin integration tests) +
// `<setName>Test/kotlin/` KMP multi-target dir segments
// (commonTest/jvmTest/androidTest/iosTest/nativeTest/jsTest/wasmTest/...).
// S-004-bench additions: Ruby Minitest prefix `test_*.rb` (jekyll uses this)
// alongside the existing `_test.rb`/`_spec.rb` suffix arms.
// Per docs/guide/dataset-for-new-language.md §0.6-G + §4.2 lock-step.
// v1.2-php S-002 AS-010 — PHPUnit suffix `*Test.php` / `*Tests.php` added.
const TEST_PATTERN = /(^|\/)(test|tests|__tests__|spec|specs|testing)\/|\/[a-zA-Z]+Test\/kotlin\/|(^|\/)[^/]*(\.test|\.spec|_test|_spec)\.[a-z]+$|(^|\/)test_[^/]+\.py$|(^|\/)[^/]+(Test|Tests|Spec|IT)\.java$|(^|\/)[^/]+(Test|Tests|Spec|IT)\.kt$|(^|\/)[^/]+(Test|Tests)\.cs$|(^|\/)[^/]+_(test|spec)\.rb$|(^|\/)test_[^/]+\.rb$|(^|\/)[^/]+(Test|Tests)\.php$/;
// VENDORED_PATTERN: v1.1-M4 baseline + Kotlin generated dirs (kapt/ksp output
// land in build/generated/, already covered by `build` segment; explicit ksp
// path only fires when projects use non-standard layouts).
const VENDORED_PATTERN = /(^|\/)(node_modules|vendor|third_party|dist|build|target\/|out\/|\.gradle|\.venv|venv|__pycache__)\//;
const DOC_ONLY_PATTERN = /(\.md$|\.txt$|CHANGELOG|LICENSE|\.github\/|docs\/)/i;

interface Candidate {
  sha: string;
  parent_sha: string;
  subject: string;
  source_files: string[];
  test_files: string[];
}

function isTest(path: string): boolean {
  return TEST_PATTERN.test(path);
}

function isVendored(path: string): boolean {
  return VENDORED_PATTERN.test(path);
}

function isDocOnly(path: string): boolean {
  // Only count as "doc" if it's purely documentation/config, not source
  return DOC_ONLY_PATTERN.test(path) && !path.endsWith(".py") && !path.endsWith(".ts")
    && !path.endsWith(".js") && !path.endsWith(".go") && !path.endsWith(".rs")
    && !path.endsWith(".rb");
}

async function gitLogBatch(
  repoDir: string,
  offset: number,
  limit: number,
): Promise<{ sha: string; subject: string }[]> {
  // 2026-04-28 Story C v3 — subject-only filter.
  // `git log --grep` matches against the entire commit message (subject +
  // body) by default. Long-running PRs squash-merge many "fix: typo",
  // "fix: import" sub-commit lines into the body of a feature commit
  // whose SUBJECT is "Remove ContentLengthLimit" — body grep let those
  // through. Solution: pull all non-merge commits, post-filter on subject.
  const out = await $`git -C ${repoDir} log --all --no-merges --format=%H%x09%s --skip=${offset} -n ${limit}`.text();
  const subjectFilter = /\b(fix|fixes|fixed|bug)\b/i;
  return out
    .split("\n")
    .filter(Boolean)
    .map((line) => {
      const [sha, ...rest] = line.split("\t");
      return { sha, subject: rest.join("\t") };
    })
    .filter((row) => subjectFilter.test(row.subject));
}

async function gitParent(repoDir: string, sha: string): Promise<string | null> {
  try {
    const out = await $`git -C ${repoDir} rev-parse ${sha}^`.text();
    return out.trim() || null;
  } catch {
    // Root commit has no parent
    return null;
  }
}

async function gitShowFiles(repoDir: string, sha: string): Promise<string[]> {
  const out = await $`git -C ${repoDir} show --name-only --format= ${sha}`.text();
  return out.split("\n").filter(Boolean);
}

// 2026-04-28 — Subject blocklist for mining filter (Story C, /mf-voices
// Round 4 consensus). Catches typo/grammar/fmt/docs sweeps that match the
// `/fix|fixes|bug/i` filter but produce wide-touch GT with no static-graph
// relationship → low recall artefact.
//
// Exported for mine-fix-commits.test.ts.
export function isNoiseSubject(subject: string): boolean {
  const s = subject.trim();
  // Conventional-commit prefix sweeps
  if (/^(chore|docs?|fmt|style)\s*[:(]/i.test(s)) return true;
  // Standalone keywords anywhere — typo/grammar/fmt/spelling/whitespace
  if (/\b(typos?|grammar|cargo\s*fmt|rustfmt|formatting|whitespace|spelling|misspell)\b/i.test(s))
    return true;
  // "Fix typo in <X>" without conventional prefix
  if (/^fix(ed|es)?\s+(some\s+)?(typos?|grammar|spelling)/i.test(s)) return true;
  // Story C v3 — lint/warning cleanup. "Fix <X> warning|lint|clippy".
  // These touch many files for non-bug reasons (lint sweep). The leading
  // anchor prevents false-positives on commits that happen to mention
  // a warning as a side effect ("fix: missing X causes warning" is OK).
  if (/^fix(ed|es)?\b[^:]{0,80}\b(clippy|lint|warning)s?\b/i.test(s)) return true;
  return false;
}

// Detect a diff where every changed line is whitespace-only (fmt sweeps,
// trailing-whitespace cleanup). Counts only lines starting with `+` or `-`
// (not `+++`/`---` headers); if the trimmed content of every such line
// matches its counterpart, the diff is whitespace-noise.
//
// Implementation: pair `+` and `-` lines in order; treat as noise iff
// every pair is identical after trim. Standalone +/- without a partner
// counts as a real change (additions or deletions).
export function isWhitespaceOnlyDiff(diff: string): boolean {
  if (diff.trim().length === 0) return true;
  const lines = diff.split("\n");
  const adds: string[] = [];
  const dels: string[] = [];
  for (const line of lines) {
    if (line.startsWith("+++") || line.startsWith("---")) continue;
    if (line.startsWith("+")) adds.push(line.slice(1));
    else if (line.startsWith("-")) dels.push(line.slice(1));
  }
  if (adds.length !== dels.length) return false;
  for (let i = 0; i < adds.length; i++) {
    if (adds[i].trim() !== dels[i].trim()) return false;
  }
  return true;
}

function classifyFiles(files: string[]): {
  sources: string[];
  tests: string[];
  skip: boolean; // true if vendored/docs-only reject
} {
  const sources: string[] = [];
  const tests: string[] = [];
  let docOnlyCount = 0;

  for (const f of files) {
    if (isVendored(f)) continue;
    if (isTest(f)) {
      tests.push(f);
      continue;
    }
    if (isDocOnly(f)) {
      docOnlyCount++;
      continue;
    }
    sources.push(f);
  }

  // If all non-test files are docs, skip
  const skip = sources.length === 0 && docOnlyCount > 0;
  return { sources, tests, skip };
}

async function mineRepo(
  repoName: string,
  target: number,
): Promise<Candidate[]> {
  const repoDir = join(FIXTURES_DIR, repoName);
  const valid: Candidate[] = [];
  let offset = 0;
  let totalSampled = 0;
  let rejectVendored = 0;
  let rejectSrcRange = 0;
  let rejectNoTest = 0;
  let rejectNoiseSubject = 0;
  let rejectWhitespaceOnly = 0;

  console.log(`\n[${repoName}] mining fix commits (target=${target})`);

  while (valid.length < target && offset < MAX_CANDIDATES) {
    const batch = await gitLogBatch(repoDir, offset, BATCH_SIZE);
    if (batch.length === 0) break;

    for (const { sha, subject } of batch) {
      totalSampled++;

      // 2026-04-28 — subject blocklist (Story C, /mf-voices Round 4):
      // drop typo/grammar/fmt/docs sweeps before any expensive git work.
      if (isNoiseSubject(subject)) {
        rejectNoiseSubject++;
        continue;
      }

      const files = await gitShowFiles(repoDir, sha);
      const { sources, tests, skip } = classifyFiles(files);

      if (skip) {
        rejectVendored++;
        continue;
      }
      if (sources.length < 1 || sources.length > 15) {
        rejectSrcRange++;
        continue;
      }
      if (tests.length < 1) {
        rejectNoTest++;
        continue;
      }

      // Diff-based gate (Codex/Claude consensus): even if subject looks
      // like a real fix, drop commits whose entire patch is whitespace
      // changes. One git-show per surviving candidate is acceptable —
      // subject blocklist already filtered the cheapest noise.
      try {
        const diff = await $`git -C ${repoDir} show --format= ${sha}`.text();
        if (isWhitespaceOnlyDiff(diff)) {
          rejectWhitespaceOnly++;
          continue;
        }
      } catch {
        // git show failures are non-fatal; let candidate through.
      }

      const parent = await gitParent(repoDir, sha);
      if (!parent) continue;

      valid.push({
        sha,
        parent_sha: parent,
        subject,
        source_files: sources,
        test_files: tests,
      });

      if (valid.length === target) break;
    }

    offset += BATCH_SIZE;
    if (offset % 200 === 0) {
      console.log(`  [${repoName}] progress: ${valid.length}/${target} valid, ${totalSampled} sampled`);
    }
  }

  console.log(
    `[${repoName}] done: ${valid.length}/${target} valid | sampled=${totalSampled} | ` +
    `rejected: noise-subject=${rejectNoiseSubject} vendored/docs=${rejectVendored} ` +
    `src-range=${rejectSrcRange} no-test=${rejectNoTest} whitespace-only=${rejectWhitespaceOnly}`,
  );

  if (valid.length < target) {
    console.error(
      `[${repoName}] WARNING: only ${valid.length}/${target} commits pass filter ` +
      `(hit cap ${MAX_CANDIDATES})`,
    );
  }

  return valid;
}

// --- Main ---

// Guard so importing this file from tests doesn't trigger mining. Bun
// sets `import.meta.main` true only for the entry script.
if (import.meta.main) {
  await main();
}

async function main() {
const args = process.argv.slice(2);
let onlyRepo: string | null = null;
let target = TARGET_PER_REPO;

for (let i = 0; i < args.length; i++) {
  if (args[i] === "--repo" && args[i + 1]) {
    onlyRepo = args[i + 1];
    i++;
  } else if (args[i] === "--target" && args[i + 1]) {
    target = parseInt(args[i + 1], 10);
    i++;
  }
}

mkdirSync(OUT_DIR, { recursive: true });

const reposToMine = onlyRepo ? REPOS.filter((r) => r.name === onlyRepo) : REPOS;
if (reposToMine.length === 0) {
  console.error(`Unknown repo: ${onlyRepo}. Valid: ${REPOS.map((r) => r.name).join(", ")}`);
  process.exit(1);
}

for (const repo of reposToMine) {
  const candidates = await mineRepo(repo.name, target);
  const outPath = join(OUT_DIR, `${repo.name}-candidates.json`);
  writeFileSync(
    outPath,
    JSON.stringify(
      {
        repo: repo.name,
        lang: repo.lang,
        mined_at: new Date().toISOString(),
        target,
        actual: candidates.length,
        candidates,
      },
      null,
      2,
    ),
  );
  console.log(`  wrote ${outPath}`);
}

console.log("\nDone. Next: bun run scripts/extract-seeds.ts (Phase A2)");
}
