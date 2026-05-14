#!/usr/bin/env bun
// Phase A2 — Extract seeds + convention-match tests + compute depth labels
//
// Input:  benches/uc-impact/raw/<repo>-candidates.json (from Phase A1)
// Output: benches/uc-impact/raw/<repo>-tasks.json (seed-enriched)
//
// Per candidate:
//   1. Strip build-manifest/lock files from source_files (Cargo.lock, package.json,
//      go.sum, etc.) — they aren't "affected code".
//   2. Pick seed_file = largest-changed source file by diff line count.
//   3. Pick seed_symbol = enclosing function/class name from git hunk header
//      at first changed line in seed_file. Per-lang signature parser.
//   4. Augment test_files with convention-match for each source file.
//   5. max_expected_depth = 1 if |source_files| == 1 (seed-only),
//      else 2 (seed + direct neighbors).

import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { $ } from "bun";

const FIXTURES_DIR = "benches/fixtures";
const RAW_DIR = "benches/uc-impact/raw";

// v1.1-M4 — mockito added per Lang-C1 Java contract closure.
// S-002-bench — Kotlin fixtures added per Lang-C1 Kotlin closure.
// S-003-bench — MQTTnet (C#) added per Lang-C1 C# closure.
const REPOS = [
  "django", "preact", "nest", "gin", "axum", "regex", "tokio", "mockito",
  "kotlinx-coroutines", "kotlinx-serialization",
  "MQTTnet", "Polly",
  "jekyll", "faraday",
  // v1.2-php S-002 AS-008 — PHP fixtures pinned per preflight log
  // docs/explore/php-fixture-preflight-2026-05-14.md
  "php-symfony-console", "php-monolog",
] as const;

// Build/config files — exclude from "affected source" set.
// Extended in v4: includes TS declaration files (.d.ts — no executable body,
// GA doesn't index), bundler configs (vitest.config.*, mangle.json), and YAML
// lint configs (.golangci.yml) that aren't GA-parseable source.
const BUILD_FILE_PATTERN =
  /(^|\/)(Cargo\.lock|Cargo\.toml|package\.json|package-lock\.json|yarn\.lock|pnpm-lock\.yaml|go\.mod|go\.sum|poetry\.lock|requirements.*\.txt|setup\.py|setup\.cfg|pyproject\.toml|Gemfile\.lock|Pipfile\.lock|composer\.json|composer\.lock|Makefile|CMakeLists\.txt|\.gitignore|\.gitattributes|\.editorconfig|tsconfig.*\.json|jest\.config.*|webpack\.config.*|vite\.config.*|vitest\.config.*|vitest\.setup\.*|rollup\.config.*|babel\.config.*|\.eslintrc.*|\.prettierrc.*|mangle\.json|\.golangci\.ya?ml|\.codecov\.ya?ml|pom\.xml|build\.gradle.*|settings\.gradle.*|gradlew|gradlew\.bat|gradle\.properties|\.mvn|\.idea|\.iml|build\.sbt|project\.clj|deps\.edn|gemspec|Rakefile|phpunit\.xml.*)$/;

// Extensions GA parsers understand. Anything else will never return a graph
// node → should not be a seed_file or counted in expected_files.
// v1.1-M4 — Java added (S-001), Kotlin added (S-002 — kt + kts), C# added
// (S-003 — cs), Ruby added (S-004 — rb). `.rake`/`.gemspec` are Ruby DSL
// files; tree-sitter-ruby parses them but they're build/config artifacts —
// excluded from seed/expected sets via BUILD_FILE_PATTERN.
const GA_SUPPORTED_EXT = new Set([
  "py", "ts", "tsx", "js", "jsx", "mjs", "cjs", "go", "rs", "java", "kt", "kts", "cs", "rb",
  // v1.2 S-001 — PHP support. `.phtml` templates intentionally excluded
  // (mixed HTML+PHP, different parse strategy — see v1.2-php.md Not in Scope).
  "php",
]);

function fileExt(path: string): string | null {
  const m = path.match(/\.([a-zA-Z0-9]+)$/);
  return m ? m[1].toLowerCase() : null;
}

function isBuildFile(path: string): boolean {
  if (BUILD_FILE_PATTERN.test(path)) return true;
  // .d.ts declaration files — no executable body, GA doesn't parse them for
  // graph edges. Exclude from both seed and expected set.
  if (path.endsWith(".d.ts")) return true;
  return false;
}

function isGaParseable(path: string): boolean {
  const ext = fileExt(path);
  return ext !== null && GA_SUPPORTED_EXT.has(ext);
}

interface Candidate {
  sha: string;
  parent_sha: string;
  subject: string;
  source_files: string[];
  test_files: string[];
}

interface EnrichedTask {
  task_id: string;
  repo: string;
  base_commit: string; // parent_sha (pre-fix state)
  fix_commit: string; // sha (the fix)
  subject: string;
  seed_file: string;
  seed_symbol: string;
  source_files: string[]; // filtered (no build)
  expected_files: string[]; // = source_files (same)
  expected_tests: string[]; // from commit + convention match
  should_touch_files: string[]; // structural blast radius (schema v3)
  max_expected_depth: number;
}

// --- Symbol extractor from git hunk headers ---

