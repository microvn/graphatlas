# M3 Gate — `architecture` on `gin`

**Rule:** Ha-import-edge

**Policy bias:** Ha-import-edge — primary metric: F1 on edge-pairs (Spearman rank correlation utility not yet wired; promoted from EXP follow-up in Phase 3). Marker-based module GT (kind=module) is tautological-by-design vs ga_architecture::discover_modules — tracked as DIAGNOSTIC only, never as the primary spec gate. Module = dir-basename of nearest ancestor with `__init__.py` / `Cargo.toml` / `package.json` marker (mirrors ga_architecture::dir_basename for definitional alignment per spec C2). Files outside any marked dir are dropped. Cross-module edge is `(module_of_importer, module_of_imported)`; self-edges excluded. Import target → owning module via root-prefix match (longest wins) on the import's parsed target_path; unresolved imports (external packages, relative paths) are dropped.

| Status | Retriever | Score | Spec target | p95 latency (ms) |
|---|---|---|---|---|
| **TAUTOLOGY-SUSPECT** | ga | 1.000 | 0.600 | 70 |

### Secondary metrics

**ga**:
- `actual_edge_count` = 15.000
- `edge_f1` = 1.000
- `edge_f2` = 1.000
- `edge_precision` = 1.000
- `edge_recall` = 1.000
- `expected_edge_count` = 15.000
- `false_negatives` = 0.000
- `false_positives` = 0.000
- `shared_edge_count` = 15.000
- `spearman_defined` = 1.000
- `true_positives` = 15.000

**SPEC GATE: 1 pass, 0 fail (target: all pass)**
