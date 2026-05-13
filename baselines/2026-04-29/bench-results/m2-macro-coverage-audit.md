# M2 Macro Coverage Audit — Phase 0

> Read-only classification audit. 30 dev tasks × 10 macros (M1-M10 per
> `docs/IMPACT_MACRO_STRATEGY.md`). Output: shortlist of macros worth
> validate-before-code harness in Phase 1.

**Date:** 2026-04-24
**Dataset:** `benches/uc-impact/ground-truth.json` (30 dev tasks, 6/repo × 5 repos)
**Current GA composite:** 0.601 (post-M2-05, baseline pre-macro work)
**Classification source:** per-task subject + expected_files + expected_tests + seed symbol; GA-pipeline predicted capture based on audit of `crates/ga-query/src/impact/` (2026-04-24)

## Capture prediction legend

For each GT file, predicted capture by current GA pipeline:

- `B` — BFS CALLS/REFERENCES in/out catches it
- `C` — TESTED_BY `*1..3` chain (M2-05)
- `P` — path_mentions convention (seed name or stem in path)
- `R` — routes.rs (5 frameworks)
- `F` — text_filter seed-as-token post-intersect
- `G` — configs.rs (yaml/toml/env/json)
- `—` — current pipeline has no handler → **macro gap**

## Per-task classification (30 tasks)

### Django (6 tasks)