// Matches function/class/method/struct signatures per lang. Catches the most
// common shapes; if it fails we fallback to first identifier on the header line.
const SIG_PATTERNS: RegExp[] = [
  // Python: def foo(, async def foo(, class Foo(
  /\b(?:async\s+)?def\s+([A-Za-z_][\w]*)/,
  /\bclass\s+([A-Za-z_][\w]*)/,
  // Go: func foo(, func (r *R) foo(
  /\bfunc\s+(?:\([^)]*\)\s+)?([A-Za-z_][\w]*)/,
  // Rust: fn foo(, pub fn foo(, pub(crate) fn foo(, impl Foo, struct Foo, enum Foo, trait Foo
  /\b(?:pub\s*(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?(?:const\s+)?fn\s+([A-Za-z_][\w]*)/,
  // 2026-04-28 (Story B, /mf-voices Round 4) — `impl <Trait> for <Target>`
  // captures <Target> not <Trait>. Picker previously caught
  // `impl Default for Framed` → `Default` (trait), should be `Framed`
  // (the type the impl extends). Must precede the generic
  // `(impl|struct|enum|trait|type)` pattern so it wins on `impl ... for ...`.
  /\bimpl\s*(?:<[^>]+>\s*)?[\w<>,\s:'+]+?\s+for\s+(?:<[^>]+>\s*)?([A-Za-z_][\w]*)/,
  /\b(?:impl|struct|enum|trait|type)\s+(?:<[^>]+>\s*)?([A-Za-z_][\w]*)/,
  // JS/TS: function foo, class Foo, const foo =, foo: function, method(
  /\bfunction\s*\*?\s+([A-Za-z_$][\w$]*)/,
  /\b(?:export\s+(?:default\s+)?)?class\s+([A-Za-z_$][\w$]*)/,
  /\b(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*[=:]/,
  /\b([A-Za-z_$][\w$]*)\s*(?:=\s*(?:async\s+)?(?:function|\([^)]*\)\s*=>|\([^)]*\)\s*{))/,
  // v1.1-M4 — Java/Kotlin/C# class / interface / enum / record / @interface
  /\b(?:public|private|protected|abstract|static|final|sealed|@)?\s*(?:class|interface|enum|record|@interface)\s+([A-Za-z_][\w]*)/,
  // Java/C#/Kotlin method signature — captures method name before `(`.
  // Skips visibility / static / final / generic params / annotations / return
  // type. The lowercase-first-char hint excludes constructor names (which
  // would shadow the class name) — Java idiom: methods camelCase.
  // Patch C (2026-04-26 Codex audit): `[a-z_]` → `[A-Za-z_]` to allow C#
  // PascalCase methods (`AcceptAsync`, `GetSocketOption`). Pre-fix this
  // regex rejected all C# method signatures and fell back to the
  // first-non-stopword identifier — usually the class name (correct) but
  // sometimes the namespace name (broken on MQTTnet).
  /\b(?:public|private|protected|internal)?\s*(?:static\s+|final\s+|abstract\s+|synchronized\s+|default\s+|native\s+|strictfp\s+|virtual\s+|override\s+|sealed\s+|async\s+)*(?:<[^>]+>\s+)?(?:[A-Z][\w<>,\s\[\].?]*?\s+)?([A-Za-z_][\w]*)\s*\(/,
  // Ruby: def foo, class Foo, module Foo (multiple shapes)
  /\bdef\s+(?:self\.)?([a-z_][\w?!=]*)/,
  /\b(?:class|module)\s+([A-Z][\w]*)/,
  // v1.1-M4 S-002 — Kotlin function declarations.
  // Captures: regular fn, suspend fn, extension fn, inline fn, with annotations.
  // Extension form `fun ReceiverType.name(...)` — captures `name` after the `.`.
  /\b(?:public|private|protected|internal|inline|crossinline|noinline|tailrec|operator|infix|external|abstract|open|override)?\s*(?:suspend\s+)?fun\s+(?:<[^>]+>\s+)?(?:[A-Za-z_][\w<>,.?\s]*\.)?([a-z_][\w]*)\s*\(/,
  // Kotlin val/var properties — captures property name (lateinit allowed).
  /\b(?:public|private|protected|internal|const|lateinit|override|open|final|abstract)?\s*(?:val|var)\s+([A-Za-z_][\w]*)/,
  // Kotlin object/companion object/typealias.
  /\b(?:companion\s+)?object\s+([A-Za-z_][\w]*)/,
  /\btypealias\s+([A-Za-z_][\w]*)/,
];

// Stopwords: keywords that look like identifiers but are never meaningful seed
// symbols. Covers Rust/Go/Python/JS/TS imports, visibility, async, etc.
const STOPWORDS = new Set([
  "use", "mod", "pub", "crate", "super", "self", "extern", "where",
  "async", "await", "const", "let", "var", "static", "mut", "ref",
  "impl", "trait", "struct", "enum", "type", "fn", "if", "else",
  "match", "return", "for", "while", "loop", "in", "as",
  "import", "from", "export", "default", "function", "class",
  "def", "lambda", "yield", "pass", "raise", "try", "except", "finally",
  "package", "func", "interface", "chan", "go", "defer", "select",
  "true", "false", "null", "nil", "None", "True", "False", "undefined",
  // TS-specific keywords that were leaking through
  "declare", "namespace", "readonly", "abstract", "public", "private",
  "protected", "override", "implements", "extends", "new", "this",
  "typeof", "keyof", "instanceof", "satisfies", "infer",
  // Rust lifetimes / common generics
  "Self", "Box", "Vec", "Option", "Result", "String",
  // Python built-ins that show up as hunk context
  "print", "len", "range", "list", "dict", "set", "str", "int", "bool",
  // v1.1-M4 — Java/Kotlin/C#/Ruby keywords + primitive types that leak as
  // hunk-header tokens. Java return types (`void`, `int`, `boolean`, etc.)
  // are the most common false positive — without these, every method
  // signature `public void foo()` returns "void" as the seed symbol.
  "void", "byte", "char", "short", "long", "float", "double",
  "throws", "throw", "synchronized", "volatile", "transient", "native",
  "strictfp", "sealed", "non-sealed", "permits", "var", "yield",
  "record", "switch", "case", "do", "goto",
  // Kotlin extras
  "fun", "val", "data", "object", "companion", "lateinit", "by",
  "suspend", "tailrec", "inline", "crossinline", "noinline", "reified",
  "vararg", "infix", "operator", "expect", "actual", "external",
  "init", "it", "constructor", "open", "final",
  // Kotlin built-in types (leak as return types / property types)
  "Unit", "Nothing", "Any", "Int", "Long", "Float", "Double",
  "Boolean", "Char", "Byte", "Short", "Array", "List", "Map",
  // C# extras
  "using", "internal", "params", "out", "ref", "readonly",
  "delegate", "event", "checked", "unchecked", "lock", "unsafe",
  // Ruby extras
  "begin", "rescue", "ensure", "elsif", "unless", "until", "redo",
  "retry", "next", "break", "alias", "undef",
]);

// Lang tags used by the seed-quality filter. Match the `lang` field written
// by consolidate-gt.ts (LANG_BY_REPO).
type Lang =
  | "python" | "javascript" | "typescript" | "go" | "rust"
  | "java" | "kotlin" | "csharp" | "ruby";

// Lang-scoped stdlib symbols — every program in the language references
// these, so they're not graph-traversable concepts being TESTED. Step A
// (Codex insight #1, 2026-04-27): seeded from the bad symbols observed in
// MQTTnet-tasks.json post-Patches-A+B+C audit. Extend conservatively.
const STDLIB_SYMBOLS: Partial<Record<Lang, ReadonlySet<string>>> = {
  csharp: new Set([
    // String / collection methods
    "Substring", "ToString", "ToArray", "ToList", "Equals", "GetHashCode",
    "Contains", "IndexOf", "Compare", "CompareTo",
    // Task helpers
    "FromResult", "FromException", "WhenAll", "WhenAny", "ConfigureAwait",
    "ContinueWith", "Run", "Delay",
    // Common exceptions
    "ArgumentException", "ArgumentNullException", "ArgumentOutOfRangeException",
    "InvalidOperationException", "NotSupportedException", "NotImplementedException",
    "ObjectDisposedException", "TimeoutException", "OperationCanceledException",
    "NullReferenceException", "IndexOutOfRangeException",
  ]),
};

export function isLikelyBadSymbol(sym: string, lang?: Lang): boolean {
  if (sym.length < 3) return true;
  if (STOPWORDS.has(sym)) return true;
  // All uppercase = constant, not a function → GA can't graph-traverse
  if (/^[A-Z][A-Z0-9_]*$/.test(sym)) return true;
  // Single-letter generic type parameter (E, T, K, V)
  if (/^[A-Z]$/.test(sym)) return true;
  // Lang-aware filters (only when lang is known — preserves backward compat
  // with sites that don't yet pass lang).
  if (lang) {
    // C# private-field convention: `_lowerCamelCase` = instance field, not a
    // graph-traversable concept. JS/TS use `_foo` for soft-private FUNCTIONS
    // which ARE legitimate seeds — only reject for csharp.
    if (lang === "csharp" && /^_[a-z]/.test(sym)) return true;
    const stdlib = STDLIB_SYMBOLS[lang];
    if (stdlib && stdlib.has(sym)) return true;
  }
  return false;
}

// 2026-04-28 (Story B, /mf-voices Round 4) — Rust idioms that the engine
// indexer doesn't model as Symbol nodes. Picking these as seeds yields
// "symbol not found" at scoring time. Detected from tokio audit.
const RUST_SKIP_PATTERNS: RegExp[] = [
  // `macro_rules! <name> { ... }` — engine doesn't index macro_rules
  // definitions. Surface a different changed file's hunk instead.
  /^macro_rules!\s+\w+/,
];

// Rust attribute identifiers that show up as the FIRST identifier on
// `#[<attr>(...)]` / `#![<attr>(...)]` lines. Treat as non-seed; the
// fallback identifier picker should keep scanning past the attribute.
const RUST_ATTR_NAMES = new Set([
  "doc", "cfg", "cfg_attr", "derive", "inline", "cold", "no_mangle",
  "repr", "must_use", "deprecated", "allow", "warn", "deny", "forbid",
  "test", "ignore", "should_panic", "bench", "feature", "macro_use",
  "macro_export", "non_exhaustive", "track_caller", "global_allocator",
  "panic_handler", "automatically_derived", "link", "link_name",
  "no_std", "no_main", "windows_subsystem", "thread_local",
]);

export function extractSymbol(hunkContext: string, lang?: Lang): string | null {
  const ctx = hunkContext.trim();
  if (!ctx) return null;

  // Story B — Rust skip patterns: bail out so caller picks another hunk
  // instead of returning a symbol the engine can't resolve.
  if (lang === "rust") {
    for (const skip of RUST_SKIP_PATTERNS) {
      if (skip.test(ctx)) return null;
    }
  }

  for (const pat of SIG_PATTERNS) {
    const m = ctx.match(pat);
    if (m && m[1] && !isLikelyBadSymbol(m[1], lang)) {
      // Story B — for Rust, never accept an attribute name as a symbol
      // even if a SIG_PATTERN matched it. (E.g. `#[doc = "..."]` tries
      // multiple patterns and could land on one that captures `doc`.)
      if (lang === "rust" && RUST_ATTR_NAMES.has(m[1])) continue;
      return m[1];
    }
  }
  // Fallback: first non-stopword identifier (skip use/pub/async/etc.)
  const all = ctx.match(/[A-Za-z_$][\w$]*/g) ?? [];
  for (const id of all) {
    if (lang === "rust" && RUST_ATTR_NAMES.has(id)) continue;
    if (!isLikelyBadSymbol(id, lang)) return id;
  }
  return null;
}

async function gitDiffStats(
  repoDir: string,
  sha: string,
): Promise<Record<string, number>> {
  // Lines changed per file
  const out = await $`git -C ${repoDir} show --numstat --format= ${sha}`.text();
  const stats: Record<string, number> = {};
  for (const line of out.split("\n").filter(Boolean)) {
    const parts = line.split("\t");
    if (parts.length !== 3) continue;
    const adds = parseInt(parts[0], 10) || 0;
    const dels = parseInt(parts[1], 10) || 0;
    const path = parts[2];
    stats[path] = adds + dels;
  }
  return stats;
}

async function gitShowFile(
  repoDir: string,
  sha: string,
  path: string,
): Promise<string> {
  try {
    return await $`git -C ${repoDir} show --format= ${sha} -- ${path}`.text();
  } catch {
    return "";
  }
}

// Hunk contexts that aren't function signatures (imports, module decls, etc.)
//
// S-003-bench follow-up (2026-04-26 Codex audit): added C# `namespace\s` /
// `using\s` to fix 13/19 MQTTnet seeds being captured as the namespace name
// "MQTTnet" instead of real class/method symbols. Pre-emptively added Ruby
// `require\s` / `require_relative\s` / `module\s` ahead of S-004 (Codex
// insight #2 — same failure mode would hit Ruby module declarations).
export const NON_FN_HUNK_PREFIX = /^(?:use\s|pub\s+use\s|import\s|from\s|#include|package\s|mod\s+[a-z]|extern\s+crate|namespace\s|using\s|require\s|require_relative\s|module\s)/;

function extractFirstHunkContext(diffText: string): string | null {
  // Diff hunk header shape: @@ -a,b +c,d @@ <context>
  // Context after the second @@ is the enclosing function signature.
  // Skip contexts that are imports/module-level declarations — they won't yield
  // a useful seed symbol. Prefer first meaningful (function-shaped) context.
  const lines = diffText.split("\n");
  let firstAny: string | null = null;
  for (const line of lines) {
    if (!line.startsWith("@@")) continue;
    const m = line.match(/^@@\s+-\d+(?:,\d+)?\s+\+\d+(?:,\d+)?\s+@@\s*(.*)$/);
    const ctx = m?.[1]?.trim();
    if (!ctx) continue;
    if (firstAny === null) firstAny = ctx;
    if (NON_FN_HUNK_PREFIX.test(ctx)) continue;
    return ctx; // first non-import hunk
  }
  return firstAny; // fall back to whatever we saw
}

/** Verify symbol actually appears in the file at the fix commit's parent
 * state — if not, GA can't graph-resolve it. */
async function symbolInFile(
  repoDir: string,
  sha: string,
  path: string,
  symbol: string,
): Promise<boolean> {
  const content = await gitShowPath(repoDir, sha, path);
  if (!content) return false;
  // word-boundary check; escape regex metachars in symbol
  const esc = symbol.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`\\b${esc}\\b`).test(content);
}

async function gitShowPath(
  repoDir: string,
  sha: string,
  path: string,
): Promise<string> {
  try {
    return await $`git -C ${repoDir} show ${sha}:${path}`.text();
  } catch {
    return "";
  }
}

/** Extract top symbol candidates from hunk contexts (not just first one).
 * Lets pickSeed try multiple before giving up.
 *
 * Patch B (2026-04-26 Codex audit): also scan hunk BODY lines (` `, `+`,
 * `-` prefixes), not just `@@ ... @@` headers. C# diffs frequently have
 * a header like `namespace MQTTnet.Implementations` (because the change
 * is wrapped in an outer namespace block) while the actual method
 * signature lives in body lines. Pre-fix: only headers were considered →
 * `extractSymbol("namespace MQTTnet.X")` fell back to "MQTTnet" identifier.
 * Post-fix: body lines like `+    public async Task<X> AcceptAsync(...)`
 * surface as candidates (after stripping the diff prefix char).
 */
export function extractAllHunkContexts(diffText: string): string[] {
  const out: string[] = [];
  for (const line of diffText.split("\n")) {
    // Header form: `@@ -a,b +c,d @@ <ctx>`
    if (line.startsWith("@@")) {
      const m = line.match(/^@@\s+-\d+(?:,\d+)?\s+\+\d+(?:,\d+)?\s+@@\s*(.*)$/);
      const ctx = m?.[1]?.trim();
      if (ctx && !NON_FN_HUNK_PREFIX.test(ctx)) out.push(ctx);
      continue;
    }
    // Body form: lines prefixed by ` ` (context), `+` (added), `-` (removed).
    // Skip diff metadata (---, +++, diff --git, index, similarity, etc.).
    if (line.startsWith("---") || line.startsWith("+++")) continue;
    if (
      line.startsWith("diff ") ||
      line.startsWith("index ") ||
      line.startsWith("similarity ") ||
      line.startsWith("rename ") ||
      line.startsWith("new file ") ||
      line.startsWith("deleted file ") ||
      line.startsWith("Binary ")
    ) {
      continue;
    }
    if (
      line.length > 1 &&
      (line[0] === " " || line[0] === "+" || line[0] === "-")
    ) {
      const body = line.slice(1).trim();
      if (!body) continue;
      if (NON_FN_HUNK_PREFIX.test(body)) continue;
      // Heuristic: only consider body lines that look like declarations
      // (contain a `(` for method signatures OR start with `class`/`interface`/
      // `record`/`struct`/`enum`/`trait`/`def`/`fn`/`fun`/`func`).
      // Filters out arbitrary expressions / comments / brace lines.
      const looksLikeDecl =
        body.includes("(") ||
        /^(?:public|private|protected|internal|static|abstract|sealed|partial|async|virtual|override|export|default)\s/.test(
          body,
        ) ||
        /^(?:class|interface|record|struct|enum|trait|def|fn|fun|func|impl)\s/.test(
          body,
        );
      if (!looksLikeDecl) continue;
      out.push(body);
    }
  }
  return out;
}

async function pickSeed(
  repoDir: string,
  sha: string,
  sourceFiles: string[],
  lang?: Lang,
): Promise<{ seed_file: string; seed_symbol: string } | null> {
  // Pick seed_file = source file with most changed lines
  const stats = await gitDiffStats(repoDir, sha);
  const ranked = [...sourceFiles]
    .map((f) => ({ f, n: stats[f] ?? 0 }))
    .sort((a, b) => b.n - a.n);
  const seed_file = ranked[0]?.f ?? sourceFiles[0];

  // Extract seed_symbol — try multiple hunk contexts, validate each exists in
  // the parent (base) commit's version of the file. Parent commit = sha^.
  // If no hunk yields a valid symbol, reject the task (return null).
  const diff = await gitShowFile(repoDir, sha, seed_file);
  const contexts = extractAllHunkContexts(diff);
  for (const ctx of contexts) {
    const sym = extractSymbol(ctx, lang);
    if (!sym) continue;
    if (isLikelyBadSymbol(sym, lang)) continue;
    // Verify symbol exists in seed_file at parent commit (the state we'll index)
    const parentSha = `${sha}^`;
    if (await symbolInFile(repoDir, parentSha, seed_file, sym)) {
      return { seed_file, seed_symbol: sym };
    }
  }
  return null; // task rejected — no validated seed
}

function basenameNoExt(path: string): string {
  const base = path.split("/").pop() ?? path;
  return base.replace(/\.[^.]+$/, "");
}

// --- Convention test match ---

function conventionTests(source: string): string[] {
  const base = basenameNoExt(source);
  const dir = source.split("/").slice(0, -1).join("/");
  const ext = source.match(/\.(py|ts|js|tsx|jsx|go|rs|java|kt|cs|rb)$/)?.[1] ?? "";

  const candidates: string[] = [];

  // Python: tests/test_foo.py, test_foo.py sibling
  if (ext === "py") {
    candidates.push(`tests/test_${base}.py`);
    candidates.push(`test/test_${base}.py`);
    if (dir) {
      candidates.push(`${dir}/test_${base}.py`);
      candidates.push(`${dir}/tests/test_${base}.py`);
    }
  }
  // Go: foo_test.go sibling
  if (ext === "go") {
    if (dir) candidates.push(`${dir}/${base}_test.go`);
    else candidates.push(`${base}_test.go`);
  }
  // JS/TS: foo.test.ts, foo.spec.ts, __tests__/foo.ts
  if (ext === "ts" || ext === "js" || ext === "tsx" || ext === "jsx") {
    if (dir) {
      candidates.push(`${dir}/${base}.test.${ext}`);
      candidates.push(`${dir}/${base}.spec.${ext}`);
      candidates.push(`${dir}/__tests__/${base}.${ext}`);
      candidates.push(`${dir}/__tests__/${base}.test.${ext}`);
    }
  }
  // Rust: tests/foo.rs OR same file with cfg(test) (best-effort; same-file can't be listed)
  if (ext === "rs") {
    if (dir) {
      candidates.push(`${dir}/tests/${base}.rs`);
      candidates.push(`tests/${base}.rs`);
    }
  }
  // Java: src/main/java/<pkg>/Foo.java → src/test/java/<pkg>/{Foo,FooTest,FooTests,FooSpec,FooIT}.java
  // + sibling test/<base>Test.java for non-Maven projects.
  if (ext === "java") {
    if (dir) {
      // Maven/Gradle convention.
      const mainPath = source.replace(/^(.*\/)?src\/main\/java\//, "$1src/test/java/");
      if (mainPath !== source) {
        const testDir = mainPath.split("/").slice(0, -1).join("/");
        candidates.push(`${testDir}/${base}Test.java`);
        candidates.push(`${testDir}/${base}Tests.java`);
        candidates.push(`${testDir}/${base}Spec.java`);
        candidates.push(`${testDir}/${base}IT.java`);
      }
      // Sibling test/ dir variant (non-Maven).
      candidates.push(`${dir}/${base}Test.java`);
      candidates.push(`${dir}/${base}Tests.java`);
      // Top-level test path.
      candidates.push(`test/java/${base}Test.java`);
    }
  }
  // Kotlin: same suffix shape as Java + Kotlin Multiplatform multi-target
  // source-set handling per docs/guide/dataset-for-new-language.md §0.6-G.
  // KMP convention: `<sourceSetName>Main` ↔ `<sourceSetName>Test`.
  // Examples: commonMain↔commonTest, jvmMain↔jvmTest, nativeMain↔nativeTest,
  // androidMain↔androidUnitTest (Android plugin) or androidTest (instrumented).
  // The `*Main` → `*Test` substitution is regex-driven so a new source set
  // (e.g. `linuxMain` / `mingwX64Main`) auto-applies.
  if (ext === "kt") {
    const KT_TEST_SUFFIXES = ["Test.kt", "Tests.kt", "Spec.kt", "IT.kt"];
    if (dir) {
      // Maven/Gradle src/main/kotlin → src/test/kotlin (single-target).
      const mainPath = source.replace(/^(.*\/)?src\/main\/kotlin\//, "$1src/test/kotlin/");
      if (mainPath !== source) {
        const testDir = mainPath.split("/").slice(0, -1).join("/");
        for (const suf of KT_TEST_SUFFIXES) {
          candidates.push(`${testDir}/${base}${suf}`);
        }
      }
      // KMP multi-target: <module>/<sourceSetName>Main/kotlin/... →
      // <module>/<sourceSetName>Test/kotlin/...
      // Match group 1 = sourceSet prefix (e.g., "common", "jvm", "native", "ios").
      const kmpMatch = source.match(/^(.*?)\/([a-zA-Z]+)Main\/kotlin\//);
      if (kmpMatch) {
        const setName = kmpMatch[2];
        const kmpTestPath = source.replace(
          /^(.*?)\/([a-zA-Z]+)Main\/kotlin\//,
          `$1/${setName}Test/kotlin/`,
        );
        if (kmpTestPath !== source) {
          const kmpTestDir = kmpTestPath.split("/").slice(0, -1).join("/");
          for (const suf of KT_TEST_SUFFIXES) {
            candidates.push(`${kmpTestDir}/${base}${suf}`);
          }
        }
        // Android plugin uses `androidMain` ↔ `androidUnitTest` for unit tests
        // (and `androidInstrumentedTest`/`androidTest` for instrumented).
        if (setName === "android") {
          const androidUnit = source.replace(
            /^(.*?)\/androidMain\/kotlin\//,
            "$1/androidUnitTest/kotlin/",
          );
          if (androidUnit !== source) {
            const auDir = androidUnit.split("/").slice(0, -1).join("/");
            for (const suf of KT_TEST_SUFFIXES) {
              candidates.push(`${auDir}/${base}${suf}`);
            }
          }
        }
      }
      // Sibling layout (non-Maven, single-target) and bare `test/` dir.
      for (const suf of KT_TEST_SUFFIXES) {
        candidates.push(`${dir}/${base}${suf}`);
      }
    }
  }
  // C#: convention varies — common patterns are sibling .Tests project or
  // src/<Project>/<File>.cs → tests/<Project>.Tests/<File>Tests.cs.
  if (ext === "cs") {
    if (dir) {
      candidates.push(`${dir}/${base}Tests.cs`);
      candidates.push(`${dir}/${base}Test.cs`);
      // Sibling .Tests project (path heuristic).
      candidates.push(`${dir.replace(/^src\//, "tests/").replace(/^([^/]+)/, "$1.Tests")}/${base}Tests.cs`);
    }
  }
  // Ruby: lib/foo.rb → spec/foo_spec.rb (RSpec) / test/foo_test.rb (Minitest
  // suffix) / test/test_foo.rb (Minitest prefix — jekyll uses this).
  // Also handles non-lib roots: `<dir>/foo.rb` → `<dir>/foo_spec.rb` etc.
  if (ext === "rb") {
    if (dir) {
      // Strip leading lib/ for canonical mapping to spec/test parallel trees.
      const specPath = source.replace(/^lib\//, "spec/").replace(/\.rb$/, "_spec.rb");
      const testSuffixPath = source.replace(/^lib\//, "test/").replace(/\.rb$/, "_test.rb");
      // jekyll-style: `test/test_<base>.rb` (Minitest prefix, flat layout).
      const testPrefixFlat = `test/test_${base}.rb`;
      // Or test/<sub>/test_<base>.rb mirroring lib/<sub>/<base>.rb.
      const testPrefixMirror = source
        .replace(/^lib\//, "test/")
        .replace(/\/([^/]+)\.rb$/, "/test_$1.rb");

      if (specPath !== source) candidates.push(specPath);
      if (testSuffixPath !== source) candidates.push(testSuffixPath);
      if (testPrefixMirror !== source) candidates.push(testPrefixMirror);
      candidates.push(testPrefixFlat);
      candidates.push(`${dir}/${base}_spec.rb`);
      candidates.push(`${dir}/${base}_test.rb`);
      candidates.push(`${dir}/test_${base}.rb`);
    }
  }

  // v1.2 S-001 — PHP / PHPUnit: src/<Foo>.php ↔ tests/<Foo>Test.php
  // (or Tests/, both seen in the wild). symfony/console uses Tests/;
  // monolog uses tests/.
  if (source.endsWith(".php")) {
    const base = basenameNoExt(source);
    const dir = source.includes("/") ? source.replace(/\/[^/]+$/, "") : "";
    // src/<Foo>.php → tests/<Foo>Test.php  (lowercase tests/)
    const lowerTestsPath = source
      .replace(/^src\//, "tests/")
      .replace(/\.php$/, "Test.php");
    // src/<Foo>.php → Tests/<Foo>Test.php  (PascalCase Tests/ — Symfony style)
    const pascalTestsPath = source
      .replace(/^src\//, "Tests/")
      .replace(/\.php$/, "Test.php");
    // Mirror: src/Sub/<Foo>.php → tests/Sub/<Foo>Test.php
    const mirrorPath = source
      .replace(/^src\//, "tests/")
      .replace(/\/([^/]+)\.php$/, "/$1Test.php");
    // Plural form: <Foo>Tests.php (less common but seen).
    const pluralPath = source
      .replace(/^src\//, "tests/")
      .replace(/\.php$/, "Tests.php");

    if (lowerTestsPath !== source) candidates.push(lowerTestsPath);
    if (pascalTestsPath !== source) candidates.push(pascalTestsPath);
    if (mirrorPath !== source && mirrorPath !== lowerTestsPath)
      candidates.push(mirrorPath);
    if (pluralPath !== source) candidates.push(pluralPath);
    candidates.push(`${dir}/${base}Test.php`);
    candidates.push(`${dir}/${base}Tests.php`);
  }

  return candidates;
}

async function fileExists(repoDir: string, path: string): Promise<boolean> {
  try {
    const out = await $`git -C ${repoDir} cat-file -e HEAD:${path}`.quiet();
    return out.exitCode === 0;
  } catch {
    return false;
  }
}

async function augmentTests(
  repoDir: string,
  sourceFiles: string[],
  existingTests: string[],
): Promise<string[]> {
  const set = new Set(existingTests);
  for (const src of sourceFiles) {
    for (const cand of conventionTests(src)) {
      if (set.has(cand)) continue;
      if (await fileExists(repoDir, cand)) set.add(cand);
    }
  }
  return [...set].sort();
}

// --- should_touch_files derivation (schema v3) ---

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function isTestFile(path: string): boolean {
  return (
    path.startsWith("test/") ||
    path.startsWith("tests/") ||
    path.includes("/test/") ||
    path.includes("/tests/") ||
    path.includes("/__tests__/") ||
    path.includes("/spec/") ||
    // KMP multi-target test source sets (per §0.6-G + §4.2 lock-step):
    // androidTest / androidUnitTest / androidInstrumentedTest / commonTest
    // / jvmTest / nativeTest / iosTest / iosArm64Test / linuxX64Test /
    // jsTest / wasmTest etc. Matched by `<setName>Test/` segment + `kotlin/`
    // sub-segment (avoids false-positives like `myTest/` user dirs).
    /\/[a-zA-Z]+Test\/kotlin\//.test(path) ||
    path.endsWith("_test.go") ||
    path.endsWith("_test.rs") ||
    /\.(test|spec)\.(ts|tsx|js|jsx|mjs|cjs)$/.test(path) ||
    /(?:^|\/)test_[^/]+\.py$/.test(path) ||
    /\/[^/]+_test\.py$/.test(path) ||
    // v1.1-M4 — Java/Kotlin/C#/Ruby test conventions.
    /(Test|Tests|Spec|IT)\.java$/.test(path) ||
    // S-002-bench — added IT.kt (integration tests, common in Kotlin/Android).
    /(Test|Tests|Spec|IT)\.kt$/.test(path) ||
    /(Test|Tests)\.cs$/.test(path) ||
    // S-004-bench — Ruby suffix `_spec.rb`/`_test.rb` (RSpec/Minitest)
    // AND Minitest prefix `test_*.rb` (jekyll uses this).
    /(?:^|\/)[^/]+_(spec|test)\.rb$/.test(path) ||
    /(?:^|\/)test_[^/]+\.rb$/.test(path) ||
    // v1.2 S-002 AS-010 — PHP / PHPUnit suffix `*Test.php` / `*Tests.php`.
    /(Test|Tests)\.php$/.test(path)
  );
}

// importGrepSpec — per-language import grep spec for Phase A of deriveShoulTouchFiles.
//
// Returns { pattern, globs } for `git grep -l -E <pattern> <baseCommit> -- <globs>`,
// or null when the language is unsupported (Phase A skipped, falls back to co-change only).
//
// ─── Adding a new language ────────────────────────────────────────────────────
// 1. Add one `case "<lang>":` below returning { pattern, globs }.
// 2. pattern must be an extended-regex (-E) string that matches the import statement
//    as it appears in source files. Use escapeRegex() for dynamic parts.
// 3. globs are file extensions to search (e.g. ["*.java"]).
// 4. If the language has visibility rules (Go: unexported = lowercase first char),
//    add an early-return guard in deriveShoulTouchFiles().
// 5. If entry files named "index.*" are ambiguous in monorepos, use the parent
//    directory name as the identifier (see typescript/javascript cases below).
// 6. Run `bun run scripts/extract-seeds.ts --repo <new-repo>` and spot-check
//    3-5 tasks to verify should_touch_files are real importers, not noise.
//
// v1.1 extension table (not yet implemented — no fixture repos):
//   java/kotlin : `import <pkg>.<ClassName>`, globs: ["*.java"] / ["*.kt"]
//   csharp      : `using <Namespace>`, globs: ["*.cs"]
//   ruby        : `require.*['\"].*<stem>['\"]`, globs: ["*.rb"]
//   swift       : `import <module>` (module name from Package.swift), globs: ["*.swift"]
//
// ─── Design constraint ────────────────────────────────────────────────────────
// Do NOT add symbol-level grep filtering here or downstream. should_touch_files
// prioritizes recall: a file that imports the seed module is in structural blast
// radius even if it doesn't name the changed symbol directly. Symbol-level
// filtering trades recall for precision, which is wrong for this field.
// See docs/guide/uc-impact-dataset-methodology.md for full rationale.
function importGrepSpec(
  seedFile: string,
  lang: string,
  goModContent: string,
  cargoContent: string,
): { pattern: string; globs: string[] } | null {
  const stem = basenameNoExt(seedFile);

  switch (lang) {
    case "python": {
      const mod = seedFile.replace(/\.py$/, "").replace(/\//g, ".");
      const parent = mod.split(".").slice(0, -1).join(".");
      const alts = parent
        ? `from ${escapeRegex(mod)} import|from ${escapeRegex(parent)} import ${escapeRegex(stem)}|import ${escapeRegex(mod)}`
        : `from ${escapeRegex(mod)} import|import ${escapeRegex(mod)}`;
      return { pattern: alts, globs: ["*.py"] };
    }
    case "go": {
      const m = goModContent.match(/^module\s+(\S+)/m);
      if (!m) return null;
      const modRoot = m[1].trim();
      const dir = seedFile.split("/").slice(0, -1).join("/");
      const pkgPath = dir ? `${modRoot}/${dir}` : modRoot;
      // Go imports are exact quoted paths; escape only dots in domain
      return {
        pattern: `"${pkgPath.replace(/\./g, "\\.")}"`,
        globs: ["*.go"],
      };
    }
    case "rust": {
      // Infer crate name: first path component before "/src/" (workspace pattern)
      const crateFromPath = seedFile.match(/^([^/]+)\/src\//)?.[1];
      const crateFromToml = cargoContent.match(/^\[package\][\s\S]*?^\s*name\s*=\s*"([^"]+)"/m)?.[1];
      const crateName = crateFromPath ?? crateFromToml ?? "";
      // Module path: strip leading <crate>/src/ and trailing .rs / /mod.rs
      const modRaw = seedFile
        .replace(/^[^/]+\/src\//, "")
        .replace(/\/mod\.rs$/, "")
        .replace(/\.rs$/, "");
      const modPath =
        modRaw === "lib" || modRaw === "main" ? "" : modRaw.replace(/\//g, "::");
      const pattern = crateName
        ? modPath
          ? `use ${crateName}::${modPath}|use ${crateName}::\\{[^}]*${escapeRegex(stem)}`
          : `use ${crateName}::`
        : `use .*::${escapeRegex(stem)}`;
      return { pattern, globs: ["*.rs"] };
    }
    case "typescript": {
      // Use parent directory name when stem is "index" — avoids matching every
      // file that does `import ... from 'preact'` in a monorepo where
      // jsx-runtime/src/index.ts is unrelated to the package root index.
      const tsStem = stem === "index"
        ? (seedFile.split("/").slice(-2, -1)[0] ?? stem)
        : stem;
      return {
        pattern: `from ['"].*${escapeRegex(tsStem)}['"]|require\\(['"].*${escapeRegex(tsStem)}['"]`,
        globs: ["*.ts", "*.tsx"],
      };
    }
    case "javascript": {
      const jsStem = stem === "index"
        ? (seedFile.split("/").slice(-2, -1)[0] ?? stem)
        : stem;
      return {
        pattern: `from ['"].*${escapeRegex(jsStem)}['"]|require\\(['"].*${escapeRegex(jsStem)}['"]`,
        globs: ["*.js", "*.jsx", "*.mjs", "*.cjs"],
      };
    }
    // v1.1-M4 — Java importGrepSpec.
    // Maven layout: src/main/java/<pkg>/<Class>.java; package = <pkg> with /→.
    // Pattern: `import <pkg>.<Class>` (FQN exact). Wildcard `import <pkg>.*`
    // also matches because the regex anchors on the package prefix.
    case "java": {
      const mainIdx = seedFile.indexOf("src/main/java/");
      const fallbackIdx = seedFile.indexOf("src/");
      let pkgDir: string;
      if (mainIdx >= 0) {
        pkgDir = seedFile.slice(mainIdx + "src/main/java/".length);
      } else if (fallbackIdx >= 0) {
        pkgDir = seedFile.slice(fallbackIdx + "src/".length);
      } else {
        pkgDir = seedFile;
      }
      // Drop the filename, keep package directory chain.
      const pkgPath = pkgDir.split("/").slice(0, -1).join("/");
      if (!pkgPath) return null;
      const className = stem;
      const pkg = pkgPath.replace(/\//g, ".");
      return {
        pattern: `import ${escapeRegex(pkg)}\\.${escapeRegex(className)}|import ${escapeRegex(pkg)}\\.\\*`,
        globs: ["*.java"],
      };
    }
    // v1.1-M4 S-003-bench — C# importGrepSpec.
    // C# `using` imports a NAMESPACE (not per-class like Java/Kotlin).
    // Pattern: `using <Namespace>;` — matches files that pulled this
    // namespace's contents into scope. Less precise than per-class
    // matching but reflects C# semantics: a `using` declares intent to
    // use anything in the namespace. Using-static and using-alias forms
    // also match because they share the `using <prefix>` head.
    //
    // Layout heuristic: strip first source-root prefix (Src/Source/src/lib)
    // and convert remaining directory chain to namespace dots. Real C#
    // namespaces may differ from directory layout (`<RootNamespace>` in
    // .csproj), but for grep purposes the directory chain is a strong
    // proxy — most projects mirror namespace=directory by convention.
    case "csharp": {
      let pkgDir: string | null = null;
      for (const prefix of ["Src/", "Source/", "src/", "lib/"]) {
        const idx = seedFile.indexOf(prefix);
        if (idx >= 0) {
          pkgDir = seedFile.slice(idx + prefix.length);
          break;
        }
      }
      if (!pkgDir) pkgDir = seedFile;
      const pkgPath = pkgDir.split("/").slice(0, -1).join("/");
      if (!pkgPath) return null;
      const namespace = pkgPath.replace(/\//g, ".");
      return {
        pattern: `using ${escapeRegex(namespace)}\\b`,
        globs: ["*.cs"],
      };
    }
    // v1.1-M4 S-002-bench — Kotlin importGrepSpec.
    // Layouts handled:
    //   - Single-target Maven/Gradle: `src/main/kotlin/<pkg>/<File>.kt`
    //   - KMP multi-target: `<module>/src/<setName>Main/kotlin/<pkg>/<File>.kt`
    //     (commonMain, jvmMain, androidMain, iosMain, nativeMain, jsMain, ...)
    // Pattern: `import <pkg>.<Class>` (FQN) OR `import <pkg>.*` (wildcard).
    // Aliased form `import <pkg>.<Class> as <Local>` also matches the prefix.
    case "kotlin": {
      // Try multi-target sourceSet detection first (more specific).
      let pkgDir: string | null = null;
      const kmpMatch = seedFile.match(/(?:^|\/)src\/[a-zA-Z]+Main\/kotlin\/(.+)$/);
      if (kmpMatch) {
        pkgDir = kmpMatch[1];
      } else {
        // Single-target Maven/Gradle.
        const stIdx = seedFile.indexOf("src/main/kotlin/");
        if (stIdx >= 0) {
          pkgDir = seedFile.slice(stIdx + "src/main/kotlin/".length);
        } else {
          // Last-resort: take anything under `src/`.
          const fallback = seedFile.indexOf("src/");
          if (fallback >= 0) {
            pkgDir = seedFile.slice(fallback + "src/".length);
          } else {
            pkgDir = seedFile;
          }
        }
      }
      const pkgPath = pkgDir.split("/").slice(0, -1).join("/");
      if (!pkgPath) return null;
      const className = stem;
      const pkg = pkgPath.replace(/\//g, ".");
      return {
        pattern: `import ${escapeRegex(pkg)}\\.${escapeRegex(className)}|import ${escapeRegex(pkg)}\\.\\*`,
        globs: ["*.kt", "*.kts"],
      };
    }
    // v1.1-M4 S-004-bench — Ruby importGrepSpec.
    // Ruby `require '<stem>'` (load-path) and `require_relative '<rel>'`
    // (relative path). No formal package/namespace — file stem IS the
    // identity binding. jekyll uses `lib/jekyll/<file>.rb` layout;
    // faraday uses `lib/faraday/<file>.rb`. Both `require` patterns key
    // off the file stem.
    //
    // Pattern matches BOTH require forms with the stem in single OR
    // double quotes:
    //   - `require 'jekyll/foo'`           (load-path absolute)
    //   - `require_relative 'foo'`          (relative to current file)
    //   - `require_relative '../foo'`       (parent-dir relative)
    //
    // Stem disambiguation: when seed file is `lib/<gem>.rb` (root entry),
    // stem == gem name → matches every `require '<gem>'` site. For nested
    // files, stem is the bare filename (e.g., `parameter` for
    // `lib/jekyll/reader/parameter.rb`).
    case "ruby": {
      // Strip leading `lib/` if present so the require path matches load-
      // path semantics (`require 'jekyll/foo'` vs file path `lib/jekyll/foo.rb`).
      const requirePath = seedFile.replace(/^lib\//, "").replace(/\.rb$/, "");
      const altStem = stem; // bare filename for require_relative
      // Match `require '<requirePath>'` OR `require_relative '<altStem>'`
      // OR `require_relative '<...path-ending-in-altStem>'`.
      const pattern =
        `require[ ]+['\"]${escapeRegex(requirePath)}['\"]|` +
        `require_relative[ ]+['\"][^'\"]*${escapeRegex(altStem)}['\"]`;
      return { pattern, globs: ["*.rb"] };
    }
    // v1.2 S-001 — PHP importGrepSpec.
    //
    // PHP module identity = namespace declaration (NOT filename — PSR-4
    // maps namespace ↔ directory). Importers cite the class via:
    //   - `use App\Service\UserService;`       (fully-qualified)
    //   - `use App\Service\{UserService, X};`  (group import, PHP 7+)
    //   - `use App\Service\UserService as US;`  (alias)
    //   - `new \App\Service\UserService(...)`  (inline FQN)
    //
    // Heuristic without parsing composer.json's psr-4 map: extract the file
    // stem (class name) + match any `use ... <stem>` site. This sometimes
    // over-matches (different namespaces sharing a class name) but optimises
    // for recall per the design constraint above.
    case "php": {
      // Strip src/ prefix to match Symfony / Composer convention; bare
      // filename is the class name (PSR-4 canonical case).
      const className = stem;
      return {
        pattern: `use[ ]+[^;]*${escapeRegex(className)}[ ;,}]|new[ ]+[^(]*${escapeRegex(className)}\\(`,
        globs: ["*.php"],
      };
    }
    default:
      return null;
  }
}

async function gitGrepImporters(
  repoDir: string,
  spec: { pattern: string; globs: string[] },
  baseCommit: string,
): Promise<string[]> {
  try {
    // git grep exits with 1 when there are no matches — catch silently
    const raw = await $`git -C ${repoDir} grep -l -E ${spec.pattern} ${baseCommit} -- ${spec.globs}`.text();
    return raw
      .split("\n")
      .filter(Boolean)
      .map((line) => {
        // git grep with a tree-ish prefixes each result with "<treeish>:"
        const colonIdx = line.indexOf(":");
        return colonIdx >= 0 ? line.slice(colonIdx + 1) : line;
      });
  } catch {
    return [];
  }
}

/** Returns a count map of files that co-changed with seedFile in the last
 * `lookback` commits before (and including) `baseCommit`. */
async function coChangeCountMap(
  repoDir: string,
  seedFile: string,
  baseCommit: string,
  lookback: number,
): Promise<Map<string, number>> {
  try {
    const shaOut = await $`git -C ${repoDir} log --no-merges -n ${lookback} --format=%H ${baseCommit} -- ${seedFile}`.text();
    const shas = shaOut.split("\n").filter(Boolean);
    if (shas.length === 0) return new Map();

    const counts = new Map<string, number>();
    for (const sha of shas) {
      const filesOut = await $`git -C ${repoDir} diff-tree --no-commit-id -r --name-only ${sha}`.text();
      for (const f of filesOut.split("\n").filter(Boolean)) {
        if (f !== seedFile) counts.set(f, (counts.get(f) ?? 0) + 1);
      }
    }
    return counts;
  } catch {
    return new Map();
  }
}

async function readFileAtCommit(
  repoDir: string,
  path: string,
  commit: string,
): Promise<string> {
  try {
    return await $`git -C ${repoDir} show ${commit}:${path}`.text();
  } catch {
    return "";
  }
}

/** Derive structural blast radius files for a task.
 *
 * Algorithm:
 *   Phase A — static import reverse-lookup: `git grep` at baseCommit for
 *              files that import the seed module (language-specific patterns).
 *   Phase B — co-change cross-validation: files that co-changed with seedFile
 *              ≥2 times in last 100 commits.
 *   Phase C — intersection A∩B (if A non-empty) or B≥3 (fallback).
 *              Filtered: no expected_files, no test files, no build files,
 *              GA-parseable only. Capped at 15.
 */
async function deriveShoulTouchFiles(
  repoDir: string,
  seedFile: string,
  seedSymbol: string,
  lang: string,
  baseCommit: string,
  sourceFiles: string[],
  goModContent: string,
  cargoContent: string,
): Promise<string[]> {
  // Go unexported symbols (lowercase first char) cannot be referenced outside
  // their own package — skip Phase A entirely to avoid package-import false positives.
  if (lang === "go" && /^[a-z]/.test(seedSymbol)) {
    return [];
  }

  const spec = importGrepSpec(seedFile, lang, goModContent, cargoContent);
  // rawImporterFiles = Phase A result before symbol filter; preserved to distinguish
  // "Phase A found nothing" (→ coChange3 fallback) from "Phase A2 filtered everything
  // out" (→ empty result, not noise fallback).
  const rawImporterFiles = spec
    ? await gitGrepImporters(repoDir, spec, baseCommit)
    : [];
  let importerFiles = rawImporterFiles;

  const coMap = await coChangeCountMap(repoDir, seedFile, baseCommit, 100);
  const coChange2 = new Set(
    [...coMap.entries()].filter(([, c]) => c >= 2).map(([f]) => f),
  );
  const coChange3 = new Set(
    [...coMap.entries()].filter(([, c]) => c >= 3).map(([f]) => f),
  );

  // Phase C — intersection + fallback.
  // Phase A2 (symbol-level filter) is intentionally NOT applied here — blast
  // radius should prioritize recall over precision. A file that imports the
  // seed module is structurally coupled regardless of whether it spells the
  // changed symbol by name (indirect deps, re-exports, subclassing all count).
  // Co-change cross-validation (Phase B) is the noise filter, not symbol grep.
  let candidates: Set<string>;
  if (rawImporterFiles.length > 0) {
    // Phase A found importers — intersect with co-change ≥2.
    // If A2 filtered everything, candidates will be empty (no fallback).
    candidates = new Set(importerFiles.filter((f) => coChange2.has(f)));
  } else {
    // Phase A found no importers — fall back to stronger co-change signal (≥3).
    candidates = coChange3;
  }

  const sourceSet = new Set(sourceFiles);
  return [...candidates]
    .filter((f) => !sourceSet.has(f))
    .filter((f) => !isTestFile(f))
    .filter((f) => !isBuildFile(f))
    .filter((f) => isGaParseable(f))
    .sort()
    .slice(0, 15);
}

// --- Main ---
//
// Main-guard added 2026-04-26 (S-003-bench follow-up): prior versions had
// no guard, so `import { ... } from "./extract-seeds"` (used by the
// regression test file) executed top-level CLI logic + wiped tasks.json
// files in passing. Bun follows Node's `import.meta.main` contract.
if (!import.meta.main) {
  // Imported as a module — skip CLI execution. Test files re-import
  // helper functions only.
} else {

const args = process.argv.slice(2);
let onlyRepo: string | null = null;
for (let i = 0; i < args.length; i++) {
  if (args[i] === "--repo" && args[i + 1]) {
    onlyRepo = args[i + 1];
    i++;
  }
}

const reposToProcess = onlyRepo ? REPOS.filter((r) => r === onlyRepo) : [...REPOS];

for (const repo of reposToProcess) {
  const inPath = join(RAW_DIR, `${repo}-candidates.json`);
  const raw = JSON.parse(readFileSync(inPath, "utf8")) as {
    repo: string;
    lang: string;
    candidates: Candidate[];
  };
  const repoDir = join(FIXTURES_DIR, repo);

  console.log(`\n[${repo}] enriching ${raw.candidates.length} candidates`);

  const tasks: EnrichedTask[] = [];
  let rejectBuild = 0;
  let rejectSeed = 0;
  for (const cand of raw.candidates) {
    // Strip build files AND files GA can't parse (YAML/TOML/INI/MD/etc.).
    // A retriever can't be scored on files it fundamentally can't index.
    const cleanSources = cand.source_files.filter(
      (f) => !isBuildFile(f) && isGaParseable(f),
    );
    if (cleanSources.length === 0) {
      rejectBuild++;
      continue;
    }
    // Seed file must also be GA-parseable (enforced transitively via above)
    // — if only build/yaml files changed, task is rejected.

    const seed = await pickSeed(repoDir, cand.sha, cleanSources, raw.lang as Lang);
    if (!seed) {
      rejectSeed++;
      continue;
    }

    // expected_tests = ONLY tests changed in the same fix commit. Convention
    // match dropped — audit showed Django/Nest test paths don't follow
    // predictable patterns (tests/<feature>/tests.py style), penalizing
    // any convention-based retriever unfairly.
    const expected_tests = [...cand.test_files].sort();

    // Derive structural blast radius (schema v3) — Phase A + B + C.
    // Read manifest files at base_commit via git show (no checkout).
    const goModContent =
      raw.lang === "go"
        ? await readFileAtCommit(repoDir, "go.mod", cand.parent_sha)
        : "";
    const cargoContent =
      raw.lang === "rust"
        ? (await readFileAtCommit(
            repoDir,
            `${seed.seed_file.split("/")[0]}/Cargo.toml`,
            cand.parent_sha,
          )) || (await readFileAtCommit(repoDir, "Cargo.toml", cand.parent_sha))
        : "";
    const should_touch_files = await deriveShoulTouchFiles(
      repoDir,
      seed.seed_file,
      seed.seed_symbol,
      raw.lang,
      cand.parent_sha,
      cleanSources,
      goModContent,
      cargoContent,
    );

    tasks.push({
      task_id: `${repo}-${cand.sha.slice(0, 8)}`,
      repo,
      base_commit: cand.parent_sha,
      fix_commit: cand.sha,
      subject: cand.subject,
      seed_file: seed.seed_file,
      seed_symbol: seed.seed_symbol,
      source_files: cleanSources,
      expected_files: cleanSources,
      expected_tests,
      should_touch_files,
      max_expected_depth: cleanSources.length === 1 ? 1 : 2,
    });
  }
  console.log(
    `  accepted=${tasks.length}/${raw.candidates.length}  rejects: build=${rejectBuild} seed=${rejectSeed}`,
  );

  const outPath = join(RAW_DIR, `${repo}-tasks.json`);
  writeFileSync(
    outPath,
    JSON.stringify(
      {
        repo,
        lang: raw.lang,
        enriched_at: new Date().toISOString(),
        count: tasks.length,
        tasks,
      },
      null,
      2,
    ),
  );
  console.log(`  wrote ${outPath} (${tasks.length} tasks)`);
}

console.log("\nDone. Next: bun run scripts/consolidate-gt.ts (Phase A3)");

} // end main-guard else block
