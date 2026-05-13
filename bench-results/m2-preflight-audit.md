# M2-00 Pre-Flight Audit

**Run date:** 2026-04-24
**Dataset:** dev split, 30 tasks, 5 repos (axum, django, gin, nest, preact)
**Purpose:** Rule out confounders before spending M2-01..M2-08 EXP budget.

---

## Item 1 — Seed extraction audit

**Verified:** `crates/ga-bench/src/retrievers/ga.rs:147-170` (`impact_request_from_query`) passes `query["symbol"]` as-is into `ImpactRequest.symbol`. NOT file path.

```rust
symbol: query.get("symbol").and_then(|v| v.as_str()).map(str::to_string),
```

Bench ground truth at `benches/uc-impact/ground-truth.json` provides task-level `symbol` field directly (no transformation). Seed is the resolved callee symbol from git-mining.

**Status:** ✅ No issue. All 8 downstream EXPs use the correct seed.

---

## Item 2 — TESTED_BY edge density per fixture

Ran `crates/ga-query/tests/m2_preflight_edges.rs` (new, `#[ignore]`-gated, `GA_M2_EDGE_AUDIT=1`). Builds fresh index per fixture and queries `MATCH ()-[t:TESTED_BY]->() RETURN count(t)`.

| Fixture | Lang | TESTED_BY | REFERENCES | CALLS | TESTED_BY/CALLS | Build (s) |
|---------|------|----------:|-----------:|------:|----------------:|----------:|
| gin     | go      |  1,855 |     0 |   5,719 | 32.4% | 0.8 |
| axum    | rust    |    684 |     0 |   5,363 | 12.8% | 0.9 |
| preact  | ts/js   |    284 |    63 |   2,031 | 14.0% | 1.3 |
| nest    | ts/js   |    405 |   360 |   7,825 |  5.2% | 2.5 |
| django  | python  | 32,614 | 1,219 | 114,880 | 28.4% | 12.0 |

**Key findings for M2-05 (directed TESTED_BY chain `*1..3`):**
- **TESTED_BY density healthy across ALL 5 languages** — worry "Go/Rust might have 0 TESTED_BY" is **not true**. gin has 1,855 and axum has 684. M2-05 chain Cypher will move test_recall on every language.
- Biggest absolute lift expected on **django** (32k edges) and **gin** (2k).
- Smallest surface: **preact** (284) — and preact test-file parse is partial (see Item 5b).

---

## Item 3 — REFERENCES edge density per fixture

| Fixture | Lang | REFERENCES |
|---------|------|-----------:|
| axum    | rust    | **0** |
| gin     | go      | **0** |
| preact  | ts/js   |    63 |
| nest    | ts/js   |   360 |
| django  | python  | 1,219 |

**Implication:** BFS dùng `CALLS ∪ REFERENCES`. Với Go/Rust, REFERENCES không đóng góp gì → BFS recall only as strong as CALLS extraction on those langs. Future EXP adding REFERENCES-dependent signal (e.g. callback-based test discovery) sẽ không move Go/Rust. Note là separate ga-parser limitation (Foundation-C15 extension), không phải M2 scope.

---

## Item 4 — Baseline variance (composite determinism)

| Run | composite | test_recall | completeness | depth_F1 | precision | p95 ms | runtime |
|-----|----------:|------------:|-------------:|---------:|----------:|-------:|--------:|
| 1   |     0.542 |       0.404 |        0.756 |    0.900 |     0.126 | 8134   | 57.8s |
| 2   |     0.542 |       0.404 |        0.756 |    0.900 |     0.126 | 7473   | 58.1s |
| 3   |     0.542 |       0.404 |        0.756 |    0.900 |     0.126 | 8329   | 57.0s |
| **median-of-3** | **0.542** | **0.404** | **0.756** | **0.900** | **0.126** | **8134** | — |

**Finding:** Composite metrics **deterministic** (run1 == run2 identical byte-for-byte). Ground truth + indexing + query pipeline is reproducible.

Only **p95 latency varies** (661ms range observed) — inherent to wall-clock measurement, not algorithm non-determinism.

