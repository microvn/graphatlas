// Story C — mining filter tightening: subject blocklist + diff-based gate.
// Driver: M3 minimal_context audit found 3 noise commits passing the
// /fix|fixes|bug/i subject filter:
//   - "fmt: run 'cargo fmt --all'"             (regex-a2a393f1)
//   - "chore: fix some minor typos"            (tokio-a1ee3ef2)
//   - "Grammar: Fix 'it's' vs 'its'"           (axum-934b1aac)
// All produced wide-touch GT with no static-graph relationship → low
// recall = dataset noise, not engine weakness.
//
// /mf-voices Round 4 consensus (Codex + Claude Haiku):
//   - Subject blocklist is first pass; diff-based filter is the gate.
//   - Drop high-file-count commits (fmt sweeps).
//   - Drop whitespace-only / comment-only diffs.

import { test, expect } from "bun:test";
import { isNoiseSubject, isWhitespaceOnlyDiff } from "./mine-fix-commits";

test("blocks chore: fix typos commits", () => {
  expect(isNoiseSubject("chore: fix some minor typos in the comments (#7442)")).toBe(true);
  expect(isNoiseSubject("chore: fix minor typos (#7804)")).toBe(true);
  expect(isNoiseSubject("Chore: Fix typo in comment")).toBe(true);
});

test("blocks fmt sweep commits", () => {
  expect(isNoiseSubject("fmt: run 'cargo fmt --all'")).toBe(true);
  expect(isNoiseSubject("style: cargo fmt")).toBe(true);
  expect(isNoiseSubject("Run rustfmt across workspace")).toBe(true);
});

test("blocks docs prefix commits", () => {
  expect(isNoiseSubject("docs: fix typo in README")).toBe(true);
  expect(isNoiseSubject("doc: fix grammar")).toBe(true);
});

test("blocks grammar/spelling commits", () => {
  expect(isNoiseSubject("Grammar: Fix 'it's' vs 'its' in several places (#2518)")).toBe(true);
  expect(isNoiseSubject("Fix spelling mistakes")).toBe(true);
});

test("ALLOWS real bug fix commits", () => {
  expect(
    isNoiseSubject(
      "automata: fix `onepass::DFA::try_search_slots` panic when too many slots are given",
    ),
  ).toBe(false);
  expect(isNoiseSubject("sync: fix `CancellationToken` failing to cancel (#7462)")).toBe(false);
  expect(isNoiseSubject("security: fix denial-of-service bug in compiler")).toBe(false);
  expect(isNoiseSubject("api: impl Default for RegexSet")).toBe(false);
});

test("ALLOWS commits where 'fix' is in real-fix context", () => {
  // Edge: subject says "fix" but it's a real bug fix, not chore.
  expect(isNoiseSubject("fix: prevent panic on empty input")).toBe(false);
  expect(isNoiseSubject("Fixed #36973: clash detection for managers")).toBe(false);
});

// Story C v3 — block lint cleanup commits. They mention "Fix" but are
// not code-bug fixes; mining picks them, then GT contains every file
// touched by the lint sweep, polluting recall.
test("blocks 'Fix <X> warning' / 'Fix <X> lint' commits", () => {
  expect(
    isNoiseSubject("Fix items-after-test-module clippy warning on 1.75.0-beta.1 (#2318)"),
  ).toBe(true);
  expect(isNoiseSubject("Fix unused-import warning")).toBe(true);
  expect(isNoiseSubject("Fix dead-code lint")).toBe(true);
  expect(isNoiseSubject("fix clippy warnings in tests")).toBe(true);
});

test("blocks 'Remove <X>' commits with body containing fix:", () => {
  // Edge: subject is a feature ("Remove ContentLengthLimit") but body
  // has many "fix: typo" subcommits that match `git log --grep=fix`.
  // Subject-level filter must catch this — body grep is too loose.
  expect(isNoiseSubject("Remove `ContentLengthLimit` (#1400)")).toBe(false);
  // ↑ Subject alone is fine — we'll filter via subject-only grep, not blocklist.
  // The fix is in the mining loop (use --grep on subject only), not here.
});

test("ALLOWS legit warning-related fixes that happen to mention warning", () => {
  // Don't false-positive on real fixes that mention warnings as side effect.
  expect(
    isNoiseSubject("fix: missing newline causes deprecation warning"),
  ).toBe(false);
});

test("isWhitespaceOnlyDiff detects pure-whitespace patches", () => {
  // Empty diff
  expect(isWhitespaceOnlyDiff("")).toBe(true);
  // Diff with only header + whitespace adds/removes
  const wsDiff = `diff --git a/foo.rs b/foo.rs
index abc..def 100644
--- a/foo.rs
+++ b/foo.rs
@@ -1,3 +1,3 @@
 fn foo() {
-    bar()
+    bar()
 }
`;
  expect(isWhitespaceOnlyDiff(wsDiff)).toBe(true);
});

test("isWhitespaceOnlyDiff returns false on real code change", () => {
  const realDiff = `diff --git a/foo.rs b/foo.rs
@@ -1,3 +1,3 @@
 fn foo() {
-    return 42;
+    return 0;
 }
`;
  expect(isWhitespaceOnlyDiff(realDiff)).toBe(false);
});