| # | Task | Subject | GT | Macro | Predicted capture | Gap |
|:-:|---|---|---|---|---|---|
| 1 | django-26f8929f | `MultipleChoiceField` optimize validate() | 1 file (seed), 1 test | M2 + M10 | seed-file: B; test: P (stem match) | none |
| 2 | django-3157285e | `File` truthy default | 3 files (base, uploadedfile, fields/files), 1 test | **M3 class hierarchy** + M2 + M10 | base.py: B; uploadedfile.py: ? (inherits File); fields/files.py: B (imports File); test: P | **M3 gap**: EXTENDS not walked — UploadedFile isa File |
| 3 | django-33bfc66a | XML Serializer | 1 file + 1 test | M2 + M3 Serializer hierarchy + M10 | seed-file: B; test: P (stem match) | M3 latent |
| 4 | django-3b161e60 | `When()` invalid args → `Q()` | 3 files (expressions, query, query_utils) + 1 test | M2 cross-module | expressions: B; query: B (calls When); query_utils: B?; test: P (convention) | none likely |
| 5 | django-673fa46d | `check_trac_ticket` PR script | 2 files (scripts/pr/*) + 1 test | M2 simple | check_pr: B; errors.py: B (imports); test: P | none |
| 6 | django-820c7d32 | `DeferredAttribute.fetch_many()` | 1 file + 2 tests (defer, many_to_one) | M2 + M10 | seed: B; defer/tests: P (stem); many_to_one/tests: **— no stem match, no TESTED_BY chain likely** | **M10 cross-module test gap** |

### Preact (6 tasks)

| # | Task | Subject | GT | Macro | Predicted capture | Gap |
|:-:|---|---|---|---|---|---|
| 7 | preact-005d5081 | `shallowDiffers` util port | 1 file + 1 test (render.test.js) | M2 + M10 | seed-file: B; test: **— path "render" no "shallowDiffers"** | **M10 gap** |
| 8 | preact-21dd6d04 | v10 forwardport (7 files, 6 tests) | cross-module hooks/compat | **M2 + M8 component prop** | compat/*.js: B partial; hooks/src/index: B?; diff/index: B; tests cross-module: mixed P/miss | **M8 gap + M10 cross-module** |
| 9 | preact-29b0021a | `detachedClone` suspense | 1 file + 1 test (suspense.test.js) | M2 + M10 | seed: B; test: P (stem "suspense") | none |
| 10 | preact-2d6811de | `oldAfterDiff` hooks revert | 4 files + 2 tests | M2 cross-module + **M5 event** (hook lifecycle) | render/hooks/diff: B; constants.js: **—**; tests: P partial | **M5 + cross-module gap** |
| 11 | preact-4a06d3fb | `diff` cascading signals | 1 file + 1 test (useState.test.jsx) | M2 + M10 | seed: B; test: **— path "useState" no "diff"** | **M10 gap (generic seed)** |
| 12 | preact-5a029235 | `constructNewChildrenArray` | 1 file + 1 test (keys.test.jsx) | M2 + M10 | seed: B; test: **— path "keys" no seed** | **M10 gap** |

### Nest (6 tasks)

| # | Task | Subject | GT | Macro | Predicted capture | Gap |
|:-:|---|---|---|---|---|---|
| 13 | nest-0eb2340e | `BaseExceptionFilter` FastifyError | 1 file + 1 test (exceptions-handler.spec) | M2 + M10 + **M1 Convention-pair (.spec.ts)** | seed: B; test: P partial ("exception") | minor |
| 14 | nest-1c43374d | `BaseExceptionFilter` isHttpError | 1 file + 1 test | same | same | minor |
| 15 | nest-298857e2 | `JsonSocket` microservices | 1 file + 2 tests (stem match both) | M2 + M10 | seed: B; tests: P (stem "json-socket") | none |
| 16 | nest-368691c3 | `Injector` DI hang | 1 file + 1 test (injector.spec) | M2 + M10 | seed: B; test: P exact | none |
| 17 | nest-3b47081e | `IoAdapter` socket.io | 1 file + 1 test | same | same | none |
| 18 | nest-457630a6 | `RouterResponseController` SSE | **4 files** (app.controller, router-exec-ctx, sse-stream, self) + **5 tests** (sse/e2e express+fastify, 3 spec) | **M1 + M4 DI + M7 route + M10** | seed: B; router-exec-ctx: B (probably); sse-stream: P (stem "sse")?; **app.controller.ts in integration/.../sse/src/**: **— cross-package, no graph edge**; e2e tests: P partial | **M1 + M4 gap — big blast radius miss** |

### Gin (6 tasks)

| # | Task | Subject | GT | Macro | Predicted capture | Gap |
|:-:|---|---|---|---|---|---|
| 19 | gin-19b877fa | `getMinVer` debug test coverage | 1 file + 1 test | M2 + M10 (Go `_test.go`) | seed: B; test: P (stem "debug") | none |
| 20 | gin-234a6d4c | `Written` response_writer | 1 file + 1 test | same | none | none |
| 21 | gin-32065bbd | `Written` hijack panic prevention | same as 20 | — | — | none |
| 22 | gin-40725d85 | `BindUri` → 413 MaxBytesError | **5 files** (context + 4 internal/json impls: go_json, json, jsoniter, sonic) + 1 test | **M3 Go interface dispatch** + M2 | context: B; json impls: **— Go interface is implicit, EXTENDS=[] on Go, BFS CALLS won't reach via interface** | **M3 gap CRITICAL (Go interface)** |
| 23 | gin-472d086a | `walk` tree panic | 1 file + 1 test | M2 + M10 | seed: B; test: P exact | none |
| 24 | gin-5c00df8a | `Data` render content length | 1 file + 1 test (render/render_test.go) | M2 + M10 | seed: B; test: P (stem "render") | none |

### Axum (6 tasks)

| # | Task | Subject | GT | Macro | Predicted capture | Gap |
|:-:|---|---|---|---|---|---|
| 25 | axum-2e8a7e51 | `Handler::with_state` layer body | 2 files (handler/mod, examples/key-value-store) + 3 tests (.stderr compile-fail!) | **M3 trait dispatch** + M2 + M10 | handler: B (seed); examples: B (usage); tests `.stderr`: **— is_test_path may not match `.stderr`; path-mentions miss "via", "generic_without_via"** | **M3 trait + unusual test files (.stderr)** |
| 26 | axum-34d1fbc0 | Typo fix (cross-file sweep) | **8 files** across axum-extra (cookie/*, multipart, handler, routing, sse, method_routing) + 1 test | **M1 sibling folder (cookie/*)** + M3 trait | seed-file: B; siblings `cookie/private.rs, cookie/signed.rs`: **— sibling to cookie/mod.rs but no CALLS edge necessarily**; cross-file (multipart, handler, routing, sse, method_routing): B (typo means text mention only — text_filter unreliable on typo) | **M1 gap (folder siblings)** |
| 27 | axum-4847d681 | `RouteId` inherit state | **11 files** cross-crate (routing/mod, service, route, handler, matched_path, request_parts, boxed + spa, method_routing, 2 examples) + 4 tests | **M3 trait (Handler/Service)** + M1 (routing/*) + M2 + M7 route | routing/mod: B (seed); sibling routing/*.rs: **— M1 miss**; service.rs, route.rs: B (likely); examples: B; tests in routing/tests/*: P (stem "routing")? | **M1 + M3 trait** |
| 28 | axum-568394a2 | `debug_handler` macro expand | 2 files (debug_handler, lib) + 6 tests (pass/fail stderr) | M2 (macro internal) + M10 | seed: B; lib: B (re-exports); tests: P (stem "debug_handler") | likely catch |
| 29 | axum-68696b09 | `check_inputs_impls_from_request` unreachable code | 1 file + 1 test (deny_unreachable_code.rs) | M2 + M10 | seed: B; test: **— path no stem match** | **M10 gap** |
| 30 | axum-74eac39e | `expand_field` FromRef typo warning | 1 file + 1 test (json_not_deserialize.stderr!) | M2 + M10 | seed: B; test: **— path "json_not_deserialize" completely unrelated to `from_ref`** | **M10 gap (unusual GT)** + probable GT noise |

## Per-macro tabulation

| Macro | Task count (any) | Tasks with gap (GA miss likely) | Task IDs where gap critical |
|:-:|:-:|:-:|---|
| M1 Convention-pair folder sibling | ~5 | **3-4** | axum-34d1fbc0, axum-4847d681, nest-457630a6, (nest-457630a6) |
| M2 Signature / CALLS | 30 (all) | 0 | — (baseline coverage) |
| M3 Interface/trait dispatch | 6 | **5** | **gin-40725d85 (Go impls)**, axum-2e8a7e51, axum-4847d681, django-3157285e (File subclass), django-33bfc66a (Serializer), axum-34d1fbc0 |
| M4 DI/IoC registration | 1-2 | 1 | nest-457630a6 (app.controller in integration) |
| M5 Event/signal | 1 | 1 | preact-2d6811de (hook lifecycle) |
| M6 ORM schema co-edit | **0** | 0 | — (dev corpus has no ORM migration tasks) |
| M7 Route binding | 2 | 0-1 | axum-4847d681 (RouteId), nest-457630a6 |
| M8 Component prop | 1 | 1 | preact-21dd6d04 |
| M9 Config | 0 | 0 | — |
| M10 Test co-location | 30 (all) | **7** | django-820c7d32 (many_to_one), preact-005d5081, preact-4a06d3fb, preact-5a029235, axum-68696b09, axum-74eac39e + edge cases |

## Findings

### 1. M10 test convention miss is the single biggest gap (7/30 tasks)

7 tasks where `path_mentions(test_path, seed, stems)` fails because:
- Generic seeds (`diff`, `File`) — EXP-M2-06 already reverted this (fuzzy name = 0 signal)
- Unusual test folder structure (`many_to_one/tests.py` tests cross-module code)
- Compile-fail tests (`.stderr` files, `deny_unreachable_code.rs`) — test path semantic not match seed name

**BUT**: 7 M10 misses break into 3 clusters:
- (a) Cross-module test folder (`django-820c7d32`, `preact-4a06d3fb`, `preact-5a029235`, `axum-68696b09`): test file tests code in different-named module. **These are TESTED_BY edge gaps** — if indexer had emitted TESTED_BY, chain would catch.
- (b) Unusual compile-fail tests (`.stderr`): axum-74eac39e, some of axum-2e8a7e51. **These may be GT noise** (not actionable retrieval targets).
- (c) Generic seed (`diff`): **no clean fix** — fuzzy match proven net-noise (EXP-M2-06/07).

**Actionable M10 subset**: ~3-4 tasks if TESTED_BY edges densify.

### 2. M3 interface/trait is the single biggest structural gap (5/30 critical)

- **Go interface (gin-40725d85)**: 4 impl files missed — EXTENDS=[] on Go parser. Would need Go-specific interface-satisfaction extraction (implicit, non-trivial).
- **Rust trait (axum-2e8a7e51, axum-4847d681, axum-34d1fbc0)**: EXTENDS populated for `impl_item` per parser audit. **But BFS doesn't walk EXTENDS.** Single-file impl catch-able via BFS walk extension.
- **Python class (django-3157285e, django-33bfc66a)**: EXTENDS populated. Same as Rust — BFS extension needed.

**Actionable M3 subset**: 3 tasks immediately via EXTENDS walk on already-populated edges (Rust/Python). Go needs parser work (out of scope M2).

### 3. M1 convention-pair folder sibling (3-4 tasks)

- **axum-34d1fbc0**: `cookie/mod.rs + cookie/private.rs + cookie/signed.rs` — Rust module folder. Resolver: "if seed in `X/mod.rs`, add `X/*.rs` siblings".
- **axum-4847d681**: `routing/mod.rs + routing/{route,service,method_routing}.rs` — same pattern.
- **nest-457630a6**: `router-response-controller.ts + router-execution-context.ts + sse-stream.ts` — Nest internal module siblings (no `.controller/.service` naming here — more fuzzy).

**Actionable M1 subset**: 2 tasks via Rust module-folder sibling rule. Nest siblings fuzzier — depends on file proximity rule.

### 4. M6 ORM = 0 in dev corpus

Dev split has no migration+model co-edit pattern. `co_change.rs` infrastructure unused for M6 on this corpus. Doesn't mean M6 useless — means **dev split isn't representative for M6 validation**. Would need test split re-audit.

### 5. M4 (DI), M5 (event), M8 (prop) are each 1 task — low ROI solo

Single-task macros not worth EXP unless piggy-backed. M4 nest-457630a6 overlaps with M1 (same task). M5 preact-2d6811de cross-module — likely caught by M2 BFS partial.

## Priority ranking for Phase 1 harness

Per `IMPACT_MACRO_STRATEGY.md §Per-macro priority rule`: (1) task count missed, (2) tech complexity (fs < graph < AST), (3) corpus coverage.

| Rank | Macro | Candidate | Tasks | Tech | Scope | ROI hypothesis |
|:-:|:-:|---|:-:|---|---|---|
| **1** | **M3 EXTENDS walk** | H-M3 | 3 high + 2 latent | graph (existing edge) | Rust + Python | +~2 tasks composite move; same mechanism as M2-05 (add-to-set) |
| **2** | **M1 folder-sibling** | H-M1 | 2-3 | filesystem | Rust modules primarily | +~1-2 tasks; cheap fs pattern; no graph/AST change |
| **3** | **TESTED_BY densification** | H-M10-extend | 3-4 (subset of M10 7) | indexer change | Python + JS | Requires parser work — higher cost; M2-05 already partial |
| **4** | M6 co-change wire | defer | 0 dev, unknown test | git | all langs | Re-audit test split first |

## Phase 1 harness proposal

### H-M3 (highest ROI)

`crates/ga-bench/tests/m2_extends_harness.rs` — for each dev task:

1. Query GA current output (files).
2. Run EXTENDS-walk candidate: from seed symbol, traverse `(base)<-[:EXTENDS]-(derived)-[:CONTAINS]->(override)` + reverse; union resulting files.
3. Classify:
   - **Signal** = files ∈ EXTENDS-walk ∩ expected_files \ GA-current-output
   - **Noise** = files ∈ EXTENDS-walk \ expected_files \ GA-current-output
   - Gate: **signal:noise ≥ 1:5** (M2-07 pattern, strict)

Predict (from classification above):
- Signal: ~5-7 GT files across 3-4 tasks (django-3157285e uploadedfile.py; axum-2e8a7e51 handler impls; axum-4847d681 route impls; axum-34d1fbc0 cookie impls)
- Noise: depends on trait-impl fan-out in axum — need empirical measurement

Go on this corpus = 0 signal (EXTENDS=[]). Harness should report per-lang signal:noise separately.

### H-M1 (cheap to run)

`crates/ga-bench/tests/m2_convention_pair_harness.rs`:

1. For each dev task, resolve folder-siblings of seed_file:
   - Rust: if seed in `path/{mod.rs,mod.rs-equivalent}`, add `path/*.rs` siblings.
   - TypeScript Nest: if seed in `*.controller.ts`, add `*.{service,module,dto,entity,guard,spec}.ts` in same dir.
   - Python Django: if seed in `views.py`, add sibling `{models,serializers,admin,urls,tests,forms}.py` in same app.
2. Same signal/noise classification + 1:5 gate.

Predict:
- Signal: ~4-6 files across 2-3 tasks (axum-34d1fbc0 cookie/*, axum-4847d681 routing/*)
- Noise: Rust module folders usually small (<5 files), low fan-out → promising

## Next step

Execute H-M3 + H-M1 harnesses in parallel (both read-only validate-before-code).
After gates:
- Both green → EXP-M2-09 (M3) + EXP-M2-10 (M1), TDD per `m2-gate-plan.md` protocol.
- Only M3 green → EXP-M2-09 only; re-evaluate M1 with broader framework detect.
- Both red → escalate per `IMPACT_MACRO_STRATEGY.md §Exit criteria`: composite stall → semantic re-rank spike.

## Re-audit against blast_radius + adj_prec (2026-04-24 update)

Two supplementary metrics **not in composite** but **central to LLM-agent utility**:

- **`blast_radius_coverage`** = `recall(should_touch_files, actual_files)`. GA = 0.451 (rank 4/7, below bm25 0.768, CRG 0.625, ripgrep 0.530).
- **`adjusted_precision`** = `precision(expected_files ∪ should_touch_files, actual_files)`. GA = **0.510 (rank 1/7)**.

Insight: GA is **precise-but-narrow**. Best precision on the enlarged GT pool, but catches only 45% of structural blast radius.

### `should_touch_files` algorithm (`scripts/extract-seeds.ts:491-538`)

```
Phase A (static import reverse-lookup, per-lang grep @ baseCommit):
  python: "from <mod> import" | "import <mod>"
  go:     "<modRoot>/<dir>" (quoted)
  rust:   "use <crate>::<modPath>" | "use <crate>::{...<stem>"
  ts/js:  "from '.../stem'" | "require('.../stem')"

Phase B (co-change @ last 100 commits): coChange2 ≥2 hits, coChange3 ≥3 hits

Phase C: importers ∩ coChange2  (or coChange3 fallback if no importers)
         exclude expected_files, tests, build, non-GA-parseable. Cap 15.
```

### Empirical distribution on 30 dev tasks

- **11 tasks**: empty `should_touch` (no importers + no ≥3 co-change)
- **19 populated**, **9 at cap 15** (heavy fan-out clusters):
  - axum 4 tasks × 15 = `axum-core/extract/*`, `axum-extra/extract/*` — **trait impl cluster (M3)**
  - nest 3 tasks × 15 = `common/decorators/*`, `common/exceptions/*` — **framework DI/interface (M3+M4)**
  - django 2 tasks × 15 = cross-contrib (`postgres/`, `contenttypes/`, `admin/`) — **co-edit cross-module (M6-ish)**
- **Preact 4 tasks × 2-4 files** = `render.js`, `component.js`, `diff/children.js` co-change with framework core — **M6 framework-internal co-edit**

### Revised macro → metric mapping

Which macros feed which GT pool (drives which metric):

| Macro | expected_files (composite) | should_touch_files (blast_radius + adj_prec) | Notes |
|:-:|:-:|:-:|---|
| M1 folder-sibling | ✅ (fix-commit touched) | ❌ filtered out | Rust `mod private;` ≠ `use private::*`; siblings miss Phase A |
| M2 signature | ✅ | — | — |
| M3 interface/trait impl | ✅ (partial fix-touched) | ✅✅ **strong** | Impls `use trait::X` ✓ Phase A + co-change ✓ Phase B |
| M4 DI registration | — | ✅✅ **strong** | `@Module({providers:[X]})` imports X |
| M5 event/signal | — | ⚠️ partial | Depends on dispatch-lib import |
| M6 ORM/framework co-edit | — | ✅✅ **strong** | coChange3 fallback catches |
| M7 route | ✅ | ⚠️ partial | Route file imports handler |
| M8 component prop | — | ✅ **strong** | Parent imports child |
| M9 config | — | ❌ filtered out | Not GA-parseable |
| M10 test | expected_tests (separate) | — | Explicitly filtered |

**Key consequence**: M3/M4/M6/M8 lift **blast_radius + adj_prec** (LLM utility). M1/M2/M7/M10 lift **composite** (human-dev-60s metric).

### Phase 1 harness re-scope

Classify per-file hit into 3 pools:
- `hits_expected` — file ∈ expected_files ∪ expected_tests → **lifts composite**
- `hits_should_touch` — file ∈ should_touch_files → **lifts blast_radius + adj_prec**
- `hits_neither` — noise

Gate per harness:
- `(signal_expected + signal_should_touch) : noise ≥ 1:5`
- Report separately per pool so user can see which metric budget it attacks.

### Updated priority ranking

Given the reframe (composite is human-dev bias, adj_prec + blast_radius = LLM agent reality):

| Rank | Harness | composite lift | adj_prec/blast_radius lift | Cost |
|:-:|---|:-:|:-:|:-:|
| **1** | **H-M3 EXTENDS walk** | +~0.01-0.02 | **+~0.05 blast_radius** | low (edge exists) |
| 2 | H-M6 co-change wire (dormant infra) | ~0 | **+~0.04 blast_radius** | low-medium |
| 3 | H-M1 folder-sibling | +~0.01-0.02 | ~0 | lowest (fs) |
| 4 | H-M10 TESTED_BY densify | **+~0.04 test_recall** (in composite!) | ~0 | high (parser) |

**Rebalance**: if targeting LLM utility, M3 + M6 is the pair (both blast_radius lifters, infra exists). If targeting composite gate, M10 parser work needed.

**Combined strategy**: run H-M3 + H-M6 + H-M1 harnesses → report both metric impacts → let user decide whether composite 0.80 or blast_radius 0.75 is the real M2 gate post-empirical evidence.

## Caveats

1. **`m2_audit.rs` schema mismatch** (EXPECTED_SCHEMA_VERSION=3 vs GT v2). Live per-task GA output not re-run in this audit — predictions based on pipeline code audit, not empirical. **Harnesses in Phase 1 WILL run live** — that's when predictions get validated.

2. **Dev split = 30 tasks is small**. Test split (70 tasks) may redistribute macro densities. M6 ORM specifically may be higher there — revisit post-M2.

3. **GT noise**: axum-74eac39e (expand_field seed, test = `json_not_deserialize.stderr`) and axum-2e8a7e51 (`.stderr` compile-fail tests) have questionable GT. Could be mining artifacts. Consider excluding from harness denominator.

4. **Classification by judgment, not measurement**. Macro labels are my read — another reviewer may assign differently. The gap counts are rough. Use as directional signal, not precise.

## References

- `docs/IMPACT_MACRO_STRATEGY.md` — framing + macro definitions
- `EXPERIMENTS.md` — EXP history (M2-05 ✅ / M2-06 ❌ / M2-07 SKIP / M2-08 ❌)
- `m2-gate-plan.md` — per-EXP execution protocol
- `crates/ga-query/src/impact/` — pipeline source of truth
- `benches/uc-impact/ground-truth.json` — GT (schema v2, SHA-verified)
