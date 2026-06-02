# M3 Gate — `architecture` on `tokio`

**Rule:** Ha-import-edge

**Policy bias:** Ha-import-edge — primary metric: F1 on edge-pairs (Spearman rank correlation utility not yet wired; promoted from EXP follow-up in Phase 3). Marker-based module GT (kind=module) is tautological-by-design vs ga_architecture::discover_modules — tracked as DIAGNOSTIC only, never as the primary spec gate. Module = dir-basename of nearest ancestor with `__init__.py` / `Cargo.toml` / `package.json` marker (mirrors ga_architecture::dir_basename for definitional alignment per spec C2). Files outside any marked dir are dropped. Cross-module edge is `(module_of_importer, module_of_imported)`; self-edges excluded. Import target → owning module via root-prefix match (longest wins) on the import's parsed target_path; unresolved imports (external packages, relative paths) are dropped.

| Status | Retriever | Score | Spec target | p95 latency (ms) |
|---|---|---|---|---|
| PASS | ga | 0.667 | 0.600 | 217 |

### Secondary metrics

**ga**:
- `actual_edge_count` = 41.000
- `edge_f1` = 0.667
- `edge_f2` = 0.814
- `edge_precision` = 0.512
- `edge_recall` = 0.955
- `expected_edge_count` = 22.000
- `false_negatives` = 1.000
- `false_positives` = 20.000
- `shared_edge_count` = 21.000
- `spearman_defined` = 0.000
- `true_positives` = 21.000

**SPEC GATE: 1 pass, 0 fail (target: all pass)**