**Gate design impact:**
- ✅ Composite-dim gates (test_recall, precision, etc.) can use threshold `Δ ≥ +X` directly — no median-of-3 needed.
- ⚠️ p95 latency gates need median-of-3 OR accept ±700ms tolerance. Updated gate wording: "p95 giảm ≥ X" → "median-of-3 p95 giảm ≥ X".

---

## Item 5 — `is_test_path` heuristic quality

**File:** `crates/ga-query/src/common.rs:22-67`

### 5a. Per-language conventions (verified)

| Lang | Patterns matched |
|---|---|
| Python | `test_*.py`, `*_test.py`, any `tests\|test\|__tests__\|` segment |
| Go | `*_test.go` |
| Rust | `*_test.rs`, any `tests\|test` segment |
| TS/JS | `*.test.{ts,tsx,js,jsx,mjs,cjs}`, `*.spec.*`, any `tests\|test\|__tests__` segment |

### 5b. EXPERIMENTS.md EXP-013 false-positive check

TS-side bug (EXP-013) was `/test_/` regex matching `test_flame_game.dart` as test. Rust-side check:
- `is_test_path("test_flame_game.dart")` → falls through all extension branches (no `.dart` handler) → returns `false`. ✅ Safe.
- `is_test_path("src/testutil.ts")` → `testutil.ts` strips to `testutil`, doesn't end with `.test` or `.spec`, not in `tests/` dir → `false`. ✅ Safe.

### 5c. Parser reliability on test files (new concern surfaced during audit)

Edge density audit logged tree-sitter parse warnings:
- **preact**: 19 test files with 1–310 syntax errors each. `test/ts/dom-attributes.test-d.tsx` = 310 errors (type-declaration test pattern).
- **django**: `tests/test_runner_apps/tagged/tests_syntax_error.py` intentionally malformed (test fixture).
- **nest**: 2 `.spec.ts` files with 5 syntax errors each.

**Implication:** When parser fails, Symbol extraction gives partial results → TESTED_BY emission misses those edges. Preact's 284 TESTED_BY is an **undercount**. Not a M2 blocker but flag as ga-parser bug (separate ticket): `.test-d.tsx` declaration-only syntax not fully supported.

### 5d. Coverage gaps (corpus-irrelevant but noted)

Missing language conventions in `is_test_path`:
- Dart: no handling → returns false for any `.dart` file (safe, not false-positive)
- Java/Kotlin: no handling → `HpackTest.kt` wouldn't be detected (EXPERIMENTS.md EXP-008 TS fix). Not relevant to M2 corpus, but note for v1.1.
- C#, Ruby: same — out of M2 corpus.

**Status:** ✅ No false-positive risk on M2 corpus. Heuristic correct for all 5 fixtures.

---

## Pre-flight gate decisions

| Gate | Status | Note |
|---|---|---|
| Seed extraction correct | ✅ | Bench → `ImpactRequest.symbol` = task symbol name |
| TESTED_BY ≥ 3/5 languages | ✅ | All 5 populated (284 to 32,614 edges) |
| Baseline std ≤ 0.005 | ✅ | Composite deterministic (std = 0), latency variable |
| is_test_path correct on corpus | ✅ | No false-positive; parser bugs are orthogonal |

**Verdict:** All 8 downstream EXPs (M2-05/06/07/08/01/02/03/04) may proceed. Revised projection table in plan remains valid.

---

## Side-findings to track (not M2-blocking)

1. **Parser bug (preact `.test-d.tsx`)** — ga-parser chokes on TSX type-declaration tests. Separate ticket. Impacts preact `test_recall` ceiling but not within M2 fix scope.
2. **REFERENCES=0 on rust/go** — Foundation-C15 extension needed to emit value-references on these langs. v1.1 work.
3. **p95 latency variance ±700ms** — wall-clock noise. M2-01/02/03/04 gates now explicitly require median-of-3 p95.

---

## Artifacts

- Audit test: `crates/ga-query/tests/m2_preflight_edges.rs` (#[ignore], GA_M2_EDGE_AUDIT=1)
- Raw edge counts: `bench-results/m2-edge-density.md`
- Baseline run logs: `/tmp/m2-baseline-run{1,2,3}.log`
